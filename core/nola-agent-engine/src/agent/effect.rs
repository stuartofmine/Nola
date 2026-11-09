// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines Core-planned I/O that only a Host may execute.
// - Boundary: Effects contain intent; delivery identity and replay live in the recovery journal.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::protocol::{AgentItem, CallId, ModelRequest, PermissionRequestId, SessionId, TurnId};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AgentEffect {
    PollCodeCell { cell: nola_domain::code_mode::CodeCellIdentity },
    ResolveCodeTool {
        result: nola_domain::code_mode::CodeToolResult,
    },
    InvokeModel {
        request: ModelRequest,
    },
    RequestPermission {
        allow_conversation: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        permission_instructions: Vec<nola_host_protocol::PermissionInstruction>,
        turn_id: TurnId,
        request_id: PermissionRequestId,
        call_id: CallId,
        tool_name: String,
        arguments: Value,
        resource: String,
        permission_domain: String,
        rationale: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evaluation: Option<super::permissions::PermissionEvaluation>,
    },
    ResolveDelegatedPermission {
        occurrence: nola_host_protocol::DelegatedPermissionOccurrence,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        permission_instructions: Vec<nola_host_protocol::PermissionInstruction>,
        turn_id: TurnId,
        target_effect_id: String,
        backend_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backend_session_id: Option<String>,
        permission_request_id: PermissionRequestId,
        resolution: super::host::HostPermissionResolution,
    },
    ResolvePermissionResources {
        turn_id: TurnId,
        call_id: CallId,
        tool_name: String,
        input: Value,
        request: nola_domain::permission_resource_plan::PermissionResolutionRequest,
    },
    ExecuteTool {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        permission_instructions: Vec<nola_host_protocol::PermissionInstruction>,
        turn_id: TurnId,
        call_id: CallId,
        tool_name: String,
        arguments: Value,
        #[serde(default)]
        grant: Option<Value>,
    },
    CancelHostEffect {
        turn_id: TurnId,
        target_effect_id: String,
    },
    CancelTask {
        turn_id: TurnId,
        task_id: String,
    },
    RefreshCapabilityCatalog {
        turn_id: TurnId,
        run_id: String,
        catalog_revision: u64,
        errors: Vec<super::capability::RecoverableCapabilityError>,
    },
    CompactContext {
        handoff: bool,
        session_id: SessionId,
        items_to_summarize: Vec<AgentItem>,
        target_tokens: usize,
    },
    RunLifecycleHook {
        event: crate::lifecycle_hooks::LifecycleHookEvent,
    },
}
