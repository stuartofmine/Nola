// SPDX-License-Identifier: Apache-2.0
//! Host adapters for Core's cross-platform Goal coordinator. No Goal state
//! transitions, acceptance rules, retry policy or orchestration loop live here.
use std::time::Duration;
use crate::{core_agent_runtime, session};
use crate::host_invocation_request::HostInvocationRequest;
use crate::runtime_output::{GoalCompleteSignal, RuntimeOutputPort};
use crate::runtime_control::RuntimeExecutionControl;
use nola_core::agent::{GoalRuntimeCommand, GoalRuntimeCommandEnvelope, AgentInputOrigin};
pub(crate) use nola_application::goal_coordinator::GoalCyclePause;
#[cfg(test)]
pub(crate) use nola_application::goal_coordinator::{terminal_reason_pauses_goal, compose_goal_prompt,
    goal_retry_delay_for_attempt, goal_requirement_records};

pub(crate) trait GoalRoundDriver {
    fn stop_requested(&self) -> bool { false }
    /// Exact-Run drivers restore and validate their own immutable configuration.
    fn restores_exact_run_configuration(&self) -> bool { false }
    fn resume(&mut self, request: &HostInvocationRequest, run_id: &str) -> Result<Option<PromptRunResult>, String>;
}

pub(crate) fn restore_goal_execution_request(conversation_id: &str, goal_id: &str)
    -> Result<HostInvocationRequest, String> {
    core_agent_runtime::restore_goal_execution_request(conversation_id, goal_id)
}

pub(crate) fn host_goal_control(request: &HostInvocationRequest) -> Result<Option<(String, String)>, String> {
    nola_application::goal_coordinator::host_goal_control(&request.input.host_context_components)
}

pub(crate) fn goal_now_epoch_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis().min(i64::MAX as u128) as i64
}
fn goal_runtime_limit() -> Duration {
    Duration::from_secs(std::env::var("NOLA_GOAL_MAX_RUNTIME_SEC").ok()
        .and_then(|value| value.trim().parse::<u64>().ok()).unwrap_or(3_600).clamp(60, 86_400))
}

#[derive(Debug)]
pub(crate) struct PromptRunResult {
    pub(crate) run_outcome: core_agent_runtime::CoreRunOutcome,
    pub(crate) output: String,
    pub(crate) goal_signal: Option<GoalCompleteSignal>,
    pub(crate) terminal_reason: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) queued_user_inputs: Vec<String>,
    pub(crate) draft_user_input: Option<String>,
    pub(crate) context_usage: Option<nola_core::agent::context_window::ContextUsageSnapshot>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct InteractiveTurnHandoff {
    pub(crate) queued_user_inputs: Vec<String>,
    pub(crate) draft_user_input: Option<String>,
    pub(crate) context_usage: Option<nola_core::agent::context_window::ContextUsageSnapshot>,
}

#[allow(dead_code)]
pub(crate) type PromptHandoff = InteractiveTurnHandoff;

