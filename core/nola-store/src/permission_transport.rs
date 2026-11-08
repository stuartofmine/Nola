// SPDX-License-Identifier: Apache-2.0
//! Validate guidance before consuming a Host's pending response transport.
use super::*;
use nola_domain::PermissionInstruction;
use nola_store_port::permission::StoredPermissionResolutionOutcome;

/// Admission and insertion must share the same write transaction as cancellation.
/// Replay of an existing occurrence is handled before this gate, never revived.
pub(super) fn require_admission(transaction: &Connection, task_id: &str) -> StoreResult<()> {
    if transaction.is_autocommit() {
        return Err(StoreError::Invariant("permission admission requires a write transaction".into()));
    }
    let admitted: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE id=?1 AND cancel_requested=0
         AND status IN ('queued','running','retry_waiting','waiting_permission','suspended'))",
        params![task_id], |row| row.get(0),
    )?;
    if !admitted {
        return Err(StoreError::Conflict("task cannot admit a new permission request".into()));
    }
    Ok(())
}

/// Revoke only grants derived from these exact stored permission requests.
/// Independent Host/global grants and historical user decisions are untouched.
pub(super) fn revoke_task_request_grants(
    transaction: &Connection, task_id: &str, at: i64,
) -> StoreResult<()> {
    transaction.execute(
        "UPDATE permission_grants SET revoked_at_ms=?2,updated_at_ms=?2
         WHERE revoked_at_ms IS NULL AND scope='host' AND host_id='task:' || ?1
           AND id IN (SELECT 'permission-request:' || request_id || ':task-scope'
                      FROM permission_requests WHERE task_id=?1)",
        params![task_id, at],
    )?;
    Ok(())
}

pub(super) fn context(
    request: &StoredPermissionRequest,
) -> StoreResult<Vec<PermissionInstruction>> {
    if request.permission_instructions_json.len() > 131072 {
        return Err(StoreError::Invariant(
            "permission instruction metadata exceeds its storage budget".into(),
        ));
    }
    let history: Vec<PermissionInstruction> =
        serde_json::from_str(&request.permission_instructions_json)?;
    if request
        .core_permission_request_id
        .as_deref()
        .is_some_and(|id| id.trim().is_empty())
        || (request.core_permission_request_id.is_none() && !history.is_empty())
    {
        return Err(StoreError::Invariant(
            "permission instruction metadata requires a real Core request identity".into(),
        ));
    }
    for (index, entry) in history.iter().enumerate() {
        require_id(
            "permission_instruction.permission_request_id",
            &entry.permission_request_id,
        )?;
        PermissionInstruction::validate_append(
            &history[..index],
            &entry.permission_request_id,
            &entry.instruction,
        )
        .map_err(StoreError::Invariant)?;
    }
    Ok(history)
}

