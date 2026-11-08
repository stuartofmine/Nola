// SPDX-License-Identifier: Apache-2.0
//! Pure selection of complete tool transactions and summary token budgets.
use nola_domain::compaction_plan::{CompactionIndexPlan, CompactionItemFact, CompactionItemKind};
use std::collections::BTreeSet;

pub fn plan_compaction(items: &[CompactionItemFact], target_tokens: usize) -> CompactionIndexPlan {
    let tail_budget = target_tokens.saturating_div(2).max(1);
    let persistent = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            matches!(item.kind, CompactionItemKind::PersistentInstruction).then_some(index)
        })
        .collect::<BTreeSet<_>>();
    let mut rounds: Vec<Vec<usize>> = Vec::new();
    let mut pending_calls = BTreeSet::new();
    let mut transaction_finished = false;
    for (index, item) in items.iter().enumerate() {
        if persistent.contains(&index) {
            continue;
        }
        if rounds.is_empty()
            || (pending_calls.is_empty()
                && (transaction_finished || matches!(item.kind, CompactionItemKind::UserMessage)))
        {
            rounds.push(Vec::new());
        }
        transaction_finished = false;
        match &item.kind {
            CompactionItemKind::ToolCall { call_id } => {
                pending_calls.insert(call_id.clone());
            }
            CompactionItemKind::ToolResult { call_id } => {
                pending_calls.remove(call_id);
                transaction_finished = pending_calls.is_empty();
            }
            _ => {}
        }
        rounds
            .last_mut()
            .expect("round initialized above")
            .push(index);
    }
    let mut retained = persistent.clone();
    // The active user request is an instruction anchor, independently of the
    // bounded tail of completed tool transactions that follows it.
    if let Some(index) = items.iter().rposition(|item| matches!(item.kind, CompactionItemKind::UserMessage)) {
        retained.insert(index);
    }
    let mut retained_tokens = retained.iter().fold(0usize, |total, index| {
        total.saturating_add(items[*index].estimated_tokens)
    });
    for (reverse_index, round) in rounds.iter().rev().enumerate() {
        let round_tokens = round.iter().fold(0usize, |total, index| {
            total.saturating_add(if retained.contains(index) { 0 } else { items[*index].estimated_tokens })
        });
        // Preserve the newest transaction even when it exceeds the goal.
        // Older rounds form a contiguous tail: stop at the first non-fitting one.
        if reverse_index == 0 || retained_tokens.saturating_add(round_tokens) <= tail_budget {
            retained.extend(round.iter().copied());
            retained_tokens = retained_tokens.saturating_add(round_tokens);
        } else {
            break;
        }
    }
    let calls = retained
        .iter()
        .filter_map(|index| match &items[*index].kind {
            CompactionItemKind::ToolCall { call_id } => Some(call_id.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let results = retained
        .iter()
        .filter_map(|index| match &items[*index].kind {
            CompactionItemKind::ToolResult { call_id } => Some(call_id.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    // Keep both sides of a selected call, including malformed/recovered
    // histories. Never send an unresolved call to a summarizer as if completed.
    let selected_calls = calls.union(&results).copied().collect::<BTreeSet<_>>();
    for (index, item) in items.iter().enumerate() {
        if let CompactionItemKind::ToolCall { call_id } | CompactionItemKind::ToolResult { call_id } = &item.kind {
            if selected_calls.contains(call_id.as_str()) {
                retained.insert(index);
            }
        }
    }
    let mut plan = CompactionIndexPlan::default();
    for index in 0..items.len() {
        if retained.contains(&index) {
            plan.kept_indices.push(index);
        } else {
            plan.summarize_indices.push(index);
        }
    }
    plan
}

pub fn summary_token_budget(
    retained_item_tokens: impl IntoIterator<Item = usize>,
    target_tokens: usize,
    prompt_limit: usize,
) -> Result<usize, &'static str> {
    let retained = retained_item_tokens
        .into_iter()
        .fold(0usize, usize::saturating_add);
    // Four framing tokens account for the inserted summary AgentItem.
    let hard_remaining = prompt_limit.saturating_sub(retained.saturating_add(4));
    if hard_remaining == 0 {
        return Err("retained context leaves no prompt budget for a compaction summary");
    }
    // A soft full-request target may be exhausted by mandatory instructions
    // and schemas. Zero is then an unattainable goal, not a one-token output
    // contract. The hard remaining budget still permits useful compression.
    if target_tokens == 0 {
        return Ok(hard_remaining);
    }
    let target_remaining = target_tokens.saturating_sub(retained.saturating_add(4));
    Ok(if target_remaining > 0 {
        target_remaining.min(hard_remaining)
    } else {
        target_tokens.max(1).min(hard_remaining)
    })
}

/// Validate the exact returned text with AgentItem's character estimator.
/// Framing tokens are excluded: summary_token_budget already reserved them.
pub fn validate_summary(summary: &str, token_limit: usize) -> Result<(), &'static str> {
    if summary.trim().is_empty() {
        return Err("compaction summary must not be empty");
    }
    if summary.chars().count().saturating_add(3) / 4 > token_limit {
        return Err("compaction summary exceeds the planned token budget");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fact(kind: CompactionItemKind, estimated_tokens: usize) -> CompactionItemFact {
        CompactionItemFact {
            kind,
            estimated_tokens,
        }
    }
    #[test]
    fn summary_validity_uses_untrimmed_unicode_character_budget() {
        for blank in ["", " ", "\n\t", "\u{2003}"] {
            assert_eq!(
                validate_summary(blank, 100),
                Err("compaction summary must not be empty")
            );
        }
        assert!(validate_summary("text", 1).is_ok());
        assert!(validate_summary("texts", 1).is_err());
        assert!(validate_summary("🦀中文é", 1).is_ok());
        assert!(validate_summary("🦀中文é!", 1).is_err());
        assert!(validate_summary(" text", 1).is_err());
        assert!(validate_summary("a", 0).is_err());
        let budget = summary_token_budget([10], 20, 20).unwrap();
        assert_eq!(budget, 6);
        assert!(validate_summary(&"x".repeat(budget * 4), budget).is_ok());
        assert!(validate_summary(&"x".repeat(budget * 4 + 1), budget).is_err());
    }

    #[test]
    fn round_selection_preserves_boundaries_and_pair_cleanup() {
        use CompactionItemKind::*;
        let cases = [
            (vec![], 0, vec![]),
            (vec![fact(PersistentInstruction, 999)], 0, vec![0]),
            (
                vec![
                    fact(UserMessage, 10),
                    fact(Other, 10),
                    fact(UserMessage, 10),
                ],
                60,
                vec![0, 1, 2],
            ),
            (
                vec![
                    fact(UserMessage, 10),
                    fact(Other, 10),
                    fact(UserMessage, 10),
                ],
                59,
                vec![2],
            ),
            (
                vec![
                    fact(UserMessage, 1),
                    fact(UserMessage, 100),
                    fact(UserMessage, 1),
                ],
                20,
                vec![2],
            ),
            (vec![fact(Other, 1), fact(UserMessage, 1000)], 1, vec![1]),
            (
                vec![
                    fact(UserMessage, 1),
                    fact(PersistentInstruction, 1),
                    fact(Other, 1),
                    fact(UserMessage, 1),
                ],
                8,
                vec![0, 1, 2, 3],
            ),
            (
                vec![
                    fact(
                        ToolCall {
                            call_id: "x".into(),
                        },
                        1,
                    ),
                    fact(
                        ToolResult {
                            call_id: "x".into(),
                        },
                        1,
                    ),
                ],
                0,
                vec![0, 1],
            ),
            (
                vec![fact(
                    ToolCall {
                        call_id: "x".into(),
                    },
                    1,
                )],
                100,
                vec![0],
            ),
            (
                vec![
                    fact(
                        ToolCall {
                            call_id: "x".into(),
                        },
                        1,
                    ),
                    fact(UserMessage, 100),
                    fact(
                        ToolResult {
                            call_id: "x".into(),
                        },
                        1,
                    ),
                ],
                0,
                vec![0, 1, 2],
            ),
            (
                vec![
                    fact(PersistentInstruction, usize::MAX),
                    fact(UserMessage, usize::MAX),
                    fact(UserMessage, usize::MAX),
                ],
                1,
                vec![0, 2],
            ),
        ];
        for (items, target, expected) in cases {
            let plan = plan_compaction(&items, target);
            assert_eq!(plan.kept_indices, expected, "{items:?}, target={target}");
            assert_eq!(
                plan.summarize_indices,
                (0..items.len())
                    .filter(|i| !expected.contains(i))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn exhaustive_short_histories_keep_instructions_current_user_and_complete_pairs() {
        use CompactionItemKind::*;
        let kinds = [
            PersistentInstruction,
            UserMessage,
            Other,
            ToolCall {
                call_id: "a".into(),
            },
            ToolResult {
                call_id: "a".into(),
            },
            ToolResult {
                call_id: "b".into(),
            },
        ];
        for length in 0..=5u32 {
            for mut word in 0..6usize.pow(length) {
                let items = (0..length)
                    .map(|index| {
                        let kind = kinds[word % 6].clone();
                        word /= 6;
                        fact(kind, index as usize + 1)
                    })
                    .collect::<Vec<_>>();
                for budget in [0, 1, 5, 10, 100, usize::MAX] {
                    let plan = plan_compaction(&items, budget);
                    let mut combined = plan.kept_indices.clone();
                    combined.extend(&plan.summarize_indices);
                    combined.sort_unstable();
                    assert_eq!(combined, (0..items.len()).collect::<Vec<_>>());
                    assert!(plan.kept_indices.windows(2).all(|p| p[0] < p[1]));
                    assert!(plan.summarize_indices.windows(2).all(|p| p[0] < p[1]));
                    for (index, item) in items.iter().enumerate() {
                        if matches!(item.kind, PersistentInstruction) {
                            assert!(plan.kept_indices.contains(&index));
                        }
                    }
                    if let Some(last_user) =
                        items.iter().rposition(|i| matches!(i.kind, UserMessage))
                    {
                        assert!(plan.kept_indices.contains(&last_user));
                    }
                    for index in &plan.kept_indices {
                        if let ToolCall { call_id } | ToolResult { call_id } = &items[*index].kind {
                            for (other_index, other) in items.iter().enumerate() {
                                if matches!(&other.kind, ToolCall { call_id: id } | ToolResult { call_id: id } if id == call_id) {
                                    assert!(plan.kept_indices.contains(&other_index));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn one_long_user_turn_compacts_old_batches_without_splitting_parallel_calls() {
        use CompactionItemKind::*;
        let mut items = vec![fact(PersistentInstruction, 10), fact(UserMessage, 10)];
        for batch in 0..10 {
            for call in 0..2 {
                items.push(fact(ToolCall { call_id: format!("{batch}-{call}") }, 10));
            }
            for call in 0..2 {
                items.push(fact(ToolResult { call_id: format!("{batch}-{call}") }, 100));
            }
        }
        let plan = plan_compaction(&items, 600);
        assert_eq!(plan.kept_indices, vec![0, 1, 38, 39, 40, 41]);
        assert_eq!(plan.summarize_indices, (2..38).collect::<Vec<_>>());
    }

    #[test]
    fn summary_budget_respects_framing_soft_goal_and_hard_limit() {
        for (retained, target, limit, expected) in [
            (vec![], 0, 5, Ok(1)),
            (vec![], 100, 10, Ok(6)),
            (vec![6], 10, 20, Ok(10)),
            (vec![6], 11, 20, Ok(1)),
            (
                vec![6],
                50,
                10,
                Err("retained context leaves no prompt budget for a compaction summary"),
            ),
            (
                vec![usize::MAX, 1],
                usize::MAX,
                usize::MAX,
                Err("retained context leaves no prompt budget for a compaction summary"),
            ),
        ] {
            assert_eq!(summary_token_budget(retained, target, limit), expected);
        }
    }
}
