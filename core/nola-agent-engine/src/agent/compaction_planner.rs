// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Selects loss-minimizing, transaction-safe context compaction boundaries.
// - Boundary: Kernel selects body-free indices; Core retains and partitions original items.

use nola_domain::compaction_plan::{CompactionItemFact, CompactionItemKind};
use std::collections::BTreeSet;

use super::protocol::{AgentItem, AgentRole, ItemId};

#[derive(Debug, Clone, Default)]
pub struct CompactionPlan {
    pub items_to_summarize: Vec<AgentItem>,
    pub kept_items: Vec<AgentItem>,
}

/// A recovery-only model projection for one atomic source unit which cannot fit
/// in a provider request by itself. The source items are retained separately so
/// the caller can fingerprint and locate the exact canonical transcript bodies.
#[derive(Debug, Clone)]
pub(crate) struct OversizedItemRecovery {
    pub source_items: Vec<AgentItem>,
    pub model_items: Vec<AgentItem>,
}

impl CompactionPlan {
    /// The target is a soft working-set goal; the prompt limit is a hard
    /// ceiling. Preserve the current round even when it exceeds the soft goal.
    pub fn summary_token_budget(
        &self,
        target_tokens: usize,
        prompt_limit: usize,
        handoff: bool,
    ) -> Result<usize, String> {
        let window_budget = nola_kernel::compaction_plan::summary_token_budget(
            self.kept_items.iter().map(AgentItem::estimated_tokens),
            target_tokens,
            prompt_limit,
        )
        .map_err(str::to_string)?;
        let source_tokens = self.items_to_summarize.iter()
            .fold(0usize, |total, item| total.saturating_add(item.estimated_tokens()));
        // Reserve summary framing and require a real reduction. Available
        // window space is not permission to expand a tiny archive.
        // A handoff may already consist mostly of concise checkpoints. Do not
        // demand another 2x reduction of those facts just because immutable
        // current input makes the complete-request 50% target unattainable.
        // Five tokens reserve framing plus a strictly positive reduction.
        let savings_budget = if handoff { source_tokens.saturating_sub(5) }
            else { source_tokens.saturating_sub(4) / 2 };
        if savings_budget == 0 {
            return Err("compaction archive is too small to summarize profitably".into());
        }
        Ok(window_budget.min(savings_budget))
    }
}

pub use nola_kernel::compaction_maintenance::{COMPACTION_EVIDENCE_CONTRACT, with_compaction_evidence_contract, compaction_instructions};

/// Keeps system/developer instructions, the latest user request, and a tail
/// of complete tool transactions, including within one long user turn.
///
/// Pending call batches cannot be split just because a large result crosses
/// the token target. Canonical transcript bodies are never rewritten here.
pub fn plan_compaction(items: &[AgentItem], target_tokens: usize) -> CompactionPlan {
    let summaries = items.iter().filter(|item| matches!(item, AgentItem::Compaction { .. })).collect::<Vec<_>>();
    let summary_tokens = summaries.iter().fold(0usize, |sum,item| sum.saturating_add(item.estimated_tokens()));
    // Low-frequency summary-layer maintenance. Do not mix this with a fresh
    // raw-history reduction: its source lineage must remain explicit.
    if summaries.len() >= 2 && summary_tokens > target_tokens.saturating_div(4).max(1024) {
        return CompactionPlan {
            items_to_summarize: summaries.into_iter().cloned().collect(),
            kept_items: items.iter().filter(|item| !matches!(item, AgentItem::Compaction { .. })).cloned().collect(),
        };
    }
    let facts = compaction_facts(items, false);
    let plan = nola_kernel::compaction_plan::plan_compaction(&facts, target_tokens);
    CompactionPlan {
        items_to_summarize: plan
            .summarize_indices
            .into_iter()
            .map(|i| items[i].clone())
            .collect(),
        kept_items: plan
            .kept_indices
            .into_iter()
            .map(|i| items[i].clone())
            .collect(),
    }
}

