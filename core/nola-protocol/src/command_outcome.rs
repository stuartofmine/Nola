// SPDX-License-Identifier: Apache-2.0
//! Exact transaction/lifecycle codes shared by producer and transport clients.
//! These codes describe owner retirement, never a business authorization result.
pub fn command_outcome_requires_owner_reopen(code: &str) -> bool {
    matches!(
        code,
        "command_transaction_commit_outcome_unknown"
            | "runtime_reconciliation_commit_outcome_unknown"
            | "command_transaction_commit_failed"
            | "runtime_transaction_commit_failed"
            | "runtime_adoption_failed"
            | "runtime_reopen_required"
            | "reopen_required"
            | "command_transaction_rollback_failed"
            | "command_savepoint_rollback_failed"
            | "command_savepoint_release_failed"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_retirement_is_not_inferred_from_generic_retry_flags_or_business_errors() {
        assert!(command_outcome_requires_owner_reopen(
            "command_savepoint_release_failed"
        ));
        assert!(command_outcome_requires_owner_reopen(
            "command_savepoint_rollback_failed"
        ));
        assert!(command_outcome_requires_owner_reopen(
            "command_transaction_commit_outcome_unknown"
        ));
        assert!(!command_outcome_requires_owner_reopen(
            "application_rejected"
        ));
        assert!(!command_outcome_requires_owner_reopen(
            "command_savepoint_begin_failed"
        ));
        assert!(!command_outcome_requires_owner_reopen("permission_denied"));
    }
}
