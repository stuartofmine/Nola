// SPDX-License-Identifier: Apache-2.0
//! Persistence facts independent of the concrete database.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnqueueAgentCommandInput {
    pub session_id: String,
    pub command_id: String,
    pub payload_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredAgentCommand {
    pub sequence: i64,
    pub session_id: String,
    pub command_id: String,
    pub payload_json: String,
    pub created_at_ms: i64,
    pub lease_owner: Option<String>,
    pub lease_until_ms: Option<i64>,
    pub attempt_count: i64,
}

/// Mutations participate in the caller's existing receipt Unit of Work.
pub trait AgentQueueRepository {
    type Error: std::fmt::Display;
    /// Observation only. The head may be leased and must not be skipped.
    fn peek_agent_command(&mut self, session_id: &str)
        -> Result<Option<StoredAgentCommand>, Self::Error>;
    fn enqueue_agent_command(
        &mut self,
        input: &EnqueueAgentCommandInput,
    ) -> Result<bool, Self::Error>;
    fn claim_next_agent_command(
        &mut self,
        session_id: &str,
        lease_owner: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<Option<StoredAgentCommand>, Self::Error>;
    fn leased_agent_commands(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<StoredAgentCommand>, Self::Error>;
    fn release_agent_command_lease(
        &mut self,
        session_id: &str,
        command_id: &str,
        lease_owner: &str,
    ) -> Result<bool, Self::Error>;
    fn acknowledge_agent_command(
        &mut self,
        session_id: &str,
        command_id: &str,
        lease_owner: &str,
    ) -> Result<bool, Self::Error>;
}
