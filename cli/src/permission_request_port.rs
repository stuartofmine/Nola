use nola_application::{
    ApplicationCommand, NewPermissionRequest, PermissionAuthorizationDecision,
    PermissionIntervention, PermissionRequest, PermissionRequestCommand, PermissionResolutionOutcome,
};

pub fn enqueue_permission(request: NewPermissionRequest) -> Result<(), String> {
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let _: bool = application.execute(ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::Enqueue { input: request },
    })?;
    Ok(())
}

pub fn permission_request(request_id: &str) -> Result<Option<PermissionRequest>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Get {
                request_id: request_id.to_string(),
            },
        },
    )
}

pub fn pending_permission_for_task(task_id: &str) -> Result<Option<PermissionRequest>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::PendingForTask {
                task_id: task_id.to_string(),
            },
        },
    )
}

pub fn pending_permissions_for_owner(
    owner_session_id: &str,
) -> Result<Vec<PermissionRequest>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::PendingForOwner {
                owner_session_id: owner_session_id.to_string(),
            },
        },
    )
}

pub fn authorize_permission(
    request_id: &str,
    decision: PermissionAuthorizationDecision,
    resolved_at_ms: i64,
) -> Result<PermissionResolutionOutcome, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Authorize {
                request_id: request_id.to_string(),
                decision,
                resolved_at_ms,
            },
        },
    )
}

pub fn intervene_permission(
    request_id: &str,
    intervention: PermissionIntervention,
    resolved_at_ms: i64,
) -> Result<PermissionResolutionOutcome, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Intervene {
                request_id: request_id.to_string(),
                intervention,
                resolved_at_ms,
            },
        },
    )
}

pub fn ensure_core_permission(request: NewPermissionRequest) -> Result<PermissionRequest, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::EnsureCoreRequest { input: request },
        },
    )
}

/// A false value here means an explicitly confirmed same-decision replay,
/// never a missing request or conflict. The public transport stays typed.
pub(crate) fn validate_resolution(
    outcome: PermissionResolutionOutcome, expected: &PermissionRequest,
) -> Result<bool, String> {
    let (receipt, first) = match outcome {
        PermissionResolutionOutcome::Closed { .. } => return Err("permission request is closed; execution cannot resume".into()),
        PermissionResolutionOutcome::Resolved { receipt } => (receipt, true),
        PermissionResolutionOutcome::AlreadyResolved { receipt } => (receipt, false),
        PermissionResolutionOutcome::Conflict { .. } => return Err("permission request already has a different decision".into()),
        PermissionResolutionOutcome::Missing { .. } => return Err("permission request is missing".into()),
        PermissionResolutionOutcome::StopPending { .. } => return Err("task cancellation is pending; permission was not changed".into()),
    };
    if receipt.request_id != expected.request_id || receipt.task_id != expected.task_id
        || receipt.child_session_id != expected.child_session_id
        || receipt.core_permission_request_id != expected.core_permission_request_id
    {
        return Err("permission resolution receipt changed request ownership".into());
    }
    Ok(first)
}

#[cfg(test)]
mod resolution_tests {
    use super::*;

    #[test]
    fn only_typed_matching_resolution_can_acknowledge_a_choice() {
        let expected = PermissionRequest {
            closure: None,
            request_id: "request".into(), task_id: "task".into(), child_session_id: "child".into(),
            capability: "write_file".into(), input: serde_json::json!({}), scope_rule: "scope".into(),
            status: nola_application::PermissionRequestStatus::Pending, resolution: None,
            created_at_ms: 1, resolved_at_ms: None, core_permission_request_id: Some("core".into()),
            permission_instructions: vec![],
        };
        let receipt = nola_application::PermissionResolutionReceipt {
            request_id: expected.request_id.clone(), task_id: expected.task_id.clone(),
            child_session_id: expected.child_session_id.clone(), core_permission_request_id: expected.core_permission_request_id.clone(),
            resolution: nola_application::PermissionRequestResolution::Authorization { decision: PermissionAuthorizationDecision::Deny },
            resolved_at_ms: 2,
        };
        assert!(validate_resolution(PermissionResolutionOutcome::Resolved { receipt: receipt.clone() }, &expected).unwrap());
        assert!(!validate_resolution(PermissionResolutionOutcome::AlreadyResolved { receipt: receipt.clone() }, &expected).unwrap());
        assert!(validate_resolution(PermissionResolutionOutcome::Conflict { receipt: receipt.clone() }, &expected).is_err());
        assert!(validate_resolution(PermissionResolutionOutcome::Missing { request_id: expected.request_id.clone() }, &expected).is_err());
        let mut wrong_owner = receipt;
        wrong_owner.child_session_id = "other-child".into();
        assert!(validate_resolution(PermissionResolutionOutcome::Resolved { receipt: wrong_owner }, &expected).is_err());
    }
}
