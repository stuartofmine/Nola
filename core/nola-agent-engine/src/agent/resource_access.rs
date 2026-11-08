// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Evaluates read/edit/delete/share access to managed Agent resources.
// - Boundary: Resource access never grants Skill, MCP, tool, or data-source execution.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const RESOURCE_ACCESS_SCHEMA_VERSION: u32 = 1;
pub const RESOURCE_ACCESS_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResourceKind {
    Skill,
    McpServer,
    DataSource,
    PromptTemplate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResourcePermission {
    ReadMetadata,
    ReadContent,
    EditContent,
    Delete,
    ManageSharing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResourcePrincipalKind {
    LocalUser,
    Account,
    Device,
    Group,
    Anyone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResourcePrincipal {
    pub kind: AgentResourcePrincipalKind,
    #[serde(default)]
    pub id: String,
}

impl AgentResourcePrincipal {
    fn is_valid(&self) -> bool {
        match self.kind {
            AgentResourcePrincipalKind::Anyone => self.id.trim().is_empty(),
            _ => !self.id.trim().is_empty(),
        }
    }

    fn matches(&self, actor: &Self) -> bool {
        self.kind == AgentResourcePrincipalKind::Anyone || self == actor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResourceIdentity {
    pub kind: AgentResourceKind,
    pub resource_id: String,
    pub owner: AgentResourcePrincipal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResourceAccessGrant {
    pub grant_id: String,
    pub resource_kind: AgentResourceKind,
    pub resource_id: String,
    pub principal: AgentResourcePrincipal,
    #[serde(default)]
    pub permissions: BTreeSet<AgentResourcePermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_epoch_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResourceAccessRequest {
    pub schema_version: u32,
    pub resource: AgentResourceIdentity,
    pub actor: AgentResourcePrincipal,
    pub permission: AgentResourcePermission,
    #[serde(default)]
    pub grants: Vec<AgentResourceAccessGrant>,
    pub now_epoch_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_host_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResourceAccessReason {
    Owner,
    ExplicitGrant,
    InvalidRequest,
    HostBoundaryMismatch,
    NoMatchingGrant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResourceGrantRejectionReason {
    InvalidGrant,
    ResourceMismatch,
    PrincipalMismatch,
    HostBoundaryMismatch,
    RevisionMismatch,
    ContentDigestMismatch,
    Expired,
    PermissionMissing,
    PublicMutationForbidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentResourceGrantRejection {
    pub grant_id: String,
    pub reason: AgentResourceGrantRejectionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResourceAccessDecision {
    pub schema_version: u32,
    pub allowed: bool,
    pub reason: AgentResourceAccessReason,
    pub resource_kind: AgentResourceKind,
    pub resource_id: String,
    pub permission: AgentResourcePermission,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_grant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_grants: Vec<AgentResourceGrantRejection>,
    /// This is deliberately always false. Resource visibility/editability is
    /// not a capability grant and cannot authorize execution.
    pub tool_execution_authorized: bool,
}

pub fn evaluate_resource_access(
    request: &AgentResourceAccessRequest,
) -> AgentResourceAccessDecision {
    let base = |allowed, reason, matched_grant_id, rejected_grants| AgentResourceAccessDecision {
        schema_version: RESOURCE_ACCESS_SCHEMA_VERSION,
        allowed,
        reason,
        resource_kind: request.resource.kind,
        resource_id: request.resource.resource_id.clone(),
        permission: request.permission,
        matched_grant_id,
        rejected_grants,
        tool_execution_authorized: false,
    };

    if request.schema_version != RESOURCE_ACCESS_SCHEMA_VERSION
        || request.resource.resource_id.trim().is_empty()
        || !request.resource.owner.is_valid()
        || !request.actor.is_valid()
        || request.now_epoch_ms < 0
        || !optional_digest_is_valid(request.resource.content_digest.as_deref())
    {
        return base(
            false,
            AgentResourceAccessReason::InvalidRequest,
            None,
            Vec::new(),
        );
    }

    let request_host_id = request.request_host_id.as_deref().map(str::trim);
    if let Some(resource_host_id) = request.resource.host_id.as_deref().map(str::trim) {
        if resource_host_id.is_empty() || request_host_id != Some(resource_host_id) {
            return base(
                false,
                AgentResourceAccessReason::HostBoundaryMismatch,
                None,
                Vec::new(),
            );
        }
    }

    if request.actor == request.resource.owner {
        return base(true, AgentResourceAccessReason::Owner, None, Vec::new());
    }

    let mut rejected = Vec::new();
    for grant in &request.grants {
        let rejection = validate_grant(request, grant, request_host_id);
        if let Some(reason) = rejection {
            rejected.push(AgentResourceGrantRejection {
                grant_id: grant.grant_id.clone(),
                reason,
            });
            continue;
        }
        return base(
            true,
            AgentResourceAccessReason::ExplicitGrant,
            Some(grant.grant_id.clone()),
            rejected,
        );
    }
    base(
        false,
        AgentResourceAccessReason::NoMatchingGrant,
        None,
        rejected,
    )
}

fn validate_grant(
    request: &AgentResourceAccessRequest,
    grant: &AgentResourceAccessGrant,
    request_host_id: Option<&str>,
) -> Option<AgentResourceGrantRejectionReason> {
    if grant.grant_id.trim().is_empty()
        || !grant.principal.is_valid()
        || grant.permissions.is_empty()
        || !optional_digest_is_valid(grant.content_digest.as_deref())
    {
        return Some(AgentResourceGrantRejectionReason::InvalidGrant);
    }
    if grant.resource_kind != request.resource.kind
        || grant.resource_id != request.resource.resource_id
    {
        return Some(AgentResourceGrantRejectionReason::ResourceMismatch);
    }
    if !grant.principal.matches(&request.actor) {
        return Some(AgentResourceGrantRejectionReason::PrincipalMismatch);
    }
    if grant.principal.kind == AgentResourcePrincipalKind::Anyone
        && matches!(
            request.permission,
            AgentResourcePermission::EditContent
                | AgentResourcePermission::Delete
                | AgentResourcePermission::ManageSharing
        )
    {
        return Some(AgentResourceGrantRejectionReason::PublicMutationForbidden);
    }
    if grant
        .host_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|host_id| host_id.is_empty() || request_host_id != Some(host_id))
    {
        return Some(AgentResourceGrantRejectionReason::HostBoundaryMismatch);
    }
    if grant
        .resource_revision
        .is_some_and(|revision| revision != request.resource.revision)
    {
        return Some(AgentResourceGrantRejectionReason::RevisionMismatch);
    }
    if grant
        .content_digest
        .as_deref()
        .is_some_and(|digest| request.resource.content_digest.as_deref() != Some(digest))
    {
        return Some(AgentResourceGrantRejectionReason::ContentDigestMismatch);
    }
    if grant
        .expires_at_epoch_ms
        .is_some_and(|expires_at| expires_at <= request.now_epoch_ms)
    {
        return Some(AgentResourceGrantRejectionReason::Expired);
    }
    if !grant.permissions.contains(&request.permission) {
        return Some(AgentResourceGrantRejectionReason::PermissionMissing);
    }
    None
}

fn optional_digest_is_valid(value: Option<&str>) -> bool {
    let Some(value) = value else { return true };
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
