// SPDX-License-Identifier: Apache-2.0
//! Permission request use cases.
//!
//! Application owns the permission protocol. Persistence strings and encoded
//! JSON are translated into typed commands/results here; the repository carries
//! durable facts without depending on SQLite or defining permission policy.

use nola_store_port::permission::{PermissionRepository, StoredPermissionRequest,
    StoredPermissionResolutionOutcome, UpsertPermissionGrantInput,
    PermissionGrantScope, StoredPermissionDecision};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPermissionRequest {
    pub request_id: String,
    pub task_id: String,
    pub child_session_id: String,
    pub capability: String,
    pub input: Value,
    pub scope_rule: String,
    pub created_at_ms: i64,
    #[serde(default)]
    pub core_permission_request_id: Option<String>,
    #[serde(default)]
    pub permission_instructions: Vec<nola_host_protocol::PermissionInstruction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRequest {
    pub closure: Option<nola_domain::permission_closure::PermissionRequestClosure>,
    pub request_id: String,
    pub task_id: String,
    pub child_session_id: String,
    pub capability: String,
    pub input: Value,
    pub scope_rule: String,
    pub status: PermissionRequestStatus,
    pub resolution: Option<PermissionRequestResolution>,
    pub created_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
    #[serde(default)]
    pub core_permission_request_id: Option<String>,
    #[serde(default)]
    pub permission_instructions: Vec<nola_host_protocol::PermissionInstruction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRequestStatus {
    Pending,
    Resolved,
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum PermissionRequestCommand {
    EnsureCoreRequest {
        input: NewPermissionRequest,
    },
    Enqueue {
        input: NewPermissionRequest,
    },
    Get {
        request_id: String,
    },
    PendingForTask {
        task_id: String,
    },
    PendingForOwner {
        owner_session_id: String,
    },
    Authorize {
        request_id: String,
        decision: PermissionAuthorizationDecision,
        resolved_at_ms: i64,
    },
    Intervene {
        request_id: String,
        intervention: PermissionIntervention,
        resolved_at_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum PermissionAuthorizationDecision {
    AllowOnce,
    AllowConversation,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum PermissionIntervention {
    ProvideInstruction { instruction: String },
    ReturnToComposer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum PermissionRequestResolution {
    Authorization {
        decision: PermissionAuthorizationDecision,
    },
    Intervention {
        intervention: PermissionIntervention,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionResolutionReceipt {
    pub request_id: String,
    pub task_id: String,
    pub child_session_id: String,
    pub core_permission_request_id: Option<String>,
    pub resolution: PermissionRequestResolution,
    pub resolved_at_ms: i64,
}

// An intervention may contain private instructions. Never emit them through
// generic receipt diagnostics; the typed wire remains available to its owner.
impl std::fmt::Debug for PermissionResolutionReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PermissionResolutionReceipt { owner_and_resolution: [redacted] }")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum PermissionResolutionOutcome {
    Closed { request: PermissionRequest },
    Resolved { receipt: PermissionResolutionReceipt },
    AlreadyResolved { receipt: PermissionResolutionReceipt },
    Conflict { receipt: PermissionResolutionReceipt },
    Missing { request_id: String },
    StopPending { request_id: String, task_id: String, requested_at_ms: i64 },
}

impl std::fmt::Debug for PermissionResolutionOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Closed { .. } => "Closed([redacted])",
            Self::Resolved { .. } => "Resolved([redacted])",
            Self::AlreadyResolved { .. } => "AlreadyResolved([redacted])",
            Self::Conflict { .. } => "Conflict([redacted])",
            Self::Missing { .. } => "Missing([redacted])",
            Self::StopPending { .. } => "StopPending([redacted])",
        })
    }
}

impl PermissionResolutionOutcome {
    fn from_stored(outcome: StoredPermissionResolutionOutcome) -> Result<Self, String> {
        let receipt = |stored| -> Result<PermissionResolutionReceipt, String> {
            let request = PermissionRequest::from_stored(stored)?;
            if request.status != PermissionRequestStatus::Resolved {
                return Err("permission resolution receipt is not resolved".into());
            }
            Ok(PermissionResolutionReceipt {
                request_id: request.request_id, task_id: request.task_id,
                child_session_id: request.child_session_id,
                core_permission_request_id: request.core_permission_request_id,
                resolution: request.resolution.ok_or("permission resolution receipt lacks decision")?,
                resolved_at_ms: request.resolved_at_ms.ok_or("permission resolution receipt lacks timestamp")?,
            })
        };
        Ok(match outcome {
            StoredPermissionResolutionOutcome::Closed(request) => Self::Closed { request: PermissionRequest::from_stored(request)? },
            StoredPermissionResolutionOutcome::Resolved(request) => Self::Resolved { receipt: receipt(request)? },
            StoredPermissionResolutionOutcome::AlreadyResolved(request) => Self::AlreadyResolved { receipt: receipt(request)? },
            StoredPermissionResolutionOutcome::Conflict(request) => Self::Conflict { receipt: receipt(request)? },
            StoredPermissionResolutionOutcome::Missing { request_id } => Self::Missing { request_id },
            StoredPermissionResolutionOutcome::StopPending { request_id, task_id, requested_at_ms } =>
                Self::StopPending { request_id, task_id, requested_at_ms },
        })
    }
}

fn resolve_permission<R: PermissionRepository + ?Sized>(
    store: &mut R, request_id: &str, resolution: PermissionRequestResolution, at: i64,
) -> Result<Value, String> {
    let (decision, instruction) = resolution.store_values()?;
    // This scope is derived from the stored request, never accepted from a
    // client grant payload. Store settles grant + first decision atomically.
    let grant = if decision == "allow_conversation" {
        store.permission_request(request_id).map_err(|error| error.to_string())?
            .map(|request| {
                if request.task_id.trim().is_empty() || request.scope_rule.trim().is_empty() {
                    return Err("permission scope requires its original task and rule".to_string());
                }
                Ok(UpsertPermissionGrantInput {
                    id: format!("permission-request:{request_id}:task-scope"),
                    scope: PermissionGrantScope::Host, conversation_id: None,
                    host_id: Some(format!("task:{}", request.task_id)),
                    capability_pattern: "cli::tool::allow_rule".into(),
                    resource_pattern: request.scope_rule,
                    decision: StoredPermissionDecision::Allow,
                    created_at_ms: at, updated_at_ms: at,
                })
            }).transpose()?
    } else { None };
    let result = store.resolve_permission_request(request_id, decision, instruction.as_deref(), at, grant.as_ref())
        .map_err(|error| error.to_string())?;
    serde_json::to_value(PermissionResolutionOutcome::from_stored(result)?)
        .map_err(|error| error.to_string())
}

pub fn execute_permission_request_command<R: PermissionRepository + ?Sized>(
    store: &mut R,
    command: PermissionRequestCommand,
) -> Result<Value, String> {
    match command {
        PermissionRequestCommand::EnsureCoreRequest { input } => {
            if input
                .core_permission_request_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
            {
                return Err("ensure Core permission requires a real Core request ID".into());
            }
            let stored = store
                .ensure_core_permission_request(&input.into_stored()?)
                .map_err(|error| error.to_string())?;
            serde_json::to_value(PermissionRequest::from_stored(stored)?)
                .map_err(|error| error.to_string())
        }
        PermissionRequestCommand::Enqueue { input } => {
            store
                .enqueue_permission_request(&input.into_stored()?)
                .map_err(|error| error.to_string())?;
            Ok(json!(true))
        }
        PermissionRequestCommand::Get { request_id } => encode_optional(
            store
                .permission_request(&request_id)
                .map_err(|error| error.to_string())?,
        ),
        PermissionRequestCommand::PendingForTask { task_id } => encode_optional(
            store
                .pending_permission_for_task(&task_id)
                .map_err(|error| error.to_string())?,
        ),
        PermissionRequestCommand::PendingForOwner { owner_session_id } => {
            let requests = store
                .pending_permissions_for_owner(&owner_session_id)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(PermissionRequest::from_stored)
                .collect::<Result<Vec<_>, _>>()?;
            serde_json::to_value(requests).map_err(|error| error.to_string())
        }
        PermissionRequestCommand::Authorize {
            request_id,
            decision,
            resolved_at_ms,
        } => {
            let resolution = PermissionRequestResolution::Authorization { decision };
            resolve_permission(store, &request_id, resolution, resolved_at_ms)
        }
        PermissionRequestCommand::Intervene {
            request_id,
            intervention,
            resolved_at_ms,
        } => {
            // Repository validates the instruction against the same locked
            // request/history used to decide first write, replay or conflict.
            let resolution = PermissionRequestResolution::Intervention { intervention };
            resolve_permission(store, &request_id, resolution, resolved_at_ms)
        }
    }
}

fn encode_optional(request: Option<StoredPermissionRequest>) -> Result<Value, String> {
    let request = request.map(PermissionRequest::from_stored).transpose()?;
    serde_json::to_value(request).map_err(|error| error.to_string())
}

impl NewPermissionRequest {
    fn into_stored(self) -> Result<StoredPermissionRequest, String> {
        validate_permission_transport_context(
            self.core_permission_request_id.as_deref(),
            &self.permission_instructions,
        )?;
        Ok(StoredPermissionRequest {
            closure: None,
            request_id: self.request_id,
            task_id: self.task_id,
            child_session_id: self.child_session_id,
            capability: self.capability,
            input_json: serde_json::to_string(&self.input).map_err(|error| error.to_string())?,
            scope_rule: self.scope_rule,
            status: "pending".into(),
            decision: None,
            custom_response: None,
            created_at_ms: self.created_at_ms,
            resolved_at_ms: None,
            core_permission_request_id: self.core_permission_request_id,
            permission_instructions_json: serde_json::to_string(&self.permission_instructions)
                .map_err(|error| error.to_string())?,
        })
    }
}

impl PermissionRequest {
    pub(crate) fn from_stored(request: StoredPermissionRequest) -> Result<Self, String> {
        let permission_instructions: Vec<nola_host_protocol::PermissionInstruction> =
            serde_json::from_str(&request.permission_instructions_json)
                .map_err(|error| format!("invalid permission instruction metadata: {error}"))?;
        validate_permission_transport_context(
            request.core_permission_request_id.as_deref(),
            &permission_instructions,
        )?;
        let input = serde_json::from_str(&request.input_json).map_err(|error| {
            format!(
                "invalid persisted permission input for {}: {error}",
                request.request_id
            )
        })?;
        let status = match request.status.as_str() {
            "pending" => PermissionRequestStatus::Pending,
            "resolved" => PermissionRequestStatus::Resolved,
            "withdrawn" => PermissionRequestStatus::Withdrawn,
            other => return Err(format!("invalid persisted permission status: {other}")),
        };
        if (status == PermissionRequestStatus::Withdrawn) != request.closure.is_some() {
            return Err("permission closure and status disagree".into());
        }
        if let Some(closure) = &request.closure {
            closure.validate_for(&nola_domain::permission_closure::PermissionClosureRequest {
                request_id: &request.request_id, task_id: &request.task_id,
                child_session_id: &request.child_session_id,
                core_permission_request_id: request.core_permission_request_id.as_deref(),
                created_at_ms: request.created_at_ms, resolved_at_ms: request.resolved_at_ms,
            }).map_err(|error| error.to_string())?;
            if request.decision.is_some() != request.resolved_at_ms.is_some()
                || (request.decision.is_none() && request.custom_response.is_some()) {
                return Err("closed permission contains a partial user decision".into());
            }
        }
        let resolution = match request.decision.as_deref() {
            None => None,
            Some("allow_once") => Some(PermissionRequestResolution::Authorization {
                decision: PermissionAuthorizationDecision::AllowOnce,
            }),
            Some("allow_conversation") => Some(PermissionRequestResolution::Authorization {
                decision: PermissionAuthorizationDecision::AllowConversation,
            }),
            Some("deny") => Some(PermissionRequestResolution::Authorization {
                decision: PermissionAuthorizationDecision::Deny,
            }),
            Some("return_to_composer") => Some(PermissionRequestResolution::Intervention {
                intervention: PermissionIntervention::ReturnToComposer,
            }),
            Some("provide_instruction") => Some(PermissionRequestResolution::Intervention {
                intervention: PermissionIntervention::ProvideInstruction {
                    instruction: request
                        .custom_response
                        .clone()
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| {
                            "permission intervention is missing instruction".to_string()
                        })?,
                },
            }),
            Some(other) => return Err(format!("invalid persisted permission decision: {other}")),
        };
        match status {
            PermissionRequestStatus::Pending
                if resolution.is_some() || request.resolved_at_ms.is_some() =>
            {
                return Err("pending permission request contains a resolution".into());
            }
            PermissionRequestStatus::Resolved
                if resolution.is_none() || request.resolved_at_ms.is_none() =>
            {
                return Err("resolved permission request is missing its resolution".into());
            }
            _ => {}
        }
        Ok(Self {
            closure: request.closure,
            request_id: request.request_id,
            task_id: request.task_id,
            child_session_id: request.child_session_id,
            capability: request.capability,
            input,
            scope_rule: request.scope_rule,
            status,
            resolution,
            created_at_ms: request.created_at_ms,
            resolved_at_ms: request.resolved_at_ms,
            core_permission_request_id: request.core_permission_request_id,
            permission_instructions,
        })
    }
}

impl PermissionRequestResolution {
    fn store_values(self) -> Result<(&'static str, Option<String>), String> {
        match self {
            Self::Authorization {
                decision: PermissionAuthorizationDecision::AllowOnce,
            } => Ok(("allow_once", None)),
            Self::Authorization {
                decision: PermissionAuthorizationDecision::AllowConversation,
            } => Ok(("allow_conversation", None)),
            Self::Authorization {
                decision: PermissionAuthorizationDecision::Deny,
            } => Ok(("deny", None)),
            Self::Intervention {
                intervention: PermissionIntervention::ReturnToComposer,
            } => Ok(("return_to_composer", None)),
            Self::Intervention {
                intervention: PermissionIntervention::ProvideInstruction { instruction },
            } => {
                let instruction = instruction.trim().to_string();
                if instruction.is_empty() {
                    return Err("permission instruction must not be empty".to_string());
                }
                Ok(("provide_instruction", Some(instruction)))
            }
        }
    }
}

fn validate_permission_transport_context(
    core_id: Option<&str>,
    history: &[nola_host_protocol::PermissionInstruction],
) -> Result<(), String> {
    if core_id.is_some_and(|value| value.trim().is_empty())
        || (core_id.is_none() && !history.is_empty())
    {
        return Err("permission instruction metadata requires a real Core request identity".into());
    }
    for (index, entry) in history.iter().enumerate() {
        if entry.permission_request_id.trim().is_empty() {
            return Err("permission instruction has no originating request identity".into());
        }
        nola_host_protocol::PermissionInstruction::validate_append(
            &history[..index],
            &entry.permission_request_id,
            &entry.instruction,
        )?;
    }
    Ok(())
}
