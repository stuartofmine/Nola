// SPDX-License-Identifier: Apache-2.0
//! Shared client and lifecycle boundary for the local Nola Application Service.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(test)]
use nola_protocol::CommandReceiptOutcome;
use nola_protocol::{
    CommandEnvelope, CommandPayload, CommandReceipt, CommandReceiptLookupRequest, EventSuffix,
    EventSuffixRequest, ProjectionSnapshot, ProjectionSnapshotRequest, RuntimeEffectWorkSchedule,
    RuntimeEffectWorkWaitRequest, APPLICATION_SERVICE_MAX_FRAME_BYTES,
    APPLICATION_SERVICE_PROTOCOL_VERSION,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);
mod command_error;
mod conversation_recall;
pub use command_error::{command_outcome_requires_reconciliation, ApplicationServiceCommandError};
use command_error::{decode_committed_result, receipt_result_typed, unknown_receipt_error};

/// Stable error returned when a cancellable request is stopped by its caller.
pub const APPLICATION_SERVICE_REQUEST_CANCELLED: &str = "application_service_request_cancelled";
const RUNTIME_EFFECT_WORK_TRANSPORT_GRACE: Duration = Duration::from_secs(1);

fn runtime_effect_work_transport_timeout(wait_ms: u32) -> Duration {
    Duration::from_millis(u64::from(wait_ms)) + RUNTIME_EFFECT_WORK_TRANSPORT_GRACE
}

#[derive(Debug, Clone)]
pub struct ApplicationServiceClient {
    database_path: PathBuf,
    socket_path: PathBuf,
    client_id: String,
    request_timeout: Duration,
}

/// One-shot cancellation for a blocking Application Service request.
///
/// Clones refer to the same request. Calling [`Self::cancel`] from another
/// thread actively shuts down the request connection, so a long poll does not
/// have to wait for its RPC deadline. A cancelled handle cannot be reused.
#[derive(Clone, Default)]
pub struct ApplicationServiceRequestCancellation {
    inner: Arc<platform::RequestCancellation>,
}

impl std::fmt::Debug for ApplicationServiceRequestCancellation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplicationServiceRequestCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl ApplicationServiceRequestCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
}

impl ApplicationServiceClient {
    pub fn for_database(database_path: impl Into<PathBuf>) -> Self {
        let database_path = database_path.into();
        let socket_path = socket_path_for_database(&database_path);
        let client_id = client_id_for_database(&database_path);
        Self {
            database_path,
            socket_path,
            client_id,
            request_timeout: Duration::from_secs(30),
        }
    }

    pub fn with_socket(database_path: impl Into<PathBuf>, socket_path: impl Into<PathBuf>) -> Self {
        let database_path = database_path.into();
        Self {
            client_id: client_id_for_database(&database_path),
            database_path,
            socket_path: socket_path.into(),
            request_timeout: Duration::from_secs(30),
        }
    }

    pub fn with_identity(
        database_path: impl Into<PathBuf>,
        socket_path: impl Into<PathBuf>,
        client_id: impl Into<String>,
    ) -> Result<Self, String> {
        let client_id = client_id.into();
        if client_id.trim().is_empty() || client_id.chars().any(char::is_control) {
            return Err("application service client_id is invalid".to_string());
        }
        Ok(Self {
            database_path: database_path.into(),
            socket_path: socket_path.into(),
            client_id,
            request_timeout: Duration::from_secs(30),
        })
    }

    /// Bounds one RPC, not the lifetime of a durable task started by that RPC.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Result<Self, String> {
        if timeout.is_zero() || std::time::Instant::now().checked_add(timeout).is_none() {
            return Err("application service request timeout must be positive and finite".into());
        }
        self.request_timeout = timeout;
        Ok(self)
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn ensure_running(&self) -> Result<(), String> {
        platform::ensure_running(self)
    }

    pub fn health(&self) -> Result<serde_json::Value, String> {
        self.request_with_timeout(
            CommandPayload {
                command_type: "health".to_string(),
                arguments: Value::Object(Map::new()),
            },
            Some(Duration::from_millis(250)),
        )
    }

    pub fn command_receipt(&self, request_id: &str) -> Result<Option<CommandReceipt>, String> {
        let request = CommandReceiptLookupRequest {
            request_id: request_id.to_string(),
        };
        request
            .validate()
            .map_err(|error| format!("invalid command receipt lookup: {error}"))?;
        self.ensure_running()?;
        let result = self.request(CommandPayload {
            command_type: "command_receipt".to_string(),
            arguments: serde_json::to_value(&request)
                .map_err(|error| format!("encode command receipt lookup failed: {error}"))?,
        })?;
        let receipt: Option<CommandReceipt> = serde_json::from_value(result)
            .map_err(|error| format!("decode command receipt lookup failed: {error}"))?;
        if let Some(receipt) = &receipt {
            receipt
                .validate()
                .map_err(|error| format!("invalid stored command receipt: {error}"))?;
            if receipt.client_id != self.client_id || receipt.request_id != request_id {
                return Err("stored command receipt identity mismatch".to_string());
            }
        }
        Ok(receipt)
    }

    pub fn projection_event_suffix(
        &self,
        request: &EventSuffixRequest,
    ) -> Result<EventSuffix, String> {
        request
            .validate()
            .map_err(|error| format!("invalid projection suffix request: {error}"))?;
        self.ensure_running()?;
        let arguments = serde_json::to_value(request)
            .map_err(|error| format!("encode projection suffix request failed: {error}"))?;
        let result = self.request(CommandPayload {
            command_type: "projection_event_suffix".to_string(),
            arguments,
        })?;
        let suffix: EventSuffix = serde_json::from_value(result)
            .map_err(|error| format!("decode projection event suffix failed: {error}"))?;
        suffix
            .validate()
            .map_err(|error| format!("invalid projection event suffix: {error}"))?;
        Ok(suffix)
    }

    pub fn projection_event_suffix_cancellable(
        &self,
        request: &EventSuffixRequest,
        cancellation: &ApplicationServiceRequestCancellation,
    ) -> Result<EventSuffix, String> {
        request
            .validate()
            .map_err(|error| format!("invalid projection suffix request: {error}"))?;
        if cancellation.is_cancelled() {
            return Err(APPLICATION_SERVICE_REQUEST_CANCELLED.to_string());
        }
        self.ensure_running()?;
        let arguments = serde_json::to_value(request)
            .map_err(|error| format!("encode projection suffix request failed: {error}"))?;
        let result = self.request_with_cancellation(
            CommandPayload {
                command_type: "projection_event_suffix".to_string(),
                arguments,
            },
            None,
            cancellation,
        )?;
        let suffix: EventSuffix = serde_json::from_value(result)
            .map_err(|error| format!("decode projection event suffix failed: {error}"))?;
        suffix
            .validate()
            .map_err(|error| format!("invalid projection event suffix: {error}"))?;
        Ok(suffix)
    }

    pub fn projection_snapshot(
        &self,
        request: &ProjectionSnapshotRequest,
    ) -> Result<ProjectionSnapshot, String> {
        request
            .validate()
            .map_err(|error| format!("invalid projection snapshot request: {error}"))?;
        self.ensure_running()?;
        let arguments = serde_json::to_value(request)
            .map_err(|error| format!("encode projection snapshot request failed: {error}"))?;
        let result = self.request(CommandPayload {
            command_type: "projection_snapshot".to_string(),
            arguments,
        })?;
        let snapshot: ProjectionSnapshot = serde_json::from_value(result)
            .map_err(|error| format!("decode projection snapshot failed: {error}"))?;
        snapshot
            .validate()
            .map_err(|error| format!("invalid projection snapshot: {error}"))?;
        Ok(snapshot)
    }

    pub fn assistant_journal_wait(
        &self, request: &nola_protocol::AssistantJournalWaitRequest,
        cancellation: &ApplicationServiceRequestCancellation,
    ) -> Result<nola_protocol::AssistantJournalCursor, String> {
        request.validate()?;
        if cancellation.is_cancelled() { return Err(APPLICATION_SERVICE_REQUEST_CANCELLED.into()); }
        self.ensure_running()?;
        let value = self.request_with_cancellation(CommandPayload {
            command_type: "assistant_journal_wait".into(),
            arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        }, Some(runtime_effect_work_transport_timeout(request.wait_ms)), cancellation)?;
        let head: nola_protocol::AssistantJournalCursor =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        if head.identity != request.after.identity || head.sequence < request.after.sequence
            || (head.sequence == request.after.sequence && head != request.after) {
            return Err("journal wait returned a different source cursor".into());
        }
        Ok(head)
    }

    pub fn runtime_effect_work_wait(
        &self,
        request: &RuntimeEffectWorkWaitRequest,
    ) -> Result<RuntimeEffectWorkSchedule, String> {
        request
            .validate()
            .map_err(|error| format!("invalid Runtime effect-work wait request: {error}"))?;
        self.ensure_running()?;
        let arguments = serde_json::to_value(request)
            .map_err(|error| format!("encode Runtime effect-work wait request failed: {error}"))?;
        let result = self.request_with_timeout(
            CommandPayload {
                command_type: "runtime_effect_work_wait".to_string(),
                arguments,
            },
            Some(runtime_effect_work_transport_timeout(request.wait_ms)),
        )?;
        decode_runtime_effect_work_schedule(result, request)
    }

