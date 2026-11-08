// SPDX-License-Identifier: Apache-2.0
//! Command outcomes retain receipt identity across transport and commit uncertainty.

use nola_protocol::{CommandReceipt, CommandReceiptOutcome, ProtocolFailure, RevisionConflict};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApplicationServiceCommandError {
    OutcomeUnknown {
        client_id: String,
        request_id: String,
        code: String,
        message: String,
        reopen_required: bool,
    },
    Pending {
        client_id: String,
        request_id: String,
    },
    Rejected {
        client_id: String,
        request_id: String,
        error: ProtocolFailure,
    },
    Conflict {
        client_id: String,
        request_id: String,
        conflict: RevisionConflict,
    },
    Transport {
        client_id: String,
        request_id: String,
        message: String,
        /// False only when no command bytes could have reached the Service.
        outcome_unknown: bool,
    },
}

impl ApplicationServiceCommandError {
    pub fn client_id(&self) -> &str {
        match self {
            Self::OutcomeUnknown { client_id, .. }
            | Self::Pending { client_id, .. }
            | Self::Rejected { client_id, .. }
            | Self::Conflict { client_id, .. }
            | Self::Transport { client_id, .. } => client_id,
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::OutcomeUnknown { request_id, .. }
            | Self::Pending { request_id, .. }
            | Self::Rejected { request_id, .. }
            | Self::Conflict { request_id, .. }
            | Self::Transport { request_id, .. } => request_id,
        }
    }

    pub fn requires_reconciliation(&self) -> bool {
        matches!(
            self,
            Self::OutcomeUnknown { .. }
                | Self::Pending { .. }
                | Self::Transport {
                    outcome_unknown: true,
                    ..
                }
        )
    }

    /// Transport boundaries may add diagnostic context around an error.
    /// Decode only our structured outcome marker, never natural-language hints
    /// or the protocol's unrelated `recoverable` flag.
    pub fn unresolved_from_boundary_message(message: &str) -> Option<Self> {
        for marker in ["command_outcome_unknown: ", "command_outcome_pending: "] {
            if let Some((_, encoded)) = message.split_once(marker) {
                let error = serde_json::Deserializer::from_str(encoded)
                    .into_iter::<Self>()
                    .next()?
                    .ok()?;
                if error.requires_reconciliation() {
                    return Some(error);
                }
            }
        }
        None
    }

    pub(crate) fn transport(
        client_id: &str,
        request_id: &str,
        message: String,
        outcome_unknown: bool,
    ) -> Self {
        Self::Transport {
            client_id: client_id.into(),
            request_id: request_id.into(),
            message,
            outcome_unknown,
        }
    }
}

impl std::fmt::Display for ApplicationServiceCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.requires_reconciliation() {
            let marker = if matches!(self, Self::Pending { .. }) {
                "command_outcome_pending"
            } else {
                "command_outcome_unknown"
            };
            return write!(
                formatter,
                "{marker}: {}; request_id={}; reconcile the original receipt before retry",
                serde_json::to_string(self).map_err(|_| std::fmt::Error)?,
                self.request_id()
            );
        }
        match self {
            Self::Rejected { error, .. } => write!(formatter, "{}: {}", error.code, error.message),
            Self::Conflict { conflict, .. } => write!(
                formatter,
                "revision_conflict: {} expected revision {}, actual revision {}",
                conflict.scope_id, conflict.expected_revision, conflict.actual_revision
            ),
            Self::Transport { message, .. } => formatter.write_str(message),
            Self::OutcomeUnknown { .. } | Self::Pending { .. } => unreachable!(),
        }
    }
}

impl std::error::Error for ApplicationServiceCommandError {}

/// These are transaction/lifecycle protocol codes, not business rejections.
/// A false retry flag does not turn an uncertain commit into a terminal failure.
pub fn command_outcome_requires_reconciliation(code: &str) -> bool {
    nola_protocol::command_outcome_requires_owner_reopen(code)
}

pub(crate) fn unknown_receipt_error(
    receipt: &CommandReceipt,
) -> Option<ApplicationServiceCommandError> {
    let CommandReceiptOutcome::Rejected { error, .. } = &receipt.outcome else {
        return None;
    };
    command_outcome_requires_reconciliation(&error.code).then(|| {
        ApplicationServiceCommandError::OutcomeUnknown {
            client_id: receipt.client_id.clone(),
            request_id: receipt.request_id.clone(),
            code: error.code.clone(),
            message: error.message.clone(),
            reopen_required: true,
        }
    })
}

