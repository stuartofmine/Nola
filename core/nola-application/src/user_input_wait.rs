// SPDX-License-Identifier: Apache-2.0
//! Answers target a committed question, never a process-local window or grant.
use crate::{
    repositories::AgentRuntimeCommandRepositories, AcceptedActiveSubmission,
    ReduceAndCommitActiveSubmission,
};
use nola_domain::user_input_wait::{RespondToUserInputWait, UserInputWaitState};
use nola_store_port::user_input_wait::StoredUserInputWait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RespondToConversationWait {
    pub conversation_id: String,
    pub run_id: String,
    pub response: RespondToUserInputWait,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitResumePolicy {
    ConversationRun,
    TaskScheduler,
}

fn resume_policy<R: nola_store_port::runtime_task::RuntimeTaskRepository + ?Sized>(
    store: &R,
    conversation: &str,
) -> Result<WaitResumePolicy, String> {
    Ok(
        if store
            .is_runtime_task_child_session(conversation)
            .map_err(|e| e.to_string())?
        {
            WaitResumePolicy::TaskScheduler
        } else {
            WaitResumePolicy::ConversationRun
        },
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitConflictReason {
    SchemaChanged,
    WaitNotOpen,
    RevisionChanged,
    CancellationRequested,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConversationWaitResponse {
    Applied {
        wait: StoredUserInputWait,
        submission: AcceptedActiveSubmission,
        resume_policy: WaitResumePolicy,
    },
    Replay {
        wait: StoredUserInputWait,
        resume_policy: WaitResumePolicy,
    },
    Conflict {
        wait: StoredUserInputWait,
        reason: WaitConflictReason,
    },
}

pub(crate) enum PreparedWaitResponse {
    Apply(ReduceAndCommitActiveSubmission),
    Settled(ConversationWaitResponse),
}

pub(crate) fn prepare<
    R: nola_store_port::user_input_wait::UserInputWaitRepository
        + nola_store_port::run_effect::RunEffectRepository
        + nola_store_port::runtime_task::RuntimeTaskRepository
        + ?Sized,
>(
    store: &R,
    input: &RespondToConversationWait,
) -> Result<PreparedWaitResponse, String> {
    if input.conversation_id.trim().is_empty()
        || input.run_id.trim().is_empty()
        || input.committed_at_ms < 0
    {
        return Err("invalid wait response owner or time".into());
    }
    crate::expert_peer_task::validate_expert_question_execution(store,&input.conversation_id,&input.run_id)?;
    let digest = input.response.digest()?;
    let saved = store
        .load_user_input_wait(&input.conversation_id, &input.response.wait_id)
        .map_err(|e| e.to_string())?
        .ok_or("user input wait not found in this conversation")?;
    if saved.run_id != input.run_id {
        return Err("wait response belongs to another Run".into());
    }
    saved.wait.validate()?;
    if saved.wait.response_schema_digest != input.response.response_schema_digest {
        return Ok(PreparedWaitResponse::Settled(
            ConversationWaitResponse::Conflict {
                wait: saved,
                reason: WaitConflictReason::SchemaChanged,
            },
        ));
    }
    saved
        .wait
        .question_set
        .validate_response(&input.response.response)?;
    if let UserInputWaitState::Resolved {
        command_id,
        response_digest,
    } = &saved.wait.state
    {
        if command_id == &input.response.command_id && response_digest == &digest {
            return Ok(PreparedWaitResponse::Settled(
                ConversationWaitResponse::Replay {
                    resume_policy: resume_policy(store, &saved.conversation_id)?,
                    wait: saved,
                },
            ));
        }
    }
    if !saved.wait.is_open() {
        return Ok(PreparedWaitResponse::Settled(
            ConversationWaitResponse::Conflict {
                wait: saved,
                reason: WaitConflictReason::WaitNotOpen,
            },
        ));
    }
    if saved.wait.revision != input.response.expected_revision {
        return Ok(PreparedWaitResponse::Settled(
            ConversationWaitResponse::Conflict {
                wait: saved,
                reason: WaitConflictReason::RevisionChanged,
            },
        ));
    }
    // Both hot Runtime and cold command dispatch call this under the same
    // write UoW as the answer reduction. Preserve an earlier exact Replay,
    // but cancellation already committed wins over every new answer.
    if store
        .user_input_wait_task_cancel_requested(&input.conversation_id, &input.run_id)
        .map_err(|error| error.to_string())?
    {
        return Ok(PreparedWaitResponse::Settled(
            ConversationWaitResponse::Conflict {
                wait: saved,
                reason: WaitConflictReason::CancellationRequested,
            },
        ));
    }
    let active = store
        .active_run(&input.conversation_id)
        .map_err(|e| e.to_string())?
        .ok_or("open wait has no active Run")?;
    if active.run_id != input.run_id {
        return Err("wait no longer owns the active Run".into());
    }
    Ok(PreparedWaitResponse::Apply(
        ReduceAndCommitActiveSubmission {
            conversation_id: input.conversation_id.clone(),
            run_id: input.run_id.clone(),
            transition_id: format!("user-input-wait:{}", input.response.command_id),
            invocation_config_digest: active
                .invocation_config_digest
                .ok_or("wait Run has no frozen configuration")?,
            submission: crate::AgentSubmission::RespondToUserInputWait {
                input: input.response.clone(),
            },
            committed_at_ms: input.committed_at_ms,
        },
    ))
}

pub(crate) fn applied<
    R: nola_store_port::user_input_wait::UserInputWaitRepository
        + nola_store_port::runtime_task::RuntimeTaskRepository
        + ?Sized,
>(
    store: &R,
    input: &RespondToConversationWait,
    submission: AcceptedActiveSubmission,
) -> Result<ConversationWaitResponse, String> {
    let wait = store
        .load_user_input_wait(&input.conversation_id, &input.response.wait_id)
        .map_err(|e| e.to_string())?
        .ok_or("committed response has no durable wait")?;
    if wait.run_id != input.run_id || wait.wait.is_open() {
        return Err("committed response did not resolve its exact wait".into());
    }
    Ok(ConversationWaitResponse::Applied {
        resume_policy: resume_policy(store, &wait.conversation_id)?,
        wait,
        submission,
    })
}

pub(crate) fn respond<R: AgentRuntimeCommandRepositories>(
    store: &mut R,
    input: RespondToConversationWait,
) -> Result<ConversationWaitResponse, String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        match prepare(store, &input)? {
            PreparedWaitResponse::Settled(result) => Ok(result),
            PreparedWaitResponse::Apply(command) => {
                let submission = crate::reduce_and_commit_active_submission(store, command)?;
                applied(store, &input, submission)
            }
        }
    })
}
