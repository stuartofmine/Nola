// SPDX-License-Identifier: Apache-2.0
//! CLI-side port for Application transactions.
//!
//! Durable sessions always use the local single-writer service. The in-process
//! runtime exists only for explicit process-retained (`--no-save`) sessions.

use nola_application_bootstrap::ApplicationRuntime;
use std::fmt;
use std::path::Path;
use std::time::Instant;

use nola_application::{
    AcceptedActiveSubmission, AcceptedAssistantTurn, AcceptedConversationRevert,
    AcceptedHostResponse, AcceptedIdleSubmission, AcknowledgeResumeSnapshot, ActiveRun,
    ApplicationCommand, BeginAssistantTurn, CommittedHostRequests, Conversation,
    ConversationBranchCreation, ConversationCheckpoint, ConversationSummary, CreateConversation,
    CreateConversationBranch, ReduceAndAcceptHostResponse, ReduceAndCommitActiveSubmission,
    ReduceAndCommitIdleSubmission, ResumeSnapshot, RevertConversation, SessionProjection,
};
use nola_protocol::ImportConversationTranscript;
use nola_service_client::{ApplicationServiceClient, ApplicationServiceCommandError};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub enum CliApplicationPort {
    Service(ApplicationServiceClient),
    InProcess(ApplicationRuntime),
    // Exercises the real service receipt transaction without spawning IPC.
    #[cfg(test)]
    ReceivedInProcess(ApplicationRuntime),
}

/// Only lifecycle facts belong in diagnostics. Error values (including receipt
/// errors) can embed command data, local paths or credentials. The original
/// error is returned separately and remains authoritative for reconciliation.
fn command_completion_diagnostic(
    command: &'static str,
    duration_ms: f64,
    succeeded: bool,
    requires_reconciliation: bool,
) -> Value {
    serde_json::json!({
        "type": "application_command",
        "command": command,
        "phase": if succeeded { "succeeded" } else if requires_reconciliation { "awaiting_reconciliation" } else { "failed" },
        "duration_ms": duration_ms,
        "failed": !succeeded,
        "requires_reconciliation": requires_reconciliation,
    })
}

/// Only this module's original-Application export can construct the value.
/// Digest protects identity/transfer; authenticated local Host IPC is the
/// trust boundary, not a claim that a public hash authenticates an issuer.
pub(crate) struct ExportedAgentTaskAdmission(nola_domain::agent_task_admission::AgentTaskAdmission);
pub(crate) struct ExportedAgentTaskTerminalProjection(nola_protocol::AgentTaskTerminalProjection);
pub(crate) struct ExportedAgentTaskGuidanceProjection(nola_protocol::AgentTaskGuidanceProjection);

impl ExportedAgentTaskTerminalProjection {
    pub(crate) fn parent_run_id(&self) -> &str {
        &self.0.admission.parent_run_id
    }
}
impl ExportedAgentTaskGuidanceProjection {
    pub(crate) fn parent_run_id(&self) -> &str {
        &self.0.admission.parent_run_id
    }
}

impl fmt::Debug for CliApplicationPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let backend = match self {
            Self::Service(_) => "service",
            Self::InProcess(_) => "in-process",
            #[cfg(test)]
            Self::ReceivedInProcess(_) => "received-in-process",
        };
        formatter
            .debug_struct("CliApplicationPort")
            .field("backend", &backend)
            .finish_non_exhaustive()
    }
}

impl CliApplicationPort {
    /// Native/server Host factory channel; never forward arbitrary renderer JSON.
    pub(crate) fn execute_expert_host_operation<T: DeserializeOwned>(
        &mut self,
        request_id: &str,
        operation: nola_application::host_expert_template_operation::HostExpertTemplateOperation,
    ) -> Result<T, nola_application::host_expert_template_operation::HostExpertOperationError> {
        use nola_application::host_expert_template_operation::HostExpertOperationError;
        let mutating = operation.mutates_state();
        let value: serde_json::Value = match self {
            Self::Service(client) => client.execute_host_expert_template_operation_with_request_id(request_id, &operation)
                .map_err(|error| {
                    if error.requires_reconciliation() || matches!(error,ApplicationServiceCommandError::Transport{..}) {
                        let reopen_required = matches!(&error, ApplicationServiceCommandError::OutcomeUnknown { reopen_required: true, .. });
                        HostExpertOperationError::OutcomePending { request_id: error.request_id().into(), message: error.to_string(), reopen_required }
                    } else { HostExpertOperationError::Rejected { code: "expert_host_operation_rejected".into(), message: error.to_string() } }
                })?,
            Self::InProcess(runtime) => runtime.execute_host_expert_template_operation("cli-expert-host", request_id, operation)?,
            #[cfg(test)]
            Self::ReceivedInProcess(runtime) => runtime.execute_host_expert_template_operation("cli-expert-host", request_id, operation)?,
        };
        serde_json::from_value(value).map_err(|error| {
            if mutating { HostExpertOperationError::OutcomePending { request_id: request_id.into(),
                message: format!("The committed Expert Host result could not be decoded: {error}"), reopen_required: false } }
            else { HostExpertOperationError::Rejected { code: "invalid_expert_host_result".into(), message: error.to_string() } }
        })
    }

    pub(crate) fn edited_resend_host_material(&mut self,target:&str)
        -> Result<Option<nola_store_port::edited_resend::EditedResendHostMaterial>,String> {
        match self {
            Self::Service(client)=>client.load_edited_resend_host_material(&uuid::Uuid::new_v4().to_string(),target).map_err(|e|e.to_string()),
            Self::InProcess(runtime)=>runtime.execute(ApplicationCommand::LoadEditedResendHostMaterial {conversation_id:target.into()})
                .and_then(|value|serde_json::from_value(value).map_err(|e|e.to_string())),
            #[cfg(test)]
            Self::ReceivedInProcess(runtime)=>runtime.execute(ApplicationCommand::LoadEditedResendHostMaterial {conversation_id:target.into()})
                .and_then(|value|serde_json::from_value(value).map_err(|e|e.to_string())),
        }
    }
    pub(crate) fn attest_edited_resend_host_policy(&mut self,target:&str,config:&nola_core::agent::AgentCoreConfig,submission:&nola_core::agent::AgentSubmission)
        -> Result<nola_domain::edited_resend::FreshResendPolicyWitness,String> {
        match self {
            Self::Service(client)=>client.attest_edited_resend_host_policy(&uuid::Uuid::new_v4().to_string(),target,config,submission).map_err(|e|e.to_string()),
            Self::InProcess(runtime)=>runtime.execute(ApplicationCommand::AttestEditedResendHostPolicy {conversation_id:target.into(),core_config:config.clone(),submission:submission.clone()})
                .and_then(|value|serde_json::from_value(value).map_err(|e|e.to_string())),
            #[cfg(test)]
            Self::ReceivedInProcess(runtime)=>runtime.execute(ApplicationCommand::AttestEditedResendHostPolicy {conversation_id:target.into(),core_config:config.clone(),submission:submission.clone()})
                .and_then(|value|serde_json::from_value(value).map_err(|e|e.to_string())),
        }
    }

