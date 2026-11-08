// SPDX-License-Identifier: Apache-2.0
//! Durable permission request facts and ordered grant repositories.
//! Decisions are evaluated by Application/Core, never by a storage adapter.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionGrantScope {
    UserGlobal,
    Host,
    Conversation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StoredPermissionDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpsertPermissionGrantInput {
    pub id: String,
    pub scope: PermissionGrantScope,
    pub conversation_id: Option<String>,
    pub host_id: Option<String>,
    pub capability_pattern: String,
    pub resource_pattern: String,
    pub decision: StoredPermissionDecision,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredPermissionGrant {
    pub id: String,
    pub scope: PermissionGrantScope,
    pub conversation_id: Option<String>,
    pub host_id: Option<String>,
    pub capability_pattern: String,
    pub resource_pattern: String,
    pub decision: StoredPermissionDecision,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredPermissionRequest {
    pub closure: Option<nola_domain::permission_closure::PermissionRequestClosure>,
    pub request_id: String,
    pub task_id: String,
    pub child_session_id: String,
    pub capability: String,
    pub input_json: String,
    pub scope_rule: String,
    pub status: String,
    pub decision: Option<String>,
    pub custom_response: Option<String>,
    pub created_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
    pub core_permission_request_id: Option<String>,
    pub permission_instructions_json: String,
}

/// Resolution and its observed request are one transaction result. A replay
/// never changes the first timestamp or recreates a subsequently revoked grant.
#[derive(Clone, PartialEq, Eq)]
pub enum StoredPermissionResolutionOutcome {
    Closed(StoredPermissionRequest),
    Resolved(StoredPermissionRequest),
    AlreadyResolved(StoredPermissionRequest),
    Conflict(StoredPermissionRequest),
    Missing { request_id: String },
    StopPending { request_id: String, task_id: String, requested_at_ms: i64 },
}

impl std::fmt::Debug for StoredPermissionResolutionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Closed(_) => "Closed([redacted])",
            Self::Resolved(_) => "Resolved([redacted])",
            Self::AlreadyResolved(_) => "AlreadyResolved([redacted])",
            Self::Conflict(_) => "Conflict([redacted])",
            Self::Missing { .. } => "Missing([redacted])",
            Self::StopPending { .. } => "StopPending([redacted])",
        })
    }
}

/// All writes participate in the caller's existing Unit of Work. Grant reads
/// return active facts ordered global, Host, conversation, then timestamp/ID;
/// this is the stable precedence input consumed by Core's permission rules.
pub trait PermissionRepository {
    type Error: std::fmt::Display;
    fn ensure_core_permission_request(
        &mut self,
        request: &StoredPermissionRequest,
    ) -> Result<StoredPermissionRequest, Self::Error>;
    fn enqueue_permission_request(
        &mut self,
        request: &StoredPermissionRequest,
    ) -> Result<(), Self::Error>;
    fn permission_request(
        &self,
        request_id: &str,
    ) -> Result<Option<StoredPermissionRequest>, Self::Error>;
    fn pending_permission_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<StoredPermissionRequest>, Self::Error>;
    fn pending_permissions_for_owner(
        &self,
        owner_session_id: &str,
    ) -> Result<Vec<StoredPermissionRequest>, Self::Error>;
    /// Atomically compare against the original request, validate instruction
    /// history for a first write, and settle the optional derived grant with
    /// that write. Existing decisions never write grants or timestamps.
    fn resolve_permission_request(
        &mut self,
        request_id: &str,
        decision: &str,
        custom_response: Option<&str>,
        resolved_at_ms: i64,
        grant: Option<&UpsertPermissionGrantInput>,
    ) -> Result<StoredPermissionResolutionOutcome, Self::Error>;
    fn upsert_permission_grant(
        &mut self,
        input: &UpsertPermissionGrantInput,
    ) -> Result<StoredPermissionGrant, Self::Error>;
    fn revoke_permission_grant(
        &mut self,
        id: &str,
        revoked_at_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn effective_permission_grants(
        &self,
        conversation_id: Option<&str>,
        host_id: Option<&str>,
    ) -> Result<Vec<StoredPermissionGrant>, Self::Error>;
}