fn compaction_facts(items: &[AgentItem], handoff: bool) -> Vec<CompactionItemFact> {
    items
        .iter()
        .map(|item| CompactionItemFact {
            estimated_tokens: item.estimated_tokens(),
            kind: match item {
                AgentItem::Message {
                    role: AgentRole::System | AgentRole::Developer,
                    ..
                } => CompactionItemKind::PersistentInstruction,
                // Retention classification only: summaries remain historical
                // data, never system instructions or permission grants.
                AgentItem::Compaction { .. } if !handoff => CompactionItemKind::PersistentInstruction,
                AgentItem::Attachment { .. } if handoff => CompactionItemKind::PersistentInstruction,
                AgentItem::UiAction { .. } | AgentItem::Message {
                    role: AgentRole::User, ..
                } => CompactionItemKind::UserMessage,
                AgentItem::ToolCall { call_id, .. } => CompactionItemKind::ToolCall {
                    call_id: call_id.0.clone(),
                },
                AgentItem::ToolResult { call_id, .. } => CompactionItemKind::ToolResult {
                    call_id: call_id.0.clone(),
                },
                _ => CompactionItemKind::Other,
            },
        })
        .collect::<Vec<_>>()
}

pub(crate) fn plan_context_handoff(items: &[AgentItem]) -> CompactionPlan {
    let indices = nola_kernel::compaction_plan::plan_context_handoff(&compaction_facts(items, true));
    CompactionPlan {
        items_to_summarize: indices.summarize_indices.into_iter().map(|i| items[i].clone()).collect(),
        kept_items: indices.kept_indices.into_iter().map(|i| items[i].clone()).collect(),
    }
}

/// Builds a bounded provider view without changing any canonical item.
///
/// Tool calls and results are selected and projected as one transaction. Raw
/// excerpts keep their original protocol role; they are never elevated into a
/// synthetic system/compaction message.
pub(crate) fn project_oversized_atomic_item(
    items: &[AgentItem],
    target_tokens: usize,
) -> Option<OversizedItemRecovery> {
    let oversized_index = items.iter().position(|item| {
        !matches!(item, AgentItem::Attachment { .. }) && item.estimated_tokens() > target_tokens
    });
    if oversized_index.is_none() {
        return project_oversized_active_round(items, target_tokens);
    }
    let oversized_index = oversized_index?;
    let source_indices = match &items[oversized_index] {
        AgentItem::ToolCall { call_id, .. } | AgentItem::ToolResult { call_id, .. } => items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                AgentItem::ToolCall {
                    call_id: candidate, ..
                }
                | AgentItem::ToolResult {
                    call_id: candidate, ..
                } if candidate == call_id => Some(index),
                _ => None,
            })
            .collect::<Vec<_>>(),
        _ => vec![oversized_index],
    };
    let kept_tokens = items
        .iter()
        .enumerate()
        .filter(|(index, _)| !source_indices.contains(index))
        .fold(0usize, |total, (_, item)| {
            total.saturating_add(item.estimated_tokens())
        });
    // Leave explicit room for the source checkpoint which the runtime adds.
    let projection_token_budget = target_tokens
        .saturating_sub(256)
        .saturating_sub(kept_tokens);
    let total_char_budget = projection_token_budget.saturating_mul(2).min(16_000);
    if total_char_budget < 512 {
        return None;
    }
    let mut per_item_budget = total_char_budget
        .saturating_sub(256)
        .checked_div(source_indices.len().max(1))
        .unwrap_or(0)
        .max(128);
    let source_items = source_indices
        .iter()
        .map(|index| items[*index].clone())
        .collect::<Vec<_>>();
    let first_source_index = *source_indices.iter().min()?;
    let source_index_set = source_indices.into_iter().collect::<BTreeSet<_>>();
    let original_tokens = items.iter().fold(0usize, |total, item| {
        total.saturating_add(item.estimated_tokens())
    });
    loop {
        let projected_items = source_items
            .iter()
            .map(|item| bounded_source_projection(item, per_item_budget))
            .collect::<Vec<_>>();
        let mut model_items = Vec::with_capacity(items.len() + projected_items.len());
        for (index, item) in items.iter().enumerate() {
            if index == first_source_index {
                model_items.extend(projected_items.iter().cloned());
            }
            if !source_index_set.contains(&index) {
                model_items.push(item.clone());
            }
        }
        let projected_tokens = model_items.iter().fold(0usize, |total, item| {
            total.saturating_add(item.estimated_tokens())
        });
        if projected_tokens <= projection_token_budget && projected_tokens < original_tokens {
            return Some(OversizedItemRecovery {
                source_items,
                model_items,
            });
        }
        if per_item_budget <= 128 {
            return None;
        }
        per_item_budget = (per_item_budget / 2).max(128);
    }
}