impl InteractiveTurnHandoff {
    fn from_run_result(result: &mut PromptRunResult) -> Self {
        Self {
            queued_user_inputs: std::mem::take(&mut result.queued_user_inputs),
            draft_user_input: result.draft_user_input.take(),
            context_usage: result.context_usage.take(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum GoalCycleOutcome {
    Completed(Option<InteractiveTurnHandoff>),
    Failed {
        reason: Option<String>,
        handoff: Option<InteractiveTurnHandoff>,
    },
    Cancelled(Option<InteractiveTurnHandoff>),
    Paused(Option<InteractiveTurnHandoff>),
    NeedsUser {
        reason: Option<String>,
        handoff: Option<InteractiveTurnHandoff>,
    },
    Waiting {
        wake_at_epoch_ms: Option<i64>,
        handoff: Option<InteractiveTurnHandoff>,
    },
    Yielded(InteractiveTurnHandoff),
}

const MAX_INLINE_GOAL_WAIT_MS: i64 = 15_000;
const GOAL_WAIT_SLICE_MS: u64 = 200;

fn goal_reason_from_step(result: &serde_json::Value) -> Option<String> {
    result["instruction"]["reason"]
        .as_str()
        .filter(|reason| !reason.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            result["goal"]["audit_reports"]
                .as_array()
                .and_then(|reports| reports.last())
                .and_then(|report| report["summary"].as_str())
                .filter(|summary| !summary.trim().is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            result["goal"]["stop_reason"]
                .as_str()
                .filter(|reason| !reason.trim().is_empty())
                .map(str::to_string)
        })
}

fn classify_terminal_outcome(
    result: &serde_json::Value,
    handoff: Option<InteractiveTurnHandoff>,
) -> GoalCycleOutcome {
    let outcome = result["instruction"]["outcome"]
        .as_str()
        .or_else(|| result["goal"]["status"].as_str());
    match outcome {
        Some("completed") => GoalCycleOutcome::Completed(handoff),
        Some("failed") => GoalCycleOutcome::Failed {
            reason: goal_reason_from_step(result),
            handoff,
        },
        _ => GoalCycleOutcome::Cancelled(handoff),
    }
}

fn classify_non_run_instruction(
    result: &serde_json::Value,
    handoff: Option<InteractiveTurnHandoff>,
) -> Result<GoalCycleOutcome, String> {
    match result["instruction"]["type"].as_str() {
        Some("terminal") => Ok(classify_terminal_outcome(result, handoff)),
        Some("paused") => Ok(GoalCycleOutcome::Paused(handoff)),
        Some("needs_user") => {
            if result["goal"]["status"].as_str() == Some("paused") {
                Ok(GoalCycleOutcome::Paused(handoff))
            } else {
                Ok(GoalCycleOutcome::NeedsUser {
                    reason: goal_reason_from_step(result),
                    handoff,
                })
            }
        }
        Some("wait") => {
            let wake_at_epoch_ms = result["instruction"]["wake_at_epoch_ms"].as_i64();
            if wake_at_epoch_ms.is_none()
                && (result["instruction"]["reason"].as_str() == Some("host_unavailable")
                    || result["goal"]["status"].as_str() == Some("paused"))
            {
                Ok(GoalCycleOutcome::Paused(handoff))
            } else {
                Ok(GoalCycleOutcome::Waiting {
                    wake_at_epoch_ms,
                    handoff,
                })
            }
        }
        _ => Err("Invalid Core Goal instruction".into()),
    }
}


pub(crate) fn run_goal_cycle(request: &mut HostInvocationRequest, user_prompt: &str, objective: &str, constraints: &str,
    driver: &mut dyn GoalRoundDriver) -> Result<GoalCycleOutcome, String> {
    let scheduled = request.input.host_input_origin == AgentInputOrigin::ScheduledAction;
    run_goal_cycle_with_dispatch(request, user_prompt, objective, constraints, driver, scheduled)
}

fn run_goal_cycle_with_dispatch(request: &mut HostInvocationRequest, user_prompt: &str, objective: &str, constraints: &str,
    driver: &mut dyn GoalRoundDriver, _scheduled: bool) -> Result<GoalCycleOutcome, String> {
    if request.session.session_id.as_deref().is_none_or(|id| id.trim().is_empty()) {
        request.session.session_id = Some(uuid::Uuid::new_v4().to_string());
    }
    request.session.save_session = true;
    let owner = request.session.session_id.clone().unwrap();
    if session::load_session_goal_projection(&owner)?.and_then(|p|p.active).is_none_or(|g| g.status.is_terminal()) {
        core_agent_runtime::submit_agent_core_goal_command(request,
            nola_application::goal_coordinator::create_goal_command(&format!("goal-start:{}",uuid::Uuid::new_v4()), objective, constraints, goal_now_epoch_ms()))?;
    }
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let mut user_prompt = user_prompt.to_string();
    let mut submitted_run_id = request.session.host_submission_id.take();
    let mut submitted_user_message_id = request.session.host_user_message_id.take();
    let mut submitted_assistant_message_id = request.session.host_assistant_message_id.take();
    let started = std::time::Instant::now();
    let mut previous_run:Option<String>=None;
    let mut handoff=None;
    loop {
        let execution: nola_core::agent::HostExecutionState = application.execute(
            nola_application::ApplicationCommand::LoadHostExecutionState { conversation_id: owner.clone() })?;
        let host = execution.configuration.host_catalog.as_ref().ok_or("Goal Host catalog unavailable")?.host_id.0.clone();
        let available=!driver.stop_requested() && started.elapsed()<goal_runtime_limit();
        let result: serde_json::Value = application.execute(nola_application::ApplicationCommand::StepGoal {
            conversation_id: owner.clone(), source_host_id: host, control:None,
            start_configuration: None, selected_model: None, user_prompt: std::mem::take(&mut user_prompt), host_available: Some(available),
            submitted_run_id: submitted_run_id.take(), submitted_user_message_id: submitted_user_message_id.take(),
            submitted_assistant_message_id: submitted_assistant_message_id.take(), stop_after_sealed_round: false,
        })?;
        if let Some(error) = result["error"].as_str() { return Err(error.into()); }
        match result["instruction"]["type"].as_str() {
            Some("execute_run" | "needs_user") if result["instruction"]["run_id"].is_string() => {
                let run = result["instruction"]["run_id"].as_str().ok_or("Goal execution instruction has no Run")?;
                if previous_run.as_deref()==Some(run) {return Ok(GoalCycleOutcome::Yielded(handoff.take().unwrap_or_default()));}
                let goal = result["goal"]["id"].as_str().ok_or("Goal execution instruction has no Goal")?;
                if !driver.restores_exact_run_configuration() {
                    *request = restore_goal_execution_request(&owner, goal)?;
                }
                let outcome = driver.resume(request, run)?;
                previous_run=Some(run.into());
                if let Some(mut outcome) = outcome {
                    handoff = Some(InteractiveTurnHandoff::from_run_result(&mut outcome));
                }
                // A competing Host may have sealed the instructed Run. Let
                // the same Core step consume it instead of requiring a wake.
            }
            Some("wait") => {
                if let Some(wake_ms) = result["instruction"]["wake_at_epoch_ms"].as_i64() {
                    let delay_ms = wake_ms.saturating_sub(goal_now_epoch_ms());
                    if delay_ms <= MAX_INLINE_GOAL_WAIT_MS && !driver.stop_requested() {
                        while goal_now_epoch_ms() < wake_ms {
                            if driver.stop_requested() {
                                return Ok(GoalCycleOutcome::Paused(handoff.take()));
                            }
                            let remaining_ms = wake_ms.saturating_sub(goal_now_epoch_ms()).max(0) as u64;
                            std::thread::sleep(Duration::from_millis(
                                remaining_ms.min(GOAL_WAIT_SLICE_MS),
                            ));
                        }
                        if driver.stop_requested() {
                            return Ok(GoalCycleOutcome::Paused(handoff.take()));
                        }
                        continue;
                    }
                }
                return classify_non_run_instruction(&result, handoff.take());
            }
            Some("terminal" | "paused" | "needs_user") => {
                return classify_non_run_instruction(&result, handoff.take());
            }
            _ => return Err("Invalid Core Goal instruction".into()),
        }
    }
}

pub(crate) fn submit_goal_command(request: &HostInvocationRequest, operation_id: String, command: GoalRuntimeCommand)
    -> Result<nola_core::agent::protocol::AgentControlSnapshot, String> {
    core_agent_runtime::submit_agent_core_goal_command(request, GoalRuntimeCommandEnvelope { operation_id, command })
}
pub(crate) fn pause_goal(request: &HostInvocationRequest, cause: GoalCyclePause) -> Result<GoalCycleOutcome, String> {
    let owner = request.session.session_id.as_deref().ok_or("Goal runtime requires a durable session id")?;
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let execution: nola_core::agent::HostExecutionState = application.execute(
        nola_application::ApplicationCommand::LoadHostExecutionState {conversation_id:owner.into()})?;
    let now_epoch_ms=goal_now_epoch_ms();
    let command=match cause {
        GoalCyclePause::Incomplete=>GoalRuntimeCommand::ConcludeIncomplete {now_epoch_ms},
        GoalCyclePause::UserPaused=>GoalRuntimeCommand::Pause {status:nola_core::agent::GoalStatus::Paused,wake_at_epoch_ms:None,now_epoch_ms},
        GoalCyclePause::NeedsUser=>GoalRuntimeCommand::Pause {status:nola_core::agent::GoalStatus::NeedsUser,wake_at_epoch_ms:None,now_epoch_ms},
    };
    let result:serde_json::Value=application.execute(nola_application::ApplicationCommand::StepGoal {
        conversation_id:owner.into(),source_host_id:execution.configuration.host_catalog.ok_or("Goal Host unavailable")?.host_id.0,
        control:Some(GoalRuntimeCommandEnvelope {operation_id:format!("pause:{}",uuid::Uuid::new_v4()),command}),
        start_configuration:None,selected_model:None,user_prompt:String::new(),host_available:Some(true),
        submitted_run_id:None,submitted_user_message_id:None,submitted_assistant_message_id:None,stop_after_sealed_round:false })?;
    if let Some(error)=result["error"].as_str(){return Err(error.into());}
    Ok(GoalCycleOutcome::Paused(None))
}
#[cfg(test)]
pub(crate) fn pending_goal_run(request: &HostInvocationRequest) -> Result<Option<(String,String)>,String> {
    let Some(owner)=request.session.session_id.as_deref() else{return Ok(None)};
    let mut application=crate::application_port::CliApplicationPort::persistent_default()?;
    let Some(run)=application.active_run(owner)? else{return Ok(None)};
    let goal=application.load_session_goal_projection(owner)?.and_then(|p|p.active);
    Ok(goal.and_then(|g|g.contracts.into_iter().find(|c|c.execution_run_id.as_deref()==Some(&run.run_id)).map(|c|(c.id,run.run_id))))
}

// A completed Turn can seal a blocked or incomplete Run. Goal scheduling must
// use the committed result rather than infer success from Turn termination.
fn canonical_goal_terminal_reason(outcome: &core_agent_runtime::CoreRunOutcome) -> Result<Option<&'static str>, String> {
    use nola_domain::run_result::RunResultValue;
    use nola_core::agent::AgentTurnStatus;
    Ok(match outcome.run_result.as_ref() {
        Some(RunResultValue::Answered { .. } | RunResultValue::ActionCompleted { .. }) => Some("completed"),
        Some(RunResultValue::DelegationAccepted { .. }) => return Err("A delegation command receipt cannot complete a Goal round".into()),
        Some(RunResultValue::Blocked { .. }) => Some("blocked"),
        Some(RunResultValue::Incomplete { .. }) => Some("incomplete"),
        Some(RunResultValue::Failed { .. }) => Some("failed"),
        Some(RunResultValue::Cancelled { .. }) => Some("cancelled"),
        // The cold cancellation fast path has already verified the exact stop
        // confirmation before returning; it does not enter a model round.
        None if outcome.turn_status == AgentTurnStatus::Cancelled => Some("cancelled"),
        None if outcome.turn_status.is_terminal() => return Err("Goal terminal Run is missing its committed result".into()),
        None => None,
    })
}

struct CoreGoalRoundDriver<'a> {
    output: &'a mut dyn RuntimeOutputPort,
    control: RuntimeExecutionControl,
}
impl CoreGoalRoundDriver<'_> {
    fn result(&self, request: &HostInvocationRequest, outcome: core_agent_runtime::CoreRunOutcome) -> Result<PromptRunResult, String> {
        let terminal_reason = canonical_goal_terminal_reason(&outcome)?.map(str::to_owned);
        Ok(PromptRunResult { goal_signal: outcome.goal_signal.clone(), run_outcome: outcome,
            output: String::new(), terminal_reason, session_id: request.session.session_id.clone(),
            queued_user_inputs: Vec::new(), draft_user_input: None, context_usage: None })
    }
}
impl GoalRoundDriver for CoreGoalRoundDriver<'_> {
    fn restores_exact_run_configuration(&self) -> bool { true }
    fn stop_requested(&self) -> bool { self.control.is_requested() }
    fn resume(&mut self, request: &HostInvocationRequest, run_id: &str) -> Result<Option<PromptRunResult>, String> {
        let session = request.session.session_id.as_deref().ok_or("Goal recovery requires a session")?;
        core_agent_runtime::resume_frozen_core_agent_run(session, run_id, self.output, self.control.clone())?
            .map(|outcome| self.result(request, outcome)).transpose()
    }
}

