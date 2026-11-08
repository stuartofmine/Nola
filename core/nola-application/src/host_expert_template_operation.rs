// SPDX-License-Identifier: Apache-2.0
//! Private native/server Host operations. No renderer Application-command alias.
//! Stable business intent and authenticated actor bind receipts; authorization
//! time and the current builtin catalog are execution context, not retry keys.
use crate::expert_template_capture::{
    HostExpertTemplateAuthority, HostExpertTemplateCaptureContext,
};
use nola_domain::{
    edited_resend::PrepareEditedResend, expert_message::AdmitExpertConversationMessage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostExpertTemplateOperation {
    PreviewCapture {
        input: AdmitExpertConversationMessage,
        authority: HostExpertTemplateAuthority,
    },
    Admit {
        input: AdmitExpertConversationMessage,
        context: HostExpertTemplateCaptureContext,
    },
    PrepareEditedResend {
        input: PrepareEditedResend,
        authority: HostExpertTemplateAuthority,
    },
    LoadManifest {
        conversation_id: String,
        message_id: String,
        authority: HostExpertTemplateAuthority,
    },
    LoadPreparedResend {
        conversation_id: String,
        authority: HostExpertTemplateAuthority,
    },
    LoadChildInstructions {
        conversation_id: String,
        expected_run_id: String,
        authority: HostExpertTemplateAuthority,
    },
    ReadableTemplates {
        authority: HostExpertTemplateAuthority,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostExpertTemplateRequest {
    pub request_id: String,
    pub operation: HostExpertTemplateOperation,
}

impl HostExpertTemplateOperation {
    pub fn authority(&self) -> HostExpertTemplateAuthority {
        match self {
            Self::Admit { context, .. } => context.authority(),
            Self::PreviewCapture { authority, .. }
            | Self::PrepareEditedResend { authority, .. }
            | Self::LoadManifest { authority, .. }
            | Self::LoadPreparedResend { authority, .. }
            | Self::LoadChildInstructions { authority, .. }
            | Self::ReadableTemplates { authority } => authority.clone(),
        }
    }
    /// Built only from this same typed operation. Callers cannot supply a
    /// different stable command_json alongside the actual execution input.
    fn stable_mutation(&self) -> Result<Option<(&'static str, Value)>, String> {
        let authority = self.authority();
        authority.validate()?;
        match self {
            Self::Admit { input, .. } => {
                input.validate()?;
                Ok(Some((
                    "admit_expert_conversation_message",
                    json!({"type":"admit_expert_conversation_message","input":input,"authority":authority}),
                )))
            }
            Self::PrepareEditedResend { input, .. } => {
                uuid::Uuid::parse_str(&input.operation_id)
                    .map_err(|_| "invalid_expert_resend_operation_id")?;
                Ok(Some((
                    "prepare_edited_expert_resend",
                    json!({"type":"prepare_edited_expert_resend","input":input,"authority":authority}),
                )))
            }
            _ => Ok(None),
        }
    }
}

#[derive(Debug, Clone)]
pub enum HostExpertOperationError {
    Rejected {
        code: String,
        message: String,
    },
    OutcomePending {
        request_id: String,
        message: String,
        reopen_required: bool,
    },
}
impl From<String> for HostExpertOperationError {
    fn from(message: String) -> Self {
        Self::Rejected {
            code: "expert_host_operation_rejected".into(),
            message,
        }
    }
}
impl From<&str> for HostExpertOperationError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl std::fmt::Display for HostExpertOperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected { code, message } => write!(f, "{code}: {message}"),
            Self::OutcomePending {
                request_id,
                message,
                ..
            } => write!(f, "expert_host_outcome_pending ({request_id}): {message}"),
        }
    }
}
impl std::error::Error for HostExpertOperationError {}

