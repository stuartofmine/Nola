// SPDX-License-Identifier: Apache-2.0
//! Pure ordered tool batch admission from immutable domain facts.
use nola_domain::{ToolBatchCandidate, ToolBatchCandidateState, ToolBatchSelection};

pub const MAX_CONCURRENT_TOOL_BATCH_SIZE: usize = 4;

/// Running work or unresolved permission anywhere blocks new admission.
/// Otherwise admit one serial/unknown call or a consecutive concurrent-safe
/// prefix of pending calls. Inactive calls are ignored without reordering.
pub fn plan_tool_batch(candidates: &[ToolBatchCandidate]) -> ToolBatchSelection {
    if candidates.iter().any(|candidate| {
        matches!(
            candidate.state,
            ToolBatchCandidateState::Running | ToolBatchCandidateState::AwaitingPermission
        )
    }) {
        return ToolBatchSelection::default();
    }
    let mut pending = candidates
        .iter()
        .filter(|candidate| candidate.state == ToolBatchCandidateState::Pending);
    let Some(first) = pending.next() else {
        return ToolBatchSelection::default();
    };
    let concurrent = |candidate: &ToolBatchCandidate| {
        candidate.capability.available
            && candidate.capability.is_tool
            && candidate.capability.concurrent_safe
    };
    let mut call_ids = vec![first.call_id.clone()];
    if concurrent(first) {
        for candidate in pending {
            if call_ids.len() == MAX_CONCURRENT_TOOL_BATCH_SIZE || !concurrent(candidate) {
                break;
            }
            call_ids.push(candidate.call_id.clone());
        }
    }
    ToolBatchSelection { call_ids }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::ToolBatchCapabilityFacts;
    fn candidate(id: usize, state: ToolBatchCandidateState, bits: u8) -> ToolBatchCandidate {
        ToolBatchCandidate {
            call_id: id.to_string(),
            state,
            capability: ToolBatchCapabilityFacts {
                available: bits & 1 != 0,
                is_tool: bits & 2 != 0,
                concurrent_safe: bits & 4 != 0,
            },
        }
    }
    #[test]
    fn all_capability_fact_combinations_require_three_positive_facts() {
        for bits in 0..8 {
            let candidates = vec![
                candidate(0, ToolBatchCandidateState::Pending, bits),
                candidate(1, ToolBatchCandidateState::Pending, 7),
            ];
            let expected = if bits == 7 { vec!["0", "1"] } else { vec!["0"] };
            assert_eq!(
                plan_tool_batch(&candidates).call_ids,
                expected,
                "facts={bits}"
            );
        }
    }
    #[test]
    fn global_barrier_at_every_position_blocks_even_after_batch_capacity() {
        for barrier in [
            ToolBatchCandidateState::Running,
            ToolBatchCandidateState::AwaitingPermission,
        ] {
            for position in 0..8 {
                let mut candidates = (0..8)
                    .map(|id| candidate(id, ToolBatchCandidateState::Pending, 7))
                    .collect::<Vec<_>>();
                candidates[position].state = barrier;
                assert!(plan_tool_batch(&candidates).call_ids.is_empty());
            }
        }
    }
    #[test]
    fn exhaustive_short_sequences_preserve_order_and_serial_boundaries() {
        // 5^5 combinations: inactive, both global barriers, serial and concurrent pending.
        for encoded in 0..3125_u32 {
            let mut value = encoded;
            let mut candidates = Vec::new();
            let mut barrier = false;
            let mut pending = Vec::new();
            for id in 0..5 {
                let digit = value % 5;
                value /= 5;
                let (state, bits) = match digit {
                    0 => (ToolBatchCandidateState::Inactive, 7),
                    1 => (ToolBatchCandidateState::Running, 7),
                    2 => (ToolBatchCandidateState::AwaitingPermission, 7),
                    3 => (ToolBatchCandidateState::Pending, 0),
                    _ => (ToolBatchCandidateState::Pending, 7),
                };
                barrier |= matches!(digit, 1 | 2);
                if digit >= 3 {
                    pending.push((id, digit == 4));
                }
                candidates.push(candidate(id, state, bits));
            }
            let selected = plan_tool_batch(&candidates).call_ids;
            if barrier || pending.is_empty() {
                assert!(selected.is_empty());
                continue;
            }
            let count = if !pending[0].1 {
                1
            } else {
                pending.iter().take_while(|(_, safe)| *safe).count().min(4)
            };
            assert_eq!(
                selected,
                pending[..count]
                    .iter()
                    .map(|(id, _)| id.to_string())
                    .collect::<Vec<_>>(),
                "case {encoded}"
            );
        }
    }
    #[test]
    fn four_then_one_and_inactive_gaps_do_not_lose_pending_calls() {
        let mut candidates = (0..5)
            .map(|id| candidate(id, ToolBatchCandidateState::Pending, 7))
            .collect::<Vec<_>>();
        assert_eq!(
            plan_tool_batch(&candidates).call_ids,
            vec!["0", "1", "2", "3"]
        );
        for candidate in &mut candidates[..4] {
            candidate.state = ToolBatchCandidateState::Inactive;
        }
        assert_eq!(plan_tool_batch(&candidates).call_ids, vec!["4"]);
        candidates[1].state = ToolBatchCandidateState::Pending;
        assert_eq!(plan_tool_batch(&candidates).call_ids, vec!["1", "4"]);
        assert!(plan_tool_batch(&[]).call_ids.is_empty());
    }
}