pub(super) fn resolve(
    store: &mut NolaStore,
    request_id: &str,
    decision: &str,
    custom_response: Option<&str>,
    at: i64,
    grant: Option<&UpsertPermissionGrantInput>,
) -> StoreResult<StoredPermissionResolutionOutcome> {
    require_id("permission_request.request_id", request_id)?;
    if !matches!(
        decision,
        "allow_once" | "allow_conversation" | "return_to_composer" | "provide_instruction" | "deny"
    ) {
        return Err(StoreError::Invariant(format!(
            "invalid permission request decision: {decision}"
        )));
    }
    let transaction = store.write_transaction()?;
    let current = transaction
        .query_row(
            &format!(
                "SELECT {PERMISSION_REQUEST_COLUMNS} FROM permission_requests WHERE request_id=?1"
            ),
            params![request_id],
            permission_request_row,
        )
        .optional()?;
    let Some(mut current) = current else {
        transaction.commit()?;
        return Ok(StoredPermissionResolutionOutcome::Missing { request_id: request_id.into() });
    };
    if let Some(closure) = &current.closure {
        closure.validate_for(&nola_domain::permission_closure::PermissionClosureRequest {
            request_id: &current.request_id, task_id: &current.task_id,
            child_session_id: &current.child_session_id,
            core_permission_request_id: current.core_permission_request_id.as_deref(),
            created_at_ms: current.created_at_ms, resolved_at_ms: current.resolved_at_ms,
        }).map_err(|error| StoreError::Invariant(error.to_string()))?;
        if current.status != "withdrawn" {
            return Err(StoreError::Invariant("closed permission has an invalid projection".into()));
        }
        transaction.commit()?;
        return Ok(StoredPermissionResolutionOutcome::Closed(current));
    }
    // Arbitrate against stop intent inside the same transaction that would
    // publish a decision and its derived grant. A UI preflight cannot fence it.
    let stopping: Option<Option<i64>> = transaction.query_row(
        "SELECT cancel_requested_at_ms FROM durable_tasks WHERE id=?1 AND cancel_requested=1",
        [&current.task_id], |row| row.get(0),
    ).optional()?;
    if let Some(requested_at_ms) = stopping {
        let requested_at_ms = requested_at_ms.filter(|at| *at >= 0)
            .ok_or_else(|| StoreError::Invariant("task stop intent lacks its timestamp".into()))?;
        transaction.commit()?;
        return Ok(StoredPermissionResolutionOutcome::StopPending {
            request_id: current.request_id, task_id: current.task_id, requested_at_ms,
        });
    }
    let history = context(&current)?;
    let custom_response = custom_response.map(str::trim);
    if decision != "provide_instruction" && custom_response.is_some() {
        return Err(StoreError::Invariant("authorization cannot contain an instruction".into()));
    }
    if current.status == "resolved" {
        if current.decision.is_none() || current.resolved_at_ms.is_none() {
            return Err(StoreError::Invariant("resolved permission is missing its decision".into()));
        }
        let same = current.decision.as_deref() == Some(decision)
            && current.custom_response.as_deref() == custom_response;
        transaction.commit()?;
        return Ok(if same {
            StoredPermissionResolutionOutcome::AlreadyResolved(current)
        } else {
            StoredPermissionResolutionOutcome::Conflict(current)
        });
    }
    if current.status != "pending" || current.decision.is_some() || current.custom_response.is_some() || current.resolved_at_ms.is_some() {
        return Err(StoreError::Invariant("invalid pending permission state".into()));
    }
    if decision == "provide_instruction" {
        let core_id = current.core_permission_request_id.as_deref().ok_or_else(|| StoreError::Invariant("custom instruction requires a Core-bound permission request; approve or deny this legacy request".into()))?;
        PermissionInstruction::validate_append(
            &history,
            core_id,
            custom_response.unwrap_or_default(),
        )
        .map_err(StoreError::Invariant)?;
    }
    if (decision == "allow_conversation") != grant.is_some() {
        return Err(StoreError::Invariant("scope resolution requires its atomic derived grant".into()));
    }
    if let Some(grant) = grant {
        let task_child: Option<Option<String>> = transaction.query_row(
            "SELECT child_session_id FROM durable_tasks WHERE id=?1", params![current.task_id],
            |row| row.get(0),
        ).optional()?;
        if task_child.flatten().as_deref() != Some(current.child_session_id.as_str()) {
            return Err(StoreError::Invariant("permission scope does not belong to the original task child".into()));
        }
        if decision != "allow_conversation"
            || grant.id != format!("permission-request:{request_id}:task-scope")
            || grant.created_at_ms != at || grant.updated_at_ms != at
            || grant.scope != PermissionGrantScope::Host
            || grant.host_id.as_deref() != Some(format!("task:{}", current.task_id).as_str())
            || grant.conversation_id.is_some()
            || grant.capability_pattern != "cli::tool::allow_rule"
            || grant.resource_pattern != current.scope_rule
            || grant.decision != StoredPermissionDecision::Allow
        {
            return Err(StoreError::Invariant("permission grant does not match original request scope".into()));
        }
        // First-decision insertion only. Same-decision replay returned above;
        // never use the generic upsert path to revive a revoked derived grant.
        transaction.execute(
            "INSERT INTO permission_grants(id,scope,conversation_id,host_id,capability_pattern,
                resource_pattern,decision,created_at_ms,updated_at_ms,revoked_at_ms)
             VALUES(?1,'host',NULL,?2,?3,?4,'allow',?5,?5,NULL)",
            params![grant.id,grant.host_id,grant.capability_pattern,grant.resource_pattern,at],
        )?;
        // Persist the user's selection for this occurrence, not new decisions
        // for sibling effects. scope_rule is Host presentation data, not a
        // Core-issued authorization proof. Core alone evaluates reuse and
        // closes any covered waits through their canonical lifecycle.
    }
    let changed = transaction.execute("UPDATE permission_requests SET status='resolved',decision=?2,custom_response=?3,resolved_at_ms=?4 WHERE request_id=?1 AND status='pending'", params![request_id,decision,custom_response,at])?;
    if changed != 1 {
        return Err(StoreError::Invariant("permission resolution lost its transaction owner".into()));
    }
    current.status = "resolved".into();
    current.decision = Some(decision.into());
    current.custom_response = custom_response.map(str::to_owned);
    current.resolved_at_ms = Some(at);
    transaction.commit()?;
    Ok(StoredPermissionResolutionOutcome::Resolved(current))
}

pub(super) fn ensure(
    store: &mut NolaStore,
    request: &StoredPermissionRequest,
) -> StoreResult<StoredPermissionRequest> {
    validate_permission_request(request)?;
    if request.status != "pending" {
        return Err(StoreError::Invariant(
            "ensure candidate must be pending".into(),
        ));
    }
    let core_id = request
        .core_permission_request_id
        .as_deref()
        .ok_or_else(|| StoreError::Invariant("ensure requires a Core request ID".into()))?;
    let transaction = store.write_transaction()?;
    let existing = transaction.query_row(&format!("SELECT {PERMISSION_REQUEST_COLUMNS} FROM permission_requests WHERE task_id=?1 AND core_permission_request_id=?2"), params![request.task_id, core_id], permission_request_row).optional()?;
    if let Some(existing) = existing {
        if existing.child_session_id != request.child_session_id
            || existing.capability != request.capability
            || existing.scope_rule != request.scope_rule
            || serde_json::from_str::<serde_json::Value>(&existing.input_json)?
                != serde_json::from_str::<serde_json::Value>(&request.input_json)?
            || context(&existing)? != context(request)?
        {
            return Err(StoreError::Invariant(
                "Core permission identity was reused with conflicting authority or payload".into(),
            ));
        }
        transaction.commit()?;
        return Ok(existing);
    }
    require_admission(&transaction, &request.task_id)?;
    transaction.execute("INSERT INTO permission_requests(request_id,task_id,child_session_id,capability,input_json,scope_rule,status,decision,custom_response,created_at_ms,resolved_at_ms,core_permission_request_id,permission_instructions_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)", params![request.request_id,request.task_id,request.child_session_id,request.capability,request.input_json,request.scope_rule,request.status,request.decision,request.custom_response,request.created_at_ms,request.resolved_at_ms,request.core_permission_request_id,request.permission_instructions_json])?;
    permission_source::retain_bound_source(&transaction, request)?;
    transaction.commit()?;
    Ok(request.clone())
}
