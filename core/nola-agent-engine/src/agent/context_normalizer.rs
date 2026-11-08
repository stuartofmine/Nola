// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Normalizes request-only model history before provider encoding.
// - Boundary: Never mutates the canonical transcript or durable ownership.

use std::collections::{HashMap, HashSet};

use serde_json::json;

use super::protocol::{AgentItem, CallId, ItemId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextNormalizationStats {
    pub inserted_tool_results: usize,
    pub dropped_orphan_tool_results: usize,
    /// Results that arrived out of order are projected back into the order of
    /// their corresponding calls. Host completion timing must not perturb the
    /// next provider request or its cache identity.
    pub reordered_tool_results: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedModelContext {
    pub items: Vec<AgentItem>,
    pub stats: ContextNormalizationStats,
}

/// Enforces the canonical call/result invariant on an ephemeral model request.
/// Interrupted calls receive a deterministic synthetic error result; orphaned
/// results are omitted. The lossless transcript remains untouched.
pub fn normalize_model_context(items: Vec<AgentItem>) -> NormalizedModelContext {
    let mut output = Vec::with_capacity(items.len());
    let mut open_calls = Vec::<(CallId, String)>::new();
    let mut pending_results = HashMap::<String, (usize, AgentItem)>::new();
    let mut result_sequence = 0usize;
    let mut stats = ContextNormalizationStats::default();
    let mut used_item_ids = items
        .iter()
        .map(|item| item.id().as_str().to_string())
        .collect::<HashSet<_>>();

    for item in items {
        let starts_new_boundary = matches!(
            item,
            AgentItem::UiAction { .. } | AgentItem::Message { .. } | AgentItem::Compaction { .. }
        );
        if starts_new_boundary {
            flush_open_calls(
                &mut output,
                &mut open_calls,
                &mut pending_results,
                &mut used_item_ids,
                &mut stats,
            );
        }
        match &item {
            AgentItem::ToolCall { call_id, name, .. } => {
                if open_calls.iter().any(|(candidate, _)| candidate == call_id) {
                    flush_open_calls(
                        &mut output,
                        &mut open_calls,
                        &mut pending_results,
                        &mut used_item_ids,
                        &mut stats,
                    );
                }
                open_calls.push((call_id.clone(), name.clone()));
                output.push(item);
            }
            AgentItem::ToolResult { call_id, .. } => {
                if open_calls.iter().any(|(candidate, _)| candidate == call_id) {
                    pending_results.insert(call_id.as_str().to_string(), (result_sequence, item));
                    result_sequence = result_sequence.saturating_add(1);
                    // A settled native batch is a causal boundary. Waiting
                    // for another prose message merges every tool-only model
                    // decision into one giant call list, moving the evidence
                    // *after* decisions that depended on it.
                    if pending_results.len() == open_calls.len() {
                        flush_open_calls(
                            &mut output,
                            &mut open_calls,
                            &mut pending_results,
                            &mut used_item_ids,
                            &mut stats,
                        );
                    }
                } else {
                    stats.dropped_orphan_tool_results =
                        stats.dropped_orphan_tool_results.saturating_add(1);
                }
            }
            _ => output.push(item),
        }
    }
    flush_open_calls(
        &mut output,
        &mut open_calls,
        &mut pending_results,
        &mut used_item_ids,
        &mut stats,
    );
    NormalizedModelContext {
        items: output,
        stats,
    }
}

fn flush_open_calls(
    output: &mut Vec<AgentItem>,
    open_calls: &mut Vec<(CallId, String)>,
    pending_results: &mut HashMap<String, (usize, AgentItem)>,
    used_item_ids: &mut HashSet<String>,
    stats: &mut ContextNormalizationStats,
) {
    let mut last_arrival = None;
    for (call_id, name) in open_calls.drain(..) {
        if let Some((arrival, item)) = pending_results.remove(call_id.as_str()) {
            if last_arrival.is_some_and(|previous| arrival < previous) {
                stats.reordered_tool_results = stats.reordered_tool_results.saturating_add(1);
            }
            last_arrival = Some(arrival);
            output.push(item);
            continue;
        }
        let base = format!("nola:step-normalized-tool-result:{}", call_id.as_str());
        let mut item_id = base.clone();
        let mut suffix = 1usize;
        while !used_item_ids.insert(item_id.clone()) {
            item_id = format!("{base}:{suffix}");
            suffix = suffix.saturating_add(1);
        }
        output.push(AgentItem::ToolResult {
            id: ItemId::from_string(item_id),
            call_id,
            name,
            output: json!({
                "code": "tool_execution_interrupted",
                "message": "Tool execution was interrupted before a terminal result was recorded."
            }),
            is_error: true,
            full_output_ref: None,
            artifacts: Vec::new(),
        });
        stats.inserted_tool_results = stats.inserted_tool_results.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::protocol::AgentRole;

    fn call(item: &str, call: &str) -> AgentItem {
        AgentItem::ToolCall {
            id: ItemId::from_string(item.to_string()),
            call_id: CallId::from_string(call.to_string()),
            name: "read".to_string(),
            arguments: json!({}),
        }
    }

    fn result(item: &str, call: &str) -> AgentItem {
        AgentItem::ToolResult {
            id: ItemId::from_string(item.to_string()),
            call_id: CallId::from_string(call.to_string()),
            name: "read".to_string(),
            output: json!({"ok": true}),
            is_error: false,
            full_output_ref: None,
            artifacts: Vec::new(),
        }
    }

    #[test]
    fn fills_interrupted_calls_before_the_next_message() {
        let normalized = normalize_model_context(vec![
            call("call-item", "call-1"),
            AgentItem::Message {
                id: ItemId::from_string("next".to_string()),
                role: AgentRole::User,
                content: "continue".to_string(),
            },
        ]);
        assert_eq!(normalized.stats.inserted_tool_results, 1);
        assert!(matches!(
            normalized.items[1],
            AgentItem::ToolResult { is_error: true, .. }
        ));
        assert_eq!(normalized.items[2].id().as_str(), "next");
    }

    #[test]
    fn drops_orphans_and_normalizes_parallel_results_to_call_order() {
        let normalized = normalize_model_context(vec![
            result("orphan", "missing"),
            call("a", "call-a"),
            call("b", "call-b"),
            result("result-b", "call-b"),
            result("result-a", "call-a"),
        ]);
        assert_eq!(normalized.stats.dropped_orphan_tool_results, 1);
        assert_eq!(normalized.stats.inserted_tool_results, 0);
        assert_eq!(normalized.items.len(), 4);
        assert_eq!(normalized.stats.reordered_tool_results, 1);
        assert_eq!(normalized.items[2].id().as_str(), "result-a");
        assert_eq!(normalized.items[3].id().as_str(), "result-b");
    }

    #[test]
    fn successive_tool_only_decisions_keep_their_evidence_boundaries() {
        let input = vec![
            call("discovery", "discover"),
            result("catalog", "discover"),
            call("a", "read-a"),
            call("b", "read-b"),
            result("result-b", "read-b"),
            result("result-a", "read-a"),
            call("dependent", "inspect-result"),
            result("final", "inspect-result"),
        ];
        let normalized = normalize_model_context(input);
        assert_eq!(normalized.stats.inserted_tool_results, 0);
        assert_eq!(normalized.stats.dropped_orphan_tool_results, 0);
        assert_eq!(
            normalized
                .items
                .iter()
                .map(|item| item.id().as_str())
                .collect::<Vec<_>>(),
            [
                "discovery",
                "catalog",
                "a",
                "b",
                "result-a",
                "result-b",
                "dependent",
                "final"
            ]
        );
        assert_eq!(
            normalize_model_context(normalized.items.clone()).items,
            normalized.items
        );
    }
}
