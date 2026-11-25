// SPDX-License-Identifier: Apache-2.0
//! Three-Tier Context Compaction Engine:
//! - Tier 1 (Zero-Cost Micro-Compaction):
//!   1A) Prunes stale `Assistant.reasoning` beyond the last 2 assistant turns + deduplicates
//!       superseded `read_file` observations (files re-read or mutated later).
//!   1B) Shrinks older oversized tool payloads while preserving `output_id` & key scalars
//!       (`exit_code`, `path`, `error`) so spilled files stay inspectable via `tool_output`.
//! - Tier 2 (Structured Working-State Summary Compaction):
//!   Preserves Initial User Goal, deduplicated Files Read/Modified, Commands & Exit Codes,
//!   Active Errors, and Prior Summary Highlights.
//! - Tier 3 (Emergency Hard-Limit Guard):
//!   Ensures history NEVER knowingly exceeds `prompt_token_limit()`, even when recent tool
//!   results are huge or `history.len() <= 6`, plus `compact_history_force` for reactive 400 recovery.

use std::collections::BTreeSet;

use nola_types::{
    estimate_history_message_tokens, sha256_hex, ContextBudget, HistoryMessage, ToolModelPayload,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const RECENT_TOOL_KEEP_COUNT: usize = 6;
const RECENT_REASONING_KEEP_COUNT: usize = 2;
const MICRO_COMPACT_CHAR_THRESHOLD: usize = 1_200;
const COMPACTION_LOOP_MAX_CONSECUTIVE_LOW_YIELD: usize = 3;
const COMPACTION_LOOP_MIN_REDUCTION_PCT: usize = 15;

/// Source-message lineage recorded whenever history messages are folded into a summary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionSummaryMetadata {
    pub start_index: usize,
    pub end_index: usize,
    pub message_count: usize,
    pub source_sha256: String,
}

impl CompactionSummaryMetadata {
    pub fn from_slice(start_index: usize, slice: &[HistoryMessage]) -> Self {
        let serialized = serde_json::to_vec(slice).unwrap_or_default();
        Self {
            start_index,
            end_index: start_index.saturating_add(slice.len()),
            message_count: slice.len(),
            source_sha256: sha256_hex(&serialized),
        }
    }
}

/// Prevents infinite automatic compaction loops when repeated compactions fail to reclaim
/// at least 15% of the estimated token count (`consecutive_compactions >= 3`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompactionLoopBreaker {
    pub consecutive_compactions: usize,
    pub automatic_compaction_blocked: bool,
}

impl CompactionLoopBreaker {
    pub fn is_blocked(&self) -> bool {
        self.automatic_compaction_blocked
    }

    /// Record an automatic compaction attempt. Returns `true` if automatic compaction is now blocked.
    pub fn record_attempt(&mut self, tokens_before: usize, tokens_after: usize) -> bool {
        if tokens_before == 0 {
            return self.automatic_compaction_blocked;
        }
        let saved = tokens_before.saturating_sub(tokens_after);
        let reduced_enough =
            saved.saturating_mul(100) >= tokens_before.saturating_mul(COMPACTION_LOOP_MIN_REDUCTION_PCT);
        if reduced_enough {
            self.consecutive_compactions = 0;
        } else {
            self.consecutive_compactions = self.consecutive_compactions.saturating_add(1);
            if self.consecutive_compactions >= COMPACTION_LOOP_MAX_CONSECUTIVE_LOW_YIELD {
                self.automatic_compaction_blocked = true;
            }
        }
        self.automatic_compaction_blocked
    }