/// Provider overflow can be caused by the sum of one active round even when
/// every individual item fits. If no useful summarizable prefix is available,
/// recovery retains the current user
/// request, persistent instructions, incomplete tool transactions, and the
/// newest completed evidence that fits. Older completed evidence remains exact
/// in the canonical transcript and is represented by the deterministic
/// compaction checkpoint created by the caller.
fn project_oversized_active_round(
    items: &[AgentItem],
    target_tokens: usize,
) -> Option<OversizedItemRecovery> {
    let projection_token_budget = target_tokens.saturating_sub(256);
    if projection_token_budget == 0 {
        return None;
    }
    let original_tokens = items.iter().fold(0usize, |total, item| {
        total.saturating_add(item.estimated_tokens())
    });
    if original_tokens <= projection_token_budget {
        return None;
    }

    let newest_user = items.iter().rposition(|item| {
        item.is_user_input()
    });
    let mut calls = std::collections::BTreeMap::<String, Vec<usize>>::new();
    let mut results = std::collections::BTreeMap::<String, Vec<usize>>::new();
    for (index, item) in items.iter().enumerate() {
        match item {
            AgentItem::ToolCall { call_id, .. } => {
                calls.entry(call_id.0.clone()).or_default().push(index);
            }
            AgentItem::ToolResult { call_id, .. } => {
                results.entry(call_id.0.clone()).or_default().push(index);
            }
            _ => {}
        }
    }

    let mut retained = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let persistent = matches!(
            item,
            AgentItem::Message {
                role: AgentRole::System | AgentRole::Developer,
                ..
            }
        );
        let current_user = newest_user == Some(index);
        let incomplete_tool = match item {
            AgentItem::ToolCall { call_id, .. } | AgentItem::ToolResult { call_id, .. } => {
                !calls.contains_key(call_id.0.as_str()) || !results.contains_key(call_id.0.as_str())
            }
            _ => false,
        };
        if persistent || current_user || incomplete_tool {
            retained.insert(index);
        }
    }
    let mut retained_tokens = retained.iter().fold(0usize, |total, index| {
        total.saturating_add(items[*index].estimated_tokens())
    });
    if retained_tokens > projection_token_budget {
        return None;
    }

    let mut units = Vec::<Vec<usize>>::new();
    let mut assigned = retained.clone();
    for (call_id, call_indices) in &calls {
        let Some(result_indices) = results.get(call_id) else {
            continue;
        };
        let mut unit = call_indices.clone();
        unit.extend(result_indices.iter().copied());
        unit.sort_unstable();
        unit.dedup();
        if unit.iter().all(|index| !assigned.contains(index)) {
            assigned.extend(unit.iter().copied());
            units.push(unit);
        }
    }
    for index in 0..items.len() {
        if assigned.insert(index) {
            units.push(vec![index]);
        }
    }
    units.sort_by_key(|unit| unit.iter().copied().max().unwrap_or(0));
    for unit in units.into_iter().rev() {
        let unit_tokens = unit.iter().fold(0usize, |total, index| {
            total.saturating_add(items[*index].estimated_tokens())
        });
        if retained_tokens.saturating_add(unit_tokens) <= projection_token_budget {
            retained.extend(unit);
            retained_tokens = retained_tokens.saturating_add(unit_tokens);
        }
    }

    let source_items = items
        .iter()
        .enumerate()
        .filter(|(index, _)| !retained.contains(index))
        .map(|(_, item)| item.clone())
        .collect::<Vec<_>>();
    if source_items.is_empty() {
        return None;
    }
    let model_items = items
        .iter()
        .enumerate()
        .filter(|(index, _)| retained.contains(index))
        .map(|(_, item)| item.clone())
        .collect::<Vec<_>>();
    let projected_tokens = model_items.iter().fold(0usize, |total, item| {
        total.saturating_add(item.estimated_tokens())
    });
    (projected_tokens <= projection_token_budget && projected_tokens < original_tokens).then_some(
        OversizedItemRecovery {
            source_items,
            model_items,
        },
    )
}

