// SPDX-License-Identifier: Apache-2.0
//! Native stdin execution slice. The caller supplies an already composed launch
//! request; no CLI Config, command parsing, terminal renderer or dispatcher.
use std::io::{BufRead, Read};
use crate::host_invocation_request::HostInvocationRequest;

pub(crate) fn run_stdin(mut request: HostInvocationRequest) -> Result<(), String> {
    let payload = read_payload()?;
    crate::native_host_request::apply_desktop_host_request(&mut request, &payload)?;
    execute(request)
}

pub(crate) fn read_payload() -> Result<String, String> {
    if std::env::var_os("NOLA_DESKTOP_HOST").is_none() {
        return Err("--host-request-stdin is restricted to the desktop Agent Host".into());
    }
    // Bound the actual read, not merely the allocation after reading an
    // arbitrarily large frame. The remaining stdin belongs to host control.
    let mut bytes = Vec::new();
    std::io::stdin().lock().take(4 * 1024 * 1024 + 1).read_until(b'\n', &mut bytes)
        .map_err(|error| format!("failed to read desktop Agent Host request: {error}"))?;
    let payload = String::from_utf8(bytes)
        .map_err(|_| "desktop Agent Host request is not UTF-8")?;
    if payload.len() > 4 * 1024 * 1024 {
        return Err("desktop Agent Host request exceeds the 4 MB limit".into());
    }
    Ok(payload)
}

fn apply_tool_receipt(request: &mut HostInvocationRequest) -> Result<(), String> {
    if !request.tool_settings.tools_enabled { return Ok(()); }
    let encoded = std::env::var("NOLA_CORE_TOOL_POLICY_RECEIPT")
        .map_err(|_| "Agent Host tool policy receipt is missing.")?;
    let policy_digest = std::env::var("NOLA_CORE_TOOL_POLICY_DIGEST")
        .map_err(|_| "Agent Host tool policy digest is missing.")?;
    let manifest_digest = std::env::var("NOLA_CORE_TOOL_MANIFEST_DIGEST")
        .map_err(|_| "Agent Host tool manifest digest is missing.")?;
    let session = request.session.session_id.as_deref().ok_or("Agent Host tool policy requires a session identity.")?;
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis() as u64;
    request.tool_settings.tool_allowlist = Some(crate::native_tool_policy::decode_desktop_host_tool_allowlist(
        &encoded, &policy_digest, &manifest_digest, session, now_ms,
    )?);
    request.tool_settings.tool_allowlist_is_authorization_only = true;
    Ok(())
}