pub(crate) fn decode_committed_result<T: serde::de::DeserializeOwned>(
    client_id: &str, request_id: &str, value: Value,
) -> Result<T, ApplicationServiceCommandError> {
    serde_json::from_value(value).map_err(|error| ApplicationServiceCommandError::OutcomeUnknown {
        client_id: client_id.into(), request_id: request_id.into(),
        code: "committed_result_decode_failed".into(),
        message: format!("decode committed Application result failed: {error}"),
        reopen_required: true,
    })
}

pub(crate) fn receipt_result_typed(
    receipt: CommandReceipt,
) -> Result<Value, ApplicationServiceCommandError> {
    if let Some(error) = unknown_receipt_error(&receipt) {
        return Err(error);
    }
    match receipt.outcome {
        CommandReceiptOutcome::Committed { result, .. } => Ok(result),
        CommandReceiptOutcome::Rejected { error, .. } => {
            Err(ApplicationServiceCommandError::Rejected {
                client_id: receipt.client_id,
                request_id: receipt.request_id,
                error,
            })
        }
        CommandReceiptOutcome::Conflict { conflict, .. } => {
            Err(ApplicationServiceCommandError::Conflict {
                client_id: receipt.client_id,
                request_id: receipt.request_id,
                conflict,
            })
        }
        CommandReceiptOutcome::Accepted => Err(ApplicationServiceCommandError::Pending {
            client_id: receipt.client_id,
            request_id: receipt.request_id,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_committed_result_never_becomes_a_safe_pre_send_failure() {
        let error = decode_committed_result::<String>("client", "original-write", serde_json::json!({"unexpected": true})).unwrap_err();
        assert!(error.requires_reconciliation());
        assert_eq!(error.client_id(), "client");
        assert_eq!(error.request_id(), "original-write");
        assert!(matches!(error, ApplicationServiceCommandError::OutcomeUnknown { reopen_required: true, .. }));
        assert_eq!(decode_committed_result::<String>("client", "original-write", serde_json::json!("valid")).unwrap(), "valid");
    }

    fn rejection(code: &str, recoverable: bool) -> CommandReceipt {
        CommandReceipt {
            protocol_version: nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
            request_id: "original-request".into(),
            client_id: "original-client".into(),
            received_at: "1".into(),
            outcome: CommandReceiptOutcome::Rejected {
                completed_at: "2".into(),
                error: ProtocolFailure {
                    code: code.into(),
                    message: "fixture error".into(),
                    recoverable,
                    details: None,
                },
            },
        }
    }

    #[test]
    fn transaction_uncertainty_preserves_identity_even_when_not_retryable() {
        for code in [
            "command_transaction_commit_outcome_unknown",
            "runtime_adoption_failed",
            "runtime_reopen_required",
            "command_transaction_rollback_failed",
        ] {
            let error = receipt_result_typed(rejection(code, false)).unwrap_err();
            assert!(matches!(
                error,
                ApplicationServiceCommandError::OutcomeUnknown { .. }
            ));
            assert_eq!(error.request_id(), "original-request");
            assert_eq!(error.client_id(), "original-client");
            let presented = format!("Agent Core failed: {error}");
            assert_eq!(
                ApplicationServiceCommandError::unresolved_from_boundary_message(&presented),
                Some(error)
            );
        }
    }

    #[test]
    fn ordinary_rejection_cannot_be_reclassified_by_retry_flag_or_prose() {
        let error = receipt_result_typed(rejection("application_rejected", true)).unwrap_err();
        assert!(!error.requires_reconciliation());
        assert!(
            ApplicationServiceCommandError::unresolved_from_boundary_message(
                "Search current state; commit outcome unknown; request_id=ambiguous"
            )
            .is_none()
        );
        assert!(
            ApplicationServiceCommandError::unresolved_from_boundary_message(&error.to_string())
                .is_none()
        );
    }

    #[test]
    fn only_transport_after_send_requires_reconciliation() {
        let before = ApplicationServiceCommandError::transport(
            "client",
            "request",
            "connect failed".into(),
            false,
        );
        let after = ApplicationServiceCommandError::transport(
            "client",
            "request",
            "response lost".into(),
            true,
        );
        assert!(!before.requires_reconciliation());
        assert!(after.requires_reconciliation());
        assert_eq!(
            ApplicationServiceCommandError::unresolved_from_boundary_message(&after.to_string()),
            Some(after)
        );
    }
}