    pub fn reset(&mut self) {
        self.consecutive_compactions = 0;
        self.automatic_compaction_blocked = false;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactionReport {
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub micro_compacted_tools: usize,
    pub summary_compacted_messages: usize,
    pub lineage: Option<CompactionSummaryMetadata>,
}

impl CompactionReport {
    pub fn saved_tokens(&self) -> usize {
        self.tokens_before.saturating_sub(self.tokens_after)
    }
}

pub fn estimate_history_tokens(history: &[HistoryMessage]) -> usize {
    history.iter().map(estimate_history_message_tokens).sum()
}

/// Run Tier-1 -> Tier-2 -> Tier-3 compaction when estimated tokens exceed auto-compact trigger.
pub fn compact_history(
    history: &mut Vec<HistoryMessage>,
    budget: ContextBudget,
    fixed_overhead_tokens: usize,
    summarizer: Option<&dyn Fn(&[HistoryMessage]) -> Option<String>>,
) -> CompactionReport {
    compact_history_inner(history, budget, fixed_overhead_tokens, summarizer, false)
}

/// Force immediate compaction regardless of local token estimate (used for reactive HTTP 400
/// context-length-exceeded recovery when provider tokenizer counts higher than local heuristic).
pub fn compact_history_force(
    history: &mut Vec<HistoryMessage>,
    budget: ContextBudget,
    fixed_overhead_tokens: usize,
    summarizer: Option<&dyn Fn(&[HistoryMessage]) -> Option<String>>,
) -> CompactionReport {
    compact_history_inner(history, budget, fixed_overhead_tokens, summarizer, true)
}

fn compact_history_inner(
    history: &mut Vec<HistoryMessage>,
    budget: ContextBudget,
    fixed_overhead_tokens: usize,
    summarizer: Option<&dyn Fn(&[HistoryMessage]) -> Option<String>>,
    force: bool,
) -> CompactionReport {
    let tokens_before = estimate_history_tokens(history);
    let trigger = budget.history_budget(fixed_overhead_tokens, budget.auto_compact_threshold());
    let target = budget.history_budget(fixed_overhead_tokens, budget.compact_target_tokens());
    let hard_limit = budget.history_budget(fixed_overhead_tokens, budget.prompt_token_limit());

    if !force && tokens_before <= trigger {
        return CompactionReport {
            tokens_before,
            tokens_after: tokens_before,
            ..Default::default()
        };
    }

    let mut micro_compacted_tools = 0usize;

    // --- Tier 1A-1: Prune stale `Assistant.reasoning` beyond the last 2 assistant turns ---
    let assistant_indices: Vec<usize> = history
        .iter()
        .enumerate()
        .filter_map(|(i, m)| matches!(m, HistoryMessage::Assistant { .. }).then_some(i))
        .collect();
    let prune_asst_count = assistant_indices
        .len()
        .saturating_sub(RECENT_REASONING_KEEP_COUNT);
    for &idx in assistant_indices.iter().take(prune_asst_count) {
        if let HistoryMessage::Assistant {
            reasoning,
            continuation,
            ..
        } = &mut history[idx]
        {
            if reasoning.as_ref().is_some_and(|r| r.len() > 200) {
                *reasoning = None;
                strip_continuation_reasoning(continuation);
            } else if continuation.as_ref().is_some_and(has_continuation_reasoning) {
                strip_continuation_reasoning(continuation);
            }
        }
    }

    // --- Tier 1A-2: Deduplicate superseded `read_file` observations ---
    // If a file path is later re-read or mutated (write/edit/patch/delete), older read_file
    // payloads for that same path are stale and can be safely collapsed first!
    let mut seen_paths_later: BTreeSet<String> = BTreeSet::new();
    for msg in history.iter_mut().rev() {
        if let HistoryMessage::Tool {
            tool_name,
            payload,
            media,
            ..
        } = msg
        {
            let val = payload.to_model_value();
            let path = val
                .get("path")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            if matches!(
                tool_name.as_str(),
                "write_file" | "edit_file" | "multi_edit_file" | "delete_file"
            ) {
                if let Some(p) = path {
                    seen_paths_later.insert(p);
                }
            } else if tool_name == "read_file" {
                if let Some(p) = path {
                    if !seen_paths_later.insert(p.clone()) {
                        let serialized_len = payload.to_model_string().len();
                        if serialized_len > 300 {
                            *payload = ToolModelPayload::structured(
                                json!({
                                    "compacted": true,
                                    "reason": "superseded_by_newer_read_or_edit",
                                    "tool": "read_file",
                                    "path": p,
                                    "original_bytes": serialized_len,
                                }),
                                format!("Superseded earlier read of {p}"),
                            );
                            media.clear();
                            micro_compacted_tools += 1;
                        }
                    }
                }
            }
        }
    }

    // --- Tier 1B: Zero-cost Micro-Compaction on older oversized tool payloads ---
    let tool_indices: Vec<usize> = history
        .iter()
        .enumerate()
        .filter_map(|(i, m)| matches!(m, HistoryMessage::Tool { .. }).then_some(i))
        .collect();
    let keep_recent_tools = if force { 2 } else { RECENT_TOOL_KEEP_COUNT };
    let eligible_count = tool_indices.len().saturating_sub(keep_recent_tools);

    for &idx in tool_indices.iter().take(eligible_count) {
        if shrink_tool_message(&mut history[idx], MICRO_COMPACT_CHAR_THRESHOLD, 240) {
            micro_compacted_tools += 1;
        }
    }

    let after_tier1 = estimate_history_tokens(history);
    if !force && after_tier1 <= target {
        return CompactionReport {
            tokens_before,
            tokens_after: after_tier1,
            micro_compacted_tools,
            summary_compacted_messages: 0,
            lineage: None,
        };
    }

    // --- Tier 2: Structured Working-State Summary Compaction ---
    let mut summary_compacted_messages = 0usize;
    let mut lineage: Option<CompactionSummaryMetadata> = None;
    let keep_recent = if force { 2 } else { 6 }.min(history.len().saturating_sub(1));
    let mut split_idx = history.len().saturating_sub(keep_recent);
    // Avoid splitting between an Assistant tool_calls turn and its matching Tool results!
    while split_idx > 0 && matches!(history.get(split_idx), Some(HistoryMessage::Tool { .. })) {
        split_idx -= 1;
    }
    if split_idx >= 2 && !matches!(history.get(split_idx), Some(HistoryMessage::Tool { .. })) {
        let old_slice = &history[..split_idx];
        let meta = CompactionSummaryMetadata::from_slice(0, old_slice);
        let summary_text = summarizer
            .and_then(|f| f(old_slice))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| deterministic_ledger_summary(old_slice));

        let mut candidate = Vec::with_capacity(history.len() - split_idx + 1);
        candidate.push(HistoryMessage::User {
            content: format!(
                "<compacted_history_summary>\n[lineage: messages={} range={}..{} sha256={}]\n{summary_text}\n</compacted_history_summary>",
                meta.message_count, meta.start_index, meta.end_index, &meta.source_sha256[..16]
            ),
            media: Vec::new(),
        });
        candidate.extend(history[split_idx..].iter().cloned());

        let after_tier2 = estimate_history_tokens(&candidate);
        if after_tier2 < after_tier1 || (force && candidate.len() < history.len()) {
            *history = candidate;
            summary_compacted_messages = split_idx;
            lineage = Some(meta);
        }
    }

    // --- Tier 3: Emergency Hard-Limit Guard (NEVER knowingly exceed prompt_token_limit!) ---
    let mut current_tokens = estimate_history_tokens(history);
    if force || current_tokens > hard_limit {
        // 3A: Strip all remaining reasoning in history (and continuation reasoning items!)
        for msg in history.iter_mut() {
            if let HistoryMessage::Assistant {
                reasoning,
                continuation,
                ..
            } = msg
            {
                *reasoning = None;
                strip_continuation_reasoning(continuation);
            }
        }
        // 3B: Shrink even recent oversized tool payloads (preserving output_id, exit_code, path, head+tail)
        for msg in history.iter_mut() {
            if shrink_tool_message(msg, 800, 400) {
                micro_compacted_tools += 1;
            }
        }
        current_tokens = estimate_history_tokens(history);
    }

    // 3C: If STILL over hard_limit (e.g. massive user message or huge assistant turn), keep only
    // compacted summary + latest turn boundary!
    if current_tokens > hard_limit && history.len() > 2 {
        let mut cut = history.len().saturating_sub(2);
        while cut > 0 && matches!(history.get(cut), Some(HistoryMessage::Tool { .. })) {
            cut -= 1;
        }
        if cut >= 1 && !matches!(history.get(cut), Some(HistoryMessage::Tool { .. })) {
            let meta = CompactionSummaryMetadata::from_slice(0, &history[..cut]);
            let summary_text = deterministic_ledger_summary(&history[..cut]);
            let mut trimmed = Vec::with_capacity(history.len() - cut + 1);
            trimmed.push(HistoryMessage::User {
                content: format!(
                    "<compacted_history_summary>\n[lineage: messages={} range={}..{} sha256={}]\n{summary_text}\n</compacted_history_summary>",
                    meta.message_count, meta.start_index, meta.end_index, &meta.source_sha256[..16]
                ),
                media: Vec::new(),
            });
            trimmed.extend(history[cut..].iter().cloned());
            if estimate_history_tokens(&trimmed) < current_tokens {
                *history = trimmed;
                summary_compacted_messages = summary_compacted_messages.saturating_add(cut);
                lineage = Some(meta);
            }
        }
    }

    // 3D: Head+tail truncation fallback for single massive messages when STILL over hard_limit
    // or when force-compacting made no progress on an oversized message.
    current_tokens = estimate_history_tokens(history);
    if current_tokens > hard_limit || (force && current_tokens >= tokens_before && current_tokens > 256) {
        let target_tokens = if current_tokens > hard_limit {
            hard_limit.max(256)
        } else {
            (current_tokens / 2).max(256)
        };
        truncate_massive_messages(history, target_tokens);
    }

    let tokens_after = estimate_history_tokens(history);
    CompactionReport {
        tokens_before,
        tokens_after,
        micro_compacted_tools,
        summary_compacted_messages,
        lineage,
    }
}

fn has_continuation_reasoning(cont: &nola_types::ProviderContinuation) -> bool {
    cont.items.iter().any(|item| {
        let t = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let is_thought = item
            .get("thought")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        matches!(t, "thinking" | "redacted_thinking" | "reasoning") || is_thought
    })
}

fn strip_continuation_reasoning(continuation: &mut Option<nola_types::ProviderContinuation>) {
    if let Some(cont) = continuation {
        cont.items.retain(|item| {
            let t = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let is_thought = item
                .get("thought")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            !matches!(t, "thinking" | "redacted_thinking" | "reasoning") && !is_thought
        });
        if cont.items.is_empty() {
            *continuation = None;
        }
    }
}

fn head_tail_truncate(text: &str, max_chars: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars || max_chars < 64 {
        return None;
    }
    let keep_head = (max_chars * 2) / 3;
    let keep_tail = max_chars.saturating_sub(keep_head).max(16);
    let dropped = chars.len().saturating_sub(keep_head + keep_tail);
    let head: String = chars[..keep_head].iter().collect();
    let tail: String = chars[chars.len().saturating_sub(keep_tail)..].iter().collect();
    Some(format!(
        "{head}\n... [truncated {dropped} chars for context budget] ...\n{tail}"
    ))
}

fn truncate_massive_messages(history: &mut [HistoryMessage], target_tokens: usize) {
    if history.is_empty() {
        return;
    }
    let per_msg_token_budget = (target_tokens / history.len().max(1)).max(128);
    // Conservative char budget (assuming worst-case ~2 chars/token for mixed CJK/ASCII)
    let max_chars = per_msg_token_budget.saturating_mul(2).max(256);
    for msg in history.iter_mut() {
        if estimate_history_message_tokens(msg) <= per_msg_token_budget {
            continue;
        }
        match msg {
            HistoryMessage::User { content, media } => {
                if let Some(truncated) = head_tail_truncate(content, max_chars) {
                    *content = truncated;
                }
                if estimate_history_message_tokens(&HistoryMessage::User {
                    content: content.clone(),
                    media: media.clone(),
                }) > per_msg_token_budget
                {
                    media.clear();
                }
            }
            HistoryMessage::Assistant {
                content,
                reasoning,
                continuation,
                ..
            } => {
                *reasoning = None;
                *continuation = None;
                if let Some(truncated) = head_tail_truncate(content, max_chars) {
                    *content = truncated;
                }
            }
            HistoryMessage::Tool {
                tool_name,
                payload,
                media,
                ..
            } => {
                media.clear();
                let serialized = payload.to_model_string();
                if let Some(truncated) = head_tail_truncate(&serialized, max_chars) {
                    let summary: String = payload.ui_summary().chars().take(160).collect();
                    *payload = ToolModelPayload::structured(
                        json!({
                            "compacted": true,
                            "tool": tool_name,
                            "original_bytes": serialized.len(),
                            "preview": truncated
                        }),
                        summary,
                    );
                }
            }
        }
    }
}

/// Shrink an oversized `HistoryMessage::Tool` while preserving `output_id` and key scalars!
fn shrink_tool_message(
    msg: &mut HistoryMessage,
    char_threshold: usize,
    preview_chars: usize,
) -> bool {
    let HistoryMessage::Tool {
        tool_name,
        payload,
        media,
        ..
    } = msg
    else {
        return false;
    };
    let serialized = payload.to_model_string();
    if serialized.len() <= char_threshold && media.is_empty() {
        return false;
    }
    let val = payload.to_model_value();
    let summary: String = payload.ui_summary().chars().take(160).collect();
    let preview: String = serialized.chars().take(preview_chars).collect();
    let mut compact = serde_json::Map::new();
    compact.insert("compacted".into(), Value::Bool(true));
    compact.insert("tool".into(), Value::String(tool_name.clone()));
    compact.insert("original_bytes".into(), json!(serialized.len()));
    // Preserve critical references & scalar outcomes across micro-compaction!
    if let Some(obj) = val.as_object() {
        for key in ["output_id", "path", "exit_code", "error", "count", "status"] {
            if let Some(v) = obj.get(key) {
                if !v.is_object() && !v.is_array() {
                    compact.insert(key.to_string(), v.clone());
                }
            }
        }
    }
    compact.insert("summary".into(), Value::String(summary.clone()));
    compact.insert("preview".into(), Value::String(preview));
    *payload = ToolModelPayload::structured(Value::Object(compact), summary);
    media.clear();
    true
}

fn deterministic_ledger_summary(slice: &[HistoryMessage]) -> String {
    let mut initial_goal: Option<String> = None;
    let mut prior_highlights: Vec<String> = Vec::new();
    let mut files_touched: BTreeSet<String> = BTreeSet::new();
    let mut files_read: BTreeSet<String> = BTreeSet::new();
    let mut commands: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    for (idx, msg) in slice.iter().enumerate() {
        match msg {
            HistoryMessage::User { content, .. } => {
                if let Some(inner) = content
                    .trim()
                    .strip_prefix("<compacted_history_summary>")
                    .and_then(|s| s.strip_suffix("</compacted_history_summary>"))
                {
                    for l in inner.lines().map(str::trim).filter(|l| !l.is_empty()).take(14) {
                        prior_highlights.push(l.chars().take(180).collect());
                    }
                } else if idx == 0 && initial_goal.is_none() {
                    let goal: String = content
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(500)
                        .collect();
                    initial_goal = Some(goal);
                } else {
                    let snippet: String = content
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(200)
                        .collect();
                    if !snippet.is_empty() {
                        notes.push(format!("User: {snippet}"));
                    }
                }
            }
            HistoryMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                for tc in tool_calls {
                    if tc.name == "bash" {
                        if let Some(cmd) = tc.arguments.get("command").and_then(|v| v.as_str()) {
                            let short: String = cmd.lines().next().unwrap_or("").chars().take(100).collect();
                            commands.push(short);
                        }
                    } else if matches!(
                        tc.name.as_str(),
                        "write_file" | "edit_file" | "multi_edit_file" | "delete_file"
                    ) {
                        if let Some(p) = tc.arguments.get("path").and_then(|v| v.as_str()) {
                            files_touched.insert(p.to_string());
                        }
                    } else if tc.name == "read_file" {
                        if let Some(p) = tc.arguments.get("path").and_then(|v| v.as_str()) {
                            files_read.insert(p.to_string());
                        }
                    }
                }
                if tool_calls.is_empty() && !content.trim().is_empty() {
                    let first: String = content.lines().next().unwrap_or("").chars().take(160).collect();
                    notes.push(format!("Assistant: {first}"));
                }
            }
            HistoryMessage::Tool {
                tool_name,
                payload,
                is_error,
                ..
            } => {
                let val = payload.to_model_value();
                if *is_error {
                    let summary: String = payload.ui_summary().chars().take(140).collect();
                    errors.push(format!("{tool_name}: {summary}"));
                } else if let Some(oid) = val.get("output_id").and_then(|v| v.as_str()) {
                    notes.push(format!("Spilled {tool_name} output_id={oid}"));
                }
            }
        }
    }

