// SPDX-License-Identifier: Apache-2.0
//! Prepare a stable Goal command through the existing durable command receipt.
//! Preparation alone does not mutate a Goal or authorize model execution.
use nola_agent_engine::agent::{GoalRuntimeCommand, GoalRuntimeCommandEnvelope, GoalStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalControlAction { Start, Pause, Resume, Cancel, Schedule }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareGoalControlIntent {
    pub conversation_id: String,
    pub operation_id: String,
    pub action: GoalControlAction,
    pub expected_goal_id: Option<String>,
    pub expected_control_token: Option<String>,
    pub objective: String,
    pub constraints: Vec<String>,
    pub wake_at_epoch_ms: Option<i64>,
}

/// Every active submission (including queued Host controls) uses the same
/// interruption decision. Core still validates the guard, Goal ownership and
/// physical cancellation acknowledgements before sealing the Run.
pub(crate) fn active_submission(submission: nola_agent_engine::agent::AgentSubmission)
    -> nola_agent_engine::agent::AgentSubmission {
    use nola_agent_engine::agent::AgentSubmission;
    let AgentSubmission::GoalRuntime { envelope } = submission else { return submission; };
    let command = match &envelope.command {
        GoalRuntimeCommand::GuardedControl { control, .. } => control.as_ref(),
        command => command,
    };
    if matches!(command, GoalRuntimeCommand::Pause { status: GoalStatus::Paused, .. }
        | GoalRuntimeCommand::Cancel { .. }) {
        AgentSubmission::Interrupt {
            cancellation_id: format!("goal-control:{}", envelope.operation_id),
            goal_control: Some(envelope), reason: Some("cancelled_by_user".into()),
        }
    } else { AgentSubmission::GoalRuntime { envelope } }
}

pub fn prepare(input: PrepareGoalControlIntent, now_ms: i64) -> Result<GoalRuntimeCommandEnvelope, String> {
    for (id, limit) in [(&input.conversation_id, 512), (&input.operation_id, 256)] {
        if id.is_empty() || id.trim() != id || id.len() > limit || id.chars().any(char::is_control) {
            return Err("invalid Goal control intent identity".into());
        }
    }
    if now_ms < 0 { return Err("invalid Goal control preparation time".into()); }
    if matches!(input.action, GoalControlAction::Start) {
        if input.objective.trim().is_empty() || input.expected_goal_id.is_some()
            || input.expected_control_token.is_some() || input.wake_at_epoch_ms.is_some() {
            return Err("Goal Start requires an objective without control-only fields".into());
        }
        let mut envelope = nola_agent_engine::agent::goal::create_goal_command(
            &input.operation_id, input.objective.trim(), &input.constraints.join("\n"), now_ms,
        );
        envelope.operation_id = input.operation_id;
        return Ok(envelope);
    }
    if !input.objective.is_empty() || !input.constraints.is_empty() {
        return Err("Goal controls cannot replace the objective or constraints".into());
    }
    let expected_goal_id = input.expected_goal_id.filter(|v| !v.trim().is_empty())
        .ok_or("Goal control requires expected Goal identity")?;
    let expected_control_token = input.expected_control_token.filter(|v| !v.trim().is_empty())
        .ok_or("Goal control requires the observed Core control token")?;
    let control = match input.action {
        GoalControlAction::Pause => GoalRuntimeCommand::Pause {
            status: GoalStatus::Paused, wake_at_epoch_ms: input.wake_at_epoch_ms, now_epoch_ms: now_ms,
        },
        GoalControlAction::Schedule => GoalRuntimeCommand::ScheduleResume {
            wake_at_epoch_ms: input.wake_at_epoch_ms.ok_or("Goal schedule requires a wake time")?, now_epoch_ms: now_ms,
        },
        GoalControlAction::Resume | GoalControlAction::Cancel if input.wake_at_epoch_ms.is_some() =>
            return Err("Goal resume/cancel cannot carry a wake time".into()),
        GoalControlAction::Resume => GoalRuntimeCommand::Resume { now_epoch_ms: now_ms },
        GoalControlAction::Cancel => GoalRuntimeCommand::Cancel { now_epoch_ms: now_ms },
        GoalControlAction::Start => unreachable!(),
    };
    Ok(GoalRuntimeCommandEnvelope { operation_id: input.operation_id,
        command: GoalRuntimeCommand::GuardedControl { expected_goal_id, expected_control_token, control: Box::new(control) } })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(action: GoalControlAction) -> PrepareGoalControlIntent {
        PrepareGoalControlIntent { conversation_id: "conversation".into(), operation_id: "fixed-operation".into(),
            action, expected_goal_id: Some("goal".into()), expected_control_token: Some("opaque-token".into()),
            objective: String::new(), constraints: vec![], wake_at_epoch_ms: None }
    }
    #[test]
    fn preparation_preserves_control_identity_and_host_time() {
        let envelope = prepare(input(GoalControlAction::Pause), 42).unwrap();
        assert_eq!(envelope.operation_id, "fixed-operation");
        let GoalRuntimeCommand::GuardedControl { expected_goal_id, expected_control_token, control } = envelope.command else { panic!("unguarded control") };
        assert_eq!(expected_goal_id, "goal");
        assert_eq!(expected_control_token, "opaque-token");
        assert!(matches!(*control, GoalRuntimeCommand::Pause { now_epoch_ms: 42, .. }));
    }
    #[test]
    fn irrelevant_control_fields_are_rejected_instead_of_silently_changed() {
        let mut cancel = input(GoalControlAction::Cancel);
        cancel.objective = "replace objective".into();
        assert!(prepare(cancel, 42).is_err());
        let mut resume = input(GoalControlAction::Resume);
        resume.wake_at_epoch_ms = Some(100);
        assert!(prepare(resume, 42).is_err());
        assert!(prepare(input(GoalControlAction::Schedule), 42).is_err());
        assert!(prepare(input(GoalControlAction::Start), 42).is_err());
    }
    #[test]
    fn start_uses_the_shared_creation_contract_and_stable_operation_identity() {
        let mut start = input(GoalControlAction::Start);
        start.expected_goal_id = None;
        start.expected_control_token = None;
        start.objective = "Review the result".into();
        let envelope = prepare(start, 42).unwrap();
        let expected = nola_agent_engine::agent::goal::create_goal_command("fixed-operation", "Review the result", "", 42);
        assert_eq!(envelope.command, expected.command);
        assert_eq!(envelope.operation_id, "fixed-operation");
    }
}