    pub(crate) fn prepare_goal_control_intent(
        &mut self,
        input: nola_application::goal_control_intent::PrepareGoalControlIntent,
    ) -> Result<nola_core::agent::GoalRuntimeCommandEnvelope, String> {
        use sha2::{Digest, Sha256};
        let identity = serde_json::to_vec(&(&input.conversation_id, &input.operation_id))
            .map_err(|error| error.to_string())?;
        let request_id = format!("goal-intent:{:x}", Sha256::digest(identity));
        let command = ApplicationCommand::PrepareGoalControlIntent { input };
        match self {
            Self::Service(client) => execute_service_command(client, &request_id, None, command)
                .map_err(|error| error.to_string()),
            Self::InProcess(application) => execute_in_process_received(application, &request_id, None, command),
            #[cfg(test)]
            Self::ReceivedInProcess(application) => execute_in_process_received(application, &request_id, None, command),
        }
    }
    /// Keep the caller's identity across lost HTTP acknowledgements. The
    /// existing Application receipt verifies that retries carry identical data.
    pub(crate) fn submit_ui_interaction(
        &mut self,
        request_id: &str,
        input: nola_application::generative_ui::SubmitUiInteraction,
    ) -> Result<nola_application::generative_ui::UiInteractionReceipt, ApplicationServiceCommandError> {
        let command = ApplicationCommand::SubmitUiInteraction { input };
        match self {
            Self::Service(client) => execute_service_command(client, request_id, None, command),
            Self::InProcess(application) => execute_in_process_received_typed(application, request_id, None, command),
            #[cfg(test)]
            Self::ReceivedInProcess(application) => execute_in_process_received_typed(application, request_id, None, command),
        }
    }
    /// Narrow producer recovery seam: receipts outlive queue consumption.
    /// Existing receipts are payload-verified by the received-command Existing
    /// branch; an absent receipt-only lookup never submits a command.
    pub(crate) fn reconcile_input_admission(
        &mut self, request_id: &str, input: nola_application::AgentCommand, allow_submit: bool,
    ) -> Result<bool, String> {
        let command = ApplicationCommand::EnqueueAgentCommand { input };
        match self {
            Self::Service(client) => {
                let existing = client.command_receipt(request_id)?;
                if existing.is_none() && !allow_submit { return Ok(false); }
                let verified = client.submit_with_request_id(request_id, command)?;
                let canonical = client.command_receipt(request_id)?;
                if canonical.as_ref() != Some(&verified) { return Err("input receipt not canonical; retain pending intent".into()); }
                match verified.outcome {
                    nola_protocol::CommandReceiptOutcome::Committed { result, .. } if result.as_bool() == Some(true) => Ok(true),
                    _ => Ok(false),
                }
            }
            Self::InProcess(application) => reconcile_in_process_input(application,request_id,command,allow_submit),
            #[cfg(test)]
            Self::ReceivedInProcess(application) => reconcile_in_process_input(application,request_id,command,allow_submit),
        }
    }
    pub(crate) fn acknowledge_task_result_delivery(
        &mut self,
        input: &nola_application::AcknowledgeRuntimeTaskResult,
        source_delivery_id: &str,
    ) -> Result<bool, String> {
        use sha2::{Digest, Sha256};
        let request_id = format!(
            "task-result-delivery:{:x}",
            Sha256::digest(
                serde_json::to_vec(&(source_delivery_id, input))
                    .map_err(|error| error.to_string())?
            )
        );
        let command = ApplicationCommand::RuntimeTask {
            command: nola_application::RuntimeTaskCommand::AcknowledgeResult {
                input: input.clone(),
            },
        };
        match self {
            Self::Service(client) => execute_service_command(client, &request_id, None, command)
                .map_err(|error| error.to_string()),
            Self::InProcess(application) => {
                execute_in_process_received(application, &request_id, None, command)
            }
            #[cfg(test)]
            Self::ReceivedInProcess(application) => {
                execute_in_process_received(application, &request_id, None, command)
            }
        }
    }
    pub(crate) fn export_agent_task_guidance_projection(
        &mut self,
        task_id: &str,
        message_id: &str,
        settlement: bool,
    ) -> Result<ExportedAgentTaskGuidanceProjection, String> {
        let projection: nola_protocol::AgentTaskGuidanceProjection =
            self.execute(ApplicationCommand::ExportAgentTaskGuidanceProjection {
                task_id: task_id.into(),
                message_id: message_id.into(),
                settlement,
            })?;
        projection.validate()?;
        if projection.admission.task_id != task_id
            || projection.guidance.message_id != message_id
            || projection.settlement != settlement
        {
            return Err("Task guidance export changed the requested identity".into());
        }
        Ok(ExportedAgentTaskGuidanceProjection(projection))
    }

    pub(crate) fn import_exported_agent_task_guidance_projection(
        &mut self,
        source: &ExportedAgentTaskGuidanceProjection,
    ) -> Result<nola_protocol::TaskGuidanceProjectionApplied, String> {
        use sha2::{Digest, Sha256};
        let projection = &source.0;
        let digest =
            Sha256::digest(serde_json::to_vec(projection).map_err(|error| error.to_string())?);
        let request_id = format!("task-guidance:{digest:x}");
        let ack: nola_protocol::TaskGuidanceProjectionApplied = match self {
            Self::Service(client) => match client
                .import_agent_task_guidance_projection_with_request_id(&request_id, projection)
            {
                Ok(ack) => ack,
                Err(error) if error.requires_reconciliation() => client
                    .reconcile_command_result(&error)
                    .map_err(|error| error.to_string())?,
                Err(error) => return Err(error.to_string()),
            },
            _ => self.execute(ApplicationCommand::ImportHostAgentTaskGuidanceProjection {
                projection: projection.clone(),
            })?,
        };
        if ack.conversation_id != projection.admission.parent_conversation_id
            || ack.task_id != projection.admission.task_id
            || ack.message_id != projection.guidance.message_id
            || ack.settlement != projection.settlement
            || ack.core_revision < projection.admission.core_revision
        {
            return Err(
                "Task guidance receipt does not prove the exact imported projection".into(),
            );
        }
        Ok(ack)
    }

    pub(crate) fn export_agent_task_terminal_projection(
        &mut self,
        task_id: &str,
        receipt_id: &str,
        guidance_message_ids: &[String],
    ) -> Result<ExportedAgentTaskTerminalProjection, String> {
        let projection: nola_protocol::AgentTaskTerminalProjection =
            self.execute(ApplicationCommand::ExportAgentTaskTerminalProjection {
                task_id: task_id.into(),
                expected_receipt_id: receipt_id.into(),
                guidance_message_ids: guidance_message_ids.to_vec(),
            })?;
        projection.validate()?;
        if projection.admission.task_id != task_id || projection.receipt_id != receipt_id {
            return Err("Task terminal export changed the requested identity".into());
        }
        let requested: std::collections::BTreeSet<_> = guidance_message_ids.iter().collect();
        let received: std::collections::BTreeSet<_> = projection
            .guidance_results
            .iter()
            .map(|value| &value.message_id)
            .collect();
        if requested != received || received.len() != projection.guidance_results.len() {
            return Err("Task terminal export changed the requested guidance selection".into());
        }
        Ok(ExportedAgentTaskTerminalProjection(projection))
    }

    pub(crate) fn import_exported_agent_task_terminal_projection(
        &mut self,
        source: &ExportedAgentTaskTerminalProjection,
    ) -> Result<nola_protocol::TaskTerminalReconciliationApplied, String> {
        use sha2::{Digest, Sha256};
        let projection = &source.0;
        let digest =
            Sha256::digest(serde_json::to_vec(projection).map_err(|error| error.to_string())?);
        let request_id = format!("task-terminal:{digest:x}");
        let ack: nola_protocol::TaskTerminalReconciliationApplied = match self {
            Self::Service(client) => {
                let request_id = task_import_request_id(&request_id, |id|
                    client.command_receipt(id).map(|receipt| receipt.map(|receipt| receipt.outcome)))?;
                match client.import_agent_task_terminal_projection_with_request_id(&request_id, projection) {
                    Ok(ack) => ack,
                    Err(error) if error.requires_reconciliation() => client
                        .reconcile_command_result(&error)
                        .map_err(|error| error.to_string())?,
                    Err(error) => return Err(error.to_string()),
                }
            },
            _ => self.execute(ApplicationCommand::ImportHostAgentTaskTerminalProjection {
                projection: projection.clone(),
            })?,
        };
        if ack.conversation_id != projection.admission.parent_conversation_id
            || ack.task_id != projection.admission.task_id
            || ack.receipt_id != projection.receipt_id
            || ack.core_revision < projection.admission.core_revision
        {
            return Err(
                "Task terminal receipt does not prove the exact imported projection".into(),
            );
        }
        Ok(ack)
    }

