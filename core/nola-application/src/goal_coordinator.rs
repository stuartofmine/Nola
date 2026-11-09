// SPDX-License-Identifier: Apache-2.0
//! Application-owned cross-platform Goal use-case coordinator. Core decides
//! Goal state; canonical ports commit transitions and Hosts execute model work.
use std::time::{Duration, Instant};
use serde_json::Value;
use nola_agent_engine::agent::{GoalAuditReport, GoalAuditVerdict, GoalContractStatus,
    GoalExecutionReport, GoalRecordStatus, GoalRuntimeCommand,
    GoalRuntimeCommandEnvelope, GoalStatus, GoalSubtaskContract, GoalProjection};
use nola_agent_engine::agent::AgentTurnStatus;

// Goal command time belongs to Application. Driver lease timing remains in its
// transport adapter; Store ownership fences still govern cross-process epochs.
pub(crate) fn monotonic_epoch_millis() -> i64 {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST: AtomicI64 = AtomicI64::new(0);
    let observed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis().min(i64::MAX as u128) as i64;
    observed.max(LAST.fetch_max(observed, Ordering::Relaxed))
}

/// An untrusted declaration extracted from the exact sealed Run journal.
/// Only AcceptCompletion in Application decides whether these claims suffice.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GoalCompleteSignal {
    pub claims: Vec<nola_agent_engine::agent::GoalCompletionClaim>,
    pub status: String,
    pub summary: String,
    pub evidence: Vec<String>,
    pub blockers: Vec<String>,
    pub next_action: Option<String>,
}

impl GoalCompleteSignal {
    /// One schema for both live and cold sealed-journal observations. This
    /// parses a declaration; Application still owns evidence acceptance.
    pub fn from_tool_output(output: &Value) -> Result<Self, String> {
    let status = output
        .get("goal_status")
        .and_then(serde_json::Value::as_str)
        .ok_or("Goal declaration missing status")?;
    let summary = output
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .ok_or("Goal declaration missing summary")?;
    if !matches!(
        status,
        "completed" | "continue" | "incomplete" | "blocked" | "needs_user" | "failed"
    ) || summary.trim().is_empty()
    {
        return Err("Goal declaration has invalid status or empty summary".into());
    }
    let strings = |key: &str| -> Result<Vec<String>, String> {
        match output.get(key) {
            None => Ok(vec![]),
            Some(serde_json::Value::Array(values)) => values
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| format!("invalid Goal {key}"))
                })
                .collect(),
            _ => Err(format!("invalid Goal {key}")),
        }
    };
    Ok(Self {
        claims: output
            .get("claims")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("invalid Goal claims: {e}"))?
            .unwrap_or_default(),
        status: status.into(),
        summary: summary.into(),
        evidence: strings("evidence")?,
        blockers: strings("blockers")?,
        next_action: output
            .get("next_action")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or("invalid Goal next action")
            })
            .transpose()?,
    })
    }
}

#[derive(Debug)]
pub enum GoalCycleOutcome<H> { Completed(H), Yielded(H), Paused }
pub enum GoalCyclePause { UserPaused, NeedsUser, Incomplete }

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GoalRoundResult<H> {
    pub turn_status: AgentTurnStatus,
    pub output: String,
    pub goal_signal: Option<GoalCompleteSignal>,
    pub terminal_reason: Option<String>,
    pub pending_user_input: bool,
    /// Transient platform input; never interpreted or persisted by the driver.
    pub handoff: H,
}

/// The Application/Host boundary. Implementations must submit through the
/// canonical transactional Core, and recover an exact Run rather than rebind it.
pub trait GoalRuntimePort {
    type Handoff: Default;
    fn session_id(&self) -> &str;
    fn load_projection(&self) -> Result<Option<GoalProjection>, String>;
    fn submit(&self, command: GoalRuntimeCommandEnvelope) -> Result<(), String>;
    fn active_run_id(&self) -> Result<Option<String>, String>;
    fn prepare_exact_run(&mut self, run_id: &str) -> Result<nola_agent_engine::agent::AgentInputOrigin, String>;
    fn execute_round(&mut self, run_id: &str, context: &str, user_prompt: &str, visible: bool) -> Result<GoalRoundResult<Self::Handoff>, String>;
    fn resume_round(&mut self, run_id: &str) -> Result<GoalRoundResult<Self::Handoff>, String>;
    /// A sealed round can lose active_run before its Goal audit commits.
    /// Read its exact result; never recover or execute that Run again.
    fn sealed_round_result(&self, _run_id: &str) -> Result<Option<GoalRoundResult<Self::Handoff>>, String> { Ok(None) }
    /// Embedded Hosts execute an already admitted round asynchronously. This
    /// yields transport, without converting that handoff into a retry schedule.
    fn round_handoff_pending(&self) -> bool { false }
    fn now_epoch_ms(&self) -> i64;
    fn next_nonce(&self) -> String;
    fn new_lease_owner(&self) -> String;
    /// True only from trusted Host process/owner-liveness evidence.
    fn owner_is_dead(&self, owner: &str) -> bool;
    fn is_scheduled(&self) -> bool;
    fn explicit_resume(&self) -> bool;
    fn runtime_limit(&self) -> Duration;
    fn stop_requested(&self) -> bool;
    fn acknowledge_slice(&mut self) -> Result<(), String>;
    fn discard_slice_snapshot(&mut self);
}

pub use nola_agent_engine::agent::goal::{create_goal_command, goal_requirement_records};

fn submit_goal_command<H: Default>(port: &dyn GoalRuntimePort<Handoff = H>, operation_id: String, command: GoalRuntimeCommand) -> Result<(), String> {
    port.submit(GoalRuntimeCommandEnvelope { operation_id, command })
}

