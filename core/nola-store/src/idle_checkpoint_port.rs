// SPDX-License-Identifier: Apache-2.0
//! Idle checkpoint and memory output share the owning Store transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::{
    conversation_revert::SaveCheckpointInput,
    memory::{IdleCheckpointRepository, StoredMemoryRecord, StoredMemoryUsageEntry},
    run_effect::StoredCheckpoint,
};
impl IdleCheckpointRepository for NolaStore {
    fn save_checkpoint_with_memories(
        &mut self,
        input: &SaveCheckpointInput,
        memories: &[StoredMemoryRecord],
        expected_memories: &[(String, i64)],
        memory_usage_entries: &[StoredMemoryUsageEntry],
    ) -> Result<(StoredCheckpoint, Vec<StoredMemoryRecord>), StoreError> {
        NolaStore::save_checkpoint_with_memories(
            self,
            input,
            memories,
            expected_memories,
            memory_usage_entries,
        )
    }
}
