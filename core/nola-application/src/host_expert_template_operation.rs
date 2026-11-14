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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostExpertTemplateOperation {
    ExportRemoteParentAdmission {conversation_id:String,message_id:String,source_store_epoch:String,authority:HostExpertTemplateAuthority},
    AbortPeerTaskPreparation {request:Box<nola_agent_engine::agent::host::HostRequestEnvelope>,exchange_id:String,authority:HostExpertTemplateAuthority},
    LoadPeerForEffect {request:Box<nola_agent_engine::agent::host::HostRequestEnvelope>,authority:HostExpertTemplateAuthority},
    DerivePeerAuthority { request:Box<nola_agent_engine::agent::host::HostRequestEnvelope>, #[serde(default)] authority:Option<HostExpertTemplateAuthority> },
    PreparePeerTask { request:Box<nola_agent_engine::agent::host::HostRequestEnvelope>,exchange_id:String,
        target_configuration:Box<nola_agent_engine::agent::AgentCoreConfig>,#[serde(default)] target_invocation:Option<Box<nola_host_protocol::delegation::DelegatedInvocationSnapshot>>,
        project_root:String,authority:HostExpertTemplateAuthority },
    PeerTaskAction { request:Box<nola_agent_engine::agent::host::HostRequestEnvelope>,authority:HostExpertTemplateAuthority },
    PeerDirectory { source_context_id:String, source_run_id:String, authority:HostExpertTemplateAuthority },
    PreviewPeer { input: crate::expert_peer::PeerExchangeInput, authority: HostExpertTemplateAuthority },
    AdmitPeer { input: crate::expert_peer::PeerExchangeInput, context: HostExpertTemplateCaptureContext },
    LoadPeer { room_id: String, exchange_id: String, authority: HostExpertTemplateAuthority },
    LoadPeerForSubmission { room_id: String, exchange_id: String, authority: HostExpertTemplateAuthority },
    ClarifyPeer { source_context_id:String, source_run_id:String, call_id:String, exchange_id:String, text:String, authority:HostExpertTemplateAuthority },
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
impl PartialEq for HostExpertTemplateOperation {
    fn eq(&self, other: &Self) -> bool {
        match (serde_json::to_value(self), serde_json::to_value(other)) {
            (Ok(left),Ok(right)) => left==right,
            _ => false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostExpertTemplateRequest {
    pub request_id: String,
    pub operation: HostExpertTemplateOperation,
}

impl HostExpertTemplateOperation {
    /// Exhaustive transport classification. Unknown action inputs remain writes
    /// until validation rejects them, so uncertain mutation outcomes stay pending.
    pub fn mutates_state(&self) -> bool {
        match self {
            Self::Admit {..} | Self::PrepareEditedResend {..} | Self::AdmitPeer {..}
            | Self::ClarifyPeer {..} | Self::PreparePeerTask {..} | Self::AbortPeerTaskPreparation {..} => true,
            Self::PeerTaskAction {request,..} => !matches!(&request.payload,
                nola_agent_engine::agent::host::HostRequest::ExecuteTool {input,..}
                if matches!(input.get("action").and_then(Value::as_str),Some("directory"|"read"|"wait"))),
            Self::DerivePeerAuthority {..} | Self::LoadPeerForEffect {..} | Self::PeerDirectory {..}
            | Self::PreviewPeer {..} | Self::LoadPeer {..} | Self::LoadPeerForSubmission {..}
            | Self::PreviewCapture {..} | Self::LoadManifest {..} | Self::LoadPreparedResend {..}
            | Self::LoadChildInstructions {..} | Self::ReadableTemplates {..} | Self::ExportRemoteParentAdmission {..} => false,
        }
    }
    pub fn authority(&self) -> Result<HostExpertTemplateAuthority,String> {
        Ok(match self {
            Self::DerivePeerAuthority{authority:Some(authority),..} => authority.clone(),
            Self::DerivePeerAuthority{authority:None,..} => return Err("expert_peer_authority_is_source_derived".into()),
            Self::AbortPeerTaskPreparation{authority,..} | Self::LoadPeerForEffect{authority,..} | Self::PreparePeerTask{authority,..} | Self::PeerTaskAction{authority,..} => authority.clone(),
            Self::Admit { context, .. } | Self::AdmitPeer { context, .. } => context.authority(),
            Self::PeerDirectory { authority, .. } | Self::PreviewPeer { authority, .. } | Self::LoadPeer { authority, .. } | Self::LoadPeerForSubmission { authority, .. } | Self::ClarifyPeer { authority, .. }
            | Self::PreviewCapture { authority, .. }
            | Self::PrepareEditedResend { authority, .. }
            | Self::LoadManifest { authority, .. }
            | Self::LoadPreparedResend { authority, .. }
            | Self::LoadChildInstructions { authority, .. }
            | Self::ReadableTemplates { authority } | Self::ExportRemoteParentAdmission {authority,..} => authority.clone(),
        })
    }
    /// Built only from this same typed operation. Callers cannot supply a
    /// different stable command_json alongside the actual execution input.
    fn stable_mutation(&self) -> Result<Option<(&'static str, Value)>, String> {
        if matches!(self,Self::DerivePeerAuthority{..}) {return Ok(None);}
        let authority = self.authority()?;
        authority.validate()?;
        match self {
            Self::AbortPeerTaskPreparation{request,exchange_id,..} => Ok(Some(("abort_expert_peer_task_preparation",json!({"type":"abort_expert_peer_task_preparation","exchange_id":exchange_id,"source_effect":request.effect_id,"authority":authority})))),
            Self::PreparePeerTask{request,exchange_id,..} => Ok(Some(("prepare_expert_peer_task",json!({"type":"prepare_expert_peer_task","exchange_id":exchange_id,
                "source_effect":request.effect_id,"source_session":request.session_id,"source_turn":request.turn_id,"authority":authority})))),
            Self::PeerTaskAction{request,..} => {
                let nola_agent_engine::agent::host::HostRequest::ExecuteTool{input,..}=&request.payload else {return Err("expert_task_requires_tool_effect".into())};
                if matches!(input.get("action").and_then(Value::as_str),Some("directory"|"read"|"wait")) {Ok(None)}
                else {Ok(Some(("expert_peer_task_action",json!({"type":"expert_peer_task_action","source_effect":request.effect_id,"input":input,"authority":authority}))))}
            }
            Self::AdmitPeer { input, .. } => {
                input.validate()?;
                Ok(Some(("admit_expert_peer",json!({"type":"admit_expert_peer","input":input,"authority":authority}))))
            }
            Self::ClarifyPeer { source_context_id,source_run_id,call_id,exchange_id,text,.. } => {
                let input=crate::expert_peer::PeerClarificationInput{source_context_id:source_context_id.clone(),source_run_id:source_run_id.clone(),call_id:call_id.clone(),exchange_id:exchange_id.clone(),text:text.clone()};
                input.validate()?;
                Ok(Some(("clarify_expert_peer",json!({"type":"clarify_expert_peer","input":input,"authority":authority}))))
            }
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
            HostExpertTemplateOperation::ExportRemoteParentAdmission {conversation_id,message_id,source_store_epoch,authority} => {
                if authority.actor.kind!=nola_agent_engine::agent::resource_access::AgentResourcePrincipalKind::LocalUser {
                    return Err("remote_expert_parent_export_requires_local_host_user".into());
                }
                let host=authority.request_host_id.ok_or("remote_expert_parent_export_requires_host_identity")?;
                crate::ApplicationCommand::ExportRemoteExpertParentAdmission {conversation_id,message_id,source_host_id:host,source_store_epoch}
            },
            HostExpertTemplateOperation::AbortPeerTaskPreparation{request,exchange_id,authority} => crate::ApplicationCommand::AbortExpertPeerTaskPreparation{request,exchange_id,authority,now_ms:now_epoch_ms},
            HostExpertTemplateOperation::LoadPeerForEffect{request,authority} => crate::ApplicationCommand::LoadExpertPeerForEffect{request,authority,now_ms:now_epoch_ms},
            HostExpertTemplateOperation::DerivePeerAuthority{request,authority} => crate::ApplicationCommand::DeriveExpertPeerAuthority{request,authority,now_ms:now_epoch_ms},
            HostExpertTemplateOperation::PreparePeerTask{request,exchange_id,target_configuration,target_invocation,project_root,authority} => crate::ApplicationCommand::PrepareExpertPeerTask {
                input:Box::new(crate::expert_peer_task::PreparePeerTask{request,exchange_id,target_configuration,target_invocation,project_root,authority}),now_ms:now_epoch_ms},
            HostExpertTemplateOperation::PeerTaskAction{request,authority} => crate::ApplicationCommand::ExpertPeerTaskAction {
                input:Box::new(crate::expert_peer_task::PeerTaskAction{request,authority}),now_ms:now_epoch_ms},
            HostExpertTemplateOperation::PeerDirectory {source_context_id,source_run_id,authority} => crate::ApplicationCommand::ExpertPeerDirectory {source_context_id,source_run_id,authority},
            HostExpertTemplateOperation::PreviewPeer { input, authority } => crate::ApplicationCommand::PreviewExpertPeer { input,authority },
            HostExpertTemplateOperation::AdmitPeer { input, context } => crate::ApplicationCommand::AdmitExpertPeer { input,context,captured_at_ms:now_epoch_ms },
            HostExpertTemplateOperation::LoadPeerForSubmission { room_id,exchange_id,authority } => crate::ApplicationCommand::LoadExpertPeerForSubmission {room_id,exchange_id,authority},
            HostExpertTemplateOperation::LoadPeer { room_id,exchange_id,authority } => crate::ApplicationCommand::LoadExpertPeer {room_id,exchange_id,authority},
            HostExpertTemplateOperation::ClarifyPeer { source_context_id,source_run_id,call_id,exchange_id,text,authority } => crate::ApplicationCommand::ClarifyExpertPeer {
                input:crate::expert_peer::PeerClarificationInput{source_context_id,source_run_id,call_id,exchange_id,text},authority,captured_at_ms:now_epoch_ms },
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
