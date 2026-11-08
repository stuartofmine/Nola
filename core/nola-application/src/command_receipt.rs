// SPDX-License-Identifier: Apache-2.0
//! Idempotent Application command admission and durable outcome replay.

use std::time::{SystemTime, UNIX_EPOCH};

use nola_domain::{ProjectionExecutionOwner, RunAttemptId, RunId};
use nola_store_port::protocol_journal::{
    BeginCommandReceiptInput, BeginCommandReceiptResult, CommandProtocolRepositoryProvider,
    CommandReceiptRepository, CommandRevisionEntity, CommandTransactionRepository,
    CompleteCommandReceiptOutcome, ReceiptAdmissionError, StoredCommandReceipt,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    projection_journal::conversation_id_for_command, ApplicationCommand, ApplicationRuntime,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteReceivedCommand {
    pub client_id: String,
    pub request_id: String,
    pub command_type: String,
    pub command_json: String,
    pub expected_revision: Option<u64>,
    pub received_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationCommandReceipt {
    pub client_id: String,
    pub request_id: String,
    pub received_at: String,
    pub completed_at: Option<String>,
    pub replayed: bool,
    pub outcome: ApplicationCommandReceiptOutcome,
}

fn measurement_command_identity(command: &ApplicationCommand) -> (nola_measurement::CommandKind, Option<&str>) {
    use nola_measurement::CommandKind;
    match command {
        ApplicationCommand::BeginAssistantTurn { .. } => (CommandKind::BeginTurn, None),
        ApplicationCommand::BeginTaskAssistantTurn { .. } => (CommandKind::TaskTurn, None),
        ApplicationCommand::BeginAutomationAssistantTurn { .. } => (CommandKind::AutomationTurn, None),
        ApplicationCommand::PromoteQueuedAssistantTurn { .. } => (CommandKind::QueuedTurn, None),
        ApplicationCommand::BeginQueuedUserTurn { .. } => (CommandKind::QueuedTurn, None),
        ApplicationCommand::ReduceAndCommitActiveSubmission { input } =>
            (CommandKind::ActiveSubmission, Some(&input.run_id)),
        ApplicationCommand::ReduceAndCommitIdleSubmission { .. } => (CommandKind::IdleSubmission, None),
        _ => (CommandKind::Other, None),
    }
}

fn measurement_receipt_outcome(outcome: &ApplicationCommandReceiptOutcome) -> nola_measurement::Outcome {
    match outcome {
        ApplicationCommandReceiptOutcome::Committed { .. } => nola_measurement::Outcome::Succeeded,
        ApplicationCommandReceiptOutcome::Rejected { .. } | ApplicationCommandReceiptOutcome::Conflict { .. } =>
            nola_measurement::Outcome::Rejected,
        ApplicationCommandReceiptOutcome::RecoveryRequired => nola_measurement::Outcome::Incomplete,
    }
}

fn measurement_new_receipt(receipt: &ApplicationCommandReceipt) -> Option<nola_measurement::Outcome> {
    (!receipt.replayed).then(|| measurement_receipt_outcome(&receipt.outcome))
}

fn measurement_admitted_run(kind: Option<nola_measurement::CommandKind>, outcome: &ApplicationCommandReceiptOutcome) -> Option<&str> {
    use nola_measurement::CommandKind;
    // Only AcceptedAssistantTurn has this result shape. Never discover a Run
    // by recursively inspecting arbitrary JSON or user-controlled payloads.
    if !matches!(kind, Some(CommandKind::BeginTurn | CommandKind::TaskTurn |
        CommandKind::AutomationTurn | CommandKind::QueuedTurn)) { return None; }
    let ApplicationCommandReceiptOutcome::Committed { result, .. } = outcome else { return None; };
    result.get("begin")?.get("run_id")?.as_str().filter(|value| !value.is_empty())
}

impl ApplicationCommandReceipt {
    /// Transport-independent receipt projection for daemon and embedded hosts.
    /// A recovered admission is still accepted, never a fabricated terminal failure.
    pub fn into_protocol_receipt(self) -> nola_protocol::CommandReceipt {
        use nola_protocol::{
            CommandReceipt, CommandReceiptOutcome, ProtocolFailure, RevisionConflict,
        };
        let completed_at = self.completed_at.unwrap_or_else(timestamp_now);
        let outcome = match self.outcome {
            ApplicationCommandReceiptOutcome::RecoveryRequired => CommandReceiptOutcome::Accepted,
            ApplicationCommandReceiptOutcome::Committed {
                committed_revision,
                result,
            } => CommandReceiptOutcome::Committed {
                completed_at,
                committed_revision,
                result,
            },
            ApplicationCommandReceiptOutcome::Rejected {
                code,
                message,
                recoverable,
                details,
            } => CommandReceiptOutcome::Rejected {
                completed_at,
                error: ProtocolFailure {
                    code,
                    message,
                    recoverable,
                    details,
                },
            },
            ApplicationCommandReceiptOutcome::Conflict {
                scope_id,
                expected_revision,
                actual_revision,
            } => CommandReceiptOutcome::Conflict {
                completed_at,
                conflict: RevisionConflict {
                    scope_id,
                    expected_revision,
                    actual_revision,
                },
            },
        };
        CommandReceipt {
            protocol_version: nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
            request_id: self.request_id,
            client_id: self.client_id,
            received_at: self.received_at,
            outcome,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationCommandReceiptOutcome {
    Committed {
        committed_revision: Option<u64>,
        result: Value,
    },
    Rejected {
        code: String,
        message: String,
        recoverable: bool,
        details: Option<Value>,
    },
    Conflict {
        scope_id: String,
        expected_revision: u64,
        actual_revision: u64,
    },
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationCommandExecutionError {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

impl std::fmt::Display for ApplicationCommandExecutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApplicationCommandExecutionError {}

impl ApplicationCommandExecutionError {
    /// Shared native/daemon owner retirement classification. A failed ACK for
    /// these boundaries is never proof that the business intent was rejected.
    pub fn code_requires_owner_reopen(code: &str) -> bool {
        nola_protocol::command_outcome_requires_owner_reopen(code)
    }

    pub fn requires_owner_reopen(&self) -> bool { Self::code_requires_owner_reopen(&self.code) }
}



#[derive(Debug, Serialize, Deserialize)]
struct StoredFailure {
    code: String,
    message: String,
    recoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredConflict {
    scope_id: String,
    expected_revision: u64,
    actual_revision: u64,
}

fn load_receipt<R: CommandReceiptRepository + ?Sized>(
    repository: &R,
    client_id: &str,
    request_id: &str,
) -> Result<Option<ApplicationCommandReceipt>, ApplicationCommandExecutionError> {
    repository
        .command_receipt(client_id, request_id)
        .map_err(|error| {
            execution_error(
                "receipt_read_failed",
                format!("load Application command receipt failed: {error}"),
                true,
            )
        })?
        .map(|receipt| decode_receipt(receipt, true))
        .transpose()
}

impl<R: CommandProtocolRepositoryProvider> ApplicationRuntime<R> {
    pub fn command_receipt(
        &mut self,
        client_id: &str,
        request_id: &str,
    ) -> Result<Option<ApplicationCommandReceipt>, ApplicationCommandExecutionError> {
        load_receipt(
            &self.store.command_protocol_repository(),
            client_id,
            request_id,
        )
    }
}

impl<R: crate::repositories::ReceivedCommandRepositories> ApplicationRuntime<R> {
    pub fn execute_received_command(
        &mut self,
        input: ExecuteReceivedCommand,
        command: ApplicationCommand,
    ) -> Result<ApplicationCommandReceipt, ApplicationCommandExecutionError> {
        let (kind, run) = measurement_command_identity(&command);
        let context = nola_measurement::command_context(&input.client_id, &input.request_id,
            conversation_id_for_command(&command), run, kind);
        let _scope = nola_measurement::enter(context);
        let measurement = nola_measurement::Span::start(nola_measurement::Stage::CommandExecution);
        let _child = nola_measurement::enter(measurement.child_context());
        let result = self.execute_received_command_measured(input, command);
        measurement.finish(match &result {
            Ok(receipt) => measurement_receipt_outcome(&receipt.outcome),
            Err(_) => nola_measurement::Outcome::Failed,
        });
        result
    }

    fn execute_received_command_measured(
        &mut self,
        input: ExecuteReceivedCommand,
        command: ApplicationCommand,
    ) -> Result<ApplicationCommandReceipt, ApplicationCommandExecutionError> {
        self.ensure_runtime_usable()
            .map_err(|error| execution_error("runtime_reopen_required", error, false))?;
        // Only typed commands with a defined single-entity scope can use CAS.
        if input.expected_revision.is_some() && command_revision_scope(&command).is_none() {
            return Err(execution_error(
                "unsupported_revision_precondition",
                "this command does not support envelope revision preconditions; no command was executed",
                false,
            ));
        }
        CommandTransactionRepository::begin_write_unit_of_work(
            &mut self.store.command_protocol_repository(),
        )
        .map_err(|error| {
            execution_error(
                "command_transaction_begin_failed",
                format!("begin Application command transaction failed: {error}"),
                true,
            )
        })?;
        if let Err(error) = self.begin_runtime_unit_of_work() {
            let rollback = CommandTransactionRepository::rollback_write_unit_of_work(
                &mut self.store.command_protocol_repository(),
            );
            if let Err(rollback_error) = rollback {
                let message = format!(
                    "rollback Application command transaction after Runtime begin failed: {error}; rollback failed; reopen required: {rollback_error}"
                );
                self.poison_runtime(message.clone());
                return Err(execution_error(
                    "command_transaction_rollback_failed",
                    message,
                    false,
                ));
            }
            return Err(execution_error(
                "runtime_transaction_begin_failed",
                error,
                false,
            ));
        }
        let outcome = self.execute_received_command_in_unit_of_work(input, command);
        match outcome {
            Ok(receipt) => {
                let commit = {
                    let mut protocol = self.store.command_protocol_repository();
                    protocol.commit_write_unit_of_work()
                };
                if let Err(error) = commit {
                    let rollback_error = {
                        let mut protocol = self.store.command_protocol_repository();
                        protocol.rollback_write_unit_of_work().err()
                    };
                    let message = match rollback_error {
                        Some(rollback_error) => format!(
                            "Application command commit outcome unknown; reopen required: {error}; rollback also failed: {rollback_error}"
                        ),
                        None => format!(
                            "Application command commit outcome unknown; reopen required: {error}"
                        ),
                    };
                    self.poison_runtime(message.clone());
                    return Err(execution_error(
                        "command_transaction_commit_outcome_unknown",
                        message,
                        false,
                    ));
                }
                // The receipt is durable here even if adopting Runtime state
                // below fails. Replaying a receipt is not a new admission.
                if let Some(outcome) = measurement_new_receipt(&receipt) {
                    let context = nola_measurement::context();
                    let run = measurement_admitted_run(context.command_kind, &receipt.outcome);
                    let _scope = nola_measurement::enter(nola_measurement::with_run(context, run));
                    nola_measurement::point_outcome(nola_measurement::Stage::CommandReceiptCommitted, outcome);
                }
                if let Err(error) = self.commit_runtime_unit_of_work() {
                    let message = format!(
                        "committed Application command could not adopt Runtime state; reopen required: {error}"
                    );
                    self.poison_runtime(message.clone());
                    return Err(execution_error(
                        "runtime_transaction_commit_failed",
                        message,
                        false,
                    ));
                }
                Ok(receipt)
            }
            Err(error) => {
                let rollback = {
                    let mut protocol = self.store.command_protocol_repository();
                    protocol.rollback_write_unit_of_work()
                };
                if let Err(rollback_error) = rollback {
                    let message = format!(
                        "rollback Application command transaction after {error} failed; reopen required: {rollback_error}"
                    );
                    self.poison_runtime(message.clone());
                    return Err(execution_error(
                        "command_transaction_rollback_failed",
                        message,
                        false,
                    ));
                }
                self.rollback_runtime_unit_of_work().map_err(|message|
                    execution_error("runtime_rollback_replay_failed", message, false))?;
                Err(error)
            }
        }
    }

    fn execute_received_command_in_unit_of_work(
        &mut self,
        input: ExecuteReceivedCommand,
        command: ApplicationCommand,
    ) -> Result<ApplicationCommandReceipt, ApplicationCommandExecutionError> {
        let expected_revision = input
            .expected_revision
            .map(i64::try_from)
            .transpose()
            .map_err(|_| {
                execution_error(
                    "invalid_expected_revision",
                    "expected revision exceeds the Store integer range",
                    false,
                )
            })?;
        let started = CommandReceiptRepository::begin_command_receipt(
            &mut self.store.command_protocol_repository(),
            &BeginCommandReceiptInput {
                client_id: input.client_id,
                request_id: input.request_id,
                command_type: input.command_type,
                command_json: input.command_json,
                expected_revision,
                received_at: input.received_at,
            },
        )
        .map_err(map_admission_error)?;
        match started {
            BeginCommandReceiptResult::Existing(receipt) => decode_receipt(receipt, true),
            BeginCommandReceiptResult::Started(receipt) => {
                let client_id = receipt.client_id.clone();
                let request_id = receipt.request_id.clone();
                let completed_at = timestamp_now();
                let revision_scope = command_revision_scope(&command);
                if let (Some(expected), Some(scope)) = (expected_revision, revision_scope.as_ref())
                {
                    let actual = scope
                        .read_revision(&self.store.command_protocol_repository())
                        .map_err(|error| {
                            execution_error("revision_read_failed", error.to_string(), true)
                        })?
                        .unwrap_or(0);
                    let actual_revision = u64::try_from(actual).map_err(|_| {
                        execution_error(
                            "invalid_stored_revision",
                            "negative entity revision",
                            false,
                        )
                    })?;
                    if expected != actual {
                        let outcome = CompleteCommandReceiptOutcome::Conflict {
                            conflict_json: serde_json::to_string(&StoredConflict {
                                scope_id: scope.identity(),
                                expected_revision: expected as u64,
                                actual_revision,
                            })
                            .map_err(|error| {
                                execution_error("receipt_encoding_failed", error.to_string(), true)
                            })?,
                        };
                        let completed = CommandReceiptRepository::complete_command_receipt(
                            &mut self.store.command_protocol_repository(),
                            &client_id,
                            &request_id,
                            &completed_at,
                            &outcome,
                        )
                        .map_err(|error| {
                            execution_error("receipt_persistence_failed", error.to_string(), true)
                        })?;
                        return decode_receipt(completed, false);
                    }
                }
                let conversation_id = match &command {
                    ApplicationCommand::AuxiliaryInvocation { command } => {
                        use nola_domain::{AuxiliaryInvocationCommand as C, AuxiliaryInvocationOwner as O};
                        let owner = match command {
                            C::Admit { owner, .. } => Some(owner.clone()),
                            C::ObserveSource { local_owner, .. } => Some(local_owner.clone()),
                            C::StartAttempt { operation_id, .. } | C::RecordReceipt { operation_id, .. }
                            | C::Cancel { operation_id, .. } => nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository::auxiliary_operation(&self.store, operation_id)
                                .map_err(|error|execution_error("projection_read_failed",error.to_string(),true))?.map(|operation| operation.owner),
                            C::Inspect { .. } | C::InspectSource { .. } => None,
                        };
                        match owner { Some(O::Conversation { conversation_id }) => Some(conversation_id), _ => None }
                    }
                    ApplicationCommand::ApplyTaskGuidance {task_id,..} => self.store.runtime_task_run_binding(task_id)
                        .map_err(|error|execution_error("projection_read_failed",error.to_string(),true))?
                        .map(|binding|binding.child_session_id),
                    ApplicationCommand::SubmitTaskGuidance {task_id,..} => self.store.runtime_task(task_id)
                        .map_err(|error|execution_error("projection_read_failed",error.to_string(),true))?
                        .and_then(|task|task.owner_session_id),
                    _=>conversation_id_for_command(&command).map(str::to_owned),
                };
                let commits_archive =
                    matches!(&command, ApplicationCommand::CommitArchiveRestore { .. });
                let replica_conversations = match &command {
                    ApplicationCommand::ApplyRemoteSyncPage { input } => {
                        let mut ids = input
                            .changes
                            .iter()
                            .filter(|change| {
                                matches!(
                                    change.entity_type.as_str(),
                                    "conversation"
                                        | "conversation_append"
                                        | "conversation_metadata"
                                        | "conversation_transition"
                                )
                            })
                            .map(|change| change.entity_id.clone())
                            .collect::<std::collections::BTreeSet<_>>();
                        ids.extend(
                            self.store
                                .archive_restore_projection_ids(input.changes.iter().map(
                                    |change| {
                                        (change.entity_type.as_str(), change.payload_json.as_str())
                                    },
                                ))
                                .map_err(|error| {
                                    execution_error(
                                        "projection_read_failed",
                                        error.to_string(),
                                        true,
                                    )
                                })?,
                        );
                        ids
                    }
                    ApplicationCommand::CommitArchiveRestore { input } => self
                        .store
                        .archive_restore_conversation_ids(&input.restore_id)
                        .map_err(|error| {
                            execution_error("projection_read_failed", error.to_string(), true)
                        })?
                        .into_iter()
                        .collect(),
                    _ => std::collections::BTreeSet::new(),
                };
                let mut replica_summaries_before = Vec::with_capacity(replica_conversations.len());
                for id in replica_conversations {
                    let summary = self
                        .load_conversation_summary_view(&id)
                        .map_err(|error| execution_error("projection_read_failed", error, true))?;
                    replica_summaries_before.push((id, summary));
                }
                // Core commits already publish exact assistant changes in the
                // transactional assistant journal. The conversation feed is a
                // bounded refresh signal, not a second transcript transport.
                // In particular, diffing before/after a Host response invokes
                // cold checkpoint recovery on every streamed delta and replays
                // its entire committed prefix (quadratic work on a warm Run).
                // Keep control and terminal transitions on this same path: a
                // permission change must not reintroduce a full-history diff.
                // Metadata/import/admission likewise need only a refresh.
                let invalidates_projection = matches!(
                    &command,
                    ApplicationCommand::BeginAssistantTurn { .. }
                        | ApplicationCommand::AuxiliaryInvocation { .. }
                        | ApplicationCommand::PromoteQueuedAssistantTurn { .. }
                        | ApplicationCommand::BeginQueuedUserTurn { .. }
                        | ApplicationCommand::RespondToConversationWait { .. }
                        | ApplicationCommand::BeginReceiveAssistantRunTerminal { .. }
                        | ApplicationCommand::StageAssistantRunTerminalManifest { .. }
                        | ApplicationCommand::CommitReceivedAssistantRunTerminal { .. }
                        | ApplicationCommand::ReceiveAssistantJournalRecord { .. }
                        | ApplicationCommand::ReceiveRunMessageAdmissions { .. }
                        | ApplicationCommand::AcceptRemoteRunContextUsage { .. }
                        | ApplicationCommand::BeginTaskAssistantTurn { .. }
                        | ApplicationCommand::BeginAutomationAssistantTurn { .. }
                        | ApplicationCommand::ReduceAndAcceptHostResponse { .. }
                        | ApplicationCommand::ReduceAndCommitActiveSubmission { .. }
                        | ApplicationCommand::ReduceAndCommitIdleSubmission { .. }
                        | ApplicationCommand::RequestRunCancellation { .. }
                        | ApplicationCommand::ClaimCancellationHostRequests { .. }
                        | ApplicationCommand::SubmitTaskGuidance { .. }
                        | ApplicationCommand::ApplyTaskGuidance { .. }
                        | ApplicationCommand::ApplyActivePermissionPolicy { .. }
                        | ApplicationCommand::CreateConversationBranch { .. }
                        | ApplicationCommand::AdmitConversationUserMessage { .. }
                        | ApplicationCommand::ImportConversationTranscript { .. }
                        | ApplicationCommand::PublishUploadedTranscript { .. }
                        | ApplicationCommand::PatchConversationConfiguration { .. }
                        | ApplicationCommand::SelectConversationExecution { .. }
                        | ApplicationCommand::SetConversationPermission { .. }
                        | ApplicationCommand::PatchConversationMetadata { .. }
                        | ApplicationCommand::RenameConversation { .. }
                );
                // Content/progress already has an owner-bound journal cursor.
                // Waking the row-window feed for each chunk forces clients to
                // reload the same page even though no message was admitted.
                // Lifecycle/control commands keep their normal invalidations.
                let journal_only_update = match &command {
                    ApplicationCommand::ReduceAndAcceptHostResponse { input } => matches!(
                        &input.response.payload,
                        nola_agent_engine::agent::host::HostResponse::ModelDelta { .. }
                            | nola_agent_engine::agent::host::HostResponse::ModelCommittedDelta { .. }
                            | nola_agent_engine::agent::host::HostResponse::ModelToolCallDelta { .. }
                            | nola_agent_engine::agent::host::HostResponse::ModelContinuation { .. }
                            | nola_agent_engine::agent::host::HostResponse::ToolProgress { .. }
                            | nola_agent_engine::agent::host::HostResponse::ToolCallProgress { .. }
                    ),
                    ApplicationCommand::ReceiveAssistantJournalRecord { .. } => true,
                    _ => false,
                };
                let conversation_summary_before = match conversation_id.as_deref() {
                    Some(conversation_id) => self
                        .load_conversation_summary_view(conversation_id)
                        .map_err(|error| {
                        execution_error("projection_read_failed", error, true)
                    })?,
                    None => None,
                };
                let projection_before = match conversation_id.as_deref() {
                    Some(_) if invalidates_projection => None,
                    Some(conversation_id) => self
                        .load_optional_conversation_projection(conversation_id)
                        .map_err(|error| execution_error("projection_read_failed", error, true))?,
                    None => None,
                };
                let projection_stream_identity = projection_stream_identity(&command);
                let outcome = match self
                    .execute_command_mutations(|runtime| runtime.execute(command))?
                {
                    Ok(result) => {
                        // A remote page can mutate several conversations. Emit
                        // one bounded refresh per affected live conversation,
                        // not a full transcript diff. Summary equality alone is
                        // insufficient: delayed content can change Parts while
                        // preserving newer metadata and its revision.
                        if commits_archive
                            || result
                                .get("applied")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0)
                                > 0
                        {
                            for (id, before) in &replica_summaries_before {
                                if self
                                    .load_conversation_summary_view(id)
                                    .map_err(|error| {
                                        execution_error("projection_read_failed", error, true)
                                    })?
                                    .is_some()
                                {
                                    self.journal_conversation_invalidation(
                                        &client_id,
                                        &request_id,
                                        id,
                                        &completed_at,
                                    )
                                    .map_err(|error| {
                                        execution_error("projection_journal_failed", error, true)
                                    })?;
                                }
                                self.journal_conversation_list_change(
                                    &client_id,
                                    &request_id,
                                    id,
                                    before.as_ref(),
                                    &completed_at,
                                )
                                .map_err(|error| {
                                    execution_error("projection_journal_failed", error, true)
                                })?;
                            }
                        }
                        if let Some(conversation_id) = conversation_id.as_deref() {
                            if invalidates_projection {
                                // Admission can create the conversation. Check
                                // post-command existence rather than suppressing
                                // the first turn's notification for new clients.
                                let after = self
                                    .load_conversation_summary_view(conversation_id)
                                    .map_err(|error| {
                                        execution_error("projection_read_failed", error, true)
                                    })?;
                                let unchanged_rows = conversation_summary_before.as_ref()
                                    .zip(after.as_ref())
                                    .is_some_and(|(before, after)| {
                                        before.message_count == after.message_count
                                            && before.title == after.title
                                            && before.project_root == after.project_root
                                    });
                                if after.is_some() && !(journal_only_update && unchanged_rows) {
                                    self.journal_conversation_invalidation(
                                        &client_id,
                                        &request_id,
                                        conversation_id,
                                        &completed_at,
                                    )
                                } else {
                                    Ok(())
                                }
                            } else {
                                self.journal_conversation_projection_change(
                                    &client_id,
                                    &request_id,
                                    conversation_id,
                                    projection_before.as_ref(),
                                    projection_stream_identity.as_ref(),
                                    &completed_at,
                                )
                            }
                            .map_err(|error| {
                                execution_error("projection_journal_failed", error, true)
                            })?;
                            self.journal_conversation_list_change(
                                &client_id,
                                &request_id,
                                conversation_id,
                                conversation_summary_before.as_ref(),
                                &completed_at,
                            )
                            .map_err(|error| {
                                execution_error("projection_journal_failed", error, true)
                            })?;
                        }
                        CompleteCommandReceiptOutcome::Committed {
                            committed_revision: if let Some(scope) = revision_scope.as_ref() {
                                scope
                                    .read_revision(&self.store.command_protocol_repository())
                                    .map_err(|error| {
                                        execution_error(
                                            "revision_read_failed",
                                            error.to_string(),
                                            true,
                                        )
                                    })?
                            } else {
                                None
                            },
                            result_json: serde_json::to_string(&result).map_err(|error| {
                                execution_error(
                                    "receipt_encoding_failed",
                                    format!("encode Application command result failed: {error}"),
                                    true,
                                )
                            })?,
                        }
                    }
                    Err(message) => CompleteCommandReceiptOutcome::Rejected {
                        error_json: serde_json::to_string(&StoredFailure {
                            code: "application_rejected".to_string(),
                            message,
                            recoverable: false,
                            details: None,
                        })
                        .map_err(|error| {
                            execution_error(
                                "receipt_encoding_failed",
                                format!("encode Application command failure failed: {error}"),
                                true,
                            )
                        })?,
                    },
                };
                let completed = CommandReceiptRepository::complete_command_receipt(
                    &mut self.store.command_protocol_repository(),
                    &client_id,
                    &request_id,
                    &completed_at,
                    &outcome,
                )
                .map_err(|error| {
                    execution_error(
                        "receipt_persistence_failed",
                        format!("complete Application command failed: {error}"),
                        true,
                    )
                })?;
                decode_receipt(completed, false)
            }
        }
    }

    fn execute_command_mutations(
        &mut self,
        execute: impl FnOnce(&mut Self) -> Result<Value, String>,
    ) -> Result<Result<Value, String>, ApplicationCommandExecutionError> {
        CommandTransactionRepository::begin_command_mutations(
            &mut self.store.command_protocol_repository(),
        )
        .map_err(|error| {
            execution_error("command_savepoint_begin_failed", error.to_string(), true)
        })?;
        let result = execute(self);
        if result.is_ok() {
            let release = CommandTransactionRepository::commit_command_mutations(
                &mut self.store.command_protocol_repository(),
            );
            if let Err(error) = release {
                let message = format!(
                    "release Application command mutations failed; reopen required: {error}"
                );
                // The Store has fenced writes; a successful outer rollback
                // cannot make retained Runtime state safe to reuse either.
                self.poison_runtime(message.clone());
                return Err(execution_error(
                    "command_savepoint_release_failed", message, false,
                ));
            }
        } else {
            let rollback = CommandTransactionRepository::rollback_command_mutations(
                &mut self.store.command_protocol_repository(),
            );
            if let Err(error) = rollback {
                let message = format!(
                    "rollback Application command mutations failed; reopen required: {error}"
                );
                self.poison_runtime(message.clone());
                return Err(execution_error(
                    "command_savepoint_rollback_failed",
                    message,
                    false,
                ));
            }
            self.rollback_runtime_unit_of_work().map_err(|message|
                execution_error("runtime_rollback_replay_failed", message, false))?;
            self.begin_runtime_unit_of_work().map_err(|error| {
                execution_error("runtime_transaction_begin_failed", error, false)
            })?;
        }
        Ok(result)
    }
}

enum CommandRevisionScope {
    ModelProfile(String),
    Conversation(String),
}

impl CommandRevisionScope {
    fn identity(&self) -> String {
        match self {
            Self::ModelProfile(id) => format!("model_profile:{id}"),
            Self::Conversation(id) => format!("conversation:{id}"),
        }
    }

    fn read_revision<R: CommandTransactionRepository + ?Sized>(
        &self,
        store: &R,
    ) -> Result<Option<i64>, R::Error> {
        match self {
            Self::ModelProfile(id) => {
                store.command_revision(CommandRevisionEntity::ModelProfile(id))
            }
            Self::Conversation(id) => {
                store.command_revision(CommandRevisionEntity::Conversation(id))
            }
        }
    }
}

fn command_revision_scope(command: &ApplicationCommand) -> Option<CommandRevisionScope> {
    match command {
        ApplicationCommand::Model {
            command: crate::ModelCommand::ConfigureParameters { input },
        }
        | ApplicationCommand::Model {
            command: crate::ModelCommand::UpsertProfile { input },
        } => Some(CommandRevisionScope::ModelProfile(input.id.clone())),
        ApplicationCommand::RenameConversation {
            conversation_id, ..
        } => Some(CommandRevisionScope::Conversation(conversation_id.clone())),
        _ => None,
    }
}

fn projection_stream_identity(
    command: &ApplicationCommand,
) -> Option<(RunId, ProjectionExecutionOwner)> {
    let ApplicationCommand::ReduceAndAcceptHostResponse { input } = command else {
        return None;
    };
    let owner = match (
        &input.response.attempt_id,
        &input.response.delegated_invocation_id,
    ) {
        (Some(id), None) => ProjectionExecutionOwner::ModelAttempt {
            attempt_id: RunAttemptId(id.clone()),
        },
        (None, Some(id)) => ProjectionExecutionOwner::DelegatedInvocation {
            invocation_id: id.clone(),
        },
        _ => return None,
    };
    Some((RunId(input.run_id.clone()), owner))
}

fn decode_receipt(
    receipt: StoredCommandReceipt,
    replayed: bool,
) -> Result<ApplicationCommandReceipt, ApplicationCommandExecutionError> {
    let outcome = match receipt.status.as_str() {
        "accepted" => ApplicationCommandReceiptOutcome::RecoveryRequired,
        "committed" => {
            let result = receipt
                .result_json
                .as_deref()
                .ok_or_else(|| {
                    execution_error(
                        "receipt_corrupt",
                        "committed receipt is missing result_json",
                        false,
                    )
                })
                .and_then(|value| {
                    serde_json::from_str(value).map_err(|error| {
                        execution_error(
                            "receipt_corrupt",
                            format!("decode committed receipt result failed: {error}"),
                            false,
                        )
                    })
                })?;
            ApplicationCommandReceiptOutcome::Committed {
                committed_revision: receipt
                    .committed_revision
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| {
                        execution_error(
                            "receipt_corrupt",
                            "stored committed revision is negative",
                            false,
                        )
                    })?,
                result,
            }
        }
        "rejected" => {
            let failure: StoredFailure = receipt
                .error_json
                .as_deref()
                .ok_or_else(|| {
                    execution_error(
                        "receipt_corrupt",
                        "rejected receipt is missing error_json",
                        false,
                    )
                })
                .and_then(|value| {
                    serde_json::from_str(value).map_err(|error| {
                        execution_error(
                            "receipt_corrupt",
                            format!("decode rejected receipt failed: {error}"),
                            false,
                        )
                    })
                })?;
            ApplicationCommandReceiptOutcome::Rejected {
                code: failure.code,
                message: failure.message,
                recoverable: failure.recoverable,
                details: failure.details,
            }
        }
        "conflict" => {
            let conflict: StoredConflict = receipt
                .conflict_json
                .as_deref()
                .ok_or_else(|| {
                    execution_error(
                        "receipt_corrupt",
                        "conflict receipt is missing conflict_json",
                        false,
                    )
                })
                .and_then(|value| {
                    serde_json::from_str(value).map_err(|error| {
                        execution_error(
                            "receipt_corrupt",
                            format!("decode conflict receipt failed: {error}"),
                            false,
                        )
                    })
                })?;
            ApplicationCommandReceiptOutcome::Conflict {
                scope_id: conflict.scope_id,
                expected_revision: conflict.expected_revision,
                actual_revision: conflict.actual_revision,
            }
        }
        status => {
            return Err(execution_error(
                "receipt_corrupt",
                format!("unsupported stored command receipt status {status}"),
                false,
            ))
        }
    };
    Ok(ApplicationCommandReceipt {
        client_id: receipt.client_id,
        request_id: receipt.request_id,
        received_at: receipt.received_at,
        completed_at: receipt.completed_at,
        replayed,
        outcome,
    })
}

fn map_admission_error<E: std::fmt::Display>(
    error: ReceiptAdmissionError<E>,
) -> ApplicationCommandExecutionError {
    match error {
        ReceiptAdmissionError::IdentityConflict(message) => {
            execution_error("request_identity_conflict", message, false)
        }
        ReceiptAdmissionError::Persistence(error) => execution_error(
            "receipt_persistence_failed",
            format!("admit Application command failed: {error}"),
            true,
        ),
    }
}

fn execution_error(
    code: impl Into<String>,
    message: impl Into<String>,
    recoverable: bool,
) -> ApplicationCommandExecutionError {
    ApplicationCommandExecutionError {
        code: code.into(),
        message: message.into(),
        recoverable,
    }
}

pub(crate) fn timestamp_now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn measurement_preserves_rejection_and_only_extracts_typed_admission_run() {
        use super::*;
        use nola_measurement::{CommandKind, Outcome};
        let committed = ApplicationCommandReceiptOutcome::Committed {
            committed_revision: None, result: serde_json::json!({"begin":{"run_id":"run"}}),
        };
        assert_eq!(measurement_admitted_run(Some(CommandKind::BeginTurn), &committed), Some("run"));
        assert_eq!(measurement_admitted_run(Some(CommandKind::Other), &committed), None);
        assert_eq!(measurement_admitted_run(Some(CommandKind::IdleSubmission), &committed), None);
        assert_eq!(measurement_receipt_outcome(&ApplicationCommandReceiptOutcome::RecoveryRequired), Outcome::Incomplete);
        assert_eq!(measurement_receipt_outcome(&ApplicationCommandReceiptOutcome::Conflict {
            scope_id: "scope".into(), expected_revision: 1, actual_revision: 2,
        }), Outcome::Rejected);
        let mut receipt = ApplicationCommandReceipt { client_id: "client".into(), request_id: "request".into(),
            received_at: "now".into(), completed_at: None, replayed: false, outcome: committed };
        assert_eq!(measurement_new_receipt(&receipt), Some(Outcome::Succeeded));
        receipt.replayed = true;
        assert_eq!(measurement_new_receipt(&receipt), None);
    }
    use super::*;

    fn input() -> ExecuteReceivedCommand {
        ExecuteReceivedCommand {
            client_id: "test-client".into(),
            request_id: "request-1".into(),
            command_type: "list_conversations".into(),
            command_json: "{}".into(),
            expected_revision: None,
            received_at: "2026-09-04T00:00:00Z".into(),
        }
    }

    #[test]
    fn a_completed_request_replays_without_executing_again() {
        let directory = tempfile::tempdir().unwrap();
        let mut runtime =
            ApplicationRuntime::open(directory.path().join("application.db")).unwrap();
        let first = runtime
            .execute_received_command(input(), ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } })
            .unwrap();
        assert!(!first.replayed);
        assert!(matches!(
            &first.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        let replay = runtime
            .execute_received_command(input(), ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } })
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(first.outcome, replay.outcome);
    }

    #[test]
    fn failed_outer_commit_is_unknown_and_requires_a_fresh_runtime_before_retry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("commit-failure.db");
        let mut runtime = ApplicationRuntime::open(&path).unwrap();
        let fixture = rusqlite::Connection::open(&path).unwrap();
        fixture
            .execute_batch(
                "CREATE TABLE receipt_commit_parent(id INTEGER PRIMARY KEY);
                 CREATE TABLE receipt_commit_child(
                     parent_id INTEGER REFERENCES receipt_commit_parent(id)
                         DEFERRABLE INITIALLY DEFERRED
                 );
                 CREATE TRIGGER fail_receipt_commit
                 AFTER UPDATE OF status ON command_receipts
                 WHEN NEW.status='committed'
                 BEGIN
                     INSERT INTO receipt_commit_child(parent_id) VALUES (1);
                 END;",
            )
            .unwrap();

        let error = runtime
            .execute_received_command(input(), ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } })
            .unwrap_err();
        assert_eq!(error.code, "command_transaction_commit_outcome_unknown");
        assert!(error.message.contains("outcome unknown"));
        assert!(error.message.contains("reopen required"));
        assert!(!error.recoverable);
        assert!(!runtime.store.has_write_unit_of_work());
        assert!(runtime
            .command_receipt("test-client", "request-1")
            .unwrap()
            .is_none());

        fixture
            .execute_batch("DROP TRIGGER fail_receipt_commit;")
            .unwrap();
        assert!(runtime
            .execute(ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } })
            .unwrap_err()
            .contains("reopen required"));
        let error = runtime
            .execute_received_command(input(), ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } })
            .unwrap_err();
        assert_eq!(error.code, "runtime_reopen_required");
        assert!(!runtime.store.has_write_unit_of_work());
        assert!(runtime
            .command_receipt("test-client", "request-1")
            .unwrap()
            .is_none());

        drop(runtime);
        let mut reopened = ApplicationRuntime::open(&path).unwrap();
        let receipt = reopened
            .execute_received_command(input(), ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } })
            .unwrap();
        assert!(!receipt.replayed);
        assert!(matches!(
            receipt.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
    }

    #[test]
    fn failed_mutation_release_requires_fresh_runtime_even_after_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("release-failure.db");
        let mut runtime = ApplicationRuntime::open(&path).unwrap();
        runtime.store.begin_write_unit_of_work().unwrap();
        runtime.begin_runtime_unit_of_work().unwrap();
        let error = runtime.execute_command_mutations(|runtime| {
            runtime.store.put_product_setting("unpublished", "true", 1)
                .map_err(|error| error.to_string())?;
            // Simulate a nested participant losing the owned savepoint.
            runtime.store.commit_command_mutations().map_err(|error| error.to_string())?;
            Ok(Value::Null)
        }).unwrap_err();
        assert_eq!(error.code, "command_savepoint_release_failed");
        assert!(!error.recoverable);
        runtime.store.rollback_write_unit_of_work().unwrap();
        runtime.rollback_runtime_unit_of_work().unwrap();
        assert!(runtime.store.product_setting("unpublished").unwrap().is_none());
        let rejected = runtime.execute_received_command(input(), ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } })
            .unwrap_err();
        assert_eq!(rejected.code, "runtime_reopen_required");
        assert!(runtime.command_receipt("test-client", "request-1").unwrap().is_none());
        drop(runtime);
        let mut reopened = ApplicationRuntime::open(&path).unwrap();
        let receipt = reopened.execute_received_command(input(), ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } }).unwrap();
        assert!(matches!(receipt.outcome, ApplicationCommandReceiptOutcome::Committed { .. }));
    }

    #[test]
    fn failed_mutation_rollback_poison_refuses_further_receipt_admission() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rollback-failure.db");
        let mut runtime = ApplicationRuntime::open(&path).unwrap();
        let fixture = rusqlite::Connection::open(&path).unwrap();
        fixture
            .execute_batch(
                "CREATE TRIGGER abort_command_transaction
                 BEFORE INSERT ON command_receipts
                 WHEN NEW.request_id='abort-transaction'
                 BEGIN
                     SELECT RAISE(ROLLBACK, 'fixture forced the outer transaction to abort');
                 END;",
            )
            .unwrap();
        runtime.store.begin_write_unit_of_work().unwrap();
        runtime.begin_runtime_unit_of_work().unwrap();
        let error = runtime
            .execute_command_mutations(|runtime| {
                runtime
                    .store
                    .begin_command_receipt(&BeginCommandReceiptInput {
                        client_id: "test-client".into(),
                        request_id: "abort-transaction".into(),
                        command_type: "test".into(),
                        command_json: "{}".into(),
                        expected_revision: None,
                        received_at: "2026-09-05T00:00:00Z".into(),
                    })
                    .map(|_| Value::Null)
                    .map_err(|error| error.to_string())
            })
            .unwrap_err();
        assert_eq!(error.code, "command_savepoint_rollback_failed");
        assert!(error.message.contains("reopen required"));
        assert!(!error.recoverable);
        assert!(!runtime.store.has_write_unit_of_work());
        assert!(runtime.ensure_runtime_usable().is_err());

        fixture
            .execute_batch("DROP TRIGGER abort_command_transaction;")
            .unwrap();
        let error = runtime
            .execute_received_command(input(), ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } })
            .unwrap_err();
        assert_eq!(error.code, "runtime_reopen_required");
        assert!(runtime
            .command_receipt("test-client", "request-1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn delimiter_containing_receipt_ids_have_distinct_projection_events() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.db");
        let mut runtime = ApplicationRuntime::open(&path).unwrap();
        runtime
            .store
            .create_conversation(&nola_store::CreateConversationInput {
                id: "identity-conversation".into(),
                title: "Initial".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let mut requests = Vec::new();
        for (client_id, request_id, title) in [("a:b", "c", "First"), ("a", "b:c", "Second")] {
            let command = ApplicationCommand::RenameConversation {
                title_source: None,
                conversation_id: "identity-conversation".into(),
                title: title.into(),
                updated_at_ms: 2,
            };
            let input = ExecuteReceivedCommand {
                client_id: client_id.into(),
                request_id: request_id.into(),
                command_type: "rename_conversation".into(),
                command_json: serde_json::to_string(&command).unwrap(),
                expected_revision: None,
                received_at: "2000".into(),
            };
            let receipt = runtime
                .execute_received_command(input.clone(), command)
                .unwrap();
            assert!(matches!(
                &receipt.outcome,
                ApplicationCommandReceiptOutcome::Committed { .. }
            ));
            requests.push((input, receipt.outcome));
        }
        drop(runtime);
        let mut runtime = ApplicationRuntime::open(&path).unwrap();
        for (input, expected) in requests {
            let command = serde_json::from_str(&input.command_json).unwrap();
            let receipt = runtime.execute_received_command(input, command).unwrap();
            assert!(receipt.replayed);
            assert_eq!(receipt.outcome, expected);
        }
        assert_eq!(
            runtime
                .store
                .conversation_summary("identity-conversation")
                .unwrap()
                .unwrap()
                .title,
            "Second"
        );
    }

    #[test]
    fn rejected_command_rolls_back_partial_writes_but_retains_admission() {
        let directory = tempfile::tempdir().unwrap();
        let mut runtime =
            ApplicationRuntime::open(directory.path().join("application.db")).unwrap();
        runtime.store.begin_write_unit_of_work().unwrap();
        runtime.begin_runtime_unit_of_work().unwrap();
        let receipt = |request_id: &str| BeginCommandReceiptInput {
            client_id: "test-client".into(),
            request_id: request_id.into(),
            command_type: "test".into(),
            command_json: "{}".into(),
            expected_revision: None,
            received_at: "2026-09-05T00:00:00Z".into(),
        };
        runtime
            .store
            .begin_command_receipt(&receipt("outer"))
            .unwrap();
        let result = runtime
            .execute_command_mutations(|runtime| {
                runtime
                    .store
                    .begin_command_receipt(&receipt("partial-write"))
                    .unwrap();
                Err("failure after a durable repository mutation".into())
            })
            .unwrap();
        assert!(result.is_err());
        runtime
            .store
            .complete_command_receipt(
                "test-client",
                "outer",
                "2026-09-05T00:00:01Z",
                &CompleteCommandReceiptOutcome::Rejected {
                    error_json: serde_json::to_string(&StoredFailure {
                        code: "application_rejected".into(),
                        message: result.unwrap_err(),
                        recoverable: false,
                        details: None,
                    })
                    .unwrap(),
                },
            )
            .unwrap();
        runtime.store.commit_write_unit_of_work().unwrap();
        runtime.commit_runtime_unit_of_work().unwrap();
        drop(runtime);

        let mut runtime =
            ApplicationRuntime::open(directory.path().join("application.db")).unwrap();
        assert!(runtime
            .command_receipt("test-client", "partial-write")
            .unwrap()
            .is_none());
        let rejected = runtime
            .command_receipt("test-client", "outer")
            .unwrap()
            .unwrap();
        assert!(rejected.replayed);
        assert!(matches!(
            rejected.outcome,
            ApplicationCommandReceiptOutcome::Rejected { .. }
        ));
    }
}
