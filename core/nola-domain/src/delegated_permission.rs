// SPDX-License-Identifier: Apache-2.0
//! Durable authorization and transport evidence for one external permission occurrence.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedPermissionDecision {
    AllowOnce,
    AllowConversation,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedPermissionDeliveryState {
    AwaitingDecision,
    Prepared,
    Claimed,
    TransportWritten,
    OutcomeUnknown,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedPermissionDeliveryRecord {
    pub id: String,
    pub run_id: String,
    pub invocation_id: String,
    pub target_effect_id: String,
    pub worker_generation: String,
    pub occurrence_id: String,
    pub backend_id: String,
    pub backend_session_id: String,
    pub rpc_request_id: String,
    pub call_id: String,
    pub request_json: serde_json::Value,
    pub request_digest: String,
    pub control_effect_id: Option<String>,
    pub control_request_id: Option<String>,
    pub decision: Option<DelegatedPermissionDecision>,
    pub state: DelegatedPermissionDeliveryState,
    pub created_at_ms: i64,
    pub claimed_at_ms: Option<i64>,
    pub written_at_ms: Option<i64>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedPermissionDeliveryMutation {
    /// None inserts; Some compares the durable revision before replacing the record.
    pub expected_revision: Option<i64>,
    pub record: DelegatedPermissionDeliveryRecord,
}