    /// The authorization can only enter this method as the result of the
    /// original Application's exact started control validation.
    pub(crate) fn execute_task_cancellation_control(
        &mut self, request: &nola_core::agent::host::HostRequestEnvelope,
    ) -> Result<bool, String> {
        use nola_domain::agent_task_admission::TaskCancellationExecution;
        use sha2::{Digest, Sha256};
        let result: TaskCancellationExecution = self.execute(
            ApplicationCommand::ExecuteTaskCancellationForEffect {
                request: request.clone(), now_ms: nola_runtime_driver::monotonic_epoch_millis(),
            })?;
        let authorization = match result {
            TaskCancellationExecution::Applied { cancel_requested } => return Ok(cancel_requested),
            TaskCancellationExecution::Transfer { authorization } => authorization,
        };
        authorization.validate()?;
        let digest = Sha256::digest(serde_json::to_vec(&authorization).map_err(|e|e.to_string())?);
        let request_id = format!("task-cancellation:{digest:x}");
        let mut target = Self::persistent_default()?;
        match &mut target {
            Self::Service(client) => {
                // A lost response is reconciled through this fixed command
                // receipt; it never mints another cancellation intent.
                match client.execute_typed_with_request_id(&request_id, serde_json::json!({
                    "type": "host_import_task_cancellation", "authorization": authorization,
                })) {
                    Ok(value) => Ok(value),
                    Err(error) if error.requires_reconciliation() => client.reconcile_command_result(&error)
                        .map_err(|error|error.to_string()),
                    Err(error) => Err(error.to_string()),
                }
            }
            _ => target.execute(ApplicationCommand::ImportHostTaskCancellation {
                authorization, now_ms: nola_runtime_driver::monotonic_epoch_millis(),
            }),
        }
    }

    pub(crate) fn export_agent_task_admission(
        &mut self,
        conversation_id: &str,
        run_id: &str,
        task_id: &str,
        event_cursor: u64,
    ) -> Result<ExportedAgentTaskAdmission, String> {
        let fact: nola_domain::agent_task_admission::AgentTaskAdmission =
            self.execute(ApplicationCommand::ExportAgentTaskAdmission {
                conversation_id: conversation_id.into(),
                run_id: run_id.into(),
                task_id: task_id.into(),
                event_cursor,
            })?;
        fact.validate()?;
        if fact.parent_conversation_id != conversation_id
            || fact.parent_run_id != run_id
            || fact.task_id != task_id
            || fact.core_event_cursor != event_cursor
        {
            return Err("source Task admission changed its exact export identity".into());
        }
        Ok(ExportedAgentTaskAdmission(fact))
    }

    pub(crate) fn recover_agent_task_admission(
        &mut self,
        conversation_id: &str,
        task_id: &str,
        target: &mut Self,
    ) -> Result<bool, String> {
        let location: Option<(String, u64)> = self.execute(ApplicationCommand::RuntimeTask {
            command: nola_application::RuntimeTaskCommand::SourceAgentTaskAdmissionLocation {
                owner_conversation_id: conversation_id.into(),
                task_id: task_id.into(),
            },
        })?;
        let Some((run_id, cursor)) = location else {
            // Prepared work without a retained original commit has no execution
            // authority. Never infer a proof from a TaskHandle or another DB.
            return Ok(false);
        };
        let fact = self.export_agent_task_admission(conversation_id, &run_id, task_id, cursor)?;
        target.import_exported_agent_task_admission(&fact)?;
        Ok(true)
    }

