// SPDX-License-Identifier: Apache-2.0
//! Read immutable execution identity, never the raw prompt/tool configuration.
use nola_host_protocol::HostInvocationBinding;
use nola_agent_engine::agent::{
    protocol::AgentCoreConfig, recovery::CoreCheckpoint, AgentThreadState,
};
use nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub fn list_recoverable_runs<R: RunRecoveryInspectionRepository + ?Sized>(
    store: &R, after_run_id: Option<String>, limit: usize,
) -> Result<Vec<nola_store_port::run_recovery_inspection::RecoverableRunRef>, String> {
    if !(1..=256).contains(&limit) || after_run_id.as_ref().is_some_and(|id| id.trim().is_empty()) {
        return Err("invalid recoverable Run discovery page".into());
    }
    store.list_recoverable_runs(after_run_id.as_deref(), limit).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExactRunInvocation {
    pub remote_tool_plan: Option<nola_domain::host_tool_plan::HostToolPlanSnapshot>,
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub invocation_config_digest: String,
    pub status: String,
    pub host_invocation: HostInvocationBinding,
    /// Exact admitted context policy, not recomputed from current model caps.
    pub context_budget: nola_domain::ContextBudgetFacts,
    /// Actual Core identity, proven by this Run's checkpoint in the same read.
    /// A Run admitted before checkpoint persistence has no Core witness yet.
    pub core_session_id: Option<String>,
    pub checkpoint_revision: Option<i64>,
    pub checkpoint_event_cursor: Option<i64>,
    /// Hash of the current reconstructed checkpoint returned by LoadCheckpoint,
    /// not merely the persisted baseline when a committed replay tail exists.
    pub checkpoint_sha256: Option<String>,
}

pub fn load_exact_run_invocation<R: RunRecoveryInspectionRepository + ?Sized>(
    store: &R,
    conversation_id: String,
    expected_run_id: String,
    assistant_message_id: String,
) -> Result<Option<ExactRunInvocation>, String> {
    load_exact_run_invocation_with_config(store, conversation_id, expected_run_id, assistant_message_id)
        .map(|value| value.map(|(exact, _)| exact))
}

// Carry the already-verified configuration through the same pinned read.
// Native/remote callers still receive only the stable public witness above.
pub(crate) fn load_exact_run_invocation_with_config<R: RunRecoveryInspectionRepository + ?Sized>(
    store: &R,
    conversation_id: String,
    expected_run_id: String,
    assistant_message_id: String,
) -> Result<Option<(ExactRunInvocation, AgentCoreConfig)>, String> {
    if [&conversation_id, &expected_run_id, &assistant_message_id]
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err("exact Run invocation requires all three identities".into());
    }
    let Some(stored) = store
        .load_exact_run_invocation(&conversation_id, &expected_run_id, &assistant_message_id)
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if stored.conversation_id != conversation_id
        || stored.run_id != expected_run_id
        || stored.assistant_message_id != assistant_message_id
    {
        return Err("Run invocation identity mismatch".into());
    }
    let actual = format!(
        "{:x}",
        Sha256::digest(stored.invocation_config_json.as_bytes())
    );
    if actual != stored.invocation_config_digest {
        return Err("Run invocation config integrity failure".into());
    }
    // Decode diagnostics must not echo raw config values, which can contain
    // context/tool material outside this query's disclosure contract.
    let config: AgentCoreConfig = serde_json::from_str(&stored.invocation_config_json)
        .map_err(|_| "invalid stored Run invocation config")?;
    let binding = config
        .turn_identity
        .host_invocation
        .as_ref()
        .ok_or("Run has no frozen Host invocation binding")?;
    binding.validate().map_err(str::to_string)?;
    let terminal = stored.terminal.as_ref();
    if (stored.status == "sealed") != terminal.is_some()
        || stored.completed_at_ms != terminal.map(|value| value.completed_at_ms)
    {
        return Err("exact Run terminal header and result seal differ".into());
    }
    if let Some(terminal) = terminal {
        if terminal.conversation_id != conversation_id || terminal.run_id != expected_run_id
            || terminal.assistant_message_id != assistant_message_id
            || terminal.result.conversation_id != conversation_id
            || terminal.result.run_id != expected_run_id
            || terminal.result.computed_digest()? != terminal.result_digest
        {
            return Err("exact Run terminal result ownership or digest mismatch".into());
        }
    }
    let (core_session_id, checkpoint_revision, checkpoint_event_cursor, checkpoint_sha256) =
        match stored.core_checkpoint {
            None if terminal.is_none() => (None, None, None, None),
            None => return Err("sealed Run has no owned checkpoint".into()),
            Some(proof) => {
                let has_replay_tail = !proof.recovery.recovery.replay_inputs.is_empty();
                let initial_admission = proof.initial_admission;
                if proof
                    .recovery
                    .recovery
                    .replay_base
                    .as_ref()
                    .is_some_and(|base| base.run_id != expected_run_id)
                {
                    return Err("exact Run recovery belongs to another Run".into());
                }
                if proof
                    .recovery
                    .invocation
                    .as_ref()
                    .is_some_and(|invocation| {
                        invocation.run_id != expected_run_id
                            || invocation.invocation_config_digest
                                != stored.invocation_config_digest
                            || invocation.invocation_config_json != stored.invocation_config_json
                    })
                {
                    return Err(
                        "exact Run replay configuration differs from its frozen invocation".into(),
                    );
                }
                // The repository may reuse a presentation materialization only
                // while reading all witnesses from the same pinned snapshot.
                // A header mismatch is not permission to trust the memo. All
                // decoded identity/config/terminal checks below still run.
                let baseline = &proof.recovery.recovery.checkpoint;
                let memo = proof.recovered_checkpoint.filter(|memo| {
                    memo.conversation_id == baseline.conversation_id
                        && memo.revision == baseline.revision
                        && memo.event_cursor == baseline.event_cursor
                        && memo.updated_at_ms == baseline.updated_at_ms
                });
                let current = match memo {
                    Some(current) => current,
                    None => crate::session_projection::materialize_local_recovery_with_config(
                        proof.recovery, config.clone(),
                    )?,
                };
                let checkpoint: CoreCheckpoint<AgentThreadState> =
                    serde_json::from_str(&current.checkpoint_json)
                        .map_err(|_| "invalid exact Run checkpoint")?;
                let core_session = crate::assistant_owner::validate_checkpoint_identity(
                    &conversation_id,
                    &current.conversation_id,
                    &checkpoint.session_id,
                    checkpoint.state.session_id.as_str(),
                )
                .map_err(|_| "exact Run checkpoint ownership mismatch")?;
                checkpoint.validate_state_identity()?;
                if i64::try_from(checkpoint.last_event_cursor.sequence).ok()
                    != Some(current.event_cursor)
                {
                    return Err("exact Run checkpoint event cursor mismatch".into());
                }
                if config
                    .session_id
                    .as_ref()
                    .is_some_and(|id| id.as_str() != core_session)
                {
                    return Err("exact Run frozen Core session identity mismatch".into());
                }
                if proof.admission_turn_id.as_deref() != Some(expected_run_id.as_str()) {
                    return Err("exact Run admission Turn identity mismatch".into());
                }
                let latest_turn = match proof.latest_started_turn_id.as_deref() {
                    Some(id) if !id.trim().is_empty() => id,
                    Some(_) => return Err("exact Run has invalid committed Turn ownership".into()),
                    None => expected_run_id.as_str(),
                };
                if proof.latest_started_turn_id.is_some()
                    && proof.latest_started_session_id.as_deref() != Some(core_session)
                {
                    return Err("exact Run committed Turn Core identity mismatch".into());
                }
                let frozen = checkpoint
                    .state
                    .configuration
                    .turn_identity
                    .host_invocation
                    .as_ref()
                    .ok_or("exact Run checkpoint has no frozen Host invocation binding")?;
                binding.require_exact(frozen).map_err(str::to_string)?;
                if let Some(terminal) = terminal {
                    let decision = checkpoint.state.execution.requirements.decision.as_ref()
                        .ok_or("sealed Run checkpoint has no result decision")?;
                    if decision.result_id != terminal.result.result_id
                        || decision.value != terminal.result.value
                        || decision.requirement_revision != terminal.result.requirement_revision
                        || decision.evidence_frontier != terminal.result.evidence_frontier
                        || terminal.result.decided_at_revision.0 > checkpoint.revision
                        || terminal.result.decision_event_cursor.0 > checkpoint.last_event_cursor.sequence
                        || !proof.committed_by_run
                    {
                        return Err("sealed Run checkpoint result differs from its immutable seal".into());
                    }
                }
                match checkpoint.state.execution.active_turn.as_ref() {
                    Some(turn) => {
                        if turn.turn_id.as_str() != latest_turn
                            || (terminal.is_none() && turn.status.is_terminal())
                            || terminal.is_some_and(|terminal| {
                                use nola_domain::{AgentTurnStatus, run_result::RunResultValue};
                                let expected = match &terminal.result.value {
                                    RunResultValue::Cancelled { .. } => AgentTurnStatus::Cancelled,
                                    RunResultValue::Failed { .. } => AgentTurnStatus::Failed,
                                    RunResultValue::Answered { .. } | RunResultValue::ActionCompleted { .. } | RunResultValue::DelegationAccepted { .. }
                                    | RunResultValue::Incomplete { .. } | RunResultValue::Blocked { .. } => AgentTurnStatus::Completed,
                                };
                                turn.status != expected
                            })
                        {
                            return Err("exact Run checkpoint belongs to another Turn".into());
                        }
                        let turn_binding = turn
                            .context
                            .configuration
                            .turn_identity
                            .host_invocation
                            .as_ref()
                            .ok_or("exact Run Turn has no frozen Host invocation binding")?;
                        binding
                            .require_exact(turn_binding)
                            .map_err(str::to_string)?;
                        // An initial Run has not committed a Host transition.
                        // Prove its actual admission envelope, not the absence
                        // of a TurnStarted event or a guessed first Store CAS.
                        if !proof.committed_by_run {
                            let initial = initial_admission
                                .as_ref()
                                .ok_or("exact Run has no initial admission checkpoint witness")?;
                            if has_replay_tail
                                || terminal.is_some()
                                || initial.session_id != core_session
                                || initial.turn_id != expected_run_id
                                || turn.turn_id.as_str() != expected_run_id
                                || proof.latest_started_turn_id.as_deref()
                                    != Some(initial.turn_id.as_str())
                                || initial.revision != checkpoint.revision
                                || initial.event_cursor == 0
                                || initial.event_cursor > checkpoint.last_event_cursor.sequence
                            {
                                return Err(
                                    "exact Run initial admission checkpoint witness differs".into(),
                                );
                            }
                        }
                    }
                    None if terminal.is_some() && proof.committed_by_run => {}
                    None => {
                        return Err(
                            "exact Run checkpoint has no owned current or terminal Turn".into()
                        )
                    }
                }
                (
                    Some(core_session.to_owned()),
                    Some(current.revision),
                    Some(current.event_cursor),
                    Some(format!(
                        "{:x}",
                        Sha256::digest(current.checkpoint_json.as_bytes())
                    )),
                )
            }
        };
    Ok(Some((ExactRunInvocation {
        remote_tool_plan: config.turn_identity.remote_tool_plan.clone(),
        conversation_id,
        run_id: expected_run_id,
        assistant_message_id,
        invocation_config_digest: stored.invocation_config_digest,
        status: stored.status,
        host_invocation: binding.clone(),
        context_budget: config.context_window.budget_facts(),
        core_session_id,
        checkpoint_revision,
        checkpoint_event_cursor,
        checkpoint_sha256,
    }, config)))
}