impl<R: crate::repositories::ReceivedCommandRepositories> crate::ApplicationRuntime<R> {
    /// Only authenticated native/server adapters call this method. Public
    /// Application JSON cannot deserialize any of the internal variants below.
    pub fn execute_host_expert_template_operation(
        &mut self,
        client_id: &str,
        request_id: &str,
        operation: HostExpertTemplateOperation,
    ) -> Result<Value, HostExpertOperationError> {
        if client_id.trim().is_empty()
            || client_id.len() > 512
            || client_id.chars().any(char::is_control)
        {
            return Err("invalid_expert_host_client_identity".into());
        }
        if request_id.trim().is_empty()
            || request_id.len() > 512
            || request_id.chars().any(char::is_control)
        {
            return Err("invalid_expert_host_request_identity".into());
        }
        let stable = operation.stable_mutation()?;
        let now_epoch_ms = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "expert_host_clock_unavailable")?
                .as_millis(),
        )
        .map_err(|_| "expert_host_clock_out_of_range")?;
        let command = match operation {
            HostExpertTemplateOperation::PreviewCapture { input, authority } => {
                crate::ApplicationCommand::PreviewExpertTemplateCapture { input, authority }
            }
            HostExpertTemplateOperation::Admit { input, context } => {
                crate::ApplicationCommand::AdmitExpertConversationMessage {
                    input,
                    context,
                    captured_at_ms: now_epoch_ms,
                }
            }
            HostExpertTemplateOperation::PrepareEditedResend { input, authority } => {
                crate::ApplicationCommand::PrepareEditedExpertResend {
                    input,
                    authority,
                    captured_at_ms: now_epoch_ms,
                }
            }
            HostExpertTemplateOperation::LoadManifest {
                conversation_id,
                message_id,
                authority,
            } => crate::ApplicationCommand::LoadExpertMessageManifest {
                conversation_id,
                message_id,
                authority,
            },
            HostExpertTemplateOperation::LoadPreparedResend {
                conversation_id,
                authority,
            } => crate::ApplicationCommand::LoadPreparedExpertResend {
                conversation_id,
                authority,
            },
            HostExpertTemplateOperation::LoadChildInstructions {
                conversation_id,
                expected_run_id,
                authority,
            } => crate::ApplicationCommand::LoadExpertChildInstructions {
                conversation_id,
                expected_run_id,
                authority,
            },
            HostExpertTemplateOperation::ReadableTemplates { authority } => {
                crate::ApplicationCommand::ListReadableExpertTemplates {
                    actor: authority.actor,
                    now_epoch_ms,
                    request_host_id: authority.request_host_id,
                }
            }
        };
        let Some((command_type, stable_json)) = stable else {
            return self
                .execute(command)
                .map_err(HostExpertOperationError::from);
        };
        let receipt = self
            .execute_received_command(
                crate::command_receipt::ExecuteReceivedCommand {
                    client_id: client_id.into(),
                    request_id: request_id.to_owned(),
                    command_type: command_type.into(),
                    command_json: serde_json::to_string(&stable_json).map_err(|e| e.to_string())?,
                    expected_revision: None,
                    received_at: crate::command_receipt::timestamp_now(),
                },
                command,
            )
            .map_err(|error| {
                if error.requires_owner_reopen() || error.recoverable {
                    HostExpertOperationError::OutcomePending {
                        request_id: request_id.to_owned(),
                        message: error.to_string(),
                        reopen_required: error.requires_owner_reopen(),
                    }
                } else {
                    HostExpertOperationError::Rejected {
                        code: error.code,
                        message: error.message,
                    }
                }
            })?;
        match receipt.outcome {
            crate::command_receipt::ApplicationCommandReceiptOutcome::Committed {
                result, ..
            } => Ok(result),
            crate::command_receipt::ApplicationCommandReceiptOutcome::Rejected {
                code,
                message,
                ..
            } => Err(HostExpertOperationError::Rejected { code, message }),
            crate::command_receipt::ApplicationCommandReceiptOutcome::Conflict { .. } => {
                Err("expert_host_intent_conflict".into())
            }
            crate::command_receipt::ApplicationCommandReceiptOutcome::RecoveryRequired => {
                Err(HostExpertOperationError::OutcomePending {
                    request_id: request_id.to_owned(),
                    message: "The exact original intent requires durable reconciliation".into(),
                    reopen_required: false,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expert_template_capture::{
        HostExpertTemplateSelection, HostExpertTemplateSelectionSource,
    };
    use nola_agent_engine::agent::resource_access::{
        AgentResourcePrincipal, AgentResourcePrincipalKind,
    };
    #[test]
    fn replay_key_binds_actor_and_exact_intent_but_not_current_definition_catalog() {
        let submission = uuid::Uuid::new_v4().to_string();
        let input: AdmitExpertConversationMessage = serde_json::from_value(json!({
            "conversation_id":"parent","expected_revision":1,"room_id":uuid::Uuid::new_v4().to_string(),
            "expected_room_revision":1,"submission_id":submission,"user_message_id":format!("{submission}:user"),"text":"hello",
            "selected_participants":[{"participant_id":"member","agent_id":"agent","agent_revision":1,"context_conversation_id":"child"}],"created_at_ms":1,
        })).unwrap();
        let context = HostExpertTemplateCaptureContext {
            actor: AgentResourcePrincipal {
                kind: AgentResourcePrincipalKind::LocalUser,
                id: "owner".into(),
            },
            request_host_id: None,
            selections: vec![HostExpertTemplateSelection {
                context_conversation_id: "child".into(),
                context_configuration_revision: 3,
                model_profile: nola_host_protocol::HostModelProfileReference {
                    profile_id: "profile".into(),
                    admitted_revision: 1,
                },
                source: HostExpertTemplateSelectionSource::NoTemplate,
                template_id: None,
                host_user_instructions: vec![],
            }],
            builtins: vec![],
        };
        let original = HostExpertTemplateOperation::Admit {
            input: input.clone(),
            context: context.clone(),
        };
        let stable = original.stable_mutation().unwrap();
        let mut changed_context = context.clone();
        changed_context.selections[0].context_configuration_revision = 9;
        let changed = HostExpertTemplateOperation::Admit {
            input: input.clone(),
            context: changed_context,
        };
        assert_eq!(
            stable,
            changed.stable_mutation().unwrap(),
            "The original committed capture survives current configuration changes"
        );
        let mut other_actor = context.clone();
        other_actor.actor.id = "other".into();
        assert_ne!(
            stable,
            HostExpertTemplateOperation::Admit {
                input: input.clone(),
                context: other_actor
            }
            .stable_mutation()
            .unwrap()
        );
        let mut other_input = input;
        other_input.text = "different".into();
        assert_ne!(
            stable,
            HostExpertTemplateOperation::Admit {
                input: other_input,
                context
            }
            .stable_mutation()
            .unwrap()
        );
    }
}