    let mut sections = vec![format!("Compacted {} earlier messages:", slice.len())];
    if let Some(goal) = initial_goal {
        sections.push(format!("Goal: {goal}"));
    }
    if !prior_highlights.is_empty() {
        sections.push("Prior Summary:".into());
        for h in prior_highlights.into_iter().take(12) {
            sections.push(format!("  {h}"));
        }
    }
    if !files_touched.is_empty() {
        let list: Vec<&str> = files_touched.iter().take(20).map(String::as_str).collect();
        sections.push(format!("Files Modified: {}", list.join(", ")));
    }
    if !files_read.is_empty() {
        let list: Vec<&str> = files_read.iter().take(20).map(String::as_str).collect();
        sections.push(format!("Files Read: {}", list.join(", ")));
    }
    if !commands.is_empty() {
        let recent_cmds: Vec<&str> = commands.iter().rev().take(8).rev().map(String::as_str).collect();
        sections.push(format!("Recent Commands: {}", recent_cmds.join(" ; ")));
    }
    if !errors.is_empty() {
        let recent_errs: Vec<&str> = errors.iter().rev().take(5).rev().map(String::as_str).collect();
        sections.push(format!("Errors Seen: {}", recent_errs.join(" | ")));
    }
    for n in notes.into_iter().rev().take(6).rev() {
        sections.push(format!("- {n}"));
    }
    sections.join("\n")
}