pub(crate) fn run_host_goal(
    request: &mut HostInvocationRequest, user_prompt: &str, objective: &str, constraints: &str,
    output: &mut dyn RuntimeOutputPort, control: RuntimeExecutionControl,
) -> Result<GoalCycleOutcome, String> {
    run_goal_cycle(request, user_prompt, objective, constraints, &mut CoreGoalRoundDriver { output, control })
}

/// A durable scheduler wake never represents a new user Resume action, even
/// when the exact recovered Run was originally a visible interactive round.
pub(crate) fn run_scheduled_host_goal(
    request: &mut HostInvocationRequest, objective: &str, constraints: &str,
    output: &mut dyn RuntimeOutputPort, control: RuntimeExecutionControl,
) -> Result<GoalCycleOutcome, String> {
    run_goal_cycle_with_dispatch(request, "", objective, constraints,
        &mut CoreGoalRoundDriver { output, control }, true)
}

#[cfg(test)]
mod canonical_goal_result_tests {
    use super::*;
    use nola_core::agent::AgentTurnStatus;
    use nola_domain::run_result::{RunResultReason, RunResultValue};

    #[test]
    fn delegated_command_receipt_does_not_prove_goal_work_finished() {
        let outcome = core_agent_runtime::CoreRunOutcome {
            turn_status: AgentTurnStatus::Completed,
            run_result: Some(RunResultValue::DelegationAccepted {
                task_id: "task".into(), parent_call_id: "call".into(),
            }),
            goal_signal: None,
        };
        assert!(canonical_goal_terminal_reason(&outcome).is_err());
    }