pub fn pending_goal_run<H: Default>(port: &dyn GoalRuntimePort<Handoff = H>) -> Result<Option<(String, String)>, String> {
    let Some(run_id) = port.active_run_id()? else { return Ok(None) };
    let goal = port.load_projection()?.and_then(|p| p.active)
        .ok_or("An existing non-Goal Run must finish before Goal execution")?;
    if goal.status.is_terminal() { return Err("A terminal Goal cannot resume its pending Run; reconcile the exact Run first".into()); }
    let contract = goal.contracts.last().filter(|c| c.status == GoalContractStatus::Executing)
        .ok_or("Active Run has no executing Goal contract; refusing to rebind it")?;
    if contract.execution_run_id.as_deref() != Some(run_id.as_str()) {
        return Err("Active Run does not match the executing Goal contract".into());
    }
    Ok(Some((contract.id.clone(), run_id)))
}

pub fn run_goal_cycle<H: Default>(port: &mut dyn GoalRuntimePort<Handoff = H>, user_prompt: &str, objective: &str, constraints: &str) -> Result<GoalCycleOutcome<H>, String> {
    let outcome = run_goal_cycle_inner(port, user_prompt, objective, constraints)?;
    if !matches!(outcome, GoalCycleOutcome::Yielded(_)) { port.acknowledge_slice()?; }
    Ok(outcome)
}

pub fn terminal_reason_pauses_goal(reason: &str) -> bool {
    matches!(reason, "cancelled" | "returned_to_composer")
}

pub fn pause_goal<H: Default>(config: &dyn GoalRuntimePort<Handoff = H>, cause: GoalCyclePause) -> Result<GoalCycleOutcome<H>, String> {
    // The preceding canonical partial audit owns the explanation. A stop is
    // successful only when its Core command commits, never a local mirror/no-op.
    let now_epoch_ms = config.now_epoch_ms();
    let command = match cause {
        GoalCyclePause::Incomplete => GoalRuntimeCommand::ConcludeIncomplete { now_epoch_ms },
        GoalCyclePause::UserPaused => GoalRuntimeCommand::Pause {
            status: GoalStatus::Paused, wake_at_epoch_ms: None, now_epoch_ms,
        },
        GoalCyclePause::NeedsUser => GoalRuntimeCommand::Pause {
            status: GoalStatus::NeedsUser, wake_at_epoch_ms: None, now_epoch_ms,
        },
    };
    submit_goal_command(config, format!("pause:{}", config.next_nonce()), command)?;
    Ok(GoalCycleOutcome::Paused)
}

pub fn goal_lease_until_epoch_ms<H: Default>(config: &dyn GoalRuntimePort<Handoff = H>) -> i64 {
    let lease_duration = config.runtime_limit()
        .saturating_add(Duration::from_secs(15 * 60))
        .max(Duration::from_secs(2 * 60 * 60));
    config.now_epoch_ms().saturating_add(lease_duration.as_millis().min(i64::MAX as u128) as i64)
}

pub fn schedule_goal_continuation<H: Default>(
    config: &dyn GoalRuntimePort<Handoff = H>,
    lease_owner: Option<&str>,
    delay: Duration,
) -> Result<i64, String> {
    let now = config.now_epoch_ms();
    let wake_at_epoch_ms = now.saturating_add(delay.as_millis().min(i64::MAX as u128) as i64);
    let (operation_id, command) = if let Some(owner) = lease_owner {
        (
            format!("goal-release:{owner}:{now}"),
            GoalRuntimeCommand::Release {
                owner: owner.to_string(),
                wake_at_epoch_ms: Some(wake_at_epoch_ms),
                now_epoch_ms: now,
            },
        )
    } else {
        (
            format!("goal-schedule:{}", config.next_nonce()),
            GoalRuntimeCommand::ScheduleResume {
                wake_at_epoch_ms,
                now_epoch_ms: now,
            },
        )
    };
    submit_goal_command(config, operation_id, command)?;
    Ok(wake_at_epoch_ms)
}

pub fn goal_retry_delay<H: Default>(config: &dyn GoalRuntimePort<Handoff = H>) -> Duration {
    let attempt = config.load_projection().ok().flatten()
        .and_then(|projection| projection.active).map(|goal| goal.attempt_count).unwrap_or(0);
    goal_retry_delay_for_attempt(attempt.into())
}

pub fn goal_retry_delay_for_attempt(attempt: u64) -> Duration {
    Duration::from_secs(60_u64.saturating_mul(1_u64 << attempt.min(6)))
}


