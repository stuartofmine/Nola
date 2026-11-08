// SPDX-License-Identifier: Apache-2.0
//! Pure permission decision facts; serde contracts shared with existing checkpoints.
use serde::{Deserialize, Serialize};

/// One product policy vocabulary for all Hosts. Decisions/grants remain separate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    #[default]
    Cautious,
    AllowAll,
}

/// A committed configuration revision applied to one exact active Run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivePermissionPolicy {
    pub run_id: String,
    pub configuration_revision: i64,
    pub profile: PermissionProfile,
}

impl ActivePermissionPolicy {
    pub fn command_id(&self) -> String {
        format!("permission-policy:{}:{}", self.run_id, self.configuration_revision)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.run_id.trim().is_empty() || self.configuration_revision < 1 {
            return Err("active permission policy requires a Run and committed configuration revision".into());
        }
        Ok(())
    }
}

impl PermissionProfile {
    pub const ALL: [Self; 2] = [Self::Cautious, Self::AllowAll];

    pub const fn canonical_name(self) -> &'static str {
        match self { Self::Cautious => "cautious", Self::AllowAll => "allow_all" }
    }

    pub fn from_canonical_name(value: &str) -> Option<Self> {
        match value { "cautious" => Some(Self::Cautious), "allow_all" => Some(Self::AllowAll), _ => None }
    }

    /// Transport field for execution adapters; not an independent policy source.
    pub const fn auto_approve(self) -> bool { true }
    pub const fn network_policy(self) -> &'static str {
        match self { Self::Cautious => "ask", Self::AllowAll => "allow" }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPolicyDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionEvaluationReason {
    MatchingRule,
    OrdinaryOperationAllowed,
    CautiousProfileRequiresConfirmation,
    InteractiveGrantUnavailableForOrigin,
    NoMatchingRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionEvaluation {
    pub decision: PermissionPolicyDecision,
    pub reason: PermissionEvaluationReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<PermissionRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionSubject {
    pub capability: String,
    pub resource: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub capability_pattern: String,
    pub resource_pattern: String,
    pub decision: PermissionPolicyDecision,
    pub source: PermissionRuleSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleSource {
    ProductDefault,
    /// Explicit policy frozen/admitted for this invocation, including scheduled
    /// work. It is not a live-window grant or authority borrowed from a parent.
    InvocationPolicy,
    UserGlobal,
    Host,
    Conversation,
    Turn,
}
/// Authority established by the input adapter, never inferred from prompt text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PermissionAuthorityFacts {
    pub carries_live_user_authority: bool,
    pub accepts_conversation_permission_grants: bool,
}