    pub fn runtime_effect_work_wait_cancellable(
        &self,
        request: &RuntimeEffectWorkWaitRequest,
        cancellation: &ApplicationServiceRequestCancellation,
    ) -> Result<RuntimeEffectWorkSchedule, String> {
        request
            .validate()
            .map_err(|error| format!("invalid Runtime effect-work wait request: {error}"))?;
        if cancellation.is_cancelled() {
            return Err(APPLICATION_SERVICE_REQUEST_CANCELLED.to_string());
        }
        self.ensure_running()?;
        let arguments = serde_json::to_value(request)
            .map_err(|error| format!("encode Runtime effect-work wait request failed: {error}"))?;
        let result = self.request_with_cancellation(
            CommandPayload {
                command_type: "runtime_effect_work_wait".to_string(),
                arguments,
            },
            Some(runtime_effect_work_transport_timeout(request.wait_ms)),
            cancellation,
        )?;
        decode_runtime_effect_work_schedule(result, request)
    }

    pub fn execute<T, Command>(&self, command: Command) -> Result<T, String>
    where
        T: DeserializeOwned,
        Command: Serialize,
    {
        self.execute_with_request_id(next_request_id(), command)
    }

    pub fn execute_with_request_id<T, Command>(
        &self,
        request_id: impl Into<String>,
        command: Command,
    ) -> Result<T, String>
    where
        T: DeserializeOwned,
        Command: Serialize,
    {
        self.execute_typed_with_request_id(request_id, command)
            .map_err(|error| error.to_string())
    }

    pub fn execute_typed_with_request_id<T: DeserializeOwned, Command: Serialize>(
        &self,
        request_id: impl Into<String>,
        command: Command,
    ) -> Result<T, ApplicationServiceCommandError> {
        self.execute_typed_with_revision(request_id, None, command)
    }

    /// Transfers an admission exported by the original Application owner.
    /// Uses the existing trusted local Host transport; no automatic write
    /// retry or replacement request identity is permitted after lost ACK.
    /// Host-private model material. Never forward this result to a renderer.
    pub fn load_edited_resend_host_material<T: DeserializeOwned>(&self,request_id:&str,conversation_id:&str)
        -> Result<T,ApplicationServiceCommandError> {
        self.execute_typed_with_request_id(request_id,serde_json::json!({
            "type":"host_load_edited_resend_material","conversation_id":conversation_id,
        }))
    }

    /// Authenticated native/server Host factory only. Never proxy a renderer's
    /// actor, builtin definitions or command JSON through this operation.
    pub fn execute_host_expert_template_operation_with_request_id<T: DeserializeOwned, O: Serialize>(
        &self, request_id: &str, operation: &O,
    ) -> Result<T, ApplicationServiceCommandError> {
        self.execute_typed_with_request_id(request_id, serde_json::json!({
            "type":"host_expert_template_operation", "operation":operation,
        }))
    }

    /// Trusted Host reconstruction integrity, not a permission or execution grant.
    pub fn attest_edited_resend_host_policy<T: DeserializeOwned,C:Serialize,S:Serialize>(&self,request_id:&str,conversation_id:&str,core_config:&C,submission:&S)
        -> Result<T,ApplicationServiceCommandError> {
        self.execute_typed_with_request_id(request_id,serde_json::json!({
            "type":"host_attest_edited_resend_policy","conversation_id":conversation_id,"core_config":core_config,"submission":submission,
        }))
    }

    pub fn import_agent_task_admission_with_request_id(
        &self,
        request_id: &str,
        admission: &nola_protocol::AgentTaskAdmission,
    ) -> Result<nola_protocol::RuntimeAgentTaskAuthority, ApplicationServiceCommandError> {
        admission.validate().map_err(|message| {
            ApplicationServiceCommandError::transport(
                &self.client_id, request_id, message, false,
            )
        })?;
        let authority: nola_protocol::RuntimeAgentTaskAuthority = self.execute_typed_with_request_id(
            request_id,
            serde_json::json!({
                "type": "host_import_agent_task_admission",
                "admission": admission,
            }),
        )?;
        if authority.validate().is_err()
            || authority.task_id != admission.task_id
            || authority.phase != nola_protocol::RuntimeAgentTaskAuthorityPhase::Admitted
            || authority.admission.as_ref() != Some(admission)
        {
            // The write may have committed. A mismatched result cannot grant
            // execution or justify another import with a replacement identity.
            return Err(ApplicationServiceCommandError::transport(
                &self.client_id, request_id,
                "Task admission result differs from the exact imported source fact".into(), true,
            ));
        }
        Ok(authority)
    }

    pub fn execute_typed_with_revision<T: DeserializeOwned, Command: Serialize>(
        &self,
        request_id: impl Into<String>,
        expected_revision: Option<u64>,
        command: Command,
    ) -> Result<T, ApplicationServiceCommandError> {
        let request_id = request_id.into();
        let receipt = self.submit_typed_with_revision(&request_id, expected_revision, command)?;
        let value = receipt_result_typed(receipt)?;
        decode_committed_result(&self.client_id, &request_id, value)
    }

    pub fn import_agent_task_terminal_projection_with_request_id(
        &self,
        request_id: &str,
        projection: &nola_protocol::AgentTaskTerminalProjection,
    ) -> Result<nola_protocol::TaskTerminalReconciliationApplied, ApplicationServiceCommandError> {
        projection.validate().map_err(|message| ApplicationServiceCommandError::transport(
            &self.client_id, request_id, message, false,
        ))?;
        let ack: nola_protocol::TaskTerminalReconciliationApplied = self.execute_typed_with_request_id(
            request_id,
            serde_json::json!({"type":"host_import_agent_task_terminal_projection","projection":projection}),
        )?;
        if ack.conversation_id != projection.admission.parent_conversation_id
            || ack.task_id != projection.admission.task_id || ack.receipt_id != projection.receipt_id
            || ack.core_revision < projection.admission.core_revision {
            return Err(ApplicationServiceCommandError::transport(&self.client_id, request_id,
                "Task terminal acknowledgement belongs to another source or receipt".into(), true));
        }
        Ok(ack)
    }

    pub fn import_agent_task_guidance_projection_with_request_id(
        &self, request_id: &str, projection: &nola_protocol::AgentTaskGuidanceProjection,
    ) -> Result<nola_protocol::TaskGuidanceProjectionApplied, ApplicationServiceCommandError> {
        projection.validate().map_err(|message| ApplicationServiceCommandError::transport(
            &self.client_id, request_id, message, false,
        ))?;
        let ack: nola_protocol::TaskGuidanceProjectionApplied = self.execute_typed_with_request_id(
            request_id,
            serde_json::json!({"type":"host_import_agent_task_guidance_projection","projection":projection}),
        )?;
        if ack.conversation_id != projection.admission.parent_conversation_id
            || ack.task_id != projection.admission.task_id || ack.message_id != projection.guidance.message_id
            || ack.settlement != projection.settlement || ack.core_revision < projection.admission.core_revision {
            return Err(ApplicationServiceCommandError::transport(&self.client_id, request_id,
                "Task guidance acknowledgement belongs to another source or message".into(), true));
        }
        Ok(ack)
    }

    pub fn submit_with_request_id<Command>(
        &self,
        request_id: impl Into<String>,
        command: Command,
    ) -> Result<CommandReceipt, String>
    where
        Command: Serialize,
    {
        self.submit_with_revision(request_id, None, command)
    }

    pub fn execute_with_revision<T: DeserializeOwned, Command: Serialize>(
        &self,
        request_id: impl Into<String>,
        expected_revision: u64,
        command: Command,
    ) -> Result<T, String> {
        self.execute_typed_with_revision(request_id, Some(expected_revision), command)
            .map_err(|error| error.to_string())
    }

    /// Submit a write against its typed entity scope. Unsupported command
    /// scopes are rejected by Application. Retry with the same ID and revision.
    pub fn submit_with_revision<Command>(
        &self,
        request_id: impl Into<String>,
        expected_revision: Option<u64>,
        command: Command,
    ) -> Result<CommandReceipt, String>
    where
        Command: Serialize,
    {
        self.submit_typed_with_revision(request_id, expected_revision, command)
            .map_err(|error| error.to_string())
    }

    /// The caller owns this identity. An uncertain response is returned with
    /// that identity intact; this method never automatically resubmits a write.
    pub fn submit_typed_with_revision<Command: Serialize>(
        &self,
        request_id: impl Into<String>,
        expected_revision: Option<u64>,
        command: Command,
    ) -> Result<CommandReceipt, ApplicationServiceCommandError> {
        let request_id = request_id.into();
        let before_send = |message| {
            ApplicationServiceCommandError::transport(&self.client_id, &request_id, message, false)
        };
        let command = serde_json::to_value(command).map_err(|error| {
            before_send(format!(
                "encode application protocol command failed: {error}"
            ))
        })?;
        let payload = command_payload(command).map_err(&before_send)?;
        CommandEnvelope {
            protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
            request_id: request_id.clone(),
            client_id: self.client_id.clone(),
            expected_revision,
            command: payload.clone(),
        }
        .validate()
        .map_err(|error| before_send(format!("invalid application service command: {error}")))?;
        self.ensure_running().map_err(before_send)?;
        let receipt = self
            .receipt_with_id_and_timeout(request_id.clone(), payload, None, expected_revision)
            .map_err(|message| {
                let outcome_unknown = !is_pre_send_command_failure(&message);
                ApplicationServiceCommandError::transport(
                    &self.client_id,
                    &request_id,
                    message,
                    outcome_unknown,
                )
            })?;
        if let Some(error) = unknown_receipt_error(&receipt) {
            return Err(error);
        }
        Ok(receipt)
    }