pub fn ensure_core_goal_started<H: Default>(
    config: &mut dyn GoalRuntimePort<Handoff = H>,
    goal_target: &str,
    goal_requirements: &str,
    recover_interrupted_contract: bool,
) -> Result<Option<String>, String> {
    let session_id = config.session_id();
    let scheduled = config.is_scheduled();
    let lease_owner = Some(config.new_lease_owner());
    let projection = config.load_projection()?;
    if let Some(active) = projection.as_ref().and_then(|item| item.active.as_ref()) {
        if !active.status.is_terminal() {
            if active.objective != goal_target {
                return Err(format!(
                    "An unfinished Goal is already active: {}",
                    active.objective
                ));
            }
            if let Some(stale_owner) = active
                .lease
                .as_ref()
                .map(|lease| lease.owner.as_str())
                .filter(|owner| {
                    config.owner_is_dead(owner)
                })
            {
                submit_goal_command(
                    config,
                    format!("goal:{session_id}:release-stale:{stale_owner}"),
                    GoalRuntimeCommand::Release {
                        owner: stale_owner.to_string(),
                        wake_at_epoch_ms: None,
                        now_epoch_ms: config.now_epoch_ms(),
                    },
                )?;
            }
            if (!scheduled || config.explicit_resume())
                && matches!(
                    active.status,
                    GoalStatus::Paused | GoalStatus::NeedsUser | GoalStatus::Failed
                )
            {
                submit_goal_command(
                    config,
                    format!("goal:{session_id}:resume:{}", config.next_nonce()),
                    GoalRuntimeCommand::Resume {
                        now_epoch_ms: config.now_epoch_ms(),
                    },
                )?;
            }
            let owner = lease_owner.as_deref().expect("Goal lease owner");
            submit_goal_command(
                config,
                format!("goal:{session_id}:claim:{owner}"),
                GoalRuntimeCommand::Claim {
                    owner: owner.to_string(),
                    lease_until_epoch_ms: goal_lease_until_epoch_ms(config),
                    now_epoch_ms: config.now_epoch_ms(),
                },
            )?;
            if recover_interrupted_contract {
                recover_interrupted_goal_contract(config)?;
            }
            return Ok(lease_owner);
        }
    }
    let creation = create_goal_command(&config.next_nonce(), goal_target, goal_requirements, config.now_epoch_ms());
    config.submit(creation)?;
    if let Some(owner) = lease_owner.as_deref() {
        submit_goal_command(
            config,
            format!("goal:{session_id}:claim:{owner}"),
            GoalRuntimeCommand::Claim {
                owner: owner.to_string(),
                lease_until_epoch_ms: goal_lease_until_epoch_ms(config),
                now_epoch_ms: config.now_epoch_ms(),
            },
        )?;
    }
    Ok(lease_owner)
}

pub fn recover_interrupted_goal_contract<H: Default>(config: &dyn GoalRuntimePort<Handoff = H>) -> Result<(), String> {
    let Some(projection) = config.load_projection()? else {
        return Ok(());
    };
    let Some(goal) = projection.active else {
        return Ok(());
    };
    let Some(contract) = goal.contracts.last() else {
        return Ok(());
    };
    let contract_id = contract.id.clone();
    match contract.status {
        GoalContractStatus::Planned => {
            // A stale unexecuted plan must not be marked Executing just to
            // close it. A negative, empty Runtime audit can retire that plan.
            return apply_partial_goal_audit(config, &contract_id,
                "The planned round did not execute and will be replanned");
        }
        GoalContractStatus::Executing => {
            submit_goal_command(
                config,
                format!("{contract_id}:interrupted"),
                GoalRuntimeCommand::RecordExecution {
                    report: GoalExecutionReport {
                        contract_id: contract_id.clone(),
                        summary: "Host restarted during execution".to_string(),
                        claimed_record_ids: Vec::new(),
                        artifact_refs: Vec::new(),
                        blockers: vec!["execution interrupted".to_string()],
                        reported_at_epoch_ms: config.now_epoch_ms(),
                    },
                },
            )?;
        }
        GoalContractStatus::AwaitingAudit => {}
        GoalContractStatus::Audited | GoalContractStatus::Interrupted => return Ok(()),
    }
    apply_partial_goal_audit(config, &contract_id, "Interrupted round will be replanned")
}

pub fn begin_core_goal_round<H: Default>(config: &dyn GoalRuntimePort<Handoff = H>, request: &str, run_id: &str) -> Result<String, String> {
    let projection = config.load_projection()?
        .ok_or_else(|| "Goal checkpoint is missing".to_string())?;
    let goal = projection
        .active
        .ok_or_else(|| "Goal state is missing".to_string())?;
    let round = goal.contracts.len() as u64 + 1;
    let contract_id = format!("{}:round:{round}", goal.id);
    let criteria = goal
        .requirements
        .values()
        .filter(|item| item.status != GoalRecordStatus::Completed)
        .map(|item| item.description.clone())
        .collect::<Vec<_>>();
    let relevant_record_ids = goal
        .requirements
        .values()
        .filter(|item| item.status != GoalRecordStatus::Completed)
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let requirement_versions = goal.requirements.values()
        .filter(|item| item.status != GoalRecordStatus::Completed)
        .map(|item| (item.id.clone(), item.semantic_revision)).collect();
    submit_goal_command(
        config,
        format!("{contract_id}:plan"),
        GoalRuntimeCommand::PlanContract {
            contract: GoalSubtaskContract { execution_run_id: None,
                id: contract_id.clone(),
                round,
                objective: request.to_string(),
                proposed_checks: criteria,
                requirement_versions,
                additional_execution_constraints: goal.constraints,
                relevant_record_ids,
                status: GoalContractStatus::Planned,
                created_at_epoch_ms: config.now_epoch_ms(),
            },
            now_epoch_ms: config.now_epoch_ms(),
        },
    )?;
    submit_goal_command(
        config,
        format!("{contract_id}:begin"),
        GoalRuntimeCommand::BeginExecution { run_id: run_id.into(),
            contract_id: contract_id.clone(),
            now_epoch_ms: config.now_epoch_ms(),
        },
    )?;
    Ok(contract_id)
}

