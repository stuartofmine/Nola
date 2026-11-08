// SPDX-License-Identifier: Apache-2.0
//! Exact task execution grants shared by storage and platform workers.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTaskAttempt {
    pub id: String,
    pub task_id: String,
    pub attempt_number: i64,
    pub worker_id: String,
    pub worker_generation: String,
    pub lease_token: String,
    pub lease_expires_at_ms: i64,
    pub state: String,
    pub revision: i64,
    pub claimed_at_ms: i64,
}

/// Exact authority retained by the worker that claimed or refreshed an attempt.
///
/// A task id and token alone are not authority: either can survive or be reused
/// across attempts. Terminal writes must fence the complete current lease.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskLeaseWitness {
    pub task_id: String,
    pub attempt_id: String,
    pub expected_attempt_revision: i64,
    pub worker_generation: String,
    pub lease_token: String,
    pub current_expiry_ms: i64,
}

impl From<&RuntimeTaskAttempt> for RuntimeTaskLeaseWitness {
    fn from(attempt: &RuntimeTaskAttempt) -> Self {
        Self {
            task_id: attempt.task_id.clone(),
            attempt_id: attempt.id.clone(),
            expected_attempt_revision: attempt.revision,
            worker_generation: attempt.worker_generation.clone(),
            lease_token: attempt.lease_token.clone(),
            current_expiry_ms: attempt.lease_expires_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTaskPermissionSuspension {
    pub id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub permission_request_id: String,
    pub continuation_id: String,
    pub continuation_json: String,
    pub state: String,
    pub revision: i64,
    pub suspended_at_ms: i64,
    pub resumed_at_ms: Option<i64>,
}

/// Immutable admission identity, retained even when its Run is deleted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskRunBinding {
    pub task_id: String,
    pub owner_session_id: String,
    pub child_session_id: String,
    pub run_id: String,
    pub admission_attempt_id: String,
    pub input_watermark: Option<i64>,
    pub bound_at_ms: i64,
}
