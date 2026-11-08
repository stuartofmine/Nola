// SPDX-License-Identifier: Apache-2.0
//! Durable Host-effect delivery admission and lease mutation contracts.
use nola_domain::{EffectDeliveryAttempt, EffectDeliveryAttemptMutation};

/// Stable keyset for the bounded pending-effect scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectDeliveryCandidateCursor {
    pub created_at_ms: i64,
    pub effect_id: String,
}

/// Stable keyset for interrupted tail attempts that may create a successor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptedEffectDeliveryCursor {
    pub attempt_id: String,
}

/// Identity-only head of an outbox effect with no delivery-attempt history.
/// Request bodies are loaded only after applying the claim window and filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectDeliveryCandidate {
    pub effect_id: String,
    pub run_id: String,
    pub request_id: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEffectWorkReason {
    Pending,
    Retry,
    Resume,
    RecoverExpiredLease,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEffectWorkRunHead {
    pub conversation_id: String,
    pub run_id: String,
    pub effect_id: String,
    pub request_id: String,
    pub attempt_id: Option<String>,
    pub reason: RuntimeEffectWorkReason,
    pub ready_at_ms: i64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEffectWorkSchedule {
    pub runs: Vec<RuntimeEffectWorkRunHead>,
    pub next_wake_at_ms: Option<i64>,
}

/// Lease admission and every later worker mutation join the caller's write
/// unit of work. Implementations must not commit an enclosing command receipt.
pub trait EffectDeliveryRepository {
    type Error: std::fmt::Display;
    fn unknown_effect_recovery_receipt(&self, resolution_id: &str) -> Result<Option<(String, String)>, Self::Error>;
    fn record_unknown_effect_recovery_receipt(&mut self, resolution_id: &str, request_digest: &str, receipt_json: &str) -> Result<(), Self::Error>;

    /// Returns a bounded, stable cross-Run work schedule. Each Run contributes
    /// at most one runnable head; active leases contribute only the nearest
    /// future reconciliation deadline.
    fn runtime_effect_work_schedule(
        &self,
        conversation_ids: &[String],
        now_ms: i64,
        limit: usize,
    ) -> Result<RuntimeEffectWorkSchedule, Self::Error>;

    /// Reads at most `limit` initial-delivery candidates in stable key order.
    fn pending_effect_delivery_candidates(
        &self,
        run_id: &str,
        after: Option<&EffectDeliveryCandidateCursor>,
        limit: usize,
    ) -> Result<Vec<EffectDeliveryCandidate>, Self::Error>;

    fn effect_delivery_attempt(
        &self,
        attempt_id: &str,
    ) -> Result<Option<EffectDeliveryAttempt>, Self::Error>;

    /// Returns the sole Prepared/Leased/ExecutionStarted attempt, if present.
    fn active_effect_delivery_attempt(
        &self,
        effect_id: &str,
    ) -> Result<Option<EffectDeliveryAttempt>, Self::Error>;

    /// Returns interrupted tail attempts whose typed recovery plan authorizes
    /// Retry or Resume and whose logical outbox effect is currently pending.
    /// A resumed Core effect may bind a fresh request ID; successor admission
    /// validates that current request separately from predecessor evidence.
    /// `include_unrebound` is only for Core recovery preparation. Physical
    /// successor claims must pass false and require the fresh request binding.
    fn interrupted_effect_delivery_candidates(
        &self,
        run_id: &str,
        after: Option<&InterruptedEffectDeliveryCursor>,
        limit: usize,
        include_unrebound: bool,
    ) -> Result<Vec<EffectDeliveryAttempt>, Self::Error>;

    /// Atomically claims a bounded batch of previously unattempted outbox
    /// effects. Every mutation must insert attempt number one in Leased state.
    fn claim_effect_delivery_attempts(
        &mut self,
        claims: &[EffectDeliveryAttemptMutation],
    ) -> Result<Vec<EffectDeliveryAttempt>, Self::Error>;

    /// Atomically renews one active attempt with an exact revision CAS and
    /// appends its canonical Run event in the caller's write unit of work.
    fn heartbeat_effect_delivery_attempt(
        &mut self,
        heartbeat: &EffectDeliveryAttemptMutation,
    ) -> Result<EffectDeliveryAttempt, Self::Error>;

    /// Atomically authorizes one evidence-backed Retry from OutcomeUnknown.
    /// The adapter must CAS the exact attempt and verify that its run/effect/request
    /// still owns the current pending outbox row before appending the Run event.
    fn authorize_unknown_effect_retry(
        &mut self,
        authorization: &EffectDeliveryAttemptMutation,
    ) -> Result<EffectDeliveryAttempt, Self::Error>;

    /// Atomically applies renew/start/settle/recovery mutations. Reclaim uses
    /// one batch containing the old interruption followed by its successor.
    fn commit_effect_delivery_attempt_mutations(
        &mut self,
        mutations: &[EffectDeliveryAttemptMutation],
    ) -> Result<Vec<EffectDeliveryAttempt>, Self::Error>;
}
