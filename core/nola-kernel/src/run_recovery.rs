// SPDX-License-Identifier: Apache-2.0
//! Recovery uses current validated facts, never an older Turn's terminal event.

use nola_domain::{
    AgentTurnStatus, RunRecoveryAction, RunRecoveryFacts, RunRecoveryRejection, TraceRecoveryFacts,
};

pub fn plan_run_recovery(facts: &RunRecoveryFacts) -> RunRecoveryAction {
    if facts.has_session_lease || !facts.has_canonical_run || facts.external_run {
        return RunRecoveryAction::Preserve;
    }
    if !facts.checkpoint_verified {
        return RunRecoveryAction::Reject(RunRecoveryRejection::UnverifiedCheckpoint);
    }
    let Some(status) = facts.turn_status else {
        return RunRecoveryAction::Reject(RunRecoveryRejection::MissingCurrentTurn);
    };
    if status == AgentTurnStatus::Idle {
        return RunRecoveryAction::Reject(RunRecoveryRejection::IdleCurrentTurn);
    }
    if facts.open_user_input_wait_count > 0 && !status.is_terminal() {
        return RunRecoveryAction::Preserve;
    }
    if facts.pending_effect_count > 0 {
        return RunRecoveryAction::Preserve;
    }
    if status.is_terminal() {
        return RunRecoveryAction::Reject(RunRecoveryRejection::MissingTerminalCommit);
    }
    RunRecoveryAction::Reject(RunRecoveryRejection::MissingPendingEffect)
}

pub fn preserve_recovery_trace(facts: &TraceRecoveryFacts) -> bool {
    facts.has_session_lease || facts.external_run || facts.matches_preserved_current_turn
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(status: Option<AgentTurnStatus>, pending: usize) -> RunRecoveryFacts {
        RunRecoveryFacts {
            has_session_lease: false,
            has_canonical_run: true,
            external_run: false,
            checkpoint_verified: true,
            turn_status: status,
            pending_effect_count: pending,
            open_user_input_wait_count: 0,
        }
    }

    #[test]
    fn durable_user_wait_needs_no_executable_effect() {
        let mut facts = local(Some(AgentTurnStatus::AwaitingInput), 0);
        facts.open_user_input_wait_count = 1;
        assert_eq!(plan_run_recovery(&facts), RunRecoveryAction::Preserve);
        facts.checkpoint_verified = false;
        assert_eq!(plan_run_recovery(&facts), RunRecoveryAction::Reject(RunRecoveryRejection::UnverifiedCheckpoint));
        facts.checkpoint_verified = true;
        facts.turn_status = Some(AgentTurnStatus::Completed);
        assert_eq!(plan_run_recovery(&facts), RunRecoveryAction::Reject(RunRecoveryRejection::MissingTerminalCommit));
    }

    #[test]
    fn all_current_statuses_require_consistent_pending_work() {
        for status in [
            AgentTurnStatus::Idle,
            AgentTurnStatus::AwaitingModel,
            AgentTurnStatus::AwaitingTools,
            AgentTurnStatus::AwaitingPermission,
            AgentTurnStatus::AwaitingInput,
            AgentTurnStatus::Compacting,
            AgentTurnStatus::Cancelling,
            AgentTurnStatus::Completed,
            AgentTurnStatus::Cancelled,
            AgentTurnStatus::Failed,
        ] {
            for pending in [0, 1, 4, usize::MAX] {
                let expected = if status == AgentTurnStatus::Idle {
                    RunRecoveryAction::Reject(RunRecoveryRejection::IdleCurrentTurn)
                } else if pending > 0 {
                    RunRecoveryAction::Preserve
                } else if status.is_terminal() {
                    RunRecoveryAction::Reject(RunRecoveryRejection::MissingTerminalCommit)
                } else {
                    RunRecoveryAction::Reject(RunRecoveryRejection::MissingPendingEffect)
                };
                assert_eq!(plan_run_recovery(&local(Some(status), pending)), expected);
                let mut invalid = local(Some(status), pending);
                invalid.checkpoint_verified = false;
                assert_eq!(
                    plan_run_recovery(&invalid),
                    RunRecoveryAction::Reject(RunRecoveryRejection::UnverifiedCheckpoint)
                );
            }
        }
        for pending in [0, 1] {
            assert_eq!(
                plan_run_recovery(&local(None, pending)),
                RunRecoveryAction::Reject(RunRecoveryRejection::MissingCurrentTurn)
            );
        }
    }

    #[test]
    fn live_external_and_absent_runs_are_never_settled_by_local_recovery() {
        for lease in [false, true] {
            for canonical in [false, true] {
                for external in [false, true] {
                    for verified in [false, true] {
                        let facts = RunRecoveryFacts {
                            has_session_lease: lease,
                            has_canonical_run: canonical,
                            external_run: external,
                            checkpoint_verified: verified,
                            ..local(Some(AgentTurnStatus::Completed), 0)
                        };
                        let expected = if lease || !canonical || external {
                            RunRecoveryAction::Preserve
                        } else if verified {
                            RunRecoveryAction::Reject(RunRecoveryRejection::MissingTerminalCommit)
                        } else {
                            RunRecoveryAction::Reject(RunRecoveryRejection::UnverifiedCheckpoint)
                        };
                        assert_eq!(plan_run_recovery(&facts), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn trace_requires_exact_preserved_turn_or_external_owner() {
        for lease in [false, true] {
            for external in [false, true] {
                for matches in [false, true] {
                    assert_eq!(
                        preserve_recovery_trace(&TraceRecoveryFacts {
                            has_session_lease: lease,
                            external_run: external,
                            matches_preserved_current_turn: matches,
                        }),
                        lease || external || matches
                    );
                }
            }
        }
    }
}