pub fn record_core_goal_execution<H: Default>(
    config: &dyn GoalRuntimePort<Handoff = H>,
    contract_id: &str,
    result: &GoalRoundResult<H>,
) -> Result<(), String> {
    // RecordExecution may already have committed when the Host disappeared
    // before acceptance. Its immutable receipt must not be rewritten with a
    // fresh timestamp, and its original sealed Run must not execute again.
    if config.load_projection()?.and_then(|p|p.active).is_some_and(|goal|
        goal.contracts.iter().any(|contract| contract.id == contract_id && contract.status == GoalContractStatus::AwaitingAudit)) {
        return Ok(());
    }
    let signal = result.goal_signal.as_ref();
    submit_goal_command(
        config,
        format!("{contract_id}:execution"),
        GoalRuntimeCommand::RecordExecution {
            report: GoalExecutionReport {
                contract_id: contract_id.to_string(),
                summary: signal
                    .map(|item| item.summary.clone())
                    .filter(|item| !item.trim().is_empty())
                    .unwrap_or_else(|| {
                        let output = result.output.trim();
                        if output.is_empty() {
                            "Execution ended without a summary".to_string()
                        } else {
                            output.chars().take(500).collect()
                        }
                    }),
                claimed_record_ids: signal
                    .filter(|item| item.status.eq_ignore_ascii_case("completed"))
                    .map(|signal| signal.claims.iter().map(|claim| claim.requirement_id.clone()).collect())
                    .unwrap_or_default(),
                artifact_refs: signal.map(|item| item.evidence.clone()).unwrap_or_default(),
                blockers: signal.map(|item| item.blockers.clone()).unwrap_or_default(),
                reported_at_epoch_ms: config.now_epoch_ms(),
            },
        },
    )?;
    Ok(())
}

pub fn apply_partial_goal_audit<H: Default>(
    config: &dyn GoalRuntimePort<Handoff = H>,
    contract_id: &str,
    summary: &str,
) -> Result<(), String> {
    let projection = config.load_projection()?
        .ok_or_else(|| "Goal checkpoint is missing".to_string())?;
    let goal = projection
        .active
        .ok_or_else(|| "Goal state is missing".to_string())?;
    let unmet_requirement_ids = goal
        .requirements
        .values()
        .filter(|item| item.status != GoalRecordStatus::Completed)
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    submit_goal_command(
        config,
        format!("{contract_id}:audit"),
        GoalRuntimeCommand::ApplyAudit {
            report: GoalAuditReport {
                assessor: nola_agent_engine::agent::goal::GoalAssessor::RuntimeOperation { operation_id: format!("{contract_id}:audit") },
                expected_goal_revision: projection.revision,
                id: format!("audit:{contract_id}"),
                contract_id: contract_id.to_string(),
                verdict: GoalAuditVerdict::Partial,
                summary: summary.to_string(),
                findings: Vec::new(),
                unmet_requirement_ids,
                integrity_violations: Vec::new(),
                evidence: Vec::new(),
                audited_at_epoch_ms: config.now_epoch_ms(),
            },
        },
    )?;
    Ok(())
}

