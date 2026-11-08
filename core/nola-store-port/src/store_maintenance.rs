// SPDX-License-Identifier: Apache-2.0
//! Same-connection repository contracts used by the command dispatcher.

/// Reads join the caller snapshot; mutations join its receipt Unit of Work.
pub trait StoreMaintenanceRepository {
    type Error: std::fmt::Display;
    fn with_run_recovery_plan<F>(
        &mut self,
        conversation_id: Option<&str>,
        reason: &str,
        interrupted_at: &str,
        interrupted_at_ms: i64,
        planner: F,
    ) -> Result<usize, Self::Error>
    where
        F: FnMut(&RunRecoveryRecord) -> Result<RunRecoveryPlan, String>;
}

#[derive(Debug, Clone)]
pub struct RecoveryRunRecord {
    pub run_id: String,
    pub active_attempt_id: Option<String>,
    pub status: String,
    pub invocation_config_json: Option<String>,
    pub invocation_config_digest: Option<String>,
    /// None only when no TurnStarted exists. Empty marks a stored event with
    /// a missing or malformed Turn ID, so it cannot prove initial ownership.
    pub latest_started_turn_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RunRecoveryRecord {
    pub conversation_id: String,
    pub has_session_lease: bool,
    pub session_lease: Option<crate::workspace_ownership::StoredSessionRunLease>,
    pub run: Option<RecoveryRunRecord>,
    pub checkpoint: Option<crate::run_effect::StoredRecoveryCheckpoint>,
    pub effects: Vec<crate::run_effect::StoredOutboxEffect>,
    pub open_user_input_waits: Vec<crate::user_input_wait::StoredUserInputWait>,
    pub model_attempts: Vec<nola_domain::ModelAttemptRecord>,
    /// Pending delivery owners and nonterminal invocation records from this Run.
    pub delegated_invocations: Vec<nola_domain::DelegatedInvocationRecord>,
    pub delegated_permissions: Vec<nola_domain::DelegatedPermissionDeliveryRecord>,
    /// Nonterminal delivery leases and terminal recovery decisions for this Run.
    pub effect_delivery_attempts: Vec<nola_domain::EffectDeliveryAttempt>,
    pub traces: Vec<crate::trace_journal::StoredTraceJournalRun>,
}

#[derive(Debug, Clone, Default)]
pub struct RunRecoveryPlan {
    pub release_session_lease: Option<crate::workspace_ownership::StoredSessionRunLease>,
    pub delegated_permission_mutations: Vec<nola_domain::DelegatedPermissionDeliveryMutation>,
    pub effect_delivery_attempt_mutations: Vec<nola_domain::EffectDeliveryAttemptMutation>,
    pub preserve_all_traces: bool,
    pub preserve_trace_turn_id: Option<String>,
}