    #[test]
    fn completed_turn_preserves_blocked_run_instead_of_retrying_as_partial() {
        let outcome = core_agent_runtime::CoreRunOutcome {
            turn_status: AgentTurnStatus::Completed,
            run_result: Some(RunResultValue::Blocked {
                reason: RunResultReason { code: "context.input_budget_exceeded".into(), detail: None },
                blocking_fact_refs: vec!["budget-fact".into()],
                resolution_condition: "Change the input budget".into(),
            }),
            goal_signal: None,
        };
        assert_eq!(canonical_goal_terminal_reason(&outcome).unwrap(), Some("blocked"));
    }

    #[test]
    fn terminal_turn_without_result_cannot_be_reported_as_completed() {
        for status in [AgentTurnStatus::Completed, AgentTurnStatus::Failed] {
            let outcome = core_agent_runtime::CoreRunOutcome {
                turn_status: status, run_result: None, goal_signal: None,
            };
            assert!(canonical_goal_terminal_reason(&outcome).is_err());
        }
        let waiting = core_agent_runtime::CoreRunOutcome {
            turn_status: AgentTurnStatus::AwaitingInput, run_result: None, goal_signal: None,
        };
        assert_eq!(canonical_goal_terminal_reason(&waiting).unwrap(), None);
    }