pub fn run_goal_cycle_inner<H: Default>(
    config: &mut dyn GoalRuntimePort<Handoff = H>,
    user_prompt: &str,
    goal_target: &str,
    goal_requirements: &str,
) -> Result<GoalCycleOutcome<H>, String> {
    let sealed = config.load_projection()?.and_then(|projection| projection.active)
        .and_then(|goal| goal.contracts.last().cloned())
        .filter(|contract| matches!(contract.status, GoalContractStatus::Executing | GoalContractStatus::AwaitingAudit))
        .and_then(|contract| contract.execution_run_id.map(|run| (contract.id, run)))
        .map(|(contract, run)| config.sealed_round_result(&run).map(|result| result.map(|result| (contract, result))))
        .transpose()?.flatten();
    if sealed.is_some() && config.active_run_id()?.is_some() {
        return Err("An unrelated active Run must finish before reviewing the sealed Goal round".into());
    }
    let pending = if sealed.is_some() { None } else { pending_goal_run(config)? };
    if let Some((_, run_id)) = pending.as_ref() {
        let origin = config.prepare_exact_run(run_id)?;
        if !matches!(origin, nola_agent_engine::agent::AgentInputOrigin::GoalInteractiveUser | nola_agent_engine::agent::AgentInputOrigin::GoalContinuation) {
            return Err("Active Run does not belong to a Goal execution round".into());
        }
    }
    let existing_goal = config.load_projection()?.and_then(|p| p.active);
    let continuing_existing_goal = existing_goal.as_ref().is_some_and(|goal| !goal.work.source_turn_ids.is_empty());
    // Start is a durable user instruction even when dispatched later by a
    // Host scheduler. Materialize that exact objective once as Goal user input
    // before any hidden continuation can target its canonical message.
    let initial_goal_input = pending.is_none() && user_prompt.trim().is_empty()
        && existing_goal.as_ref().is_none_or(|goal| goal.work.source_turn_ids.is_empty());
    let user_prompt = if initial_goal_input { goal_target } else { user_prompt };
    // An active Goal does not turn a newly typed line into an automated
    // continuation. Preserve real user steering as a visible, durable user
    // item; only `/goal resume` and scheduler-driven empty rounds are hidden.
    let mut visible_user_round = !user_prompt.trim().is_empty();
    let lease_owner = ensure_core_goal_started(
        config,
        goal_target,
        goal_requirements,
        pending.is_none() && sealed.is_none(),
    )?;
    let mut recovered_round = if sealed.is_some() { sealed } else { match pending {
        Some((contract_id, run_id)) => match config.resume_round(&run_id) {
            Ok(result) => Some((contract_id, result)),
            Err(error) => {
                // Pause/Cancel may commit while the exact Run is recovering.
                // Recovery failure must not replace that user control with an
                // automatic scheduled wake (the same rule as execute_round).
                if config.load_projection()?.and_then(|projection| projection.active)
                    .is_some_and(|goal| goal.status != GoalStatus::Running) {
                    return Err(error);
                }
                // A failed recovery is not a failed execution. Keep the exact
                // contract/Run pending, release only this scheduler lease.
                schedule_goal_continuation(config, lease_owner.as_deref(), goal_retry_delay(config))?;
                return Err(error);
            }
        },
        None => None,
    }};
    let mut request = compose_goal_prompt(
        goal_target,
        user_prompt,
        goal_requirements,
        continuing_existing_goal,
        None,
        None,
    );
    let runtime_limit = config.runtime_limit();
    let started = Instant::now();
    loop {
        if recovered_round.is_none() && config.stop_requested() {
            return pause_goal(config, GoalCyclePause::UserPaused);
        }
        if let Some(owner) = lease_owner.as_deref() {
            submit_goal_command(
                config,
                format!("goal-heartbeat:{owner}:{}", config.now_epoch_ms()),
                GoalRuntimeCommand::Heartbeat {
                    owner: owner.to_string(),
                    lease_until_epoch_ms: goal_lease_until_epoch_ms(config),
                    now_epoch_ms: config.now_epoch_ms(),
                },
            )?;
        }
        if started.elapsed() >= runtime_limit {
            schedule_goal_continuation(
                config,
                lease_owner.as_deref(),
                Duration::from_secs(15 * 60),
            )?;
            return Ok(GoalCycleOutcome::Paused);
        }
        let (contract_id, result) = if let Some(recovered) = recovered_round.take() {
            recovered
        } else {
            let run_id = format!("goal-run-{}", config.next_nonce());
            let contract_id = begin_core_goal_round(config, &request, &run_id)?;
            let is_visible_user_round = visible_user_round;
            let visible_prompt = if is_visible_user_round { user_prompt } else { "" };
            let result = match config.execute_round(&run_id, &request, visible_prompt, is_visible_user_round) {
                Ok(result) => result,
                Err(error) => {
                    if config.load_projection()?.and_then(|projection| projection.active)
                        .is_some_and(|goal| goal.status != GoalStatus::Running) {
                        // A user control may already be committed while its
                        // exact stop confirmation is still pending. Preserve
                        // that intent; never convert it into an automatic wake.
                        return Err(error);
                    }
                    if pending_goal_run(config)?.is_some() {
                        // Transport/Host failure can leave a durable active Run.
                        // It must recover exactly, never be audited as finished.
                        schedule_goal_continuation(config, lease_owner.as_deref(), goal_retry_delay(config))?;
                        return Err(error);
                    }
                    let synthetic = GoalRoundResult {
                        turn_status: nola_agent_engine::agent::AgentTurnStatus::Failed,
                        output: String::new(), goal_signal: None,
                        terminal_reason: Some("failed".into()), pending_user_input: false,
                        handoff: H::default(),
                    };
                    record_core_goal_execution(config, &contract_id, &synthetic)?;
                    apply_partial_goal_audit(
                        config,
                        &contract_id,
                        &format!("Execution failed: {error}"),
                    )?;
                    schedule_goal_continuation(
                        config,
                        lease_owner.as_deref(),
                        goal_retry_delay(config),
                    )?;
                    // The failed resumed round never consumed its one-shot
                    // mailbox snapshot. Keep the durable evidence unread so
                    // the next resume can load it again from Application.
                    config.discard_slice_snapshot();
                    return Ok(GoalCycleOutcome::Paused);
                }
            };
            visible_user_round = false;
            (contract_id, result)
        };
        let mut result = result;
        if config.load_projection()?.and_then(|projection| projection.active)
            .is_some_and(|goal| goal.status != GoalStatus::Running) {
            // Preserve the user's exact pause/cancel even if its physical
            // cancellation acknowledgement remains outstanding.
            if result.turn_status == AgentTurnStatus::Cancelling {
                return Err("Goal cancellation is pending exact Run stop confirmation".into());
            }
            return if result.turn_status.is_terminal() { Ok(GoalCycleOutcome::Paused) }
                else { Ok(GoalCycleOutcome::Yielded(std::mem::take(&mut result.handoff))) };
        }
        if !result.turn_status.is_terminal() {
            // The executing contract still owns its original pending call.
            // Do not audit a partial response or create another Goal round.
            use nola_agent_engine::agent::AgentTurnStatus;
            if matches!(result.turn_status, AgentTurnStatus::AwaitingPermission | AgentTurnStatus::AwaitingInput) {
                pause_goal(config, GoalCyclePause::NeedsUser)?;
            } else if config.round_handoff_pending() && result.turn_status != AgentTurnStatus::Cancelling {
                if let Some(owner) = lease_owner.as_deref() {
                    submit_goal_command(config, format!("goal-handoff:{owner}:{}", config.now_epoch_ms()),
                        GoalRuntimeCommand::Release { owner: owner.into(), wake_at_epoch_ms: None,
                            now_epoch_ms: config.now_epoch_ms() })?;
                }
            } else {
                schedule_goal_continuation(config, lease_owner.as_deref(), goal_retry_delay(config))?;
                if result.turn_status == AgentTurnStatus::Cancelling {
                    return Err("Goal cancellation is pending exact Run stop confirmation".into());
                }
            }
            return Ok(GoalCycleOutcome::Yielded(std::mem::take(&mut result.handoff)));
        }
        if result.turn_status == AgentTurnStatus::Cancelled
            && config.load_projection()?.and_then(|projection| projection.active)
                .is_some_and(|goal| goal.contracts.last().is_some_and(|contract|
                    contract.id == contract_id && contract.status == GoalContractStatus::Interrupted)) {
            return pause_goal(config, GoalCyclePause::UserPaused);
        }
        record_core_goal_execution(config, &contract_id, &result)?;
        let terminal_reason = result.terminal_reason.as_deref().unwrap_or("unknown");

        if terminal_reason_pauses_goal(terminal_reason) {
            apply_partial_goal_audit(config, &contract_id, "Execution paused by the user")?;
            return pause_goal(config, GoalCyclePause::UserPaused);
        }
        if terminal_reason == "denied" {
            apply_partial_goal_audit(
                config,
                &contract_id,
                "Required permission or network access was denied",
            )?;
            return pause_goal(config, GoalCyclePause::NeedsUser);
        }
        if terminal_reason == "failed" {
            apply_partial_goal_audit(config, &contract_id, "The canonical Run failed")?;
            schedule_goal_continuation(config, lease_owner.as_deref(), goal_retry_delay(config))?;
            return Ok(GoalCycleOutcome::Paused);
        }
        if let Some(signal) = result.goal_signal.as_ref() {
            if signal.status.eq_ignore_ascii_case("completed") {
                let projection = config.load_projection()?
                    .ok_or("Goal projection missing")?;
                let acceptance = submit_goal_command(config, format!("{contract_id}:accept"),
                    GoalRuntimeCommand::AcceptCompletion { expected_revision: projection.revision,
                        contract_id: contract_id.clone(), claims: signal.claims.clone(),
                        now_epoch_ms: config.now_epoch_ms() });
                if acceptance.is_ok() {
                    return Ok(GoalCycleOutcome::Completed(std::mem::take(&mut result.handoff)));
                }
                let feedback = acceptance.err().unwrap();
                apply_partial_goal_audit(config, &contract_id, &feedback)?;
                // Rejected acceptance is not permission to repeatedly ask the
                // model to certify the same missing or unsupported evidence.
                // Preserve the partial record and allow explicit replanning.
                return pause_goal(config, GoalCyclePause::Incomplete);
            }
            apply_partial_goal_audit(config, &contract_id, &signal.summary)?;
            if signal.status == "incomplete" {
                return pause_goal(config, GoalCyclePause::Incomplete);
            }
            if matches!(signal.status.as_str(), "blocked" | "needs_user") {
                return pause_goal(config, GoalCyclePause::NeedsUser);
            }
            if signal.status == "failed" {
                let delay = goal_retry_delay(config);
                schedule_goal_continuation(config, lease_owner.as_deref(), delay)?;
                return Ok(GoalCycleOutcome::Paused);
            }
            request = compose_goal_prompt(
                goal_target,
                user_prompt,
                goal_requirements,
                true,
                Some(signal),
                None,
            );
            continue;
        }
        if result.pending_user_input {
            apply_partial_goal_audit(
                config,
                &contract_id,
                "User input queued for the next visible composer turn",
            )?;
            if let Some(owner) = lease_owner.as_deref() {
                submit_goal_command(
                    config,
                    format!("goal-yield:{owner}:{}", config.now_epoch_ms()),
                    GoalRuntimeCommand::Release {
                        owner: owner.to_string(),
                        wake_at_epoch_ms: None,
                        now_epoch_ms: config.now_epoch_ms(),
                    },
                )?;
            }
            return Ok(GoalCycleOutcome::Yielded(std::mem::take(&mut result.handoff)));
        }
        apply_partial_goal_audit(
            config,
            &contract_id,
            "Execution ended without an explicit Goal status",
        )?;
        if matches!(terminal_reason, "blocked" | "needs_user") {
            return pause_goal(config, GoalCyclePause::NeedsUser);
        }
        if terminal_reason == "incomplete" {
            return pause_goal(config, GoalCyclePause::Incomplete);
        }
        request = compose_goal_prompt(
            goal_target,
            user_prompt,
            goal_requirements,
            true,
            None,
            Some("No goal_complete signal was reported. Re-plan from existing evidence and report explicit goal status this cycle."),
        );
    }
}

