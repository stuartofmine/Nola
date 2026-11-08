// SPDX-License-Identifier: Apache-2.0
//! Cold delivery reads join immutable bytes; acknowledgements update only heads.
use super::*;
impl NolaStore {
    pub fn pending_effects(&self, run_id: &str) -> StoreResult<Vec<StoredOutboxEffect>> {
        outbox_payload::pending(&self.connection, run_id)
    }
    pub fn outbox_effect(&self, effect_id: &str) -> StoreResult<Option<StoredOutboxEffect>> {
        require_id("outbox.effect_id", effect_id)?;
        outbox_payload::read(&self.connection, effect_id)
    }
    pub fn acknowledge_effect(
        &mut self,
        effect_id: &str,
        acknowledged_at_ms: i64,
    ) -> StoreResult<bool> {
        Ok(self.connection.execute(
            "UPDATE effect_outbox SET status='acknowledged',acknowledged_at_ms=?2
            WHERE effect_id=?1 AND status='pending'",
            params![effect_id, acknowledged_at_ms],
        )? == 1)
    }
}