fn bounded_source_projection(item: &AgentItem, char_budget: usize) -> AgentItem {
    let marker = |body: &str, id: &ItemId| {
        let prefix = format!(
            "[Bounded provider recovery view. Exact canonical source item: {}]\n",
            id.as_str()
        );
        format!(
            "{prefix}{}",
            bounded_head_tail(body, char_budget.saturating_sub(prefix.len()))
        )
    };
    match item {
        AgentItem::UiAction { id, action } => AgentItem::user_message(id.clone(), marker(&action.model_envelope().to_string(), id)),
        AgentItem::Message { id, role, content } => AgentItem::Message {
            id: id.clone(),
            role: *role,
            content: marker(content, id),
        },
        AgentItem::Reasoning {
            id,
            summary,
            content,
            provider_continuation,
        } => AgentItem::Reasoning {
            id: id.clone(),
            summary: bounded_head_tail(summary, char_budget / 4),
            content: marker(content, id),
            provider_continuation: provider_continuation.clone(),
        },
        AgentItem::ToolCall {
            id,
            call_id,
            name,
            arguments,
        } => AgentItem::ToolCall {
            id: id.clone(),
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: serde_json::json!({
                "bounded_provider_recovery": true,
                "canonical_source_item_id": id.as_str(),
                "arguments_preview": bounded_head_tail(&arguments.to_string(), char_budget),
            }),
        },
        AgentItem::ToolResult {
            id,
            call_id,
            name,
            output,
            is_error,
            full_output_ref,
            artifacts,
        } => AgentItem::ToolResult {
            id: id.clone(),
            call_id: call_id.clone(),
            name: name.clone(),
            output: serde_json::json!({
                "bounded_provider_recovery": true,
                "canonical_source_item_id": id.as_str(),
                "output_preview": bounded_head_tail(&output.to_string(), char_budget),
                "full_output_ref": full_output_ref,
            }),
            is_error: *is_error,
            full_output_ref: full_output_ref.clone(),
            artifacts: artifacts.clone(),
        },
        AgentItem::Compaction {
            id,
            summary,
            replaced_item_count,
            source_item_ids,
            source_fingerprint,
            original_token_count,
        } => AgentItem::Compaction {
            id: id.clone(),
            summary: marker(summary, id),
            replaced_item_count: *replaced_item_count,
            source_item_ids: source_item_ids.clone(),
            source_fingerprint: source_fingerprint.clone(),
            original_token_count: *original_token_count,
        },
        AgentItem::Attachment { .. } => item.clone(),
    }
}