pub fn compose_goal_prompt(
    goal: &str,
    user_prompt: &str,
    goal_requirements: &str,
    continuing: bool,
    signal: Option<&GoalCompleteSignal>,
    verifier_feedback: Option<&str>,
) -> String {
    let mut lines = vec![
        if continuing {
            "Autonomous goal continuation mode.".to_string()
        } else {
            "Autonomous goal mode active.".to_string()
        },
        format!("Goal: {goal}"),
        "Answer in the same language as the user's request unless the user explicitly asks for another language.".to_string(),
        "Keep working toward this goal without waiting for the user to type \"continue\".".to_string(),
        "Plan, inspect, edit, run focused validation, and iterate until the goal is complete or truly blocked.".to_string(),
        "Use runtime permission prompts for environment repair, dependency installation, network access, and destructive operations. Use request_user_input only for consequential product or environment decisions that cannot be inferred. Never request secret or credential values in the transcript; ask the user to configure them outside the session, then report needs_user.".to_string(),
        "Use only the current Host’s advertised tools and supported execution modes. If a required capability is unavailable, retain the current evidence and report blocked or needs_user; do not assume another platform’s shell, filesystem, or background execution capability.".to_string(),
    ];
    if !goal_requirements.trim().is_empty() {
        lines.push("Goal constraints and conditions:".to_string());
        for line in goal_requirements.lines() {
            let line = line.trim();
            if !line.is_empty() {
                lines.push(format!("  - {line}"));
            }
        }
    }
    if !continuing {
        lines.push(format!("Current request: {user_prompt}"));
    }
    if let Some(signal) = signal {
        lines.push(format!("Latest model signal: status={}", signal.status));
        lines.push(format!("Signal summary: {}", signal.summary));
        if !signal.evidence.is_empty() {
            lines.push(format!("Signal evidence: {}", signal.evidence.join("; ")));
        }
        if !signal.blockers.is_empty() {
            lines.push(format!("Signal blockers: {}", signal.blockers.join("; ")));
        }
        if let Some(next_action) = signal.next_action.as_deref() {
            if !next_action.is_empty() {
                lines.push(format!("Suggested next action: {next_action}"));
            }
        }
    }
    if let Some(verifier_feedback) = verifier_feedback {
        lines.push(format!("Verifier feedback: {verifier_feedback}"));
    }
    lines.push(
        "Use goal_complete tool every goal turn to report completion status with concise summary/evidence."
            .to_string(),
    );
    lines.push(
        "When fully complete, call goal_complete(status=\"completed\", summary=..., evidence=[...], claims=[{requirement_id:..., requirement_revision:..., summary:..., evidence_refs:[...], execution_evidence_refs:[...], task_replacements:[...]}]). Copy each pending requirement's semantic_revision from durable_goal_state and cover each ID exactly once. Cite committed tool results as tool:<call_id> in evidence_refs; use response only for an actual response deliverable. execution_evidence_refs is optional: leave it empty for ordinary reads or response deliverables. Only same-Goal observations with outcome=succeeded and state=completed qualify there; state=null is not typed completion, even when the tool succeeded. Required execution checks still need matching typed checks, resource and version. Independent review is optional: request_completion_review only when useful. Do not repeat completed work merely to report status.".to_string(),
    );
    lines.push(
        "To keep working automatically, call goal_complete(status=\"continue\", summary=..., next_action=...). Use incomplete to end this attempt with unfinished requirements and no automatic wake; explicit user resume is required. Use blocked/needs_user when human input or permission is required, and failed for an execution failure. Preserve blockers and evidence; none of these reports claims Goal completion."
            .to_string(),
    );
    lines.join("\n")
}


