// SPDX-License-Identifier: Apache-2.0
use nola_domain::model_response_policy::{
    CompletionContractDecision, CompletionContractFacts, FinalSynthesisFacts, GoalStatusFacts,
    GoalStatusOrigin, ModelResponseContract, TextPresentationFacts,
};

pub fn evaluate_completion_contract(facts: CompletionContractFacts) -> CompletionContractDecision {
    let response = if facts.final_answer_audit_required
        && facts.has_final_answer_draft
        && !facts.has_final_answer_audit_report
    {
        ModelResponseContract::StructuredAudit
    } else if !facts.final_synthesis
        && (facts.skill_protocol_convergence || facts.artifact_completion_pending)
    {
        ModelResponseContract::RequiredTool
    } else {
        ModelResponseContract::Conversational
    };
    CompletionContractDecision {
        response,
        defer_text_presentation: should_defer_text_presentation(facts.presentation),
    }
}

pub fn should_project_goal_status(facts: GoalStatusFacts) -> bool {
    match facts.origin {
        GoalStatusOrigin::Audit => true,
        GoalStatusOrigin::GoalUserOrContinuation => facts.has_active_nonterminal_goal,
        GoalStatusOrigin::Other => false,
    }
}

pub fn should_enter_final_synthesis(facts: FinalSynthesisFacts) -> bool {
    facts.user_denied_permission
        || (facts.final_answer_retry_requested && !facts.skill_completion_pending)
}

pub fn should_defer_text_presentation(facts: TextPresentationFacts) -> bool {
    // Only a concrete candidate/audit boundary can keep output private.
    // Engineering validation policy alone is deliberately not an input.
    (facts.has_executed_tools
        && facts.final_answer_retry_requested
        && !facts.has_final_answer_draft)
        || (facts.final_answer_audit_required
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_goal_origins_and_activity_states() {
        for active in [false, true] {
            for (origin, expected) in [
                (GoalStatusOrigin::Audit, true),
                (GoalStatusOrigin::GoalUserOrContinuation, active),
                (GoalStatusOrigin::Other, false),
            ] {
                assert_eq!(
                    should_project_goal_status(GoalStatusFacts {
                        origin,
                        has_active_nonterminal_goal: active
                    }),
                    expected
                );
            }
        }
    }
    #[test]
    fn denial_wins_and_pending_skill_blocks_only_retry_synthesis() {
        for (retry, denied, pending, expected) in [
            (false, false, false, false),
            (false, false, true, false),
            (false, true, false, true),
            (false, true, true, true),
            (true, false, false, true),
            (true, false, true, false),
            (true, true, false, true),
            (true, true, true, true),
        ] {
            assert_eq!(
                should_enter_final_synthesis(FinalSynthesisFacts {
                    final_answer_retry_requested: retry,
                    user_denied_permission: denied,
                    skill_completion_pending: pending
                }),
                expected
            );
        }
    }
    #[test]
    fn every_candidate_audit_boundary_has_explicit_visibility() {
        // Bit order: audit, executed, retry, draft, report. This table is the
        // complete contract: all other states stream directly.
        let private_states = [1, 3, 5, 6, 7, 9, 11, 13, 15, 17, 19, 21, 22, 23, 25, 27, 29, 31];
        for bits in 0..32 {
            let flag = |bit| bits & (1 << bit) != 0;
            let facts = TextPresentationFacts {
                final_answer_audit_required: flag(0),
                has_executed_tools: flag(1),
                final_answer_retry_requested: flag(2),
                has_final_answer_draft: flag(3),
                has_final_answer_audit_report: flag(4),
            };
            assert_eq!(
                should_defer_text_presentation(facts),
                private_states.contains(&bits),
                "state {bits}: {facts:?}"
            );
        }
    }

    #[test]
    fn completion_contract_owns_response_mode_and_visibility_together() {
        let presentation = TextPresentationFacts {
            final_answer_audit_required: true,
            has_executed_tools: true,
            final_answer_retry_requested: false,
            has_final_answer_draft: true,
            has_final_answer_audit_report: false,
        };
        let decision = evaluate_completion_contract(CompletionContractFacts {
            presentation,
            final_answer_audit_required: true,
            has_executed_tools: true,
            has_final_answer_draft: true,
            has_final_answer_audit_report: false,
            skill_protocol_convergence: false,
            artifact_completion_pending: false,
            final_synthesis: false,
        });
        assert_eq!(decision.response, ModelResponseContract::StructuredAudit);
        assert!(decision.defer_text_presentation);
    }
}