fn bounded_head_tail(value: &str, char_budget: usize) -> String {
    let count = value.chars().count();
    if count <= char_budget {
        return value.to_string();
    }
    let separator = "\n[… canonical middle omitted from provider view …]\n";
    let content_budget = char_budget.saturating_sub(separator.chars().count());
    let head_count = content_budget / 2;
    let tail_count = content_budget.saturating_sub(head_count);
    let head = value.chars().take(head_count).collect::<String>();
    let mut tail = value.chars().rev().take(tail_count).collect::<Vec<_>>();
    tail.reverse();
    format!("{head}{separator}{}", tail.into_iter().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{CallId, ItemId};
    use serde_json::json;
    use std::collections::BTreeSet;

    #[test]
    fn maintenance_contract_preserves_evidence_without_restoring_runtime_authority() {
        let instructions = compaction_instructions(1024);
        assert!(instructions.contains("process exit code 0"));
        assert!(instructions.contains("same requirement and artifact version"));
        assert!(instructions.contains("not runtime authority"));
        assert!(instructions.contains("hypotheses"));
        assert!(instructions.contains("4096 Unicode characters"));
        assert!(instructions.contains("Omit empty headings"));
        assert!(!instructions.contains("exactly this section order"));
        assert!(!instructions.contains("requested deliverable verbatim"));
        assert_eq!(with_compaction_evidence_contract(&instructions), instructions);
        assert!(with_compaction_evidence_contract("Continue checkpoint")
            .ends_with(COMPACTION_EVIDENCE_CONTRACT));
    }

    #[test]
    fn summary_layer_consolidation_does_not_resummarize_raw_tail() {
        let mut items = (0..3).map(|index| AgentItem::Compaction {
            id: ItemId::from_string(format!("summary-{index}")), summary: "evidence ".repeat(1000),
            replaced_item_count: 2, source_item_ids: vec![ItemId::from_string(format!("source-{index}"))],
            source_fingerprint: format!("fingerprint-{index}"), original_token_count: 10000,
        }).collect::<Vec<_>>();
        let tail = AgentItem::Message { id: ItemId::from_string("latest-user".into()),
            role: AgentRole::User, content: "Continue the unfinished task".into() };
        items.push(tail.clone());
        let plan = plan_compaction(&items, 8000);
        assert_eq!(plan.items_to_summarize, items[..3]);
        assert_eq!(plan.kept_items, vec![tail]);
    }

    #[test]
    fn summary_budget_is_bounded_by_source_not_free_window_space() {
        let plan = CompactionPlan {
            items_to_summarize: vec![AgentItem::Message {
                id: ItemId::from_string("old-greeting".into()),
                role: AgentRole::Assistant,
                content: "x".repeat(60),
            }],
            kept_items: Vec::new(),
        };
        let source = plan.items_to_summarize[0].estimated_tokens();
        let budget = plan.summary_token_budget(57_600, 128_000, false).unwrap();
        assert!(budget + 4 < source);
        assert!(budget < 20);
    }

    #[test]
    fn oversized_active_round_is_trimmed_without_splitting_tool_pairs() {
        let mut items = vec![
            AgentItem::Message {
                id: ItemId::from_string("system".to_string()),
                role: AgentRole::System,
                content: "Follow the repository instructions.".to_string(),
            },
            AgentItem::Message {
                id: ItemId::from_string("user".to_string()),
                role: AgentRole::User,
                content: "Inspect the repository and fix the issue.".to_string(),
            },
        ];
        for index in 0..24 {
            let call_id = CallId::from_string(format!("call-{index}"));
            items.push(AgentItem::ToolCall {
                id: ItemId::from_string(format!("tool-call-{index}")),
                call_id: call_id.clone(),
                name: "read_file".to_string(),
                arguments: json!({"path": format!("src/file-{index}.rs")}),
            });
            items.push(AgentItem::ToolResult {
                id: ItemId::from_string(format!("tool-result-{index}")),
                call_id,
                name: "read_file".to_string(),
                output: json!({"content": "x".repeat(240)}),
                is_error: false,
                full_output_ref: None,
                artifacts: Vec::new(),
            });
        }

        let plan = plan_compaction(&items, 600);
        assert!(!plan.items_to_summarize.is_empty());
        assert!(plan.kept_items.iter().map(AgentItem::estimated_tokens).sum::<usize>() <= 300);
        assert!(plan.kept_items.iter().any(|item| matches!(
            item,
            AgentItem::Message {
                role: AgentRole::System,
                ..
            }
        )));
        assert!(plan.kept_items.iter().any(|item| item.is_user_input()));

        let kept_calls = plan
            .kept_items
            .iter()
            .filter_map(|item| match item {
                AgentItem::ToolCall { call_id, .. } => Some(call_id.0.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let kept_results = plan
            .kept_items
            .iter()
            .filter_map(|item| match item {
                AgentItem::ToolResult { call_id, .. } => Some(call_id.0.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(kept_calls, kept_results);
        assert!(kept_calls.contains("call-23"));
        assert!(!kept_calls.contains("call-0"));
    }

    #[test]
    fn newest_round_is_never_summarized_when_it_exceeds_the_budget() {
        let current_request = "current request ".repeat(800);
        let items = vec![
            AgentItem::Message {
                id: ItemId::from_string("old-user".to_string()),
                role: AgentRole::User,
                content: "Remember the old fact.".to_string(),
            },
            AgentItem::Message {
                id: ItemId::from_string("old-assistant".to_string()),
                role: AgentRole::Assistant,
                content: "Stored.".to_string(),
            },
            AgentItem::Message {
                id: ItemId::from_string("current-user".to_string()),
                role: AgentRole::User,
                content: current_request.clone(),
            },
        ];

        let plan = plan_compaction(&items, 600);

        assert!(plan.items_to_summarize.iter().any(|item| matches!(
            item,
            AgentItem::Message { id, .. } if id.0 == "old-user"
        )));
        assert!(plan.kept_items.iter().any(|item| matches!(
            item,
            AgentItem::Message {
                id,
                role: AgentRole::User,
                content,
            } if id.0 == "current-user" && content == &current_request
        )));
    }

    #[test]
    fn oversized_recovery_bounds_provider_view_and_keeps_source_exact() {
        let original =
            "开头-anchor\n".to_string() + &"middle-data-".repeat(2_000) + "\n结尾-anchor";
        let items = vec![AgentItem::Message {
            id: ItemId::from_string("oversized-user".to_string()),
            role: AgentRole::User,
            content: original.clone(),
        }];

        let recovery = project_oversized_atomic_item(&items, 1_024).expect("recovery projection");

        assert_eq!(recovery.source_items, items);
        let original_tokens = items.iter().map(AgentItem::estimated_tokens).sum::<usize>();
        let recovered_tokens = recovery
            .model_items
            .iter()
            .map(AgentItem::estimated_tokens)
            .sum::<usize>();
        assert!(recovered_tokens < original_tokens);
        assert!(recovered_tokens <= 1_024 - 256);
        assert!(matches!(
            &recovery.model_items[0],
            AgentItem::Message { id, role: AgentRole::User, content }
                if id.as_str() == "oversized-user"
                    && content.contains("开头-anchor")
                    && content.contains("结尾-anchor")
                    && content.contains("Exact canonical source item: oversized-user")
        ));
    }

    #[test]
    fn ui_action_uses_shared_compaction_budget_and_keeps_canonical_envelope() {
        let identity = json!({"owner":{"authority_id":"host","conversation_id":"conversation","participant_id":"agent",
            "context_id":"conversation","originating_run_id":"run","message_id":"assistant","part_id":"tool"},
            "surface_id":"form","generation":1});
        let action: nola_domain::generative_ui::UiActionContent = serde_json::from_value(json!({
            "surface":{"identity":identity,"revision":1},"summary":"Submitted form · submit",
            "envelope":{"type":"ui_action","surface":identity,"message":{"version":"v0.9.1",
                "action":{"name":"submit","sourceComponentId":"submit","surfaceId":"form","timestamp":"2026-10-21T00:00:00Z","context":{"value":"complete-data-".repeat(3000)}}}}
        })).unwrap();
        let item = AgentItem::UiAction { id: ItemId::from_string("typed-action".into()), action };
        assert!(item.is_user_input());
        let plan = plan_compaction(&[AgentItem::user_message(ItemId::from_string("older".into()), "old ".repeat(3000)), item.clone()], 1024);
        assert!(plan.kept_items.contains(&item), "current action is the current user boundary");
        let recovery = project_oversized_atomic_item(&[item.clone()], 1024).unwrap();
        assert_eq!(recovery.source_items, vec![item.clone()]);
        assert!(recovery.model_items.iter().map(AgentItem::estimated_tokens).sum::<usize>() <= 1024 - 256);
        assert!(matches!(&recovery.model_items[0], AgentItem::Message { role: AgentRole::User, content, .. }
            if content.contains("Exact canonical source item: typed-action")));
        assert!(item.estimated_tokens() > recovery.model_items[0].estimated_tokens());
    }

    #[test]
    fn oversized_recovery_refuses_a_working_set_that_cannot_fit_the_target() {
        let items = vec![
            AgentItem::Message {
                id: ItemId::from_string("large-current".to_string()),
                role: AgentRole::User,
                content: "current".repeat(8_000),
            },
            AgentItem::Message {
                id: ItemId::from_string("also-retained".to_string()),
                role: AgentRole::Assistant,
                content: "retained".repeat(2_000),
            },
        ];

        assert!(project_oversized_atomic_item(&items, 1_024).is_none());
    }

    #[test]
    fn oversized_tool_recovery_keeps_call_and_result_identity_together() {
        let call_id = CallId::from_string("atomic-call".to_string());
        let items = vec![
            AgentItem::ToolCall {
                id: ItemId::from_string("atomic-call-item".to_string()),
                call_id: call_id.clone(),
                name: "inspect".to_string(),
                arguments: json!({"query": "q".repeat(8_000)}),
            },
            AgentItem::ToolResult {
                id: ItemId::from_string("atomic-result-item".to_string()),
                call_id: call_id.clone(),
                name: "inspect".to_string(),
                output: json!({"text": "result"}),
                is_error: false,
                full_output_ref: Some("blob://run/atomic-call".to_string()),
                artifacts: Vec::new(),
            },
        ];

        let recovery = project_oversized_atomic_item(&items, 1_024).expect("tool recovery");

        assert_eq!(recovery.source_items, items);
        assert_eq!(recovery.model_items.len(), 2);
        assert!(matches!(
            &recovery.model_items[0],
            AgentItem::ToolCall { call_id: projected, .. } if projected == &call_id
        ));
        assert!(matches!(
            &recovery.model_items[1],
            AgentItem::ToolResult { call_id: projected, full_output_ref: Some(reference), .. }
                if projected == &call_id && reference == "blob://run/atomic-call"
        ));
    }

    #[test]
    fn aggregate_active_round_overflow_keeps_recent_complete_pairs_and_current_user() {
        let mut items = vec![
            AgentItem::Message {
                id: ItemId::from_string("system".to_string()),
                role: AgentRole::System,
                content: "Follow repository policy.".to_string(),
            },
            AgentItem::Message {
                id: ItemId::from_string("current-user".to_string()),
                role: AgentRole::User,
                content: "Continue the long investigation.".to_string(),
            },
        ];
        for index in 0..12 {
            let call_id = CallId::from_string(format!("call-{index}"));
            items.push(AgentItem::ToolCall {
                id: ItemId::from_string(format!("call-item-{index}")),
                call_id: call_id.clone(),
                name: "read_file".to_string(),
                arguments: json!({"path": format!("src/{index}.rs")}),
            });
            items.push(AgentItem::ToolResult {
                id: ItemId::from_string(format!("result-item-{index}")),
                call_id,
                name: "read_file".to_string(),
                output: json!({"content": "evidence".repeat(120)}),
                is_error: false,
                full_output_ref: Some(format!("blob://result/{index}")),
                artifacts: Vec::new(),
            });
        }
        assert!(items.iter().all(|item| item.estimated_tokens() < 1_024));
        assert!(items.iter().map(AgentItem::estimated_tokens).sum::<usize>() > 1_024);

        let recovery = project_oversized_atomic_item(&items, 1_024)
            .expect("aggregate active round should have a bounded recovery");

        assert!(recovery.source_items.len() >= 2);
        assert!(recovery.model_items.iter().any(|item| matches!(
            item,
            AgentItem::Message { id, role: AgentRole::User, .. }
                if id.as_str() == "current-user"
        )));
        let retained_calls = recovery
            .model_items
            .iter()
            .filter_map(|item| match item {
                AgentItem::ToolCall { call_id, .. } => Some(call_id.0.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let retained_results = recovery
            .model_items
            .iter()
            .filter_map(|item| match item {
                AgentItem::ToolResult { call_id, .. } => Some(call_id.0.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(retained_calls, retained_results);
        assert!(retained_calls.contains("call-11"));
        assert!(!retained_calls.contains("call-0"));
        assert!(
            recovery
                .model_items
                .iter()
                .map(AgentItem::estimated_tokens)
                .sum::<usize>()
                <= 1_024 - 256
        );
    }

    #[test]
    fn aggregate_recovery_preserves_incomplete_tool_transactions() {
        let incomplete = CallId::from_string("incomplete".to_string());
        let items = vec![
            AgentItem::Message {
                id: ItemId::from_string("current-user".to_string()),
                role: AgentRole::User,
                content: "Continue.".to_string(),
            },
            AgentItem::Message {
                id: ItemId::from_string("old-evidence".to_string()),
                role: AgentRole::Assistant,
                content: "x".repeat(4_000),
            },
            AgentItem::ToolCall {
                id: ItemId::from_string("incomplete-call".to_string()),
                call_id: incomplete.clone(),
                name: "bash".to_string(),
                arguments: json!({"command": "long-running"}),
            },
        ];

        let recovery = project_oversized_atomic_item(&items, 1_024).expect("bounded recovery");
        assert!(recovery.model_items.iter().any(|item| matches!(
            item,
            AgentItem::ToolCall { call_id, .. } if call_id == &incomplete
        )));
    }
    #[test]
    fn adapter_preserves_original_items_and_uses_kernel_summary_budget() {
        let items = vec![
            AgentItem::Message {
                id: ItemId::from_string("old".into()),
                role: AgentRole::User,
                content: "旧正文\n  spacing".repeat(100),
            },
            AgentItem::Message {
                id: ItemId::from_string("developer".into()),
                role: AgentRole::Developer,
                content: "Keep exact\n\tbytes".into(),
            },
            AgentItem::Message {
                id: ItemId::from_string("current".into()),
                role: AgentRole::User,
                content: "新请求 🦀".into(),
            },
            AgentItem::ToolCall {
                id: ItemId::from_string("call".into()),
                call_id: CallId::from_string("pending".into()),
                name: "test".into(),
                arguments: json!({"value":"untouched"}),
            },
        ];
        let plan = plan_compaction(&items, 1);
        assert_eq!(
            plan.items_to_summarize,
            vec![items[0].clone()]
        );
        assert_eq!(plan.kept_items, vec![items[1].clone(), items[2].clone(), items[3].clone()]);
        let retained: usize = plan
            .kept_items
            .iter()
            .map(AgentItem::estimated_tokens)
            .sum();
        assert_eq!(plan.summary_token_budget(1, retained + 5, false).unwrap(), 1);
        assert!(plan.summary_token_budget(1, retained + 4, false).is_err());
    }
}

#[cfg(test)]
mod handoff_budget_tests {
    use super::*;
    #[test]
    fn existing_concise_checkpoint_is_not_forced_to_halve_again() {
        let plan = CompactionPlan {
            items_to_summarize: vec![AgentItem::user_message(ItemId::from_string("fact".into()), "x".repeat(384))],
            kept_items: vec![],
        };
        let original = plan.items_to_summarize[0].estimated_tokens();
        let ordinary = plan.summary_token_budget(1000, 10000, false).unwrap();
        let handoff = plan.summary_token_budget(1000, 10000, true).unwrap();
        assert_eq!(handoff + 5, original);
        assert!(handoff > ordinary);
        assert!(handoff + 4 < original, "even a handoff must reduce history");
        assert_eq!(plan.summary_token_budget(10, 10000, true).unwrap(), 6);
    }
}