    /// Reads the durable receipt using the original client and request identity.
    /// A missing receipt provides no terminal business outcome and is never
    /// permission to replace the ID or automatically send the command again.
    pub fn reconcile_command_receipt(
        &self,
        error: &ApplicationServiceCommandError,
    ) -> Result<Option<CommandReceipt>, ApplicationServiceCommandError> {
        if error.client_id() != self.client_id || !error.requires_reconciliation() {
            return Err(ApplicationServiceCommandError::transport(
                &self.client_id,
                error.request_id(),
                "command reconciliation requires this client's unresolved receipt identity".into(),
                false,
            ));
        }
        self.command_receipt(error.request_id()).map_err(|message| {
            ApplicationServiceCommandError::transport(
                &self.client_id,
                error.request_id(),
                format!("reconcile original command receipt failed: {message}"),
                true,
            )
        })
    }

    pub fn reconcile_command_result<T: DeserializeOwned>(
        &self,
        error: &ApplicationServiceCommandError,
    ) -> Result<T, ApplicationServiceCommandError> {
        let Some(receipt) = self.reconcile_command_receipt(error)? else {
            return Err(error.clone());
        };
        decode_committed_result(&self.client_id, error.request_id(), receipt_result_typed(receipt)?)
    }

    fn request(&self, command: CommandPayload) -> Result<Value, String> {
        self.request_with_timeout(command, None)
    }

    fn request_with_timeout(
        &self,
        command: CommandPayload,
        timeout: Option<Duration>,
    ) -> Result<Value, String> {
        let receipt =
            self.receipt_with_id_and_timeout(next_request_id(), command, timeout, None)?;
        receipt_result(receipt)
    }

    fn request_with_cancellation(
        &self,
        command: CommandPayload,
        timeout: Option<Duration>,
        cancellation: &ApplicationServiceRequestCancellation,
    ) -> Result<Value, String> {
        let receipt = self.receipt_with_id_and_timeout_and_cancellation(
            next_request_id(),
            command,
            timeout,
            None,
            Some(cancellation),
        )?;
        receipt_result(receipt)
    }

    fn receipt_with_id_and_timeout(
        &self,
        request_id: String,
        command: CommandPayload,
        timeout: Option<Duration>,
        expected_revision: Option<u64>,
    ) -> Result<CommandReceipt, String> {
        self.receipt_with_id_and_timeout_and_cancellation(
            request_id,
            command,
            timeout,
            expected_revision,
            None,
        )
    }

    fn receipt_with_id_and_timeout_and_cancellation(
        &self,
        request_id: String,
        command: CommandPayload,
        timeout: Option<Duration>,
        expected_revision: Option<u64>,
        cancellation: Option<&ApplicationServiceRequestCancellation>,
    ) -> Result<CommandReceipt, String> {
        let envelope = CommandEnvelope {
            protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
            request_id,
            client_id: self.client_id.clone(),
            expected_revision,
            command,
        };
        envelope
            .validate()
            .map_err(|error| format!("invalid application service command: {error}"))?;
        let expected_request_id = envelope.request_id.clone();
        let expected_client_id = envelope.client_id.clone();
        let transport_timeout = Some(timeout.unwrap_or(self.request_timeout));
        let send = || {
            platform::send_cancellable(
                &self.socket_path,
                &envelope,
                transport_timeout,
                cancellation.map(|cancellation| &cancellation.inner),
            )
        };
        // A parent-bound owner can exit between each health probe and connect.
        // Only a failed connect proves the envelope is still unsent. Retain its
        // identity across a bounded election retry; never replay write/read
        // failures or an outcome that may already have committed.
        let mut connect_recoveries = 0;
        let response = loop {
            match send() {
                Ok(response) => break response,
                Err(error) if envelope.command.command_type != "health"
                    && is_pre_send_connect_failure(&error) && connect_recoveries < 3 => {
                    connect_recoveries += 1;
                    self.ensure_running().map_err(|recovery| {
                        format!("{error}; service recovery before send failed: {recovery}")
                    })?;
                }
                Err(error) => return Err(error),
            }
        };
        response
            .validate()
            .map_err(|error| format!("invalid application service receipt: {error}"))?;
        if response.request_id != expected_request_id {
            return Err("application service response request_id mismatch".to_string());
        }
        if response.client_id != expected_client_id {
            return Err("application service response client_id mismatch".to_string());
        }
        Ok(response)
    }
}

fn is_pre_send_connect_failure(error: &str) -> bool {
    error.starts_with("connect Nola Application Service at ")
}

fn is_pre_send_command_failure(error: &str) -> bool {
    is_pre_send_connect_failure(error)
        || error.starts_with("invalid application service command: ")
        || error.starts_with("encode application service request failed: ")
        || error == "application service request exceeds the frame limit"
}

fn receipt_result(receipt: CommandReceipt) -> Result<Value, String> {
    receipt_result_typed(receipt).map_err(|error| error.to_string())
}

fn decode_runtime_effect_work_schedule(
    result: Value,
    request: &RuntimeEffectWorkWaitRequest,
) -> Result<RuntimeEffectWorkSchedule, String> {
    let schedule: RuntimeEffectWorkSchedule = serde_json::from_value(result)
        .map_err(|error| format!("decode Runtime effect-work schedule failed: {error}"))?;
    schedule
        .validate_for_request(request)
        .map_err(|error| format!("invalid Runtime effect-work schedule: {error}"))?;
    Ok(schedule)
}

fn command_payload(value: Value) -> Result<CommandPayload, String> {
    let Value::Object(mut fields) = value else {
        return Err("Application command must encode as a JSON object".to_string());
    };
    let command_type = fields
        .remove("type")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| "Application command is missing its type tag".to_string())?;
    Ok(CommandPayload {
        command_type,
        arguments: Value::Object(fields),
    })
}

fn next_request_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{timestamp}-{sequence}", std::process::id())
}

fn client_id_for_database(database_path: &Path) -> String {
    format!("nola-local-{:016x}", database_path_hash(database_path))
}

pub fn socket_path_for_database(database_path: &Path) -> PathBuf {
    // FNV-1a matches the native Mac client and avoids exposing database paths
    // in a world-readable temporary directory name.
    let hash = database_path_hash(database_path);
    // Unix-domain socket paths have a small fixed-size address field (104
    // bytes on macOS). Process-specific TMPDIR values can already exceed that
    // limit before the socket name is appended, so keep transport endpoints
    // in the system's short, shared temporary root. The database hash prevents
    // collisions and the service restricts the socket itself to mode 0600.
    #[cfg(unix)]
    let socket_root = PathBuf::from("/tmp");
    #[cfg(not(unix))]
    let socket_root = std::env::temp_dir();
    socket_root.join(format!(
        "nola-application-v{APPLICATION_SERVICE_PROTOCOL_VERSION}-{hash:016x}.sock"
    ))
}

