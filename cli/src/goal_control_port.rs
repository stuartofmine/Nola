// SPDX-License-Identifier: Apache-2.0
//! Shared native/Web transport adapter for durable Core Goal controls.
use nola_core::agent::GoalRuntimeCommandEnvelope;

pub(crate) fn enqueue(
    session: &str,
    envelope: GoalRuntimeCommandEnvelope,
) -> Result<bool, String> {
    // A recovered execution can be owned by the shared Host supervisor before
    // any Web observer has registered. Durable Core ownership, not a process
    // UI registry, determines whether a control targets an active Goal Run.
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let Some(active) = application.active_run(session)? else {
        return Ok(false);
    };
    let Some(projection) = application.load_session_goal_projection(session)? else {
        return Ok(false);
    };
    if projection.execution_owner(&active.run_id).is_none() {
        return Ok(false);
    }
    let command_id = envelope.operation_id.clone();
    // The queue transports the command; only step_goal chooses Interrupt vs idle control.
    let input=nola_core::agent::AgentSubmission::GoalRuntime {envelope};
    crate::agent_command_port::enqueue_agent_command(
        session,
        &command_id,
        &serde_json::to_string(&input).map_err(|e| e.to_string())?,
        crate::goal_runtime::goal_now_epoch_ms(),
    )?;
    // Core applies the guarded Goal control and emits exact Run cancellation
    // effects only after its identity/revision check succeeds.
    Ok(true)
}

pub(crate) fn apply_idle(
    session: &str,
    envelope: GoalRuntimeCommandEnvelope,
) -> Result<(), String> {
    let mut application=crate::application_port::CliApplicationPort::persistent_default()?;
    let execution:nola_core::agent::HostExecutionState=application.execute(
        nola_application::ApplicationCommand::LoadHostExecutionState {conversation_id:session.into()})?;
    let result:serde_json::Value=application.execute(nola_application::ApplicationCommand::StepGoal {
        conversation_id:session.into(),source_host_id:execution.configuration.host_catalog.ok_or("Goal Host unavailable")?.host_id.0,
        control:Some(envelope),start_configuration:None,selected_model:None,user_prompt:String::new(),host_available:Some(true) })?;
    if let Some(error)=result["error"].as_str(){return Err(error.into());}
    Ok(())
}


/// The native pipe and HTTP adapters share the same fixed Core intent and queue.
/// A pending acknowledgement is never reported as an applied Goal transition.
pub(crate) fn submit_existing(
    input: nola_application::goal_control_intent::PrepareGoalControlIntent,
) -> Result<bool, String> {
    if matches!(input.action, nola_application::goal_control_intent::GoalControlAction::Start) {
        return Err("Goal Start requires an execution binding".into());
    }
    let session = input.conversation_id.clone();
    let envelope = crate::application_port::CliApplicationPort::persistent_default()?
        .prepare_goal_control_intent(input)?;
    if let Some(projection) = crate::session::load_session_goal_projection(&session)? {
        if let Some(receipt) = projection.applied_operations.get(&envelope.operation_id) {
            if receipt.command_sha256 != nola_core::agent::goal::goal_command_digest(&envelope.command)? {
                return Err("Goal operation identity was reused for different content".into());
            }
            if let Some(rejection) = &receipt.rejection { return Err(rejection.clone()); }
            return Ok(true);
        }
    }
    if enqueue(&session, envelope.clone())? { return Ok(false); }
    apply_idle(&session, envelope.clone())?;
    let projection = crate::session::load_session_goal_projection(&session)?
        .ok_or("Goal projection is unavailable")?;
    let receipt = projection.applied_operations.get(&envelope.operation_id)
        .ok_or("Goal operation remains unconfirmed")?;
    if let Some(rejection) = &receipt.rejection { return Err(rejection.clone()); }
    Ok(true)
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct NativeGoalControl {
    pub operation_id: String,
    pub action: nola_application::goal_control_intent::GoalControlAction,
    pub expected_goal_id: Option<String>,
    pub expected_control_token: Option<String>,
    pub objective: Option<String>,
    pub constraints: Option<Vec<String>>,
    pub wake_at_epoch_ms: Option<i64>,
}
impl NativeGoalControl {
    pub(crate) fn input(self, conversation_id: String)
        -> nola_application::goal_control_intent::PrepareGoalControlIntent {
        nola_application::goal_control_intent::PrepareGoalControlIntent {
            conversation_id, operation_id: self.operation_id, action: self.action,
            expected_goal_id: self.expected_goal_id, expected_control_token: self.expected_control_token,
            objective: self.objective.unwrap_or_default(), constraints: self.constraints.unwrap_or_default(),
            wake_at_epoch_ms: self.wake_at_epoch_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_goal_control_has_one_closed_current_wire_shape() {
        let current = serde_json::json!({"operationId":"fixed", "action":"pause",
            "expectedGoalId":"goal", "expectedControlToken":"token"});
        let decoded: super::NativeGoalControl = serde_json::from_value(current.clone()).unwrap();
        let input = decoded.input("conversation".into());
        assert_eq!(input.operation_id, "fixed");
        assert_eq!(input.conversation_id, "conversation");
        let mut invalid = current;
        invalid["expectedRevision"] = 4.into();
        assert!(serde_json::from_value::<super::NativeGoalControl>(invalid).is_err());
    }
}
