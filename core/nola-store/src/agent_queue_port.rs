// SPDX-License-Identifier: Apache-2.0
//! Repository adapter on the owning Store connection.
use crate::{NolaStore, StoreError};
use nola_store_port::agent_queue::*;
impl AgentQueueRepository for NolaStore {
    type Error = StoreError;
    fn peek_agent_command(&mut self, session_id: &str) -> Result<Option<StoredAgentCommand>, Self::Error> {
        NolaStore::peek_agent_command(self, session_id)
    }
    fn enqueue_agent_command(
        &mut self,
        input: &EnqueueAgentCommandInput,
    ) -> Result<bool, Self::Error> {
        NolaStore::enqueue_agent_command(self, input)
    }
    fn claim_next_agent_command(
        &mut self,
        session_id: &str,
        lease_owner: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<Option<StoredAgentCommand>, Self::Error> {
        NolaStore::claim_next_agent_command(self, session_id, lease_owner, now_ms, lease_until_ms)
    }
    fn leased_agent_commands(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<StoredAgentCommand>, Self::Error> {
        NolaStore::leased_agent_commands(self, session_id)
    }
    fn release_agent_command_lease(
        &mut self,
        session_id: &str,
        command_id: &str,
        lease_owner: &str,
    ) -> Result<bool, Self::Error> {
        NolaStore::release_agent_command_lease(self, session_id, command_id, lease_owner)
    }
    fn acknowledge_agent_command(
        &mut self,
        session_id: &str,
        command_id: &str,
        lease_owner: &str,
    ) -> Result<bool, Self::Error> {
        NolaStore::acknowledge_agent_command(self, session_id, command_id, lease_owner)
    }
}
