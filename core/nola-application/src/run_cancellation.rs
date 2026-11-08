// SPDX-License-Identifier: Apache-2.0
//! Persist cancellation intent through Core, without executing any Host work.
use nola_domain::{AgentTurnStatus, RunRecoveryObservation};
use nola_agent_engine::agent::recovery::CoreCheckpoint;
use nola_agent_engine::agent::{AgentSubmission, AgentThreadState};
use nola_store_port::{
    run_effect::RunEffectRepository, run_recovery_inspection::RunRecoveryInspectionRepository,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestRunCancellation {
    pub conversation_id: String,
    pub expected_run_id: String,
    pub assistant_message_id: String,
    pub cancellation_id: String,
    pub reason: Option<String>,
    pub requested_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCancellationDisposition {
    CancellationCommitted,
    AlreadyCancelling,
    AlreadyTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCancellationReceipt {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub cancellation_id: String,
    pub disposition: RunCancellationDisposition,
    pub terminal_result: Option<nola_domain::run_result::RunResult>,
    pub result_digest: Option<nola_domain::assistant_journal::AssistantDigest>,
}

pub(crate) struct PreparedRunCancellation {
    pub receipt: RunCancellationReceipt,
    pub submission: Option<crate::ReduceAndCommitActiveSubmission>,
}

/// The received-command transaction owns this inspection and the following
/// Core transition. No client-provided configuration, status, or OS PID is
/// interpreted as cancellation completion evidence.
pub(crate) fn prepare<R>(
    store: &R,
    input: RequestRunCancellation,
    current_control: Option<nola_agent_engine::agent::protocol::AgentControlSnapshot>,
) -> Result<PreparedRunCancellation, String>
where
    R: RunEffectRepository + RunRecoveryInspectionRepository + ?Sized,
{
    if [
        &input.conversation_id,
        &input.expected_run_id,
        &input.assistant_message_id,
        &input.cancellation_id,
    ]
    .iter()
    .any(|s| s.trim().is_empty() || s.len() > 1024)
        || input.requested_at_ms <= 0
        || input.reason.as_ref().is_some_and(|s| s.len() > 4096)
    {
        return Err(
            "cancellation requires bounded exact identities and a positive timestamp".into(),
        );
    }
    let inspection = crate::inspect_run_recovery(
        store,
        input.conversation_id.clone(),
        input.expected_run_id.clone(),
        input.assistant_message_id.clone(),
    )?;
    let mut receipt = RunCancellationReceipt {
        conversation_id: input.conversation_id.clone(),
        run_id: input.expected_run_id.clone(),
        assistant_message_id: input.assistant_message_id,
        cancellation_id: input.cancellation_id.clone(),
        disposition: RunCancellationDisposition::CancellationCommitted,
        terminal_result: None,
        result_digest: None,
    };
    match inspection.observation {
        RunRecoveryObservation::Terminal { result, result_digest, .. } => {
            receipt.disposition = RunCancellationDisposition::AlreadyTerminal;
            receipt.terminal_result = Some(result);
            receipt.result_digest = Some(result_digest);
            return Ok(PreparedRunCancellation {
                receipt,
                submission: None,
            });
        }
        RunRecoveryObservation::Active {
            source_host_id: None,
            ..
        } => {}
        _ => {
            return Err(
                "cancellation requires the exact locally owned canonical Run and assistant".into(),
            )
        }
    }
    let frozen = store
        .load_run_invocation_config(&input.expected_run_id)
        .map_err(|e| e.to_string())?
        .ok_or("cancellation requires frozen Run configuration")?;
    crate::validate_run_invocation_config_digest(
        store,
        &input.expected_run_id,
        &frozen.invocation_config_digest,
    )?;
    let turn_status = if let Some(control) = current_control {
        // Supplied only by the Runtime Supervisor after matching the Run,
        // conversation and revision read in this same command transaction.
        Some(control.turn_status)
    } else {
        let stored_checkpoint = crate::load_current_checkpoint(store, &input.conversation_id)?
            .ok_or("cancellation requires a committed checkpoint")?;
        let checkpoint: CoreCheckpoint<AgentThreadState> =
            serde_json::from_str(&stored_checkpoint.checkpoint_json)
                .map_err(|_| "cancellation checkpoint is invalid")?;
        checkpoint.validate_state_identity()?;
        if stored_checkpoint.conversation_id != input.conversation_id
            || checkpoint.session_id.trim().is_empty()
            || checkpoint.state.session_id.as_str() != checkpoint.session_id
            || i64::try_from(checkpoint.last_event_cursor.sequence).ok()
                != Some(stored_checkpoint.event_cursor)
        {
            return Err("cancellation checkpoint conversation mismatch".into());
        }
        checkpoint.state.execution.active_turn.as_ref().map(|turn| turn.status)
    };
    if turn_status == Some(AgentTurnStatus::Cancelling) {
        receipt.disposition = RunCancellationDisposition::AlreadyCancelling;
        return Ok(PreparedRunCancellation {
            receipt,
            submission: None,
        });
    }
    let transition_id = format!(
        "cancel:{}:{:x}",
        input.expected_run_id,
        Sha256::digest(input.cancellation_id.as_bytes())
    );
    Ok(PreparedRunCancellation {
        receipt,
        submission: Some(crate::ReduceAndCommitActiveSubmission {
            conversation_id: input.conversation_id,
            run_id: input.expected_run_id,
            transition_id,
            invocation_config_digest: frozen.invocation_config_digest,
            submission: AgentSubmission::Interrupt {
                goal_control: None, cancellation_id: input.cancellation_id,
                reason: input.reason,
            },
            committed_at_ms: input.requested_at_ms,
        }),
    })
}

pub(crate) fn complete<R: RunRecoveryInspectionRepository + ?Sized>(
    store: &R,
    mut receipt: RunCancellationReceipt,
) -> Result<RunCancellationReceipt, String> {
    let inspection = crate::inspect_run_recovery(store, receipt.conversation_id.clone(),
        receipt.run_id.clone(), receipt.assistant_message_id.clone())?;
    if let RunRecoveryObservation::Terminal { result, result_digest, .. } = inspection.observation {
        receipt.terminal_result = Some(result);
        receipt.result_digest = Some(result_digest);
    }
    Ok(receipt)
}

pub fn request_run_cancellation<R>(
    store: &mut R,
    input: RequestRunCancellation,
) -> Result<RunCancellationReceipt, String>
where
    R: RunEffectRepository + RunRecoveryInspectionRepository + crate::repositories::AssistantPublicationRepositories + ?Sized,
{
    let prepared = prepare(store, input, None)?;
    match prepared.submission {
        None => Ok(prepared.receipt),
        Some(submission) => {
            let _ = crate::reduce_and_commit_active_submission_with_core(
                store, submission, None, Vec::new())?;
            complete(store, prepared.receipt)
        }
    }
}