pub(crate) fn execute(mut request: HostInvocationRequest) -> Result<(), String> {
    let mut output = crate::native_runtime_output::NativeRuntimeOutput::new();
    if let Some(cancel) = &request.session.desktop_cancel {
        // Cancellation never initializes provider/tool adapters or drains work.
        return crate::core_agent_runtime::cancel_agent_core_expected(
            &request, &mut output, &cancel.expected_run_id,
            &cancel.assistant_message_id, &cancel.cancellation_id,
        );
    }
    if request.session.desktop_goal_wake.is_some() || request.session.desktop_goal_control.is_some() {
        let session = request.session.session_id.as_deref().ok_or("Goal requires conversation identity")?;
        crate::desktop_host_control::bind_session_id(session)?;
        crate::desktop_host_control::start_if_needed()?;
    }
    let goal_wake = request.session.desktop_goal_wake.take();
    let scheduled_goal = goal_wake.is_some();
    let native_goal = request.session.desktop_goal_control.take();
    let mut goal_execution = None;
    if let Some(goal_id) = goal_wake {
        let session = request.session.session_id.clone().ok_or("Goal wake requires conversation identity")?;
        request = crate::goal_runtime::restore_goal_execution_request(&session, &goal_id)?;
        let goal = crate::session::load_session_goal_projection(&session)?
            .and_then(|projection| projection.active).filter(|goal| goal.id == goal_id)
            .ok_or("Goal wake identity changed")?;
        goal_execution = Some((goal.objective, goal.constraints.join("\n")));
    } else if let Some(input) = native_goal {
        use nola_application::goal_control_intent::GoalControlAction;
        let session = input.conversation_id.clone();
        let starts = matches!(input.action, GoalControlAction::Start);
        let resumes = matches!(input.action, GoalControlAction::Resume);
        if starts {
            apply_tool_receipt(&mut request)?;
            let envelope = crate::application_port::CliApplicationPort::persistent_default()?
                .prepare_goal_control_intent(input)?;
            crate::core_agent_runtime::submit_agent_core_goal_command(&request, envelope)?;
        } else {
            let applied = crate::goal_control_port::submit_existing(input)?;
            // An active owner will consume queued controls. Do not start a
            // competing coordinator or claim an unconfirmed Resume as applied.
            if !applied || !resumes { return Ok(()); }
        }
        let goal = crate::session::load_session_goal_projection(&session)?
            .and_then(|projection| projection.active).ok_or("Goal is unavailable")?;
        request = crate::goal_runtime::restore_goal_execution_request(&session, &goal.id)?;
        goal_execution = Some((goal.objective, goal.constraints.join("\n")));
    } else {
        apply_tool_receipt(&mut request)?;
    }
    if request.session.desktop_resume.is_none() {
        crate::session_process_observation::cleanup_stale_session_run_locks();
    }
    let session_id = request.session.session_id.clone()
        .filter(|id| !id.trim().is_empty()).ok_or("native execution requires an explicit session id")?;
    let session = session_id.as_str();
    crate::desktop_host_control::bind_session_id(session)?;
    crate::desktop_host_control::start_if_needed()?;
    let _permission_session = crate::tools::enter_host_permission_session(session);
    if std::env::var_os("NOLA_MODELS_ROOT").is_none() {
        if let Ok(root) = crate::local_models::models_root() {
            std::env::set_var("NOLA_MODELS_ROOT", root);
        }
    }
    // Runtime providers remain explicit request data. Environment projection is
    // retained for existing native helpers, not read back to select a route.
    for (key, value) in [
        ("NOLA_LLM_ENDPOINT", request.model_settings.endpoint.as_str()),
        ("NOLA_LLM_API_KEY", request.model_settings.api_key.as_str()),
        ("NOLA_LLM_MODEL", request.model_settings.model.as_str()),
        ("NOLA_LLM_PROVIDER", request.runtime.provider.as_deref().unwrap_or_default()),
    ] {
        if value.trim().is_empty() { std::env::remove_var(key); }
        else { std::env::set_var(key, value.trim()); }
    }
    std::env::remove_var("NOLA_CLI_LLM_PROVIDER");
    let cancellation = crate::runtime_control::RuntimeExecutionControl::default();
    let _signals = crate::execution_signals::ExecutionSignals::install(
        cancellation.clone(), Some(crate::desktop_host_control::interrupt_pending_waits),
    )?;
    let result = (|| {
        if let Some(resume) = &request.session.desktop_resume {
            crate::native_host_request::validate_desktop_resume_identity(
                &mut crate::application_port::CliApplicationPort::persistent_default()?, resume,
            )?;
            let goal = crate::session::load_session_goal_projection(session)?.and_then(|p| p.active);
            if let Some(goal) = goal.filter(|goal| !goal.status.is_terminal()
                && goal.contracts.last().is_some_and(|contract| contract.status == nola_core::agent::GoalContractStatus::Executing)) {
                let supervisor = crate::runtime_effect_supervisor::CliRuntimeEffectSupervisor::start_if_persistent(&request)?;
                let result = crate::goal_runtime::run_host_goal(&mut request, "", &goal.objective,
                    &goal.constraints.join("\n"), &mut output, cancellation.clone()).map(|_| ());
                let shutdown = supervisor.map(crate::runtime_effect_supervisor::CliRuntimeEffectSupervisor::shutdown)
                    .transpose().map(|_| ());
                return result.and(shutdown);
            }
            let outcome = crate::core_agent_runtime::resume_agent_core_expected_with_cancellation(
                &request, &mut output, cancellation.clone(), &resume.expected_run_id,
            )?;
            let outcome = outcome.ok_or("expected Run is no longer resumable")?;
            return validate_completion(outcome.turn_status, cancellation.is_requested());
        }
        let goal_control = match goal_execution { Some(value) => Some(value), None => crate::goal_runtime::host_goal_control(&request)? };
        let supervisor = crate::runtime_effect_supervisor::CliRuntimeEffectSupervisor::start_if_persistent(&request)?;
        let result = if let Some((objective, constraints)) = goal_control {
            // The typed Host control uses the same coordinator as CLI and Web;
            // GoalRuntime projection commands alone do not execute a round.
            if scheduled_goal {
                crate::goal_runtime::run_scheduled_host_goal(&mut request, &objective, &constraints,
                    &mut output, cancellation.clone()).map(|_| ())
            } else {
                crate::goal_runtime::run_host_goal(&mut request, "", &objective, &constraints,
                    &mut output, cancellation.clone()).map(|_| ())
            }
        } else { (|| {
            if let Some(command_id) = request.session.queued_user_command_id.as_deref() {
                let now = nola_runtime_driver::monotonic_epoch_millis();
                let owner = format!("mac-ui:{}", uuid::Uuid::new_v4());
                let lease = crate::agent_command_port::claim_next_agent_command(
                    session, &owner, now, now.saturating_add(300_000))?
                    .ok_or("queued UI input is already claimed or absent; observe its Run")?;
                if lease.command_id != command_id {
                    crate::agent_command_port::release_agent_command_lease(session, &lease.command_id, &owner)?;
                    return Err("an earlier queued input must be handled first".into());
                }
                let admission_id = format!("queued-admission:{}:{}", command_id, lease.delivery_attempt);
                crate::core_agent_runtime::run_queued_user_input(
                    &request, lease, admission_id, &mut output, cancellation.clone(),
                    nola_core::context_engine::ContextPlatform::Macos, Vec::new())
            } else {
                crate::core_agent_runtime::run_agent_core_with_cancellation(
                    &request, &request.input.message, &mut output, cancellation.clone())
            }
        })().and_then(|outcome| {
            validate_completion(outcome.turn_status, cancellation.is_requested())
        }) };
        let shutdown = supervisor.map(crate::runtime_effect_supervisor::CliRuntimeEffectSupervisor::shutdown)
            .transpose().map(|_| ());
        result.and(shutdown)
    })();
    crate::local_models::shutdown();
    result
}

fn validate_completion(status: nola_core::agent::AgentTurnStatus, stop_requested: bool) -> Result<(), String> {
    use nola_core::agent::AgentTurnStatus;
    if status == AgentTurnStatus::Failed { return Err(String::new()); }
    if status == AgentTurnStatus::Cancelling || (stop_requested && !status.is_terminal()) {
        return Err("Agent Host cancellation is pending canonical stop confirmation; Run remains recoverable".into());
    }
    Ok(())
}

#[cfg(test)]
mod cancellation_completion_tests {
    use super::validate_completion;
    use nola_core::agent::AgentTurnStatus;

    #[test]
    fn pending_cancellation_is_not_a_successful_host_exit() {
        assert!(validate_completion(AgentTurnStatus::Cancelling, true).is_err());
        assert!(validate_completion(AgentTurnStatus::Cancelling, false).is_err());
        assert!(validate_completion(AgentTurnStatus::AwaitingPermission, true).is_err());
        assert!(validate_completion(AgentTurnStatus::AwaitingPermission, false).is_ok());
        assert!(validate_completion(AgentTurnStatus::Cancelled, true).is_ok());
        assert!(validate_completion(AgentTurnStatus::Completed, false).is_ok());
    }
}
