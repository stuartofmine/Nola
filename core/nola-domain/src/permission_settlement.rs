// SPDX-License-Identifier: Apache-2.0
//! Core-owned permission facts shared with transactional consumers.
//! Shape and identity are not authorization proof; Store checks the original effect.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionUserDecision {
    AllowOnce,
    AllowConversation,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PermissionReceiptOutcome {
    Authorization { decision: PermissionUserDecision },
    ConversationRuleApplied { source_request_id: String },
    PolicyApplied { policy: crate::ActivePermissionPolicy },
    Interrupted { reason: String },
    GuidanceProvided,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionReceipt {
    pub request_id: String,
    pub turn_id: String,
    pub call_id: String,
    pub capability_id: String,
    pub resource: String,
    pub outcome: PermissionReceiptOutcome,
}