    pub(crate) fn recover_prepared_agent_tasks(
        &mut self,
        conversation_id: &str,
        target: &mut Self,
    ) -> Result<usize, String> {
        let mut after_id = String::new();
        let mut recovered = 0;
        loop {
            let tasks: Vec<String> = target.execute(ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::PreparedAgentTaskIds {
                    owner_conversation_id: conversation_id.into(),
                    after_id: after_id.clone(),
                    limit: 64,
                },
            })?;
            if tasks.is_empty() { return Ok(recovered); }
            for task_id in &tasks {
                if task_id.as_str() <= after_id.as_str() {
                    return Err("Task admission recovery page did not advance".into());
                }
                recovered += usize::from(self.recover_agent_task_admission(conversation_id, task_id, target)?);
                after_id.clone_from(task_id);
            }
        }
    }

    pub(crate) fn import_exported_agent_task_admission(
        &mut self,
        source: &ExportedAgentTaskAdmission,
    ) -> Result<(), String> {
        use nola_domain::agent_task_admission::{
            RuntimeAgentTaskAuthority, RuntimeAgentTaskAuthorityPhase,
        };
        let fact = &source.0;
        let request_id = format!("task-admission:{}:{}", fact.task_id, fact.admission_digest);
        let authority: RuntimeAgentTaskAuthority = match self {
            Self::Service(client) => {
                let request_id = task_import_request_id(&request_id, |id|
                    client.command_receipt(id).map(|receipt| receipt.map(|receipt| receipt.outcome)))?;
                match client.import_agent_task_admission_with_request_id(&request_id, fact) {
                    Ok(authority) => authority,
                    Err(error) if error.requires_reconciliation() => client
                        .reconcile_command_result(&error)
                        .map_err(|error| error.to_string())?,
                    Err(error) => return Err(error.to_string()),
                }
            }
            _ => self.execute(ApplicationCommand::ImportHostAgentTaskAdmission {
                admission: fact.clone(),
            })?,
        };
        authority.validate()?;
        if authority.task_id != fact.task_id
            || authority.phase != RuntimeAgentTaskAuthorityPhase::Admitted
            || authority.authority_revision == 0
            || authority.admission.as_ref() != Some(fact)
        {
            return Err(
                "Task admission receipt does not prove the exact exported source fact".into(),
            );
        }
        Ok(())
    }
    /// A recovered command receipt proves delivery, not freshness of the Host
    /// checkpoint. Never adopt its historical checkpoint after lost transport.
    pub(crate) fn apply_task_guidance(
        &mut self,
        task_id: &str,
        message_id: &str,
        expected_run_id: &str,
        lease: Option<nola_application::RuntimeTaskLeaseWitness>,
        now_ms: i64,
    ) -> Result<Box<nola_application::TaskGuidanceApplyResult>, String> {
        let command = ApplicationCommand::ApplyTaskGuidance {
            task_id: task_id.into(),
            message_id: message_id.into(),
            expected_run_id: expected_run_id.into(),
            lease,
            now_ms,
        };
        let Self::Service(client) = self else {
            return self.execute(command);
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        match client.execute_typed_with_request_id(&request_id, command) {
            Err(error) if error.requires_reconciliation() => {
                let receipt: Box<nola_application::TaskGuidanceApplyResult> = client
                    .reconcile_command_result(&error)
                    .map_err(|error| error.to_string())?;
                if receipt.guidance.task_id != task_id
                    || receipt.guidance.message_id != message_id
                    || receipt.guidance.run_id != expected_run_id
                    || !matches!(receipt.guidance.status.as_str(), "delivered" | "rejected")
                {
                    return Err(
                        "guidance receipt does not prove the exact final delivery outcome".into(),
                    );
                }
                // New identity, no execution authority: observe current state.
                // If this response is also lost, leave FIFO unacknowledged;
                // adopting yet another historical receipt would repeat the bug.
                client
                    .execute_typed_with_request_id(
                        &uuid::Uuid::new_v4().to_string(),
                        ApplicationCommand::ApplyTaskGuidance {
                            task_id: task_id.into(),
                            message_id: message_id.into(),
                            expected_run_id: expected_run_id.into(),
                            lease: None,
                            now_ms: chrono::Utc::now().timestamp_millis(),
                        },
                    )
                    .map_err(|error| error.to_string())
            }
            result => result.map_err(|error| error.to_string()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_service_receipts(self) -> Self {
        match self {
            Self::InProcess(application) => Self::ReceivedInProcess(application),
            other => other,
        }
    }
    pub fn persistent_default() -> Result<Self, String> {
        let database = crate::platform_paths::application_database_path()?;
        Self::persistent(&database)
    }

    pub fn persistent(database: &Path) -> Result<Self, String> {
        #[cfg(test)]
        {
            static TEST_APPLICATION_OPEN_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
                std::sync::OnceLock::new();
            let _open_guard = TEST_APPLICATION_OPEN_LOCK
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            return nola_application_bootstrap::open(database)
                .map(Self::InProcess)
                .map_err(|error| format!("open test Application runtime failed: {error}"));
        }
        #[cfg(not(test))]
        {
            let client = ApplicationServiceClient::for_database(database);
            Ok(Self::Service(client))
        }
    }

    pub fn ephemeral() -> Result<Self, String> {
        nola_application_bootstrap::open(":memory:")
            .map(Self::InProcess)
            .map_err(|error| format!("open in-process Application runtime failed: {error}"))
    }

    pub fn execute<T: DeserializeOwned>(
        &mut self,
        command: impl serde::Serialize + Into<ApplicationCommand> + nola_protocol::CommandMetadata,
    ) -> Result<T, String> {
        self.execute_observed(command, |payload| {
            crate::llm::log_local_debug_payload("application_command", payload);
        })
    }

    fn execute_observed<T: DeserializeOwned>(
        &mut self,
        command: impl serde::Serialize + Into<ApplicationCommand> + nola_protocol::CommandMetadata,
        mut observe: impl FnMut(&Value),
    ) -> Result<T, String> {
        let command_type = command.wire_command_name();
        let frequent_runtime_poll = matches!(command_type, "runtime_task");
        if !frequent_runtime_poll {
            observe(&serde_json::json!({
                "type": "application_command",
                "command": command_type,
                "phase": "started",
            }));
        }
        let started = Instant::now();
        // Retain one command identity across execution and any receipt lookup.
        // A receipt query never creates a replacement write identity.
        let request_id = uuid::Uuid::new_v4().to_string();
        let result = match self {
            Self::Service(client) => execute_service_command(client, &request_id, None, command)
                .map_err(|error| error.to_string()),
            #[cfg(test)]
            Self::ReceivedInProcess(application) => {
                execute_in_process_received(application, &request_id, None, command.into())
            }
            Self::InProcess(application) => {
                let command = command.into();
                if !command.is_read_only_query() {
                    // Transport choice must not change the write contract.
                    // Use Application's exhaustive classification rather than
                    // a client-maintained subset of mutations with receipts.
                    execute_in_process_received(application, &request_id, None, command)
                } else {
                    application.execute(command).and_then(|value| {
                        serde_json::from_value(value).map_err(|error| {
                            format!("decode in-process Application result failed: {error}")
                        })
                    })
                }
            }
        };
        let duration_ms = started.elapsed().as_secs_f64() * 1_000.0;
        if !frequent_runtime_poll || result.is_err() || duration_ms >= 250.0 {
            let unresolved = result.as_ref().err().and_then(|error| {
                ApplicationServiceCommandError::unresolved_from_boundary_message(error)
            });
            observe(&command_completion_diagnostic(
                command_type, duration_ms, result.is_ok(), unresolved.is_some(),
            ));
        }
        result
    }

    pub fn list_conversations(&mut self) -> Result<Vec<ConversationSummary>, String> {
        let mut all = Vec::new();
        loop {
            let page = self.list_conversation_page(None, false, 1024, all.len())?;
            let complete = page.len() < 1024;
            all.extend(page);
            if complete {
                return Ok(all);
            }
        }
    }

    pub fn list_conversation_page(
        &mut self,
        project_root: Option<&str>,
        non_empty: bool,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ConversationSummary>, String> {
        self.execute(ApplicationCommand::ListConversations {
            query: nola_store_port::conversation_query::ConversationListQuery {
                conversation_id: None,
                project_root: project_root.map(str::to_owned),
                non_empty,
                limit,
                offset,
            },
        })
    }

    pub fn conversation_summary(
        &mut self,
        id: &str,
    ) -> Result<Option<ConversationSummary>, String> {
        let mut rows: Vec<ConversationSummary> =
            self.execute(ApplicationCommand::ListConversations {
                query: nola_store_port::conversation_query::ConversationListQuery {
                    conversation_id: Some(id.to_owned()),
                    project_root: None,
                    non_empty: false,
                    limit: 1,
                    offset: 0,
                },
            })?;
        Ok(rows.pop())
    }

    pub fn execute_with_revision<T: DeserializeOwned>(
        &mut self,
        expected_revision: u64,
        command: ApplicationCommand,
    ) -> Result<T, String> {
        let request_id = uuid::Uuid::new_v4().to_string();
        match self {
            Self::Service(client) => {
                execute_service_command(client, &request_id, Some(expected_revision), command)
                    .map_err(|error| error.to_string())
            }
            Self::InProcess(application) => execute_in_process_received(
                application,
                &request_id,
                Some(expected_revision),
                command,
            ),
            #[cfg(test)]
            Self::ReceivedInProcess(application) => execute_in_process_received(
                application,
                &request_id,
                Some(expected_revision),
                command,
            ),
        }
    }

    pub fn load_conversation(&mut self, id: &str) -> Result<Option<Conversation>, String> {
        self.execute(ApplicationCommand::LoadConversation {
            conversation_id: id.to_string(),
        })
    }

    pub fn load_conversation_projection_v1(&mut self, id: &str) -> Result<Value, String> {
        self.execute(ApplicationCommand::LoadConversationProjectionV1 {
            conversation_id: id.to_string(),
        })
    }

    pub fn load_conversation_context_usage_v1(&mut self, id: &str) -> Result<Value, String> {
        self.execute(ApplicationCommand::LoadConversationContextUsageV1 {
            conversation_id: id.to_string(),
        })
    }

    pub fn load_conversation_settings(&mut self, id: &str) -> Result<Option<String>, String> {
        self.execute(ApplicationCommand::LoadConversationSettings {
            conversation_id: id.to_string(),
        })
    }

    pub fn load_conversation_projection_window_v1(
        &mut self,
        id: &str,
        before_ordinal: Option<i64>,
        limit: usize,
        content_byte_limit: usize,
    ) -> Result<nola_application::ConversationProjectionWindowV1, String> {
        self.execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id: id.to_string(),
            before_ordinal,
            limit,
            content_byte_limit,
        })
    }

    pub fn load_compaction_lineage(
        &mut self,
        conversation_id: &str,
        compaction_part_id: Option<&str>,
    ) -> Result<Value, String> {
        self.execute(ApplicationCommand::LoadCompactionLineage {
            conversation_id: conversation_id.to_string(),
            compaction_part_id: compaction_part_id.map(str::to_string),
        })
    }

    pub fn create_conversation(&mut self, input: CreateConversation) -> Result<(), String> {
        let _: Value = self.execute(ApplicationCommand::CreateConversation { input })?;
        Ok(())
    }

    pub fn import_conversation_transcript(
        &mut self,
        input: ImportConversationTranscript,
    ) -> Result<(), String> {
        let _: Value =
            self.execute(nola_protocol::TranscriptCommand::ImportConversationTranscript { input })?;
        Ok(())
    }

    pub fn revert_conversation(
        &mut self,
        input: RevertConversation,
    ) -> Result<AcceptedConversationRevert, String> {
        self.execute(ApplicationCommand::RevertConversation { input })
    }

    pub fn create_conversation_branch(
        &mut self,
        input: CreateConversationBranch,
    ) -> Result<ConversationBranchCreation, String> {
        self.execute(ApplicationCommand::CreateConversationBranch { input })
    }

    pub fn active_run(&mut self, conversation_id: &str) -> Result<Option<ActiveRun>, String> {
        self.execute(ApplicationCommand::ActiveRun {
            conversation_id: conversation_id.to_string(),
        })
    }

    pub fn begin_assistant_turn(
        &mut self,
        input: BeginAssistantTurn,
    ) -> Result<AcceptedAssistantTurn, ApplicationServiceCommandError> {
        self.execute_typed(ApplicationCommand::BeginAssistantTurn { input })
    }

    pub fn begin_queued_user_turn(
        &mut self,
        request_id: &str,
        input: nola_application::BeginQueuedUserTurn,
    ) -> Result<AcceptedAssistantTurn, ApplicationServiceCommandError> {
        self.execute_typed_received(request_id, ApplicationCommand::BeginQueuedUserTurn { input })
    }

    pub fn begin_task_assistant_turn(
        &mut self,
        lease: nola_application::RuntimeTaskLeaseWitness,
        input: BeginAssistantTurn,
    ) -> Result<AcceptedAssistantTurn, ApplicationServiceCommandError> {
        self.execute_typed(ApplicationCommand::BeginTaskAssistantTurn { lease, input })
    }

    pub fn begin_automation_assistant_turn(
        &mut self,
        lease: nola_domain::automation_execution::AutomationExecutionLease,
        input: BeginAssistantTurn,
    ) -> Result<AcceptedAssistantTurn, ApplicationServiceCommandError> {
        self.execute_typed(ApplicationCommand::BeginAutomationAssistantTurn { lease, input })
    }

    pub(crate) fn execute_typed<T: DeserializeOwned>(&mut self, command: ApplicationCommand)
        -> Result<T, ApplicationServiceCommandError>
    {
        let request_id = uuid::Uuid::new_v4().to_string();
        self.execute_typed_received(&request_id, command)
    }

    pub(crate) fn execute_typed_received<T: DeserializeOwned>(&mut self, request_id: &str, command: ApplicationCommand)
        -> Result<T, ApplicationServiceCommandError>
    {
        let command_type = nola_protocol::CommandMetadata::wire_command_name(&command);
        let started = Instant::now();
        crate::llm::log_local_debug_payload("application_command", &serde_json::json!({
            "type": "application_command", "command": command_type, "phase": "started",
        }));
        // Preserve the receipt outcome through Host adoption. Even ephemeral
        // admission uses receipts, so rejection is distinct from lost commit ACK.
        let result = match self {
            Self::Service(client) => execute_service_command(client, &request_id, None, command),
            Self::InProcess(application) => execute_in_process_received_typed(application, &request_id, None, command),
            #[cfg(test)]
            Self::ReceivedInProcess(application) => execute_in_process_received_typed(application, &request_id, None, command),
        };
        let unresolved = result.as_ref().err().filter(|error| error.requires_reconciliation());
        crate::llm::log_local_debug_payload("application_command", &command_completion_diagnostic(
            command_type, started.elapsed().as_secs_f64() * 1_000.0,
            result.is_ok(), unresolved.is_some(),
        ));
        result
    }

    pub fn begin_maintenance_turn(
        &mut self,
        input: nola_application::BeginMaintenanceTurn,
    ) -> Result<nola_application::AcceptedMaintenanceTurn, String> {
        self.execute(ApplicationCommand::BeginMaintenanceTurn { input })
    }

    pub fn load_committed_host_requests(
        &mut self,
        run_id: &str,
    ) -> Result<CommittedHostRequests, String> {
        self.execute(ApplicationCommand::LoadCommittedHostRequests {
            run_id: run_id.to_string(),
        })
    }

    pub fn claim_committed_host_requests(
        &mut self,
        input: nola_application::ClaimCommittedHostRequests,
    ) -> Result<CommittedHostRequests, String> {
        self.execute(ApplicationCommand::ClaimCommittedHostRequests { input })
    }

    pub fn heartbeat_effect_delivery(
        &mut self,
        input: nola_application::HeartbeatEffectDelivery,
    ) -> Result<nola_core::agent::host::EffectDeliveryLease, String> {
        self.execute(ApplicationCommand::HeartbeatEffectDelivery { input })
    }

    pub fn load_checkpoint(
        &mut self,
        conversation_id: &str,
    ) -> Result<Option<ConversationCheckpoint>, String> {
        self.execute(ApplicationCommand::LoadCheckpoint {
            conversation_id: conversation_id.to_string(),
        })
    }

    pub fn load_session_goal_projection(&mut self, conversation_id: &str)
        -> Result<Option<nola_core::agent::GoalProjection>, String> {
        self.execute(ApplicationCommand::LoadSessionGoalProjection {
            conversation_id: conversation_id.to_owned(),
        })
    }

    pub fn load_session_projection(
        &mut self,
        conversation_id: &str,
    ) -> Result<Option<SessionProjection>, String> {
        self.execute(ApplicationCommand::LoadSessionProjection {
            conversation_id: conversation_id.to_string(),
        })
    }

    pub fn load_resume_snapshot(
        &mut self,
        project_root: &str,
        host_id: Option<&str>,
        requested_at_ms: i64,
    ) -> Result<Option<ResumeSnapshot>, String> {
        self.execute(ApplicationCommand::LoadResumeSnapshot {
            project_root: project_root.to_string(),
            host_id: host_id.map(str::to_string),
            requested_at_ms,
        })
    }

    pub fn acknowledge_resume_snapshot(
        &mut self,
        input: AcknowledgeResumeSnapshot,
    ) -> Result<(), String> {
        let _: Value = self.execute(ApplicationCommand::AcknowledgeResumeSnapshot { input })?;
        Ok(())
    }

    pub fn delete_conversation(&mut self, conversation_id: &str) -> Result<bool, String> {
        self.execute(ApplicationCommand::DeleteConversation {
            conversation_id: conversation_id.to_string(),
        })
    }

    pub fn reduce_and_accept_host_response(
        &mut self,
        input: ReduceAndAcceptHostResponse,
    ) -> Result<AcceptedHostResponse, String> {
        self.execute(ApplicationCommand::ReduceAndAcceptHostResponse { input })
    }

    pub fn reduce_and_commit_active_submission(
        &mut self,
        input: ReduceAndCommitActiveSubmission,
    ) -> Result<AcceptedActiveSubmission, String> {
        self.execute(ApplicationCommand::ReduceAndCommitActiveSubmission { input })
    }

    pub fn reduce_and_commit_idle_submission(
        &mut self,
        input: ReduceAndCommitIdleSubmission,
    ) -> Result<AcceptedIdleSubmission, nola_service_client::ApplicationServiceCommandError> {
        self.execute_typed(ApplicationCommand::ReduceAndCommitIdleSubmission { input })
    }
}

#[cfg(test)]
mod goal_intent_receipt_tests {
    use super::*;
    use nola_application::goal_control_intent::{GoalControlAction, PrepareGoalControlIntent};

    #[test]
    fn prepared_goal_time_and_identity_survive_reopen_and_reject_payload_reuse() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("goal-intents.sqlite");
        let input = PrepareGoalControlIntent {
            conversation_id: "conversation".into(), operation_id: "same-user-operation".into(),
            action: GoalControlAction::Start, expected_goal_id: None, expected_control_token: None,
            objective: "Verify a result".into(), constraints: vec![], wake_at_epoch_ms: None,
        };
        let mut first = CliApplicationPort::persistent(&path).unwrap();
        let original = first.prepare_goal_control_intent(input.clone()).unwrap();
        drop(first);
        let mut reopened = CliApplicationPort::persistent(&path).unwrap();
        assert_eq!(reopened.prepare_goal_control_intent(input.clone()).unwrap(), original);
        let mut changed = input;
        changed.objective = "A different result".into();
        assert!(reopened.prepare_goal_control_intent(changed).is_err());
    }
}

/// A definite non-commit permits another attempt with the same immutable
/// source proof. Uncertain or committed receipts retain their original ID.
/// Deterministic successors let concurrent/cold recovery find that same attempt.
fn task_import_request_id(
    base: &str,
    mut lookup: impl FnMut(&str) -> Result<Option<nola_protocol::CommandReceiptOutcome>, String>,
) -> Result<String, String> {
    // Bound one recovery pass; never spin indefinitely on a corrupt or
    // permanently rejected history. No replacement write is made at the cap.
    for attempt in 0..64_u64 {
        let id = if attempt == 0 { base.to_owned() } else { format!("{base}:retry:{attempt}") };
        let definitely_rejected = match lookup(&id)? {
            Some(nola_protocol::CommandReceiptOutcome::Rejected { error, .. }) =>
                !nola_service_client::command_outcome_requires_reconciliation(&error.code),
            Some(nola_protocol::CommandReceiptOutcome::Conflict { .. }) => true,
            _ => false,
        };
        if !definitely_rejected { return Ok(id); }
    }
    Err("Task import has too many rejected receipts; inspect the original failure before retrying".into())
}

fn reconcile_in_process_input(
    application: &mut ApplicationRuntime, request_id: &str, command: ApplicationCommand, allow_submit: bool,
) -> Result<bool, String> {
    let existing = application.command_receipt("cli-in-process",request_id).map_err(|e|e.to_string())?;
    if existing.is_none() && !allow_submit { return Ok(false); }
    execute_in_process_received::<bool>(application,request_id,None,command)
}

fn execute_service_command<T: DeserializeOwned>(
    client: &ApplicationServiceClient,
    request_id: &str,
    expected_revision: Option<u64>,
    command: impl serde::Serialize,
) -> Result<T, ApplicationServiceCommandError> {
    match client.execute_typed_with_revision(request_id, expected_revision, command) {
        Err(error) if error.requires_reconciliation() => client.reconcile_command_result(&error),
        result => result,
    }
}

fn execute_in_process_received<T: DeserializeOwned>(
    application: &mut ApplicationRuntime,
    request_id: &str,
    expected_revision: Option<u64>,
    command: ApplicationCommand,
) -> Result<T, String> {
    execute_in_process_received_typed(application, request_id, expected_revision, command)
        .map_err(|error| error.to_string())
}

fn execute_in_process_received_typed<T: DeserializeOwned>(
    application: &mut ApplicationRuntime,
    request_id: &str,
    expected_revision: Option<u64>,
    command: ApplicationCommand,
) -> Result<T, ApplicationServiceCommandError> {
    let not_sent = |message: String| ApplicationServiceCommandError::Transport {
        client_id: "cli-in-process".into(), request_id: request_id.into(), message, outcome_unknown: false,
    };
    let rejected = |code, message, recoverable, details| ApplicationServiceCommandError::Rejected {
        client_id: "cli-in-process".into(), request_id: request_id.into(),
        error: nola_protocol::ProtocolFailure { code, message, recoverable, details },
    };
    let unknown = |code, message| ApplicationServiceCommandError::OutcomeUnknown {
        client_id: "cli-in-process".into(), request_id: request_id.into(), code, message, reopen_required: true,
    };
    let mut arguments = serde_json::to_value(&command).map_err(|error| not_sent(error.to_string()))?;
    let command_type = arguments
        .as_object_mut()
        .and_then(|object| object.remove("type"))
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| not_sent("missing Application command type".to_string()))?;
    let receipt = application
        .execute_received_command(
            nola_application::ExecuteReceivedCommand {
                client_id: "cli-in-process".into(),
                request_id: request_id.into(),
                command_type,
                command_json: arguments.to_string(),
                expected_revision,
                received_at: chrono::Utc::now().to_rfc3339(),
            },
            command,
        )
        .map_err(|error| {
            if nola_service_client::command_outcome_requires_reconciliation(&error.code) {
                unknown(error.code, error.message)
            } else {
                rejected(error.code, error.message, error.recoverable, None)
            }
        })?;
    use nola_application::ApplicationCommandReceiptOutcome;
    match receipt.outcome {
        ApplicationCommandReceiptOutcome::Committed { result, .. } =>
            serde_json::from_value(result).map_err(|error| unknown("committed_result_decode_failed".into(), error.to_string())),
        ApplicationCommandReceiptOutcome::Conflict { scope_id, expected_revision, actual_revision } =>
            Err(ApplicationServiceCommandError::Conflict {
                client_id: "cli-in-process".into(), request_id: request_id.into(),
                conflict: nola_protocol::RevisionConflict { scope_id, expected_revision, actual_revision },
            }),
        ApplicationCommandReceiptOutcome::Rejected { code, message, recoverable, details } => {
            if nola_service_client::command_outcome_requires_reconciliation(&code) {
                Err(unknown(code, message))
            } else {
                Err(rejected(code, message, recoverable, details))
            }
        }
        ApplicationCommandReceiptOutcome::RecoveryRequired => Err(ApplicationServiceCommandError::Pending {
            client_id: "cli-in-process".into(), request_id: request_id.into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_diagnostics_preserve_lifecycle_without_copying_errors() {
        for (succeeded, pending, phase) in [
            (true, false, "succeeded"),
            (false, false, "failed"),
            (false, true, "awaiting_reconciliation"),
        ] {
            let event = command_completion_diagnostic("begin_assistant_turn", 12.5, succeeded, pending);
            assert_eq!(event, serde_json::json!({
                "type": "application_command", "command": "begin_assistant_turn",
                "phase": phase, "duration_ms": 12.5,
                "failed": !succeeded, "requires_reconciliation": pending,
            }));
        }

        #[derive(Debug)]
        struct PrivateDecodeFailure;
        impl<'de> serde::Deserialize<'de> for PrivateDecodeFailure {
            fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                Err(serde::de::Error::custom("私密正文 /private/secret.txt https://example.invalid/?key=secret"))
            }
        }
        let mut port = CliApplicationPort::ephemeral().unwrap();
        let mut events = Vec::new();
        let result: Result<PrivateDecodeFailure, _> = port.execute_observed(
            ApplicationCommand::ListConversations {
                query: nola_store_port::conversation_query::ConversationListQuery {
                    conversation_id: None, project_root: None, non_empty: false, limit: 1, offset: 0,
                },
            },
            |event| events.push(event.clone()),
        );
        // The caller still receives the original error; only the log differs.
        assert!(result.unwrap_err().contains("私密正文 /private/secret.txt"));
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["phase"], "failed");
        assert_eq!(events[1]["failed"], true);
        let serialized = serde_json::to_string(&events).unwrap();
        for private in ["私密", "/private/", "secret", "example.invalid", "https://"] {
            assert!(!serialized.contains(private), "diagnostic copied {private}");
        }
    }

    #[test]
    fn in_process_writes_have_durable_receipts_and_reads_do_not_add_them() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("application.db");
        let mut port = CliApplicationPort::persistent(&database).unwrap();
        let _: Value = port.execute(ApplicationCommand::PutProductSetting {
            key: "architecture.receipt".into(), value_json: "true".into(), updated_at_ms: 1,
        }).unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        let receipt: (String, String, String) = connection.query_row(
            "SELECT request_id,command_type,status FROM command_receipts WHERE client_id='cli-in-process'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert!(!receipt.0.is_empty());
        assert_eq!(receipt.1, "put_product_setting");
        assert_eq!(receipt.2, "committed");
        drop(port);
        let mut reopened = CliApplicationPort::persistent(&database).unwrap();
        let result: Value = reopened.execute(ApplicationCommand::GetProductSetting {
            key: "architecture.receipt".into(),
        }).unwrap();
        assert_eq!(result["value_json"], "true");
        let count: i64 = connection.query_row("SELECT count(*) FROM command_receipts", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1, "read polling must not create write receipts");
    }

    #[test]
    fn task_admission_retries_only_after_a_definite_non_commit() {
        use nola_protocol::{CommandReceiptOutcome, ProtocolFailure};
        let rejected = |code: &str| CommandReceiptOutcome::Rejected {
            completed_at: "2026-10-08T00:00:00Z".into(),
            error: ProtocolFailure {
                code: code.into(), message: "fixture".into(), recoverable: true, details: None,
            },
        };
        let mut observed = Vec::new();
        let id = task_import_request_id("task-admission:task:digest", |id| {
            observed.push(id.to_owned());
            Ok(if observed.len() == 1 { Some(rejected("rejected")) } else { None })
        }).unwrap();
        assert_eq!(id, "task-admission:task:digest:retry:1");
        assert_eq!(observed.len(), 2);
        let terminal = task_import_request_id("task-terminal:digest", |id|
            Ok((id == "task-terminal:digest").then(|| rejected("application_rejected")))).unwrap();
        assert_eq!(terminal, "task-terminal:digest:retry:1");
        for outcome in [
            CommandReceiptOutcome::Accepted,
            rejected("command_transaction_commit_outcome_unknown"),
            CommandReceiptOutcome::Committed {
                completed_at: "2026-10-08T00:00:00Z".into(), committed_revision: Some(1),
                result: serde_json::json!({}),
            },
        ] {
            assert_eq!(task_import_request_id("original", |_| Ok(Some(outcome.clone()))).unwrap(),
                "original", "uncertain and committed writes must retain their receipt identity");
        }
        assert!(task_import_request_id("original", |_| Err("observation unavailable".into())).is_err());
        let mut reads = 0;
        assert!(task_import_request_id("original", |_| {
            reads += 1;
            Ok(Some(rejected("rejected")))
        }).is_err());
        assert_eq!(reads, 64);
    }

    #[cfg(unix)]
    #[test]
    fn guidance_receipt_recovery_requires_a_fresh_authority_free_observation() {
        use nola_protocol::{
            CommandEnvelope, CommandReceipt, CommandReceiptOutcome, ProtocolFailure,
        };
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        for lose_observation in [false, true] {
            let socket = std::path::PathBuf::from(format!(
                "/tmp/nola-guidance-receipt-{}.sock",
                uuid::Uuid::new_v4()
            ));
            let listener = UnixListener::bind(&socket).unwrap();
            let server = std::thread::spawn(move || {
                let result = serde_json::json!({"guidance":{
                    "task_id":"task","message_id":"message","owner_session_id":"parent",
                    "parent_core_session_id":"parent-core","parent_turn_id":"parent-turn",
                    "child_session_id":"child","run_id":"run","input_watermark":7,
                    "content":"Continue","submitted_at_ms":1,"status":"rejected",
                    "accepted_at_ms":null,"resolved_at_ms":2,"reason":"run_terminal",
                    "delivery_transition_id":null},"accepted":null,"recovered":null});
                let mut requests: Vec<CommandEnvelope> = Vec::new();
                for index in 0..6 {
                    let (stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                        .unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let request: CommandEnvelope = serde_json::from_str(&line).unwrap();
                    let outcome = if index == 1 || (index == 5 && lose_observation) {
                        CommandReceiptOutcome::Rejected {
                            completed_at: "2".into(),
                            error: ProtocolFailure {
                                code: "command_transaction_commit_outcome_unknown".into(),
                                message: "lost ACK".into(),
                                recoverable: false,
                                details: None,
                            },
                        }
                    } else {
                        let payload = if index == 3 {
                            serde_json::to_value(Some(CommandReceipt {
                                protocol_version:
                                    nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                                client_id: requests[1].client_id.clone(),
                                request_id: requests[1].request_id.clone(),
                                received_at: "1".into(),
                                outcome: CommandReceiptOutcome::Committed {
                                    completed_at: "2".into(),
                                    committed_revision: None,
                                    result: result.clone(),
                                },
                            }))
                            .unwrap()
                        } else if index == 5 {
                            result.clone()
                        } else {
                            serde_json::json!({"service":"fixture"})
                        };
                        CommandReceiptOutcome::Committed {
                            completed_at: "2".into(),
                            committed_revision: None,
                            result: payload,
                        }
                    };
                    let response = CommandReceipt {
                        protocol_version: nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                        client_id: request.client_id.clone(),
                        request_id: request.request_id.clone(),
                        received_at: "1".into(),
                        outcome,
                    };
                    let mut stream = reader.into_inner();
                    serde_json::to_writer(&mut stream, &response).unwrap();
                    stream.write_all(b"\n").unwrap();
                    requests.push(request);
                }
                requests
            });
            let client = ApplicationServiceClient::with_identity(
                "/tmp/nola-guidance-fixture.db",
                &socket,
                "guidance-client",
            )
            .unwrap();
            let lease = nola_application::RuntimeTaskLeaseWitness {
                task_id: "task".into(),
                attempt_id: "attempt".into(),
                worker_generation: "generation".into(),
                lease_token: "fixture-token".into(),
                expected_attempt_revision: 1,
                current_expiry_ms: 100,
            };
            let result = CliApplicationPort::Service(client).apply_task_guidance(
                "task",
                "message",
                "run",
                Some(lease),
                1,
            );
            assert_eq!(
                result.is_err(),
                lose_observation,
                "lost fresh observation must not adopt the old receipt"
            );
            let requests = server.join().unwrap();
            assert_eq!(requests[3].command.command_type, "command_receipt");
            assert_eq!(requests[5].command.command_type, "apply_task_guidance");
            assert_ne!(requests[5].request_id, requests[1].request_id);
            assert!(requests[1].command.arguments["lease"].is_object());
            assert!(requests[5].command.arguments["lease"].is_null());
            assert_eq!(requests[5].command.arguments["expected_run_id"], "run");
            assert_eq!(requests[5].command.arguments["message_id"], "message");
            std::fs::remove_file(socket).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn service_execution_owner_reconciles_the_original_id_and_keeps_unknown_nonterminal() {
        use nola_protocol::{
            CommandEnvelope, CommandReceipt, CommandReceiptOutcome, ProtocolFailure,
        };
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        for committed in [true, false] {
            let socket = std::path::PathBuf::from(format!(
                "/tmp/nola-cli-reconcile-{}.sock",
                uuid::Uuid::new_v4()
            ));
            let listener = UnixListener::bind(&socket).unwrap();
            let server = std::thread::spawn(move || {
                let mut requests: Vec<CommandEnvelope> = Vec::new();
                for index in 0..4 {
                    let (stream, _) = listener.accept().unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut frame = String::new();
                    reader.read_line(&mut frame).unwrap();
                    let envelope: CommandEnvelope = serde_json::from_str(&frame).unwrap();
                    let outcome = if index == 1 {
                        CommandReceiptOutcome::Rejected {
                            completed_at: "2".into(),
                            error: ProtocolFailure {
                                code: "command_transaction_commit_outcome_unknown".into(),
                                message: "fixture commit acknowledgement lost".into(),
                                recoverable: false,
                                details: None,
                            },
                        }
                    } else {
                        let result = if index == 3 {
                            let original = &requests[1];
                            serde_json::to_value(committed.then(|| CommandReceipt {
                                protocol_version:
                                    nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                                client_id: original.client_id.clone(),
                                request_id: original.request_id.clone(),
                                received_at: "1".into(),
                                outcome: CommandReceiptOutcome::Committed {
                                    completed_at: "2".into(),
                                    committed_revision: Some(2),
                                    result: serde_json::json!({"renamed":true}),
                                },
                            }))
                            .unwrap()
                        } else {
                            serde_json::json!({"service":"fixture"})
                        };
                        CommandReceiptOutcome::Committed {
                            completed_at: "2".into(),
                            committed_revision: None,
                            result,
                        }
                    };
                    let receipt = CommandReceipt {
                        protocol_version: nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                        client_id: envelope.client_id.clone(),
                        request_id: envelope.request_id.clone(),
                        received_at: "1".into(),
                        outcome,
                    };
                    let mut stream = reader.into_inner();
                    serde_json::to_writer(&mut stream, &receipt).unwrap();
                    stream.write_all(b"\n").unwrap();
                    requests.push(envelope);
                }
                requests
            });
            let client = ApplicationServiceClient::with_identity(
                "/tmp/nola-cli-reconcile.db",
                &socket,
                "cli-reconcile-client",
            )
            .unwrap();
            let mut port = CliApplicationPort::Service(client);
            let mut events = Vec::new();
            let result: Result<Value, String> = port.execute_observed(
                ApplicationCommand::RenameConversation {
                    title_source: None,
                    conversation_id: "conversation".into(),
                    title: "Updated".into(),
                    updated_at_ms: 2,
                },
                |event| events.push(event.clone()),
            );
            let requests = server.join().unwrap();
            let _ = std::fs::remove_file(&socket);
            assert_eq!(requests[3].command.command_type, "command_receipt");
            assert_eq!(
                requests[3].command.arguments["request_id"],
                requests[1].request_id
            );
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.command.command_type == "rename_conversation")
                    .count(),
                1
            );
            if committed {
                assert_eq!(result.unwrap(), serde_json::json!({"renamed":true}));
                assert_eq!(events[1]["phase"], "succeeded");
            } else {
                let error = result.unwrap_err();
                let unresolved =
                    ApplicationServiceCommandError::unresolved_from_boundary_message(&error)
                        .unwrap();
                assert_eq!(unresolved.request_id(), requests[1].request_id);
                assert_eq!(unresolved.client_id(), requests[1].client_id);
                assert_eq!(events[1]["phase"], "awaiting_reconciliation");
                assert_eq!(events[1]["requires_reconciliation"], true);
                assert!(events[1].get("unresolved_receipt").is_none());
                assert!(events[1].get("error").is_none());
                assert!(!events[1].to_string().contains(&requests[1].request_id));
            }
        }
    }

    #[test]
    fn ten_million_character_import_diagnostics_never_serialize_the_payload() {
        use nola_protocol::{
            ConversationTranscriptImportSource, TranscriptCommand, TranscriptMessageInput,
            TranscriptPartInput,
        };
        struct NoDiagnosticSerialization(TranscriptCommand);
        impl serde::Serialize for NoDiagnosticSerialization {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                panic!("diagnostics must not serialize the command payload");
            }
        }
        impl nola_protocol::CommandMetadata for NoDiagnosticSerialization {
            fn wire_command_name(&self) -> &'static str {
                "import_conversation_transcript"
            }
        }
        impl From<NoDiagnosticSerialization> for ApplicationCommand {
            fn from(command: NoDiagnosticSerialization) -> Self {
                command.0.into()
            }
        }
        let mut port = CliApplicationPort::ephemeral().unwrap();
        port.create_conversation(CreateConversation {
            id: "large-import".into(),
            title: "Large".into(),
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
        let command = TranscriptCommand::ImportConversationTranscript {
            input: ImportConversationTranscript {
                import_id: "archive".into(),
                source: ConversationTranscriptImportSource::ExternalArchive,
                conversation_id: "large-import".into(),
                expected_message_count: 0,
                imported_at_ms: 2,
                messages: vec![TranscriptMessageInput {
                    id: "message".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(1),
                    parts: vec![TranscriptPartInput {
                        id: "part".into(),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: "x".repeat(10_000_000),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    }],
                }],
            },
        };
        let mut events = Vec::new();
        let result: Value = port
            .execute_observed(NoDiagnosticSerialization(command), |event| {
                events.push(event.clone())
            })
            .unwrap();
        assert_eq!(result["appended_message_count"], 1);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["phase"], "succeeded");
        assert!(serde_json::to_vec(&events).unwrap().len() < 1024);
        let tail: Value = port
            .execute(ApplicationCommand::ReadConversationTextRange {
                conversation_id: "large-import".into(),
                message_id: "message".into(),
                part_id: "part".into(),
                offset_bytes: 9_999_992,
                limit_bytes: 8,
            })
            .unwrap();
        assert_eq!(tail["text"], "xxxxxxxx");
        assert_eq!(tail["end_of_content"], true);
    }

    #[test]
    fn embedded_admission_success_and_failure_both_report_completion() {
        use nola_application::McpCommand;
        let mut port = CliApplicationPort::ephemeral().unwrap();
        let commands = [
            McpCommand::Create {
                id: "observed".into(),
                display_name: "Observed".into(),
                enabled: false,
                configuration_json: "{}".into(),
                created_at_ms: 1,
            },
            McpCommand::DeleteChecked {
                id: "observed".into(),
                expected_revision: 999,
                deleted_at_ms: 2,
            },
        ];
        for (index, command) in commands.into_iter().enumerate() {
            let mut events = Vec::new();
            let result: Result<Value, _> = port
                .execute_observed(ApplicationCommand::Mcp { command }, |event| {
                    events.push(event.clone())
                });
            assert_eq!(result.is_ok(), index == 0);
            assert_eq!(events.len(), 2);
            assert_eq!(events[0]["phase"], "started");
            assert_eq!(
                events[1]["phase"],
                if index == 0 { "succeeded" } else { "failed" }
            );
            assert!(events[1]["duration_ms"].as_f64().unwrap() >= 0.0);
            assert_eq!(events[1]["failed"], index != 0);
            assert_eq!(events[1]["requires_reconciliation"], false);
            assert!(events[1].get("error").is_none());
        }
    }

    #[test]
    fn embedded_execution_and_decode_errors_report_completion() {
        let mut port = CliApplicationPort::ephemeral().unwrap();
        let mut events = Vec::new();
        let failed: Result<Value, _> = port.execute_observed(
            ApplicationCommand::RenameConversation {
                title_source: None,
                conversation_id: "missing".into(),
                title: " ".into(),
                updated_at_ms: 1,
            },
            |event| events.push(event.clone()),
        );
        assert!(failed.is_err());
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["phase"], "failed");
        events.clear();
        let failed: Result<u64, _> = port.execute_observed(
            ApplicationCommand::ListConversations {
                query: nola_store_port::conversation_query::ConversationListQuery {
                    conversation_id: None,
                    project_root: None,
                    non_empty: false,
                    limit: 1024,
                    offset: 0,
                },
            },
            |event| events.push(event.clone()),
        );
        assert!(failed.unwrap_err().contains("decode in-process"));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["phase"], "started");
        assert_eq!(events[1]["phase"], "failed");
    }

    #[test]
    fn no_save_mcp_edits_use_atomic_admission() {
        use nola_application::McpCommand;
        let mut application = CliApplicationPort::ephemeral().unwrap();
        let create = || McpCommand::Create {
            id: "server".into(),
            display_name: "Server".into(),
            enabled: false,
            configuration_json: "{}".into(),
            created_at_ms: 1,
        };
        let failed: Result<Value, _> = application.execute(ApplicationCommand::Mcp {
            command: McpCommand::ApplyChanges {
                commands: vec![
                    create(),
                    McpCommand::DeleteChecked {
                        id: "server".into(),
                        expected_revision: 999,
                        deleted_at_ms: 2,
                    },
                ],
            },
        });
        assert!(failed.unwrap_err().contains("mcp_revision_conflict"));
        let created: Value = application
            .execute(ApplicationCommand::Mcp { command: create() })
            .unwrap();
        assert_eq!(created["revision"], 1);
        let stale: Result<Value, _> = application.execute(ApplicationCommand::Mcp {
            command: McpCommand::DeleteChecked {
                id: "server".into(),
                expected_revision: 999,
                deleted_at_ms: 3,
            },
        });
        assert!(stale.unwrap_err().contains("mcp_revision_conflict"));
        let servers: Value = application
            .execute(ApplicationCommand::Mcp {
                command: McpCommand::List,
            })
            .unwrap();
        assert_eq!(servers.as_array().unwrap().len(), 1);
    }

    #[test]
    fn no_save_session_uses_the_application_runtime_contract() {
        let mut application = CliApplicationPort::ephemeral().expect("open Application runtime");
        application
            .create_conversation(CreateConversation {
                id: "in-process-conversation".into(),
                title: "In process".into(),
                created_at_ms: 1,
                updated_at_ms: 1,
                settings_json: None,
            })
            .expect("create conversation through Application port");

        let conversation = application
            .load_conversation("in-process-conversation")
            .expect("load conversation through Application port")
            .expect("conversation exists");
        assert_eq!(conversation.id, "in-process-conversation");
    }
}
