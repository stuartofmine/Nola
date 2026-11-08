// SPDX-License-Identifier: Apache-2.0
//! Goal execution bindings share the checkpoint/publication transaction.
use super::*;
use nola_store_port::goal_execution::{GoalExecutionRepository, StoredGoalExecutionBinding};
impl GoalExecutionRepository for NolaStore {
    type Error = StoreError;
    fn load_goal_execution_binding(
        &self,
        conversation_id: &str,
        goal_id: &str,
    ) -> StoreResult<Option<StoredGoalExecutionBinding>> {
        self.connection.query_row(
            "SELECT operation_id,source_host_id,binding_json,binding_sha256,created_at_ms FROM goal_execution_bindings WHERE conversation_id=?1 AND goal_id=?2",
            params![conversation_id,goal_id], |row| Ok(StoredGoalExecutionBinding {
                identity: nola_domain::goal_execution::GoalExecutionBindingIdentity {
                    conversation_id: conversation_id.into(), goal_id: goal_id.into(),
                    operation_id: row.get(0)?, source_host_id: row.get(1)?,
                }, binding_json: row.get(2)?, binding_sha256: row.get(3)?, created_at_ms: row.get(4)?,
            })).optional().map_err(StoreError::from)
    }
    fn insert_goal_execution_binding(
        &mut self,
        binding: &StoredGoalExecutionBinding,
    ) -> StoreResult<()> {
        if !self.has_write_unit_of_work() {
            return Err(StoreError::Invariant(
                "Goal binding requires canonical Start transaction".into(),
            ));
        }
        binding.identity.validate().map_err(StoreError::Invariant)?;
        if binding.created_at_ms < 0
            || format!("{:x}", Sha256::digest(binding.binding_json.as_bytes()))
                != binding.binding_sha256
        {
            return Err(StoreError::Invariant(
                "invalid Goal execution binding digest or time".into(),
            ));
        }
        if let Some(existing) = self.load_goal_execution_binding(
            &binding.identity.conversation_id,
            &binding.identity.goal_id,
        )? {
            return if existing == *binding {
                Ok(())
            } else {
                Err(StoreError::Invariant(
                    "Goal execution binding is immutable".into(),
                ))
            };
        }
        self.connection.execute("INSERT INTO goal_execution_bindings(conversation_id,goal_id,operation_id,source_host_id,binding_json,binding_sha256,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![binding.identity.conversation_id,binding.identity.goal_id,binding.identity.operation_id,binding.identity.source_host_id,binding.binding_json,binding.binding_sha256,binding.created_at_ms])?;
        Ok(())
    }
}
