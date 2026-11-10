// SPDX-License-Identifier: Apache-2.0
// An exact parent-owned view of a child permission. No grants are inferred
// from a task status, display label, remote client, or provider configuration.

#[derive(Clone, Debug)]
pub(crate) struct ParentTaskPermission {
    pub prompt: TaskPermissionPrompt,
    pub source: nola_core::agent::permissions::PendingPermissionRequest,
    pub owner_session_id: String,
    pub owner_turn_id: String,
    pub owner_run_id: String,
    project_root: String,
}

impl ParentTaskPermission {
    pub fn request(&self) -> &nola_application::PermissionRequest {
        &self.prompt.permission
    }
}

pub(crate) fn permission_for_parent_task(
    owner_session_id: &str,
    owner_turn_id: &str,
    task_id: &str,
) -> Result<Option<ParentTaskPermission>, String> {
    let Some(projection) = session::load_session_task_projection(owner_session_id)? else {
        return Ok(None);
    };
    let Some(handle) = projection.tasks.get(task_id) else {
        return Ok(None);
    };
    let Some(authority) = crate::runtime_task_port::agent_authority(task_id)? else {
        return Ok(None);
    };
    authority.validate()?;
    let Some(admission) = authority.admission.as_ref().filter(|admission| {
        authority.phase
            == nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Admitted
            && admission.parent_conversation_id == owner_session_id
            && admission.parent_turn_id == owner_turn_id
    }) else {
        return Ok(None);
    };
    if handle.owner_session_id.as_str() != admission.parent_core_session_id
        || handle.owner_turn_id.as_str() != owner_turn_id
        || handle.state.is_terminal()
        || handle.cancel_requested
    {
        return Ok(None);
    }
    let Some(task) = NolaStore::open_default()?.runtime_task(task_id)? else {
        return Ok(None);
    };
    if task.owner_session_id.as_deref() != Some(owner_session_id)
        || task.status != "waiting_permission"
        || task.cancel_requested
    {
        return Ok(None);
    }
    let Some(permission) = crate::permission_request_port::pending_permission_for_task(task_id)?
    else {
        return Ok(None);
    };
    if permission.child_session_id != admission.child_conversation_id
        || permission.task_id != task_id
        || task.child_session_id.as_deref() != Some(permission.child_session_id.as_str())
        || permission.status != nola_application::PermissionRequestStatus::Pending
    {
        return Ok(None);
    }
    let Some(core_id) = permission.core_permission_request_id.as_deref() else {
        return Ok(None);
    };
    let Some(child) = session::load_session_permission_projection(&permission.child_session_id)?
    else {
        return Ok(None);
    };
    let Some(source) = child.pending.into_iter().find(|source| {
        source.request_id.as_str() == core_id
            && source.arguments == permission.input
            && source.capability_id == permission.capability
            && source.permission_instructions == permission.permission_instructions
    }) else {
        return Ok(None);
    };
    let mut prompt = render_task_permission_prompt(&task, permission);
    prompt.prompt.allow_conversation = source.allow_conversation;
    Ok(Some(ParentTaskPermission {
        prompt,
        source,
        owner_session_id: owner_session_id.into(),
        owner_turn_id: owner_turn_id.into(),
        owner_run_id: admission.parent_run_id.clone(),
        project_root: task.project_root,
    }))
}

pub(crate) fn resolve_parent_task_permission(
    expected: &ParentTaskPermission,
    choice: PermissionChoice,
) -> Result<(), String> {
    let current = permission_for_parent_task(
        &expected.owner_session_id,
        &expected.owner_turn_id,
        &expected.request().task_id,
    )?
    .ok_or("The delegated permission changed or closed")?;
    if current.owner_run_id != expected.owner_run_id
        || current.request() != expected.request()
        || current.source != expected.source
        || current.project_root != expected.project_root
    {
        return Err(
            "The displayed delegated permission no longer matches its exact owner and request"
                .into(),
        );
    }
    let outcome = resolve_task_permission_in_scope(
        &expected.project_root,
        Some(&expected.owner_session_id),
        &expected.prompt,
        choice,
    );
    if outcome.error {
        Err(outcome.display)
    } else {
        Ok(())
    }
}
