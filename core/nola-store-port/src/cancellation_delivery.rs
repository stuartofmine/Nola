// SPDX-License-Identifier: Apache-2.0
use crate::{
    run_effect::CommittedHostDeliverySnapshot,
    run_recovery_inspection::StoredRunRecoveryInspection,
    workspace_ownership::StoredSessionRunLease,
};
use nola_domain::EffectDeliveryAttempt;

pub struct StoredCancellationDeliverySnapshot {
    pub run: StoredRunRecoveryInspection,
    pub delivery: CommittedHostDeliverySnapshot<crate::remote_session::StoredLocalSessionRecovery>,
    pub session_lease: Option<StoredSessionRunLease>,
    pub attempts: Vec<EffectDeliveryAttempt>,
}

pub trait CancellationDeliveryRepository {
    type Error: std::fmt::Display;
    fn require_cancellation_write_unit_of_work(&self) -> Result<(), Self::Error>;
    /// Exact observed dead-owner release, in the cancellation claim UoW.
    /// No session-only or owner-only deletion may replace this five-field CAS.
    fn release_observed_cancellation_session_lease(
        &mut self,
        observed: &StoredSessionRunLease,
    ) -> Result<bool, Self::Error>;
    /// All identity, checkpoint, outbox and attempt facts share one snapshot.
    fn cancellation_delivery_snapshot(
        &self,
        run_id: &str,
    ) -> Result<Option<StoredCancellationDeliverySnapshot>, Self::Error>;
}
