// SPDX-License-Identifier: Apache-2.0
//! Turn input, completion and cancellation decisions without side effects.
use nola_domain::{
    AgentTurnStatus, CancellableEffectFact, ToolCallStatus, TurnCancellationPlan,
    TurnCompletionAction, TurnInputAction, TurnInputFacts, TurnInputRejection,
};

pub fn plan_turn_input(facts: TurnInputFacts) -> Result<TurnInputAction, TurnInputRejection> {
    if !facts.has_items && (facts.live_user_authority || !facts.has_context_items) {
        return Err(TurnInputRejection::Empty);
    }
    if matches!(
        facts.active_status,
        Some(
            AgentTurnStatus::AwaitingModel
                | AgentTurnStatus::AwaitingTools
                | AgentTurnStatus::AwaitingPermission
                | AgentTurnStatus::AwaitingInput
                | AgentTurnStatus::Compacting
                | AgentTurnStatus::Cancelling
        )
    ) {
        if !facts.live_user_authority {
            return Err(TurnInputRejection::AutomatedMerge);
        }
        if facts.has_context_items || facts.has_context_components {
            return Err(TurnInputRejection::ContextAlreadyCaptured);
        }
        Ok(TurnInputAction::Enqueue)
    } else {
        Ok(TurnInputAction::Start)
    }
}

pub fn plan_turn_completion(
    current_input_pending: bool,
    next_input_pending: bool,
) -> TurnCompletionAction {
    if current_input_pending {
        TurnCompletionAction::ContinueCurrent
    } else if next_input_pending {
        TurnCompletionAction::CompleteAndStartNext
    } else {
        TurnCompletionAction::Complete
    }
}

pub fn plan_turn_cancellation(
    effects: &[CancellableEffectFact],
    tools: impl IntoIterator<Item = ToolCallStatus>,
) -> TurnCancellationPlan {
    TurnCancellationPlan {
        effect_indices: effects
            .iter()
            .enumerate()
            .filter_map(|(index, effect)| {
                (effect.belongs_to_turn && !effect.is_cancellation_request && !effect.is_cancelling)
                    .then_some(index)
            })
            .collect(),
        tool_indices: tools
            .into_iter()
            .enumerate()
            .filter_map(|(index, state)| {
                matches!(
                    state,
                    ToolCallStatus::Pending
                        | ToolCallStatus::Running
                        | ToolCallStatus::AwaitingPermission
                        | ToolCallStatus::AwaitingUserInput
                )
                .then_some(index)
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_input_preserves_authority_and_context_capture_boundary() {
        for status in [
            AgentTurnStatus::AwaitingModel,
            AgentTurnStatus::AwaitingTools,
            AgentTurnStatus::AwaitingPermission,
            AgentTurnStatus::Compacting,
            AgentTurnStatus::Cancelling,
        ] {
            let mut facts = TurnInputFacts {
                has_items: true,
                live_user_authority: true,
                has_context_items: false,
                has_context_components: false,
                active_status: Some(status),
            };
            assert_eq!(plan_turn_input(facts), Ok(TurnInputAction::Enqueue));
            facts.live_user_authority = false;
            assert_eq!(
                plan_turn_input(facts),
                Err(TurnInputRejection::AutomatedMerge)
            );
            facts.live_user_authority = true;
            facts.has_context_components = true;
            assert_eq!(
                plan_turn_input(facts),
                Err(TurnInputRejection::ContextAlreadyCaptured)
            );
            facts.has_items = false;
            assert_eq!(plan_turn_input(facts), Err(TurnInputRejection::Empty));
            facts.active_status = None;
            facts.live_user_authority = false;
            facts.has_context_items = true;
            assert_eq!(plan_turn_input(facts), Ok(TurnInputAction::Start));
        }
        for status in [
            AgentTurnStatus::Idle,
            AgentTurnStatus::Completed,
            AgentTurnStatus::Cancelled,
            AgentTurnStatus::Failed,
        ] {
            assert_eq!(
                plan_turn_input(TurnInputFacts {
                    has_items: true,
                    live_user_authority: true,
                    has_context_items: false,
                    has_context_components: false,
                    active_status: Some(status)
                }),
                Ok(TurnInputAction::Start)
            );
        }
    }

    #[test]
    fn queued_current_input_precedes_completion_and_next_turn() {
        assert_eq!(
            plan_turn_completion(true, true),
            TurnCompletionAction::ContinueCurrent
        );
        assert_eq!(
            plan_turn_completion(true, false),
            TurnCompletionAction::ContinueCurrent
        );
        assert_eq!(
            plan_turn_completion(false, true),
            TurnCompletionAction::CompleteAndStartNext
        );
        assert_eq!(
            plan_turn_completion(false, false),
            TurnCompletionAction::Complete
        );
    }

    #[test]
    fn cancellation_only_targets_owned_effects_and_live_tools() {
        let effects = (0..8)
            .map(|bits| CancellableEffectFact {
                belongs_to_turn: bits & 1 != 0,
                is_cancellation_request: bits & 2 != 0,
                is_cancelling: bits & 4 != 0,
            })
            .collect::<Vec<_>>();
        let plan = plan_turn_cancellation(
            &effects,
            [
                ToolCallStatus::Pending,
                ToolCallStatus::AwaitingPermission,
                ToolCallStatus::Running,
                ToolCallStatus::Completed,
                ToolCallStatus::Failed,
                ToolCallStatus::Denied,
                ToolCallStatus::Cancelled,
            ],
        );
        assert_eq!(plan.effect_indices, [1]);
        assert_eq!(plan.tool_indices, [0, 1, 2]);
    }
}