    #[test]
    fn goal_instruction_classification_preserves_terminal_and_handoff_variants() {
        let handoff = InteractiveTurnHandoff {
            queued_user_inputs: vec!["follow-up".into()],
            draft_user_input: Some("draft".into()),
            context_usage: None,
        };
        assert_eq!(
            classify_non_run_instruction(
                &serde_json::json!({"instruction": {"type": "terminal", "outcome": "completed"}}),
                Some(InteractiveTurnHandoff {
                    queued_user_inputs: vec!["follow-up".into()],
                    draft_user_input: Some("draft".into()),
                    context_usage: None,
                }),
            )
            .unwrap(),
            GoalCycleOutcome::Completed(Some(handoff))
        );
        assert_eq!(
            classify_non_run_instruction(
                &serde_json::json!({
                    "instruction": {"type": "terminal", "outcome": "failed"},
                    "goal": {"status": "failed", "stop_reason": "failed"}
                }),
                None,
            )
            .unwrap(),
            GoalCycleOutcome::Failed {
                reason: Some("failed".into()),
                handoff: None,
            }
        );
        assert_eq!(
            classify_non_run_instruction(
                &serde_json::json!({"instruction": {"type": "terminal", "outcome": "cancelled"}}),
                None,
            )
            .unwrap(),
            GoalCycleOutcome::Cancelled(None)
        );
        assert_eq!(
            classify_non_run_instruction(
                &serde_json::json!({
                    "instruction": {"type": "needs_user"},
                    "goal": {"status": "paused"}
                }),
                Some(InteractiveTurnHandoff::default()),
            )
            .unwrap(),
            GoalCycleOutcome::Paused(Some(InteractiveTurnHandoff::default()))
        );
        assert_eq!(
            classify_non_run_instruction(
                &serde_json::json!({
                    "instruction": {"type": "needs_user"},
                    "goal": {"status": "needs_user", "stop_reason": "awaiting_user"}
                }),
                Some(InteractiveTurnHandoff::default()),
            )
            .unwrap(),
            GoalCycleOutcome::NeedsUser {
                reason: Some("awaiting_user".into()),
                handoff: Some(InteractiveTurnHandoff::default()),
            }
        );
        assert_eq!(
            classify_non_run_instruction(
                &serde_json::json!({
                    "instruction": {"type": "wait", "wake_at_epoch_ms": 9_999_999_999_999i64}
                }),
                Some(InteractiveTurnHandoff::default()),
            )
            .unwrap(),
            GoalCycleOutcome::Waiting {
                wake_at_epoch_ms: Some(9_999_999_999_999i64),
                handoff: Some(InteractiveTurnHandoff::default()),
            }
        );
    }
}