fn database_path_hash(database_path: &Path) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in database_path.as_os_str().to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::Shutdown;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::process::{Child, ChildStderr, Command, Stdio};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant};

    static OWNED_SERVICES: OnceLock<Mutex<Vec<OwnedService>>> = OnceLock::new();
    static STARTUP_LOCKS: OnceLock<
        Mutex<std::collections::HashMap<PathBuf, std::sync::Weak<Mutex<()>>>>,
    > = OnceLock::new();

    #[derive(Default)]
    pub(super) struct RequestCancellation {
        cancelled: AtomicBool,
        active: Mutex<Option<UnixStream>>,
    }

    impl RequestCancellation {
        pub(super) fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
            let mut active = self
                .active
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(stream) = active.take() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }

        pub(super) fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Acquire)
        }

        fn register(
            self: &Arc<Self>,
            stream: &UnixStream,
        ) -> Result<RequestCancellationGuard, String> {
            let cancellable_stream = stream.try_clone().map_err(|error| {
                format!("clone application service request connection failed: {error}")
            })?;
            let mut active = self
                .active
                .lock()
                .map_err(|_| "application service request cancellation is poisoned".to_string())?;
            if self.is_cancelled() {
                let _ = cancellable_stream.shutdown(Shutdown::Both);
                return Err(APPLICATION_SERVICE_REQUEST_CANCELLED.to_string());
            }
            if active.is_some() {
                return Err("application service cancellation handle is already in use".to_string());
            }
            *active = Some(cancellable_stream);
            Ok(RequestCancellationGuard {
                cancellation: Arc::clone(self),
            })
        }
    }

    struct RequestCancellationGuard {
        cancellation: Arc<RequestCancellation>,
    }

    impl Drop for RequestCancellationGuard {
        fn drop(&mut self) {
            let mut active = self
                .cancellation
                .active
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            active.take();
        }
    }

    fn startup_lock(socket: &Path) -> Result<Arc<Mutex<()>>, String> {
        let mut locks = STARTUP_LOCKS
            .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
            .lock()
            .map_err(|_| "application service startup registry is poisoned".to_string())?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(socket).and_then(std::sync::Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(socket.to_path_buf(), Arc::downgrade(&lock));
        Ok(lock)
    }

    fn startup_process_lock(socket: &Path, timeout: Duration) -> Result<std::fs::File, String> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let lock_path = socket.with_extension("startup.lock");
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true)
            .mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&lock_path).map_err(|error| format!("open service startup lock failed: {error}"))?;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err("service startup lock is not a private file owned by the current user".into());
        }
        let deadline = Instant::now() + timeout;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(file);
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted { continue; }
            if error.kind() != std::io::ErrorKind::WouldBlock {
                return Err(format!("acquire service startup lock failed: {error}"));
            }
            if Instant::now() >= deadline {
                return Err("service startup coordination deadline expired".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        // Keep the inode; closing this non-inherited descriptor releases the
        // cross-process lock, including after a client crashes.
    }

    struct StderrCapture {
        tail: Arc<Mutex<Vec<u8>>>,
        cancelled: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }

    impl StderrCapture {
        fn start(mut stderr: ChildStderr) -> Result<Self, String> {
            let fd = stderr.as_raw_fd();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(format!(
                    "configure service stderr failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let tail = Arc::new(Mutex::new(Vec::new()));
            let cancelled = Arc::new(AtomicBool::new(false));
            let worker_tail = Arc::clone(&tail);
            let worker_cancelled = Arc::clone(&cancelled);
            let worker = thread::Builder::new()
                .name("nola-service-stderr".into())
                .spawn(move || {
                    let mut buffer = [0_u8; 4096];
                    while !worker_cancelled.load(Ordering::Acquire) {
                        match stderr.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(count) => {
                                let mut tail = worker_tail
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner());
                                let excess = (tail.len() + count).saturating_sub(16 * 1024);
                                tail.drain(..excess);
                                tail.extend_from_slice(&buffer[..count]);
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                continue
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(10));
                            }
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| format!("start service stderr reader failed: {error}"))?;
            Ok(Self {
                tail,
                cancelled,
                worker: Some(worker),
            })
        }

        fn detail(&self) -> String {
            let tail = self.tail.lock().unwrap_or_else(|error| error.into_inner());
            String::from_utf8_lossy(&tail).trim().to_string()
        }
    }

    impl Drop for StderrCapture {
        fn drop(&mut self) {
            self.cancelled.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    struct OwnedService {
        child: Child,
        stderr: StderrCapture,
        socket_path: PathBuf,
    }

    impl Drop for OwnedService {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn reap_finished_services(services: &mut Vec<OwnedService>) {
        // try_wait reaps exited children. An inspection failure is not proof of
        // exit, so keep ownership rather than killing a potentially live service.
        services.retain_mut(|service| !matches!(service.child.try_wait(), Ok(Some(_))));
    }

    struct DeadlineStream {
        stream: UnixStream,
        deadline: Option<Instant>,
    }

    impl DeadlineStream {
        fn io_error(stage: &str, error: std::io::Error) -> std::io::Error {
            std::io::Error::new(error.kind(), format!("{stage}: {error}"))
        }

        fn remaining(&self) -> std::io::Result<Option<Duration>> {
            self.deadline
                .map(|deadline| {
                    deadline
                        .checked_duration_since(Instant::now())
                        .filter(|remaining| !remaining.is_zero())
                        .ok_or_else(|| {
                            std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "application service response deadline exceeded",
                            )
                        })
                })
                .transpose()
        }
    }

    impl Read for DeadlineStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            use std::os::fd::AsRawFd;
            if buffer.is_empty() {
                return Ok(0);
            }
            loop {
                self.remaining()?;
                // Darwin can reject SO_RCVTIMEO after a peer closes, even when
                // its complete response is still queued. Receive queued bytes
                // without modifying socket options; only wait when necessary.
                let received = unsafe {
                    libc::recv(
                        self.stream.as_raw_fd(),
                        buffer.as_mut_ptr().cast(),
                        buffer.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                if received >= 0 {
                    return Ok(received as usize);
                }
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                if error.kind() != std::io::ErrorKind::WouldBlock {
                    return Err(Self::io_error("receive response bytes", error));
                }
                let timeout = self
                    .remaining()?
                    .map(|remaining| {
                        remaining
                            .as_millis()
                            .saturating_add(1)
                            .min(i32::MAX as u128) as i32
                    })
                    .unwrap_or(-1);
                let mut descriptor = libc::pollfd {
                    fd: self.stream.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let ready = unsafe { libc::poll(&mut descriptor, 1, timeout) };
                if ready < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::Interrupted {
                        return Err(Self::io_error("receive response bytes", error));
                    }
                }
                if ready == 0 && self.deadline.is_some() {
                    return Err(Self::io_error(
                        "receive response bytes",
                        std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "application service response deadline exceeded",
                        ),
                    ));
                }
            }
        }
    }

    impl Write for DeadlineStream {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.stream
                .set_write_timeout(self.remaining()?)
                .map_err(|error| Self::io_error("configure request write timeout", error))?;
            self.stream
                .write(buffer)
                .map_err(|error| Self::io_error("send request bytes", error))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            // UnixStream is unbuffered. Updating SO_SNDTIMEO here can fail on
            // Darwin after a fast peer replied and closed, discarding a valid
            // queued response and incorrectly triggering service recovery.
            self.remaining()?;
            self.stream
                .flush()
                .map_err(|error| Self::io_error("flush request bytes", error))
        }
    }

    fn wait_for_socket_writable(
        descriptor: std::os::fd::RawFd,
        deadline: Option<Instant>,
        cancellation: Option<&RequestCancellation>,
    ) -> std::io::Result<()> {
        let timed_out = || {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "application service connection deadline exceeded",
            )
        };
        loop {
            if cancellation.is_some_and(RequestCancellation::is_cancelled) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    APPLICATION_SERVICE_REQUEST_CANCELLED,
                ));
            }
            let poll_budget = deadline
                .map(|deadline| {
                    deadline
                        .checked_duration_since(Instant::now())
                        .filter(|remaining| !remaining.is_zero())
                        .ok_or_else(timed_out)
                })
                .transpose()?
                .unwrap_or(Duration::from_millis(25))
                .min(Duration::from_millis(25));
            let milliseconds = poll_budget
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32;
            let mut poll_descriptor = libc::pollfd {
                fd: descriptor,
                events: libc::POLLOUT,
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut poll_descriptor, 1, milliseconds) };
            if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if result > 0 {
                return Ok(());
            }
        }
    }

    fn connect_with_deadline(
        path: &Path,
        deadline: Option<Instant>,
        cancellation: Option<&RequestCancellation>,
    ) -> std::io::Result<UnixStream> {
        if deadline.is_none() && cancellation.is_none() {
            return UnixStream::connect(path);
        }
        let address = socket2::SockAddr::unix(path)?;
        let timed_out = || {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "application service connection deadline exceeded",
            )
        };
        let cancelled = || {
            std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                APPLICATION_SERVICE_REQUEST_CANCELLED,
            )
        };
        let check_cancelled = || {
            if cancellation.is_some_and(RequestCancellation::is_cancelled) {
                Err(cancelled())
            } else {
                Ok(())
            }
        };
        let remaining = || {
            deadline
                .map(|deadline| {
                    deadline
                        .checked_duration_since(Instant::now())
                        .filter(|remaining| !remaining.is_zero())
                        .ok_or_else(timed_out)
                })
                .transpose()
        };
        loop {
            check_cancelled()?;
            remaining()?;
            let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
            socket.set_nonblocking(true)?;
            match socket.connect(&address) {
                Ok(()) => {
                    socket.set_nonblocking(false)?;
                    return Ok(UnixStream::from(std::os::fd::OwnedFd::from(socket)));
                }
                Err(error) if error.raw_os_error() == Some(libc::EINPROGRESS) => {
                    wait_for_socket_writable(socket.as_raw_fd(), deadline, cancellation)?;
                    if let Some(error) = socket.take_error()? {
                        return Err(error);
                    }
                    socket.set_nonblocking(false)?;
                    return Ok(UnixStream::from(std::os::fd::OwnedFd::from(socket)));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    // Linux may reject a nonblocking AF_UNIX connect with EAGAIN
                    // when the accept queue is full; that socket is not connected.
                    drop(socket);
                    let retry_budget = remaining()?
                        .unwrap_or(Duration::from_millis(10))
                        .min(Duration::from_millis(10));
                    thread::sleep(retry_budget);
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(super) fn send_cancellable(
        socket_path: &Path,
        envelope: &CommandEnvelope,
        timeout: Option<Duration>,
        cancellation: Option<&Arc<RequestCancellation>>,
    ) -> Result<CommandReceipt, String> {
        if cancellation.is_some_and(|cancellation| cancellation.is_cancelled()) {
            return Err(APPLICATION_SERVICE_REQUEST_CANCELLED.to_string());
        }
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let stream = connect_with_deadline(socket_path, deadline, cancellation.map(Arc::as_ref))
            .map_err(|error| {
                if cancellation.is_some_and(|cancellation| cancellation.is_cancelled()) {
                    APPLICATION_SERVICE_REQUEST_CANCELLED.to_string()
                } else {
                    format!(
                        "connect Nola Application Service at {} failed: {error}",
                        socket_path.display()
                    )
                }
            })?;
        let _cancellation_guard = cancellation
            .map(|cancellation| cancellation.register(&stream))
            .transpose()?;
        let mut stream = DeadlineStream { stream, deadline };
        let mut frame = serde_json::to_vec(envelope)
            .map_err(|error| format!("encode application service request failed: {error}"))?;
        if frame.len() + 1 > APPLICATION_SERVICE_MAX_FRAME_BYTES {
            return Err("application service request exceeds the frame limit".to_string());
        }
        frame.push(b'\n');
        stream
            .write_all(&frame)
            .and_then(|_| stream.flush())
            .map_err(|error| format!("write application service request failed: {error}"))?;

        let mut response = Vec::new();
        let read_result = BufReader::new(stream)
            .take((APPLICATION_SERVICE_MAX_FRAME_BYTES + 1) as u64)
            .read_until(b'\n', &mut response);
        if cancellation.is_some_and(|cancellation| cancellation.is_cancelled()) {
            return Err(APPLICATION_SERVICE_REQUEST_CANCELLED.to_string());
        }
        read_result
            .map_err(|error| format!("read application service response failed: {error}"))?;
        if response.is_empty() {
            return Err("application service closed without a response".to_string());
        }
        if response.len() > APPLICATION_SERVICE_MAX_FRAME_BYTES {
            return Err("application service response exceeds the frame limit".to_string());
        }
        serde_json::from_slice(&response)
            .map_err(|error| format!("decode application service response failed: {error}"))
    }

    pub(super) fn ensure_running(client: &ApplicationServiceClient) -> Result<(), String> {
        {
            let mut services = OWNED_SERVICES
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .map_err(|_| "application service process registry is poisoned".to_string())?;
            reap_finished_services(&mut services);
        }
        let mut last_health_error = match client.health() {
            Ok(_) => return Ok(()),
            Err(error) => error,
        };
        let startup = startup_lock(client.socket_path())?;
        let _startup_guard = startup
            .lock()
            .map_err(|_| "application service startup lock is poisoned".to_string())?;
        // The registry mutex coordinates threads only. Independent CLI/native
        // clients share this kernel lock while electing the database owner.
        let _process_startup_guard = startup_process_lock(client.socket_path(), Duration::from_secs(5))?;
        // Another caller may have completed startup while we waited. Never
        // spawn based on the health result observed before taking this lock.
        if client.health().is_ok() {
            return Ok(());
        }
        {
            let mut services = OWNED_SERVICES
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .map_err(|_| "application service process registry is poisoned".to_string())?;
            reap_finished_services(&mut services);
            if services.iter_mut().any(|service| {
                service.socket_path == client.socket_path()
                    && matches!(service.child.try_wait(), Ok(None))
            }) {
                return Err(format!("owned Application Service is alive but not responding; no replacement was launched: {last_health_error}"));
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        'startup: for attempt in 0..2 {
            let (executable, embedded_entry) = service_executable()?;
            let mut command = Command::new(&executable);
            if embedded_entry {
                command.arg("--nola-application-service");
            }
            let mut child = command
                .arg("--database")
                .arg(client.database_path())
                .arg("--socket")
                .arg(client.socket_path())
                .arg("--parent-pid")
                .arg(std::process::id().to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| format!("launch {} failed: {error}", executable.display()))?;

            let stderr = match StderrCapture::start(child.stderr.take().expect("piped service stderr"))
            {
                Ok(stderr) => stderr,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            };
            let mut service = OwnedService {
                child,
                stderr,
                socket_path: client.socket_path().to_path_buf(),
            };
            while Instant::now() < deadline {
                if let Some(status) = service
                    .child
                    .try_wait()
                    .map_err(|error| format!("inspect application service process failed: {error}"))?
                {
                    let detail = service.stderr.detail();
                    // Another parent-bound owner can be finishing shutdown after
                    // unlinking its socket. No application envelope was sent to
                    // this failed child. Observe a winning owner first, then allow
                    // one bounded relaunch of the SAME service executable. Never
                    // replay a request whose transport may have sent bytes.
                    let election_deadline = (Instant::now() + Duration::from_millis(250)).min(deadline);
                    while Instant::now() < election_deadline {
                        match client.health() {
                            Ok(_) => return Ok(()),
                            Err(error) => last_health_error = error,
                        }
                        thread::sleep(Duration::from_millis(25));
                    }
                    if attempt == 0 && Instant::now() < deadline {
                        continue 'startup;
                    }
                    return Err(format!(
                        "Nola Application Service exited with {status}: {}; last health probe: {}",
                        detail.trim(),
                        last_health_error
                    ));
                }
                match client.health() {
                    Ok(_) => {
                        OWNED_SERVICES
                            .get_or_init(|| Mutex::new(Vec::new()))
                            .lock()
                            .map_err(|_| {
                                "application service process registry is poisoned".to_string()
                            })?
                            .push(service);
                        return Ok(());
                    }
                    Err(error) => last_health_error = error,
                }
                thread::sleep(Duration::from_millis(25));
            }
            let _ = service.child.kill();
            let _ = service.child.wait();
            let detail = service.stderr.detail();
            return Err(format!(
                "Nola Application Service did not become ready at {}: {}; last health probe: {}",
                client.socket_path().display(),
                detail.trim(),
                last_health_error
            ));
        }
        Err("Application Service startup attempts exhausted".into())
    }

    #[cfg(test)]
    mod stderr_tests {
        use super::*;

        #[test]
        fn process_startup_lock_excludes_other_processes_and_releases_on_exit() {
            let socket = PathBuf::from(format!("/tmp/nola-startup-process-{}.sock", next_request_id()));
            let lock_path = socket.with_extension("startup.lock");
            let held = startup_process_lock(&socket, Duration::from_millis(50)).unwrap();
            let script = "import fcntl,sys; f=open(sys.argv[1], 'r+'); fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)";
            let blocked = Command::new("python3").args(["-c", script]).arg(&lock_path)
                .stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
            assert!(!blocked.success(), "a second process acquired an occupied startup lock");
            assert!(startup_process_lock(&socket, Duration::from_millis(20)).is_err());
            drop(held);
            let acquired = Command::new("python3").args(["-c", script]).arg(&lock_path)
                .stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
            assert!(acquired.success(), "descriptor close did not release the startup lock");
            drop(startup_process_lock(&socket, Duration::from_millis(50)).unwrap());
            std::fs::remove_file(lock_path).unwrap();
        }

        #[test]
        fn startup_lock_rejects_symlinks_and_non_private_files() {
            use std::os::unix::fs::{symlink, PermissionsExt};
            let socket = PathBuf::from(format!("/tmp/nola-startup-private-{}.sock", next_request_id()));
            let lock_path = socket.with_extension("startup.lock");
            let target = lock_path.with_extension("target");
            std::fs::write(&target, b"preserve").unwrap();
            symlink(&target, &lock_path).unwrap();
            assert!(startup_process_lock(&socket, Duration::from_millis(10)).is_err());
            assert_eq!(std::fs::read(&target).unwrap(), b"preserve");
            std::fs::remove_file(&lock_path).unwrap();
            std::fs::write(&lock_path, b"").unwrap();
            std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(startup_process_lock(&socket, Duration::from_millis(10)).is_err());
            std::fs::remove_file(lock_path).unwrap();
            std::fs::remove_file(target).unwrap();
        }

        fn health_envelope() -> CommandEnvelope {
            CommandEnvelope {
                protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
                request_id: next_request_id(),
                client_id: "cancellable-request-test".into(),
                expected_revision: None,
                command: CommandPayload {
                    command_type: "health".into(),
                    arguments: serde_json::json!({}),
                },
            }
        }

        #[test]
        fn cancellation_shuts_down_a_silent_request_connection() {
            use std::os::unix::net::UnixListener;
            use std::sync::mpsc;

            let path = PathBuf::from(format!(
                "/tmp/nola-cancellable-request-{}.sock",
                next_request_id()
            ));
            let listener = UnixListener::bind(&path).unwrap();
            let (request_seen_tx, request_seen_rx) = mpsc::channel();
            let (connection_closed_tx, connection_closed_rx) = mpsc::channel();
            let server = thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert!(!request.is_empty());
                request_seen_tx.send(()).unwrap();

                let mut stream = reader.into_inner();
                let mut byte = [0_u8; 1];
                assert_eq!(stream.read(&mut byte).unwrap(), 0);
                connection_closed_tx.send(()).unwrap();
            });

            let cancellation = Arc::new(RequestCancellation::default());
            let client_cancellation = Arc::clone(&cancellation);
            let client_path = path.clone();
            let client = thread::spawn(move || {
                send_cancellable(
                    &client_path,
                    &health_envelope(),
                    Some(Duration::from_secs(30)),
                    Some(&client_cancellation),
                )
            });

            request_seen_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            let started = Instant::now();
            cancellation.cancel();
            let error = client.join().unwrap().unwrap_err();
            let elapsed = started.elapsed();
            connection_closed_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
            server.join().unwrap();
            let _ = std::fs::remove_file(path);

            assert_eq!(error, APPLICATION_SERVICE_REQUEST_CANCELLED);
            assert!(
                elapsed < Duration::from_millis(500),
                "cancellation took {elapsed:?}"
            );
        }

        #[test]
        fn cancellation_before_connect_does_not_touch_transport() {
            let cancellation = Arc::new(RequestCancellation::default());
            cancellation.cancel();
            let started = Instant::now();
            let error = send_cancellable(
                Path::new("/tmp/nola-cancelled-request-must-not-connect.sock"),
                &health_envelope(),
                Some(Duration::from_secs(30)),
                Some(&cancellation),
            )
            .unwrap_err();
            assert_eq!(error, APPLICATION_SERVICE_REQUEST_CANCELLED);
            assert!(started.elapsed() < Duration::from_millis(100));
        }

        #[test]
        fn completed_response_remains_readable_after_peer_disconnects() {
            for _ in 0..100 {
                let (socket, mut peer) = UnixStream::pair().unwrap();
                peer.write_all(b"complete response").unwrap();
                drop(peer);
                let mut stream = DeadlineStream {
                    stream: socket,
                    deadline: Some(Instant::now() + Duration::from_secs(1)),
                };
                stream.flush().unwrap();
                let mut text = String::new();
                stream.read_to_string(&mut text).unwrap();
                assert_eq!(text, "complete response");
            }
        }

        #[test]
        fn expired_budget_rejects_io_before_touching_the_socket() {
            let (stream, _peer) = UnixStream::pair().unwrap();
            let mut stream = DeadlineStream {
                stream,
                deadline: Some(Instant::now()),
            };
            let mut byte = [0];
            for error in [
                stream.read(&mut byte).unwrap_err(),
                stream.write(b"request").unwrap_err(),
                stream.flush().unwrap_err(),
            ] {
                assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
                assert!(error.to_string().contains("response deadline exceeded"));
            }
        }

        #[test]
        fn silent_peer_failure_identifies_receive_phase() {
            let (stream, _peer) = UnixStream::pair().unwrap();
            let mut stream = DeadlineStream {
                stream,
                deadline: Some(Instant::now() + Duration::from_millis(50)),
            };
            let error = stream.read(&mut [0]).unwrap_err();
            assert!(matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ));
            assert!(
                error.to_string().contains("receive response bytes"),
                "{error}"
            );
        }

        #[test]
        fn io_diagnostics_preserve_error_kind_and_operating_system_code() {
            let error = DeadlineStream::io_error(
                "configure response read timeout",
                std::io::Error::from_raw_os_error(libc::EINVAL),
            );
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
            assert!(error
                .to_string()
                .contains("configure response read timeout"));
            assert!(error
                .to_string()
                .contains(&format!("os error {}", libc::EINVAL)));
        }

        #[test]
        fn startup_locks_are_shared_only_for_the_same_socket() {
            let first = startup_lock(Path::new("/tmp/startup-lock-a.sock")).unwrap();
            let same = startup_lock(Path::new("/tmp/startup-lock-a.sock")).unwrap();
            let other = startup_lock(Path::new("/tmp/startup-lock-b.sock")).unwrap();
            assert!(Arc::ptr_eq(&first, &same));
            assert!(!Arc::ptr_eq(&first, &other));
            let _held = first.lock().unwrap();
            assert!(same.try_lock().is_err());
            assert!(other.try_lock().is_ok());
        }

        #[test]
        fn registry_reaps_exited_services_without_stopping_live_ones() {
            let spawn = |script: &str| {
                let mut child = Command::new("/bin/sh")
                    .arg("-c")
                    .arg(script)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                let stderr = StderrCapture::start(child.stderr.take().unwrap()).unwrap();
                OwnedService {
                    child,
                    stderr,
                    socket_path: PathBuf::from("/tmp/registry-fixture.sock"),
                }
            };
            let mut services = vec![spawn("exec sleep 10")];
            let live_id = services[0].child.id();
            for _ in 0..8 {
                services.push(spawn("printf exited >&2; exit 0"));
                let deadline = Instant::now() + Duration::from_secs(2);
                while services
                    .last_mut()
                    .unwrap()
                    .child
                    .try_wait()
                    .unwrap()
                    .is_none()
                {
                    assert!(Instant::now() < deadline, "fixture did not exit");
                    thread::sleep(Duration::from_millis(5));
                }
                reap_finished_services(&mut services);
                assert_eq!(
                    services.len(),
                    1,
                    "dead entries accumulated across restarts"
                );
                assert_eq!(services[0].child.id(), live_id);
                assert!(services[0].child.try_wait().unwrap().is_none());
            }
            // OwnedService cleanup terminates the remaining fixture on both
            // success and assertion failure, rather than waiting ten seconds.
        }

        #[test]
        fn expired_connection_budget_is_rejected_before_connecting() {
            let error = connect_with_deadline(
                Path::new("/tmp/nola-expired-connect.sock"),
                Some(Instant::now()),
                None,
            )
            .unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        }

        #[test]
        fn full_accept_queue_cannot_block_connection_indefinitely() {
            let path = PathBuf::from(format!("/tmp/nola-queue-{}.sock", next_request_id()));
            let address = socket2::SockAddr::unix(&path).unwrap();
            let listener =
                socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
            listener.bind(&address).unwrap();
            listener.listen(1).unwrap();
            let mut queued = Vec::new();
            let mut saturated = false;
            for _ in 0..128 {
                let socket =
                    socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)
                        .unwrap();
                socket.set_nonblocking(true).unwrap();
                let result = socket.connect(&address);
                queued.push(socket);
                if result.is_err() {
                    saturated = true;
                    break;
                }
            }
            let start = Instant::now();
            let result =
                connect_with_deadline(&path, Some(start + Duration::from_millis(250)), None);
            let elapsed = start.elapsed();
            drop(queued);
            drop(listener);
            let _ = std::fs::remove_file(path);
            assert!(saturated, "fixture did not saturate the accept queue");
            assert!(
                result.is_err(),
                "connection unexpectedly bypassed the full accept queue"
            );
            assert!(
                elapsed < Duration::from_millis(800),
                "connect took {elapsed:?}"
            );
        }

        #[test]
        fn cancellation_interrupts_the_nonblocking_connect_poll() {
            use std::sync::mpsc;

            let (mut blocked_socket, peer) = UnixStream::pair().unwrap();
            blocked_socket.set_nonblocking(true).unwrap();
            let buffer = [0_u8; 16 * 1024];
            loop {
                match blocked_socket.write(&buffer) {
                    Ok(_) => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("fill socket send buffer failed: {error}"),
                }
            }

            let cancellation = Arc::new(RequestCancellation::default());
            let worker_cancellation = Arc::clone(&cancellation);
            let (done_tx, done_rx) = mpsc::channel();
            let worker = thread::spawn(move || {
                let result = wait_for_socket_writable(
                    blocked_socket.as_raw_fd(),
                    Some(Instant::now() + Duration::from_secs(30)),
                    Some(&worker_cancellation),
                );
                done_tx
                    .send(result.map_err(|error| error.to_string()))
                    .unwrap();
            });
            thread::sleep(Duration::from_millis(75));
            match done_rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {}
                result => panic!("connect completed before cancellation: {result:?}"),
            }

            let started = Instant::now();
            cancellation.cancel();
            let error = done_rx
                .recv_timeout(Duration::from_millis(500))
                .expect("connect did not observe cancellation")
                .unwrap_err();
            let elapsed = started.elapsed();
            worker.join().unwrap();
            drop(peer);

            assert_eq!(error, APPLICATION_SERVICE_REQUEST_CANCELLED);
            assert!(
                elapsed < Duration::from_millis(100),
                "cancellation took {elapsed:?}"
            );
        }

        #[test]
        fn verbose_child_does_not_fill_pipe_and_diagnostics_are_bounded() {
            let mut child = Command::new("/bin/sh")
                .arg("-c")
                .arg("dd if=/dev/zero bs=4096 count=128 >&2 2>/dev/null; printf final-marker >&2")
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let stderr = StderrCapture::start(child.stderr.take().unwrap()).unwrap();
            let mut service = OwnedService {
                child,
                stderr,
                socket_path: PathBuf::from("/tmp/stderr-fixture.sock"),
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = service.child.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                assert!(Instant::now() < deadline, "child blocked on stderr");
                thread::sleep(Duration::from_millis(10));
            }
            while !service.stderr.detail().ends_with("final-marker") && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            let detail = service.stderr.detail();
            assert!(detail.ends_with("final-marker"));
            assert!(detail.len() <= 16 * 1024);
        }

        #[test]
        fn capture_shutdown_does_not_wait_for_pipe_eof() {
            let mut child = Command::new("/bin/sleep")
                .arg("2")
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let capture = StderrCapture::start(child.stderr.take().unwrap()).unwrap();
            let started = Instant::now();
            drop(capture);
            let elapsed = started.elapsed();
            let _ = child.kill();
            let _ = child.wait();
            assert!(
                elapsed < Duration::from_millis(500),
                "shutdown took {elapsed:?}"
            );
        }
    }

    fn service_executable() -> Result<(PathBuf, bool), String> {
        if let Some(path) = std::env::var_os("NOLA_APPLICATION_SERVICE_EXECUTABLE") {
            let path = PathBuf::from(path);
            return Ok((path.clone(), is_embedded_service_host(&path)));
        }
        if let Ok(current) = std::env::current_exe() {
            if is_embedded_service_host(&current) {
                return Ok((current, true));
            }
            if let Some(parent) = current.parent() {
                let sibling = parent.join("nola-service");
                if sibling.is_file() {
                    return Ok((sibling, false));
                }
                if parent.file_name().is_some_and(|name| name == "deps") {
                    if let Some(target_directory) = parent.parent() {
                        let sibling = target_directory.join("nola-service");
                        if sibling.is_file() {
                            return Ok((sibling, false));
                        }
                    }
                }
            }
        }
        Ok((PathBuf::from("nola-service"), false))
    }

    fn is_embedded_service_host(path: &Path) -> bool {
        path.file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value == "nola" || value == "nola-agent-host")
    }
}

