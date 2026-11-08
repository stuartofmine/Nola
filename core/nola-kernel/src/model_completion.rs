// SPDX-License-Identifier: Apache-2.0
//! Ordered completion policy. Text parsing and transcript mutations stay at the boundary.
use nola_domain::{
    ModelCompletionAction as Action, ModelCompletionAuditUpdate as Audit,
    ModelCompletionFacts as Facts,
    ModelCompletionFailure as Failure, ModelCompletionPlan as Plan,
    ModelCompletionUpdate as Update,
};

pub fn plan_model_completion(f: &Facts) -> Plan {
    let mut update = Update::default();
    let next_count = f.no_progress_count.saturating_add(1);
    let text_only = !f.has_tool_calls && f.has_visible_answer;
    // The execution budget closes tools, not a valid final answer. Optional
    // review may continue if the model explicitly requested it; otherwise a
    // truthful synthesis goes to Core's result arbiter. Pending requirements
    // determine the business result there, not an implicit model retry here.
    let pending_empty_completion = !f.has_tool_calls && f.has_executed_tools
        && !f.has_visible_answer && !f.reasoning_in_items
        && (f.validation_required || f.recovery_required || f.skill_completion_pending);
    let bounded_completion_can_continue = (text_only
        && !f.structurally_repetitive
        && !f.embedded_tool_protocol) || pending_empty_completion;
    let action = if f.iteration_budget_final_synthesis && !bounded_completion_can_continue {
        Action::DiscardAndFail(Failure::IterationBudgetFinalSynthesis)
    } else if text_only && f.structurally_repetitive {
        update.no_progress_count = Some(next_count);
        if next_count >= 2 {
            Action::DiscardAndFail(Failure::RepetitiveOutput)
        } else {
            update.request_final_answer = true;
            Action::DiscardAndRequestModel
        }
    } else if !f.user_denied_permission
        && text_only
        && f.embedded_tool_protocol
        && (!f.final_answer_draft_present || f.final_answer_audit_report_present)
    {
        // Native tool calls must arrive on the typed tool channel. Reject an
        // encoded call in ordinary answer text even when optional independent
        // review was not requested. A reviewer may still quote the rejected
        // candidate as evidence while its report is being produced.
        update.no_progress_count = Some(next_count);
        update.request_final_answer = true;
        retry_or_fail(next_count, Failure::EmbeddedToolProtocol)
    } else if f.output_limited && f.output_limit_retry_requested {
        // A second truncated response is not a successful no-progress reply.
        Action::ConcludeIncomplete { reason: nola_domain::ModelCompletionIncompleteReason::OutputLimitRecoveryExhausted }
    } else if f.output_limit_retry_requested
        && !f.final_answer_audit_required
        && text_only
        && f.continuation_made_no_progress
        && !f.validation_required
        && !f.recovery_required
        && !f.skill_completion_pending
        && !(f.final_answer_audit_required && !f.final_answer_audit_report_present)
        && !(f.final_answer_draft_present && !f.final_answer_audit_report_present)
        && !(f.final_answer_retry_requested && f.embedded_tool_protocol)
    {
        Action::DiscardAndComplete
    } else if f.output_limited
        && !f.final_answer_audit_required
        && !f.has_tool_calls
        // Empty token-limit recovery must not override a denied permission's
        // required explanation. Visible truncated answers retain prior policy.
        && (f.has_visible_answer || !f.user_denied_permission)
        && !f.output_limit_retry_requested
        && !f.structurally_repetitive
    {
        Action::CommitAndRecoverOutputLimit
    } else if f.user_denied_permission && f.has_tool_calls {
        if f.final_answer_retry_requested {
            Action::DiscardAndFail(Failure::ToolsAfterDenial)
        } else {
            update.request_final_answer = true;
            Action::DiscardAndRequestModel
        }
    } else if f.user_denied_permission && !f.has_tool_calls && !f.has_visible_answer {
        if f.final_answer_retry_requested {
            Action::DiscardAndFail(Failure::NoDenialExplanation)
        } else {
            update.request_final_answer = true;
            Action::DiscardAndRequestModel
        }
    } else if f.tool_free_request && f.has_tool_calls {
        // This is a response-contract violation, not an execution failure.
        // Never create tool recovery obligations in a phase that cannot run tools.
        update.no_progress_count = Some(next_count);
        retry_or_fail(next_count, Failure::ToolsDuringTextOnlyResponse)
    } else if !f.user_denied_permission
        && f.final_answer_draft_present
        && !f.final_answer_audit_report_present
        && text_only
    {
        if f.normalized_audit_report_valid {
            if f.audit_requires_execution {
                if f.iteration_budget_final_synthesis || f.completion_recovery_count >= 2 {
                    update.audit = Audit::AcceptReport;
                    Action::ConcludeIncomplete { reason: nola_domain::ModelCompletionIncompleteReason::ExecutionRecoveryBudget }
                } else {
                    update.audit = Audit::ResumeExecution;
                    Action::DiscardAndRequestModel
                }
            } else if f.audit_accepts_candidate || f.audit_blocks_task {
                update.audit = Audit::AcceptReport;
                update.no_progress_count = Some(0);
                Action::PublishReviewedCandidate { blocked: f.audit_blocks_task }
            } else {
                update.audit = Audit::AcceptReport;
                update.no_progress_count = Some(0);
                if f.answer_revision_count >= 2 {
                    Action::ConcludeIncomplete { reason: nola_domain::ModelCompletionIncompleteReason::ReviewRevisionBudget }
                } else {
                    Action::DiscardAndRequestModel
                }
            }
        } else {
            update.no_progress_count = Some(next_count);
            retry_or_fail(next_count, Failure::InvalidAudit)
        }
    } else if !f.user_denied_permission
        && f.final_answer_draft_present
        && f.final_answer_audit_report_present
        && text_only
    {
        // A revision is another private candidate, never an unchecked final.
        update.audit = Audit::ReviseDraft;
        Action::DiscardAndRequestModel
    } else if !f.user_denied_permission
        && f.final_answer_audit_required
        && !f.final_answer_draft_present
        && text_only
    {
        update.request_final_answer = true;
        update.audit = Audit::SaveDraft;
        Action::DiscardAndRequestModel
    } else if !f.user_denied_permission
        && !f.has_tool_calls
        && !f.has_visible_answer
        && f.reasoning_in_items
    {
        if f.reasoning_only_retry_requested {
            Action::DiscardAndFail(Failure::ReasoningOnly)
        } else {
            update.request_reasoning_continuation = true;
            Action::DiscardAndRequestModel
        }
    } else if !f.user_denied_permission
        && !f.has_tool_calls
        && !f.has_visible_answer
        && !f.reasoning_in_items
        && !f.has_executed_tools
    {
        if f.empty_completion_retry_requested {
            Action::DiscardAndFail(Failure::EmptyCompletion)
        } else {
            update.request_empty_continuation = true;
            Action::DiscardAndRequestModel
        }
    } else if !f.has_tool_calls && f.has_executed_tools && !f.has_visible_answer {
        if f.recovery_required || f.validation_required || f.skill_completion_pending {
            // Empty text does not discharge debt, but neither is debt a model
            // transport failure. Let the same Core arbiter close truthfully.
            Action::CommitCompletion { has_tool_calls: false }
        } else if f.final_answer_retry_requested {
            update.no_progress_count = Some(next_count);
            retry_or_fail(next_count, Failure::EmptyFinalSynthesis)
        } else {
            update.request_final_answer = true;
            Action::DiscardAndRequestModel
        }
    } else {
        Action::CommitCompletion {
            has_tool_calls: f.has_tool_calls,
        }
    };
    Plan { action, update }
}
fn retry_or_fail(count: u8, failure: Failure) -> Action {
    if count >= 3 {
        Action::DiscardAndFail(failure)
    } else {
        Action::DiscardAndRequestModel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_protocol_validation_precedes_initial_and_revised_drafts() {
        for revision in [false, true] {
            for count in [0, 1, 2] {
                let facts = Facts {
                    final_answer_audit_required: true,
                    final_answer_draft_present: revision,
                    final_answer_audit_report_present: revision,
                    has_visible_answer: true,
                    embedded_tool_protocol: true,
                    no_progress_count: count,
                    ..Facts::default()
                };
                let plan = plan_model_completion(&facts);
                assert_eq!(plan.update.audit, Audit::default());
                assert_eq!(plan.update.no_progress_count, Some(count + 1));
                assert_eq!(plan.action, if count == 2 {
                    Action::DiscardAndFail(Failure::EmbeddedToolProtocol)
                } else { Action::DiscardAndRequestModel });
            }
        }
    }

    #[test]
    fn ordinary_final_answer_cannot_publish_encoded_tool_call() {
        for count in [0, 1, 2] {
            let facts = Facts {
                has_visible_answer: true,
                has_executed_tools: true,
                embedded_tool_protocol: true,
                no_progress_count: count,
                ..Facts::default()
            };
            let plan = plan_model_completion(&facts);
            assert_eq!(plan.update.request_final_answer, true);
            assert_eq!(plan.action, if count == 2 {
                Action::DiscardAndFail(Failure::EmbeddedToolProtocol)
            } else {
                Action::DiscardAndRequestModel
            });
        }
    }

    #[test]
    fn reviewer_can_quote_rejected_protocol_without_becoming_a_candidate() {
        let plan = plan_model_completion(&Facts {
            final_answer_audit_required: true,
            final_answer_draft_present: true,
            final_answer_retry_requested: true,
            has_visible_answer: true,
            embedded_tool_protocol: true,
            normalized_audit_report_valid: true,
            ..Facts::default()
        });
        assert_eq!(plan.update.audit, Audit::AcceptReport);
    }

    #[test]
    fn pending_empty_completion_reaches_result_arbiter_even_at_execution_budget() {
        for (validation, recovery, skill) in [(true, false, false), (false, true, false), (false, false, true)] {
            let plan = plan_model_completion(&Facts {
                has_executed_tools: true,
                iteration_budget_final_synthesis: true,
                validation_required: validation,
                recovery_required: recovery,
                skill_completion_pending: skill,
                no_progress_count: 2,
                ..Facts::default()
            });
            assert_eq!(plan.action, Action::CommitCompletion { has_tool_calls: false });
            assert!(!plan.update.request_final_answer);
        }
    }

    #[test]
    fn pending_work_can_be_reviewed_but_not_published_as_success() {
        for (validation, recovery, skill) in [(true, false, false), (false, true, false), (false, false, true)] {
            let facts = Facts {
                final_answer_audit_required: true,
                has_visible_answer: true,
                validation_required: validation,
                recovery_required: recovery,
                skill_completion_pending: skill,
                ..Facts::default()
            };
            assert_eq!(plan_model_completion(&facts).update.audit, Audit::SaveDraft);
            for execute in [true, false] {
                let reviewed = Facts {
                    final_answer_draft_present: true,
                    normalized_audit_report_valid: true,
                    audit_requires_execution: execute,
                    audit_accepts_candidate: !execute,
                    ..facts.clone()
                };
                let plan = plan_model_completion(&reviewed);
                if execute {
                    assert_eq!(plan.action, Action::DiscardAndRequestModel);
                    assert_eq!(plan.update.audit, Audit::ResumeExecution);
                    assert_eq!(plan_model_completion(&Facts {
                        completion_recovery_count: 2, ..reviewed
                    }).action, Action::ConcludeIncomplete { reason: nola_domain::ModelCompletionIncompleteReason::ExecutionRecoveryBudget });
                } else {
                    // Publication is not acceptance: Core still sees the debt
                    // and decides Incomplete/Blocked from its original facts.
                    assert_eq!(plan.action, Action::PublishReviewedCandidate { blocked: false });
                    assert_eq!(plan.update.audit, Audit::AcceptReport);
                }
            }
        }
    }

    #[test]
    fn completion_review_precedes_any_tools_and_reexecution_is_bounded() {
        let first = plan_model_completion(&Facts {
            final_answer_audit_required: true,
            has_visible_answer: true,
            ..Facts::default()
        });
        assert_eq!(first.update.audit, Audit::SaveDraft);
        assert_eq!(first.action, Action::DiscardAndRequestModel);
        for (count, budget) in [(0, false), (1, false), (2, false), (0, true)] {
            let plan = plan_model_completion(&Facts {
                final_answer_audit_required: true,
                final_answer_draft_present: true,
                has_visible_answer: true,
                normalized_audit_report_valid: true,
                audit_requires_execution: true,
                completion_recovery_count: count,
                iteration_budget_final_synthesis: budget,
                ..Facts::default()
            });
            if count >= 2 || budget {
                assert_eq!(plan.action, Action::ConcludeIncomplete { reason: nola_domain::ModelCompletionIncompleteReason::ExecutionRecoveryBudget });
            } else {
                assert_eq!(plan.update.audit, Audit::ResumeExecution);
                assert_eq!(plan.action, Action::DiscardAndRequestModel);
            }
        }
    }

    #[test]
    fn tool_free_calls_retry_without_creating_execution_obligations() {
        for audited in [false, true] {
            for count in 0..3 {
                let plan = plan_model_completion(&Facts {
                    tool_free_request: true,
                    has_tool_calls: true,
                    final_answer_retry_requested: true,
                    final_answer_draft_present: true,
                    final_answer_audit_report_present: audited,
                    no_progress_count: count,
                    ..Facts::default()
                });
                assert_eq!(plan.action, if count < 2 {
                    Action::DiscardAndRequestModel
                } else {
                    Action::DiscardAndFail(Failure::ToolsDuringTextOnlyResponse)
                });
                assert_eq!(plan.update.audit, Audit::Unchanged);
            }
        }
    }

    #[test]
    fn pending_requirements_go_to_core_arbitration_without_implicit_retries() {
        for (validation, recovery, skill) in [(true, false, false), (false, true, false), (false, false, true)] {
            for no_progress_count in [0, 2] {
                let facts = Facts {
                    output_limit_retry_requested: true,
                    continuation_made_no_progress: true,
                    has_visible_answer: true,
                    has_executed_tools: true,
                    validation_required: validation,
                    recovery_required: recovery,
                    skill_completion_pending: skill,
                    no_progress_count,
                    ..Facts::default()
                };
                let plan = plan_model_completion(&facts);
                assert_eq!(plan.action, Action::CommitCompletion { has_tool_calls: false });
                assert_eq!(plan.update.no_progress_count, None);
            }
        }
    }

    #[test]
    fn second_truncated_response_cannot_be_a_successful_continuation() {
        for no_progress in [false, true] {
            for has_tools in [false, true] {
                let facts = Facts {
                    output_limited: true,
                    output_limit_retry_requested: true,
                    continuation_made_no_progress: no_progress,
                    has_visible_answer: true,
                    has_tool_calls: has_tools,
                    ..Facts::default()
                };
                assert_eq!(plan_model_completion(&facts).action,
                    Action::ConcludeIncomplete { reason: nola_domain::ModelCompletionIncompleteReason::OutputLimitRecoveryExhausted });
            }
        }
    }

    #[test]
    fn repeated_output_limit_continuation_cannot_bypass_final_answer_audit() {
        let awaiting_report = Facts {
            output_limit_retry_requested: true,
            continuation_made_no_progress: true,
            has_visible_answer: true,
            final_answer_audit_required: true,
            final_answer_draft_present: true,
            no_progress_count: 2,
            ..Facts::default()
        };
        assert_eq!(plan_model_completion(&awaiting_report).action,
            Action::DiscardAndFail(Failure::InvalidAudit));
        let rejected = Facts {
            final_answer_audit_report_present: true,
            ..awaiting_report
        };
        assert_eq!(plan_model_completion(&rejected).action,
            Action::DiscardAndRequestModel);
        assert_eq!(plan_model_completion(&rejected).update.audit, Audit::ReviseDraft);
    }

    #[test]
    fn iteration_budget_preserves_truthful_checkpoint_without_bypassing_obligations() {
        let pending = Facts {
            iteration_budget_final_synthesis: true,
            has_visible_answer: true,
            validation_required: true,
            skill_completion_pending: true,
            ..Facts::default()
        };
        assert_eq!(
            plan_model_completion(&pending).action,
            Action::CommitCompletion { has_tool_calls: false }
        );

        let accepted = Facts {
            validation_required: false,
            skill_completion_pending: false,
            ..pending.clone()
        };
        assert_eq!(
            plan_model_completion(&accepted).action,
            Action::CommitCompletion { has_tool_calls: false }
        );
        let audited = Facts {
            has_executed_tools: true,
            final_answer_audit_required: true,
            ..accepted.clone()
        };
        let candidate = plan_model_completion(&audited);
        assert_eq!(candidate.action, Action::DiscardAndRequestModel);
        assert_eq!(candidate.update.audit, Audit::SaveDraft);
        assert_eq!(plan_model_completion(&Facts {
            has_tool_calls: true, ..audited.clone()
        }).action, Action::DiscardAndFail(Failure::IterationBudgetFinalSynthesis));
        assert_eq!(plan_model_completion(&Facts {
            validation_required: true, ..audited
        }).action, Action::DiscardAndRequestModel);

        let attempted_tool = Facts {
            has_tool_calls: true,
            ..pending.clone()
        };
        assert_eq!(
            plan_model_completion(&attempted_tool).action,
            Action::DiscardAndFail(Failure::IterationBudgetFinalSynthesis)
        );

        let empty = Facts {
            has_visible_answer: false,
            ..pending
        };
        assert_eq!(
            plan_model_completion(&empty).action,
            Action::DiscardAndFail(Failure::IterationBudgetFinalSynthesis)
        );
    }
    #[test]
    fn token_limited_uncommitted_tool_arguments_get_one_bounded_recovery() {
        let mut facts = Facts { output_limited: true, ..Default::default() };
        assert_eq!(plan_model_completion(&facts).action, Action::CommitAndRecoverOutputLimit);
        facts.output_limit_retry_requested = true;
        assert_eq!(plan_model_completion(&facts).action,
            Action::ConcludeIncomplete { reason: nola_domain::ModelCompletionIncompleteReason::OutputLimitRecoveryExhausted });
    }

    #[test]
    fn overlapping_completion_causes_obey_existing_priority() {
        let mut facts = Facts {
            has_visible_answer: true,
            structurally_repetitive: true,
            continuation_made_no_progress: true,
            output_limited: true,
            output_limit_retry_requested: true,
            user_denied_permission: true,
            final_answer_retry_requested: true,
            no_progress_count: 1,
            ..Default::default()
        };
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::DiscardAndFail(Failure::RepetitiveOutput)
        );
        facts.structurally_repetitive = false;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::ConcludeIncomplete { reason: nola_domain::ModelCompletionIncompleteReason::OutputLimitRecoveryExhausted }
        );
        facts.output_limit_retry_requested = false;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::CommitAndRecoverOutputLimit
        );
        facts.has_tool_calls = true;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::DiscardAndFail(Failure::ToolsAfterDenial)
        );
        facts.has_tool_calls = false;
        facts.has_visible_answer = false;
        facts.reasoning_in_items = true;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::DiscardAndFail(Failure::NoDenialExplanation)
        );
    }
    #[test]
    fn audit_and_synthesis_counters_have_precise_limits_and_reset() {
        let mut facts = Facts {
            has_visible_answer: true,
            final_answer_draft_present: true,
            final_answer_retry_requested: true,
            embedded_tool_protocol: true,
            ..Default::default()
        };
        for count in [0, 1, 2, u8::MAX] {
            facts.no_progress_count = count;
            let plan = plan_model_completion(&facts);
            assert_eq!(plan.update.no_progress_count, Some(count.saturating_add(1)));
            assert_eq!(
                plan.action,
                if count < 2 {
                    Action::DiscardAndRequestModel
                } else {
                    Action::DiscardAndFail(Failure::InvalidAudit)
                }
            );
        }
        facts.normalized_audit_report_valid = true;
        facts.audit_accepts_candidate = true;
        let plan = plan_model_completion(&facts);
        assert_eq!(plan.update.no_progress_count, Some(0));
        assert_eq!(plan.update.audit, Audit::AcceptReport);
        facts.final_answer_audit_report_present = true;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::DiscardAndFail(Failure::EmbeddedToolProtocol)
        );
        facts.embedded_tool_protocol = false;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::DiscardAndRequestModel
        );
        facts.final_answer_draft_present = false;
        facts.final_answer_audit_required = true;
        assert_eq!(plan_model_completion(&facts).update.audit, Audit::SaveDraft);
    }
    #[test]
    fn empty_reasoning_and_engineering_retries_remain_distinct() {
        let mut facts = Facts::default();
        assert!(
            plan_model_completion(&facts)
                .update
                .request_empty_continuation
        );
        facts.empty_completion_retry_requested = true;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::DiscardAndFail(Failure::EmptyCompletion)
        );
        facts.reasoning_in_items = true;
        assert!(
            plan_model_completion(&facts)
                .update
                .request_reasoning_continuation
        );
        facts.reasoning_only_retry_requested = true;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::DiscardAndFail(Failure::ReasoningOnly)
        );
        facts.reasoning_in_items = false;
        facts.has_executed_tools = true;
        assert!(plan_model_completion(&facts).update.request_final_answer);
        facts.final_answer_retry_requested = true;
        facts.no_progress_count = 2;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::DiscardAndFail(Failure::EmptyFinalSynthesis)
        );
        facts.validation_required = true;
        let plan = plan_model_completion(&facts);
        assert_eq!(
            plan.action,
            Action::CommitCompletion { has_tool_calls: false }
        );
        facts.recovery_required = true;
        facts.has_visible_answer = true;
        facts.skill_completion_pending = true;
        let plan = plan_model_completion(&facts);
        assert_eq!(
            plan.action,
            Action::CommitCompletion { has_tool_calls: false }
        );
        facts.has_tool_calls = true;
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::CommitCompletion {
                has_tool_calls: true
            }
        );
        facts = Facts {
            has_visible_answer: true,
            ..Default::default()
        };
        assert_eq!(
            plan_model_completion(&facts).action,
            Action::CommitCompletion {
                has_tool_calls: false
            }
        );
    }
}
