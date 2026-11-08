// SPDX-License-Identifier: Apache-2.0
//! Durable permission grant ownership and Core rule projection.

use nola_agent_engine::agent::permissions::{
    PermissionPolicyDecision as CorePermissionDecision, PermissionRule, PermissionRuleSet,
    PermissionRuleSource,
};
use nola_store_port::permission::{
    PermissionGrantScope as StoreScope, PermissionRepository,
    StoredPermissionDecision as StoreDecision, StoredPermissionGrant,
    UpsertPermissionGrantInput as StoreInput,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum PermissionGrantTarget {
    UserGlobal,
    Host { host_id: String },
    Conversation { conversation_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionGrantDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionGrantInput {
    pub id: String,
    pub target: PermissionGrantTarget,
    pub capability_pattern: String,
    pub resource_pattern: String,
    pub decision: PermissionGrantDecision,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionGrant {
    pub id: String,
    pub target: PermissionGrantTarget,
    pub capability_pattern: String,
    pub resource_pattern: String,
    pub decision: PermissionGrantDecision,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
}

pub fn record_permission_grant<R: PermissionRepository + ?Sized>(
    store: &mut R,
    input: PermissionGrantInput,
) -> Result<PermissionGrant, String> {
    let stored = store
        .upsert_permission_grant(&input.into_stored())
        .map_err(|error| format!("record permission grant failed: {error}"))?;
    PermissionGrant::from_stored(stored)
}

pub fn revoke_permission_grant<R: PermissionRepository + ?Sized>(
    store: &mut R,
    id: &str,
    revoked_at_ms: i64,
) -> Result<bool, String> {
    store
        .revoke_permission_grant(id, revoked_at_ms)
        .map_err(|error| format!("revoke permission grant failed: {error}"))
}

/// Produces the exact ordered rule overlay consumed by Core. Platform clients
/// never translate grants or choose precedence themselves.
pub fn load_permission_rule_set<R: PermissionRepository + ?Sized>(
    store: &R,
    conversation_id: Option<&str>,
    host_id: Option<&str>,
) -> Result<PermissionRuleSet, String> {
    let grants = store
        .effective_permission_grants(conversation_id, host_id)
        .map_err(|error| format!("load permission grants failed: {error}"))?;
    Ok(PermissionRuleSet {
        rules: grants
            .into_iter()
            .map(|grant| PermissionRule {
                capability_pattern: grant.capability_pattern,
                resource_pattern: grant.resource_pattern,
                decision: match grant.decision {
                    StoreDecision::Allow => CorePermissionDecision::Allow,
                    StoreDecision::Deny => CorePermissionDecision::Deny,
                },
                source: match grant.scope {
                    StoreScope::UserGlobal => PermissionRuleSource::UserGlobal,
                    StoreScope::Host => PermissionRuleSource::Host,
                    StoreScope::Conversation => PermissionRuleSource::Conversation,
                },
            })
            .collect(),
    })
}

/// Returns durable grant facts for settings projections. Permission evaluation
/// itself must use load_permission_rule_set.
pub fn list_effective_permission_grants<R: PermissionRepository + ?Sized>(
    store: &R,
    conversation_id: Option<&str>,
    host_id: Option<&str>,
) -> Result<Vec<PermissionGrant>, String> {
    store
        .effective_permission_grants(conversation_id, host_id)
        .map_err(|error| format!("load permission grant facts failed: {error}"))?
        .into_iter()
        .map(PermissionGrant::from_stored)
        .collect()
}

impl PermissionGrantInput {
    pub(crate) fn into_stored(self) -> StoreInput {
        let (scope, conversation_id, host_id) = self.target.into_stored();
        StoreInput {
            id: self.id,
            scope,
            conversation_id,
            host_id,
            capability_pattern: self.capability_pattern,
            resource_pattern: self.resource_pattern,
            decision: self.decision.into_stored(),
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        }
    }
}

impl PermissionGrant {
    pub(crate) fn from_stored(grant: StoredPermissionGrant) -> Result<Self, String> {
        Ok(Self {
            id: grant.id,
            target: PermissionGrantTarget::from_stored(
                grant.scope,
                grant.conversation_id,
                grant.host_id,
            )?,
            capability_pattern: grant.capability_pattern,
            resource_pattern: grant.resource_pattern,
            decision: PermissionGrantDecision::from_stored(grant.decision),
            created_at_ms: grant.created_at_ms,
            updated_at_ms: grant.updated_at_ms,
            revoked_at_ms: grant.revoked_at_ms,
        })
    }
}

impl PermissionGrantTarget {
    fn into_stored(self) -> (StoreScope, Option<String>, Option<String>) {
        match self {
            Self::UserGlobal => (StoreScope::UserGlobal, None, None),
            Self::Host { host_id } => (StoreScope::Host, None, Some(host_id)),
            Self::Conversation { conversation_id } => {
                (StoreScope::Conversation, Some(conversation_id), None)
            }
        }
    }

    fn from_stored(
        scope: StoreScope,
        conversation_id: Option<String>,
        host_id: Option<String>,
    ) -> Result<Self, String> {
        match (scope, conversation_id, host_id) {
            (StoreScope::UserGlobal, None, None) => Ok(Self::UserGlobal),
            (StoreScope::Host, None, Some(host_id)) => Ok(Self::Host { host_id }),
            (StoreScope::Conversation, Some(conversation_id), None) => {
                Ok(Self::Conversation { conversation_id })
            }
            _ => Err("persisted permission grant has an invalid target".into()),
        }
    }
}

impl PermissionGrantDecision {
    fn into_stored(self) -> StoreDecision {
        match self {
            Self::Allow => StoreDecision::Allow,
            Self::Deny => StoreDecision::Deny,
        }
    }

    fn from_stored(decision: StoreDecision) -> Self {
        match decision {
            StoreDecision::Allow => Self::Allow,
            StoreDecision::Deny => Self::Deny,
        }
    }
}