pub fn host_goal_control(components: &[nola_agent_engine::context_engine::ContextComponent]) -> Result<Option<(String, String)>, String> {
    let Some(component) = components
        .iter()
        .find(|component| component.id == "nola-goal-control")
    else {
        return Ok(None);
    };
    let value = match &component.content {
        Value::String(value) => serde_json::from_str::<Value>(value)
            .map_err(|error| format!("invalid nola-goal-control component: {error}"))?,
        value => value.clone(),
    };
    let objective = value
        .get("objective")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "nola-goal-control component is missing objective".to_string())?;
    let constraints = match value.get("constraints") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::String(value)) => value.trim().to_string(),
        Some(_) => {
            return Err("nola-goal-control constraints must be a string or array".to_string())
        }
        None => String::new(),
    };
    Ok(Some((objective.to_string(), constraints)))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_identity_is_action_scoped_and_replay_is_idempotent() {
        let first = create_goal_command("first-action", "Finish the work", "Verify output\nKeep input", 100);
        let mut projection = GoalProjection::default();
        projection.apply(first.clone()).unwrap();
        let committed = projection.clone();
        projection.apply(first).unwrap();
        assert_eq!(projection, committed, "retrying one Start is not another Goal");
        projection.apply(GoalRuntimeCommandEnvelope {
            operation_id: "finish-first".into(),
            command: GoalRuntimeCommand::ConcludeIncomplete { now_epoch_ms: 101 },
        }).unwrap();
        assert!(projection.apply(create_goal_command("second-action", "Finish the work", "Verify output", 102)).is_err(),
            "an incomplete Goal remains unfinished until explicitly cancelled or completed");
        projection.apply(GoalRuntimeCommandEnvelope {
            operation_id: "cancel-first".into(),
            command: GoalRuntimeCommand::Cancel { now_epoch_ms: 103 },
        }).unwrap();
        projection.apply(create_goal_command("second-action", "Finish the work", "Verify output", 104)).unwrap();
        let active = projection.active.unwrap();
        assert_eq!(active.id, "goal-second-action");
        assert_eq!(active.requirements.len(), 2);
        assert!(active.contracts.is_empty());
        assert_eq!(active.status, GoalStatus::Running);
        assert!(active.lease.is_none(), "Start is a durable intent, never proof of execution ownership");
    }
    #[test]
    fn recovery_error_after_committed_user_pause_never_schedules_resume() {
        use std::cell::{Cell, RefCell};
        struct Port { projection: RefCell<GoalProjection>, paused: Option<GoalProjection>, nonce: Cell<u64> }
        impl GoalRuntimePort for Port {
            type Handoff = ();
            fn session_id(&self) -> &str { "conversation" }
            fn load_projection(&self) -> Result<Option<GoalProjection>, String> { Ok(Some(self.projection.borrow().clone())) }
            fn submit(&self, command: GoalRuntimeCommandEnvelope) -> Result<(), String> { self.projection.borrow_mut().apply(command) }
            fn active_run_id(&self) -> Result<Option<String>, String> { Ok(Some("exact-run".into())) }
            fn prepare_exact_run(&mut self, run: &str) -> Result<nola_agent_engine::agent::AgentInputOrigin, String> {
                assert_eq!(run, "exact-run");
                Ok(nola_agent_engine::agent::AgentInputOrigin::GoalInteractiveUser)
            }
            fn execute_round(&mut self, _: &str, _: &str, _: &str, _: bool) -> Result<GoalRoundResult<()>, String> { panic!("must recover exact existing Run") }
            fn resume_round(&mut self, run: &str) -> Result<GoalRoundResult<()>, String> {
                assert_eq!(run, "exact-run");
                let projection = self.projection.borrow().clone();
                self.submit(GoalRuntimeCommandEnvelope { operation_id: "user-pause".into(),
                    command: GoalRuntimeCommand::GuardedControl {
                        expected_goal_id: projection.active.as_ref().unwrap().id.clone(),
                        expected_control_token: projection.control_token().unwrap(),
                        control: Box::new(GoalRuntimeCommand::Pause { status: GoalStatus::Paused,
                            wake_at_epoch_ms: None, now_epoch_ms: 100 }),
                    } })?;
                self.paused = Some(self.projection.borrow().clone());
                Err("recovery transport failed after Pause".into())
            }
            fn now_epoch_ms(&self) -> i64 { 100 }
            fn next_nonce(&self) -> String { let n=self.nonce.get()+1; self.nonce.set(n); n.to_string() }
            fn new_lease_owner(&self) -> String { "scheduled-owner".into() }
            fn owner_is_dead(&self, _: &str) -> bool { false }
            fn is_scheduled(&self) -> bool { true }
            fn explicit_resume(&self) -> bool { false }
            fn runtime_limit(&self) -> Duration { Duration::from_secs(60) }
            fn stop_requested(&self) -> bool { false }
            fn acknowledge_slice(&mut self) -> Result<(), String> { Ok(()) }
            fn discard_slice_snapshot(&mut self) {}
        }
        let mut projection = GoalProjection::default();
        projection.apply(create_goal_command("action", "Goal", "", 1)).unwrap();
        let mut port = Port { projection: RefCell::new(projection), paused: None, nonce: Cell::new(0) };
        begin_core_goal_round(&port, "Goal", "exact-run").unwrap();
        assert_eq!(run_goal_cycle(&mut port, "", "Goal", "").unwrap_err(), "recovery transport failed after Pause");
        assert_eq!(port.projection.borrow().clone(), port.paused.unwrap());
        let goal = port.projection.borrow().active.clone().unwrap();
        assert_eq!(goal.status, GoalStatus::Paused);
        assert!(goal.next_wake_at_epoch_ms.is_none());
    }

    #[test]
    fn incomplete_attempt_stops_and_requires_explicit_resume() {
        use std::cell::{Cell, RefCell};
        struct Port { projection: RefCell<GoalProjection>, nonce: Cell<u64>, signal: bool, rounds: usize, scheduled: bool, resume: bool }
        impl GoalRuntimePort for Port {
            type Handoff = ();
            fn session_id(&self) -> &str { "conversation" }
            fn load_projection(&self) -> Result<Option<GoalProjection>, String> { Ok(Some(self.projection.borrow().clone())) }
            fn submit(&self, command: GoalRuntimeCommandEnvelope) -> Result<(), String> { self.projection.borrow_mut().apply(command) }
            fn active_run_id(&self) -> Result<Option<String>, String> { Ok(None) }
            fn prepare_exact_run(&mut self, _: &str) -> Result<nola_agent_engine::agent::AgentInputOrigin, String> { unreachable!() }
            fn execute_round(&mut self, _: &str, _: &str, _: &str, _: bool) -> Result<GoalRoundResult<()>, String> {
                self.rounds += 1;
                assert_eq!(self.rounds, 1, "incomplete must not start another model round");
                Ok(GoalRoundResult { turn_status: AgentTurnStatus::Completed, output: "Work remains".into(),
                    goal_signal: self.signal.then(|| GoalCompleteSignal { status: "incomplete".into(), summary: "Work remains".into(),
                        claims: vec![], evidence: vec![], blockers: vec![], next_action: Some("Inspect remaining work".into()) }),
                    terminal_reason: Some("incomplete".into()), pending_user_input: false, handoff: () })
            }
            fn resume_round(&mut self, _: &str) -> Result<GoalRoundResult<()>, String> { unreachable!() }
            fn now_epoch_ms(&self) -> i64 { 100 }
            fn next_nonce(&self) -> String { let n=self.nonce.get()+1; self.nonce.set(n); n.to_string() }
            fn new_lease_owner(&self) -> String { format!("owner-{}", self.next_nonce()) }
            fn owner_is_dead(&self, _: &str) -> bool { false }
            fn is_scheduled(&self) -> bool { self.scheduled }
            fn explicit_resume(&self) -> bool { self.resume }
            fn runtime_limit(&self) -> Duration { Duration::from_secs(60) }
            fn stop_requested(&self) -> bool { false }
            fn acknowledge_slice(&mut self) -> Result<(), String> { Ok(()) }
            fn discard_slice_snapshot(&mut self) {}
        }
        for signal in [true, false] {
            let mut projection = GoalProjection::default();
            projection.apply(create_goal_command("action", "Goal", "Inspect work", 1)).unwrap();
            let mut port=Port { projection: RefCell::new(projection), nonce: Cell::new(0), signal, rounds:0, scheduled:false, resume:false };
            assert!(matches!(run_goal_cycle(&mut port, "", "Goal", "Inspect work").unwrap(), GoalCycleOutcome::Paused));
            let goal=port.projection.borrow().active.clone().unwrap();
            assert_eq!(goal.status, GoalStatus::Paused);
            assert_eq!(goal.stop_reason, Some(nola_agent_engine::agent::goal::GoalStopReason::Incomplete));
            assert!(goal.next_wake_at_epoch_ms.is_none());
            assert!(goal.requirements.values().all(|r|r.status != GoalRecordStatus::Completed));
            port.scheduled=true;
            assert!(ensure_core_goal_started(&mut port, "Goal", "Inspect work", true).is_err());
            port.resume=true;
            ensure_core_goal_started(&mut port, "Goal", "Inspect work", true).unwrap();
            assert_eq!(port.projection.borrow().active.as_ref().unwrap().status, GoalStatus::Running);
            assert_eq!(port.rounds,1);
        }
    }

}
