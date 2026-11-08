// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_store_port::{
    cancellation_delivery::{CancellationDeliveryRepository, StoredCancellationDeliverySnapshot},
    run_recovery_inspection::RunRecoveryInspectionRepository,
};

// Storage needs only cancellation target identity, not the Host execution
// schema or Core reducer. Other envelopes are deliberately not interpreted.
#[derive(serde::Deserialize)]
struct StoredCancellationEnvelope {
    payload: StoredCancellationTarget,
}
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredCancellationTarget {
    Cancel {
        target_effect_id: String,
    },
    CancelTask,
    #[serde(other)]
    Other,
}

impl CancellationDeliveryRepository for NolaStore {
    type Error = StoreError;
    fn require_cancellation_write_unit_of_work(&self) -> StoreResult<()> {
        if !self.has_write_unit_of_work() {
            return Err(StoreError::Invariant(
                "cancellation claim requires an outer write unit of work".into(),
            ));
        }
        Ok(())
    }
    fn release_observed_cancellation_session_lease(
        &mut self,
        observed: &nola_store_port::workspace_ownership::StoredSessionRunLease,
    ) -> StoreResult<bool> {
        Ok(self.connection.execute(
            "DELETE FROM session_run_leases WHERE session_id=?1 AND owner=?2 AND pid=?3 AND process_start=?4 AND acquired_at_ms=?5",
            params![observed.session_id, observed.owner, i64::from(observed.pid), observed.process_start, observed.acquired_at_ms],
        )? == 1)
    }
    fn cancellation_delivery_snapshot(
        &self,
        run_id: &str,
    ) -> StoreResult<Option<StoredCancellationDeliverySnapshot>> {
        require_id("run_id", run_id)?;
        let _snapshot = read_snapshot::pin(&self.connection)?;
        let Some(run) = self.inspect_exact_run(run_id)? else {
            return Ok(None);
        };
        let delivery = nola_store_port::run_effect::CommittedHostDeliverySnapshot {
            conversation_id: Some(run.conversation_id.clone()),
            checkpoint: self.load_local_session_recovery(&run.conversation_id)?,
            effects: self.pending_effects(run_id)?,
        };
        if delivery.effects.len() > 256 {
            return Err(StoreError::Invariant(
                "cancellation delivery exceeds bounded effect window".into(),
            ));
        }
        let session_lease = self.session_run_lease(&run.conversation_id)?;
        // Bound by CURRENT cancellation targets, not all historical attempts.
        // Target history is read even if that target's outbox row was already
        // acknowledged. An indexed per-effect history cannot confuse old executions
        // with a target that never crossed an execution barrier.
        let mut relevant_ids = std::collections::BTreeSet::new();
        for effect in &delivery.effects {
            let envelope: StoredCancellationEnvelope = serde_json::from_str(&effect.payload_json)
                .map_err(|error| {
                StoreError::Invariant(format!("invalid cancellation outbox request: {error}"))
            })?;
            match envelope.payload {
                StoredCancellationTarget::Cancel { target_effect_id } => {
                    relevant_ids.insert(effect.effect_id.clone());
                    relevant_ids.insert(target_effect_id);
                }
                StoredCancellationTarget::CancelTask => {
                    relevant_ids.insert(effect.effect_id.clone());
                }
                _ => {}
            }
        }
        let mut statement = self.connection.prepare(
            "SELECT record_json FROM effect_delivery_attempts WHERE effect_id=?1 ORDER BY attempt_number DESC")?;
        let mut attempts = Vec::new();
        for id in relevant_ids {
            let rows = statement.query_map(params![id], |row| row.get::<_, String>(0))?;
            for raw in rows {
                attempts.push(serde_json::from_str(&raw?).map_err(|error| {
                    StoreError::Invariant(format!("invalid cancellation attempt record: {error}"))
                })?);
            }
        }
        Ok(Some(StoredCancellationDeliverySnapshot {
            run,
            delivery,
            session_lease,
            attempts,
        }))
    }
}