#[cfg(not(unix))]
mod platform {
    use super::*;

    #[derive(Default)]
    pub(super) struct RequestCancellation {
        cancelled: std::sync::atomic::AtomicBool,
    }

    impl RequestCancellation {
        pub(super) fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
        }

        pub(super) fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Acquire)
        }
    }

    pub(super) fn send_cancellable(
        _socket_path: &Path,
        _envelope: &CommandEnvelope,
        _timeout: Option<Duration>,
        cancellation: Option<&Arc<RequestCancellation>>,
    ) -> Result<CommandReceipt, String> {
        if cancellation.is_some_and(|cancellation| cancellation.is_cancelled()) {
            return Err(APPLICATION_SERVICE_REQUEST_CANCELLED.to_string());
        }
        Err("Nola Application Service transport is not implemented on this platform".to_string())
    }

    pub(super) fn ensure_running(_client: &ApplicationServiceClient) -> Result<(), String> {
        Err("Nola Application Service lifecycle is not implemented on this platform".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pre_send_connect_failures_are_safe_for_automatic_reconnect() {
        assert!(is_pre_send_connect_failure(
            "connect Nola Application Service at /tmp/nola.sock failed: No such file"
        ));
        assert!(!is_pre_send_connect_failure(
            "write application service request failed: Broken pipe"
        ));
        assert!(!is_pre_send_connect_failure(
            "read application service response failed: Connection reset"
        ));
        assert!(is_pre_send_command_failure(
            "application service request exceeds the frame limit"
        ));
        assert!(!is_pre_send_command_failure(
            "write application service request failed: Broken pipe"
        ));
        assert!(!is_pre_send_command_failure(
            "decode application service response failed: Invalid JSON"
        ));
    }

    #[cfg(unix)]
    fn runtime_wait_from_fixture(
        schedule: Value,
    ) -> (
        Result<RuntimeEffectWorkSchedule, String>,
        Vec<CommandEnvelope>,
    ) {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let socket = PathBuf::from(format!(
            "/tmp/nola-runtime-wait-client-{}.sock",
            next_request_id()
        ));
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for result in [serde_json::json!({"service":"fixture"}), schedule] {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut frame = String::new();
                reader.read_line(&mut frame).unwrap();
                let envelope: CommandEnvelope = serde_json::from_str(&frame).unwrap();
                let receipt = CommandReceipt {
                    protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
                    request_id: envelope.request_id.clone(),
                    client_id: envelope.client_id.clone(),
                    received_at: "2026-09-09T00:00:00Z".into(),
                    outcome: CommandReceiptOutcome::Committed {
                        completed_at: "2026-09-09T00:00:00Z".into(),
                        committed_revision: None,
                        result,
                    },
                };
                let mut stream = reader.into_inner();
                serde_json::to_writer(&mut stream, &receipt).unwrap();
                stream.write_all(b"\n").unwrap();
                requests.push(envelope);
            }
            requests
        });
        let client =
            ApplicationServiceClient::with_socket("/tmp/nola-runtime-wait-client.db", &socket);
        let request = RuntimeEffectWorkWaitRequest {
            conversation_ids: vec!["conversation-a".into()],
            limit: 2,
            wait_ms: 30_000,
        };
        let cancellation = ApplicationServiceRequestCancellation::new();
        let result = client.runtime_effect_work_wait_cancellable(&request, &cancellation);
        let requests = server.join().unwrap();
        let _ = std::fs::remove_file(socket);
        (result, requests)
    }

    #[cfg(unix)]
    #[test]
    fn cancellable_runtime_work_wait_decodes_a_valid_service_schedule() {
        let (schedule, requests) = runtime_wait_from_fixture(serde_json::json!({
            "observed_at_ms": 100,
            "runs": [],
            "next_wake_at_ms": 200
        }));
        let schedule = schedule.unwrap();
        assert_eq!(schedule.observed_at_ms, 100);
        assert_eq!(schedule.next_wake_at_ms, Some(200));
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].command.command_type, "health");
        assert_eq!(requests[1].command.command_type, "runtime_effect_work_wait");
        assert_eq!(
            requests[1].command.arguments,
            serde_json::json!({
                "conversation_ids": ["conversation-a"],
                "limit": 2,
                "wait_ms": 30_000
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn cancellable_runtime_work_wait_rejects_an_invalid_service_schedule() {
        let (result, _) = runtime_wait_from_fixture(serde_json::json!({
            "observed_at_ms": -1,
            "runs": [],
            "next_wake_at_ms": null
        }));
        let error = result.unwrap_err();
        assert!(
            error.starts_with("invalid Runtime effect-work schedule:"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn runtime_work_wait_rejects_a_service_schedule_outside_the_requested_scope() {
        let (result, _) = runtime_wait_from_fixture(serde_json::json!({
            "observed_at_ms": 100,
            "runs": [{
                "conversation_id": "conversation-outside-scope",
                "run_id": "run-a",
                "effect_id": "effect-a",
                "request_id": "request-a",
                "attempt_id": null,
                "reason": "pending",
                "ready_at_ms": 100,
                "created_at_ms": 100
            }],
            "next_wake_at_ms": null
        }));
        let error = result.unwrap_err();
        assert!(
            error.starts_with("invalid Runtime effect-work schedule:"),
            "{error}"
        );
        assert!(
            error.contains("outside the requested conversation scope"),
            "{error}"
        );
    }

    #[test]
    fn runtime_work_wait_rejects_an_invalid_request_before_transport() {
        let client = ApplicationServiceClient::with_socket(
            "/tmp/nola-invalid-runtime-wait.db",
            "/tmp/nola-invalid-runtime-wait.sock",
        );
        let request = RuntimeEffectWorkWaitRequest {
            conversation_ids: Vec::new(),
            limit: 1,
            wait_ms: 30_000,
        };
        let error = client.runtime_effect_work_wait(&request).unwrap_err();
        assert!(
            error.starts_with("invalid Runtime effect-work wait request:"),
            "{error}"
        );
    }

    #[test]
    fn runtime_work_wait_transport_deadline_includes_bounded_service_grace() {
        assert_eq!(
            runtime_effect_work_transport_timeout(30_000),
            Duration::from_secs(31)
        );
        assert!(runtime_effect_work_transport_timeout(0) > Duration::ZERO);
    }

    #[test]
    fn accepted_receipt_preserves_request_identity_without_claiming_rejection() {
        let receipt = CommandReceipt {
            protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
            request_id: "pending-request".into(),
            client_id: "pending-client".into(),
            received_at: "2026-09-06T00:00:00Z".into(),
            outcome: CommandReceiptOutcome::Accepted,
        };
        let error = receipt_result(receipt).unwrap_err();
        assert!(error.starts_with("command_outcome_pending:"));
        assert!(error.contains("pending-request"));
        assert!(error.contains("pending-client"));
    }

    #[cfg(unix)]
    fn reconcile_from_fixture(
        initial: Option<CommandReceiptOutcome>,
        stored: Option<CommandReceiptOutcome>,
    ) -> (
        Result<Value, ApplicationServiceCommandError>,
        Vec<CommandEnvelope>,
    ) {
        reconcile_from_fixture_command(initial, stored, None)
    }

    #[cfg(unix)]
    fn reconcile_from_fixture_command(
        initial: Option<CommandReceiptOutcome>,
        stored: Option<CommandReceiptOutcome>,
        admission: Option<&nola_protocol::AgentTaskAdmission>,
    ) -> (Result<Value, ApplicationServiceCommandError>, Vec<CommandEnvelope>) {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let socket = PathBuf::from(format!(
            "/tmp/nola-command-reconcile-{}.sock",
            next_request_id()
        ));
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for index in 0..4 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut frame = String::new();
                reader.read_line(&mut frame).unwrap();
                let envelope: CommandEnvelope = serde_json::from_str(&frame).unwrap();
                let outcome = match index {
                    1 => initial.clone(),
                    3 => Some(CommandReceiptOutcome::Committed {
                        completed_at: "2".into(),
                        committed_revision: None,
                        result: serde_json::to_value(stored.clone().map(|outcome| {
                            CommandReceipt {
                                protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
                                client_id: "typed-client".into(),
                                request_id: "original-write".into(),
                                received_at: "1".into(),
                                outcome,
                            }
                        }))
                        .unwrap(),
                    }),
                    _ => Some(CommandReceiptOutcome::Committed {
                        completed_at: "2".into(),
                        committed_revision: None,
                        result: serde_json::json!({"service":"fixture"}),
                    }),
                };
                if let Some(outcome) = outcome {
                    let receipt = CommandReceipt {
                        protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
                        request_id: envelope.request_id.clone(),
                        client_id: envelope.client_id.clone(),
                        received_at: "1".into(),
                        outcome,
                    };
                    let mut stream = reader.into_inner();
                    serde_json::to_writer(&mut stream, &receipt).unwrap();
                    stream.write_all(b"\n").unwrap();
                }
                requests.push(envelope);
            }
            requests
        });
        let client = ApplicationServiceClient::with_identity(
            "/tmp/nola-command-reconcile.db",
            &socket,
            "typed-client",
        )
        .unwrap();
        let error = match admission {
            Some(admission) => client
                .import_agent_task_admission_with_request_id("original-write", admission)
                .unwrap_err(),
            None => client.execute_typed_with_request_id::<Value, _>(
                "original-write",
                serde_json::json!({"type":"fixture_write"}),
            )
            .unwrap_err(),
        };
        assert!(error.requires_reconciliation());
        assert_eq!(error.request_id(), "original-write");
        let result = client.reconcile_command_result(&error);
        let requests = server.join().unwrap();
        let _ = std::fs::remove_file(&socket);
        (result, requests)
    }

    // Synthetic metadata tests transport only; source authenticity is covered
    // by Application tests that export an actual committed Task promotion.
    fn wire_task_admission() -> nola_protocol::AgentTaskAdmission {
        nola_protocol::AgentTaskAdmission {
            protocol_version: 1,
            task_id: "task".into(),
            predecessor_task_id: None,
            parent_conversation_id: "parent".into(),
            parent_core_session_id: "parent-core".into(),
            parent_run_id: "run".into(),
            parent_turn_id: "turn".into(),
            child_conversation_id: "child".into(),
            parent_tool_call_id: "call".into(),
            parent_effect_id: "effect".into(),
            input_watermark: Some(1),
            source_store_epoch: "source".into(),
            source_transition_id: "transition".into(),
            source_host_event_id: "event".into(),
            source_checkpoint_revision: 1,
            core_revision: 1,
            core_event_cursor: 1,
            admission_digest: String::new(),
        }.seal().unwrap()
    }

    #[test]
    fn invalid_host_task_admission_is_rejected_before_transport() {
        let client = ApplicationServiceClient::with_socket(
            "/tmp/nola-invalid-admission.db", "/tmp/nola-invalid-admission.sock",
        );
        let mut admission = wire_task_admission();
        admission.parent_run_id = "tampered-run".into();
        let error = client.import_agent_task_admission_with_request_id("invalid-admission", &admission).unwrap_err();
        assert!(!error.requires_reconciliation());
        assert_eq!(error.request_id(), "invalid-admission");
        assert!(error.to_string().contains("digest differs"));
    }

    #[cfg(unix)]
    #[test]
    fn host_task_admission_lost_ack_queries_original_receipt_without_another_import() {
        let admission = wire_task_admission();
        let authority = serde_json::json!({
            "task_id":"task", "phase":"admitted", "authority_revision":2,
            "admission": admission,
        });
        let mut wrong_task = authority.clone();
        wrong_task["task_id"] = "another-task".into();
        let mut unadmitted = authority.clone();
        unadmitted["phase"] = "prepared".into();
        let mut wrong_source = authority.clone();
        wrong_source["admission"]["source_store_epoch"] = "another-source".into();
        let mut missing_revision = authority.clone();
        missing_revision["authority_revision"] = 0.into();
        let mut prepared_revision = authority.clone();
        prepared_revision["authority_revision"] = 1.into();
        let initial_responses = std::iter::once(None).chain(
            [wrong_task, unadmitted, wrong_source, missing_revision, prepared_revision].into_iter().map(|result| {
                Some(CommandReceiptOutcome::Committed {
                    completed_at: "2".into(), committed_revision: None, result,
                })
            }),
        );
        for initial in initial_responses {
        let (result, requests) = reconcile_from_fixture_command(
            initial,
            Some(CommandReceiptOutcome::Committed {
                completed_at: "3".into(), committed_revision: None, result: authority.clone(),
            }),
            Some(&admission),
        );
        assert_eq!(result.unwrap(), authority);
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[1].command.command_type, "host_import_agent_task_admission");
        assert_eq!(requests[1].request_id, "original-write");
        assert_eq!(requests[1].command.arguments, serde_json::json!({"admission":admission}));
        assert_eq!(requests[3].command.command_type, "command_receipt");
        assert_eq!(requests[3].command.arguments, serde_json::json!({"request_id":"original-write"}));
        assert!(requests.iter().all(|request| request.client_id == "typed-client"));
        assert_eq!(requests.iter().filter(|request| request.command.command_type == "host_import_agent_task_admission").count(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn unknown_pending_and_lost_response_reconcile_the_same_receipt_without_resubmit() {
        for initial in [
            Some(CommandReceiptOutcome::Rejected {
                completed_at: "2".into(),
                error: nola_protocol::ProtocolFailure {
                    code: "command_transaction_commit_outcome_unknown".into(),
                    message: "fixture lost commit acknowledgement".into(),
                    recoverable: false,
                    details: None,
                },
            }),
            Some(CommandReceiptOutcome::Accepted),
            None,
        ] {
            let (result, requests) = reconcile_from_fixture(
                initial,
                Some(CommandReceiptOutcome::Committed {
                    completed_at: "3".into(),
                    committed_revision: Some(4),
                    result: serde_json::json!({"committed":true}),
                }),
            );
            assert_eq!(result.unwrap(), serde_json::json!({"committed":true}));
            assert_eq!(requests.len(), 4);
            assert_eq!(requests[1].request_id, "original-write");
            assert_eq!(requests[3].command.command_type, "command_receipt");
            assert_eq!(
                requests[3].command.arguments,
                serde_json::json!({"request_id":"original-write"})
            );
            assert!(requests
                .iter()
                .all(|request| request.client_id == "typed-client"));
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.command.command_type == "fixture_write")
                    .count(),
                1
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn absent_or_pending_receipt_never_fabricates_a_terminal_rejection() {
        for stored in [None, Some(CommandReceiptOutcome::Accepted)] {
            let (result, _) = reconcile_from_fixture(Some(CommandReceiptOutcome::Accepted), stored);
            let error = result.unwrap_err();
            assert!(matches!(
                error,
                ApplicationServiceCommandError::Pending { .. }
            ));
            assert_eq!(error.client_id(), "typed-client");
            assert_eq!(error.request_id(), "original-write");
        }
    }

    #[cfg(unix)]
    fn assert_request_deadline(trickle: bool, health: bool) {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;
        use std::time::Instant;
        let socket = PathBuf::from(format!("/tmp/nola-health-{}.sock", next_request_id()));
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let peer = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_millis(100)))
                            .unwrap();
                        let mut request = [0; 4096];
                        let _ = stream.read(&mut request);
                        for _ in 0..20 {
                            if trickle && stream.write_all(b" ").is_err() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept fixture: {error}"),
                }
            }
            panic!("client did not connect to fixture");
        });
        let client =
            ApplicationServiceClient::with_socket("/tmp/unused-health-fixture.db", &socket)
                .with_request_timeout(Duration::from_millis(250))
                .unwrap();
        let started = Instant::now();
        let result = if health {
            client.health()
        } else {
            client.request(CommandPayload {
                command_type: "health".into(),
                arguments: serde_json::json!({}),
            })
        };
        let elapsed = started.elapsed();
        let joined = peer.join();
        let _ = std::fs::remove_file(socket);
        joined.unwrap();
        let error = result.expect_err("incomplete response must fail");
        assert!(
            error.contains("read application service response failed"),
            "{error}"
        );
        assert!(
            elapsed < Duration::from_millis(800),
            "health took {elapsed:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn health_deadline_bounds_silent_connected_peer() {
        assert_request_deadline(false, true);
    }

    #[cfg(unix)]
    #[test]
    fn health_deadline_cannot_be_extended_by_trickling_bytes() {
        assert_request_deadline(true, true);
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_rpc_uses_configured_deadline_without_health_override() {
        assert_request_deadline(true, false);
    }

    #[test]
    fn ordinary_rpc_has_finite_default_and_rejects_zero_budget() {
        let client = ApplicationServiceClient::for_database("/tmp/unused-budget-fixture.db");
        assert_eq!(client.request_timeout, Duration::from_secs(30));
        assert!(client.with_request_timeout(Duration::ZERO).is_err());
    }

    #[test]
    fn database_socket_identity_is_stable_and_path_safe() {
        let first = socket_path_for_database(Path::new("/tmp/Nola State/nola.db"));
        let second = socket_path_for_database(Path::new("/tmp/Nola State/nola.db"));
        let other = socket_path_for_database(Path::new("/tmp/Other/nola.db"));
        assert_eq!(first, second);
        assert_ne!(first, other);
        assert!(!first.file_name().unwrap().to_string_lossy().contains(' '));
        let first_client = ApplicationServiceClient::for_database("/tmp/Nola State/nola.db");
        let second_client = ApplicationServiceClient::for_database("/tmp/Nola State/nola.db");
        assert_eq!(first_client.client_id(), second_client.client_id());
    }
}
