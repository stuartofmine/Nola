// SPDX-License-Identifier: Apache-2.0
//! Exact Run header and aggregate delivery facts from one pinned read snapshot.
pub struct StoredRunRecoveryInspection {
    pub conversation_id: String,
    pub assistant_message_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub completed_at_ms: Option<i64>,
    pub terminal: Option<crate::remote_session::CanonicalRunTerminal>,
    pub assistant_owner_valid: bool,
    pub checkpoint_present: bool,
    pub pending_effect_count: u64,
    pub source_host_id: Option<String>,
    pub has_session_lease: bool,
    pub deliveries: Vec<nola_domain::RunDeliveryStateCount>,
}

/// Internal persistence record, never a client response. Application must verify
/// the config digest and expose only its validated non-secret invocation binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredExactRunInvocation {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub status: String,
    pub completed_at_ms: Option<i64>,
    /// Validated immutable result seal, read under the same snapshot as the
    /// invocation and checkpoint; never inferred from a terminal-looking label.
    pub terminal: Option<crate::remote_session::CanonicalRunTerminal>,
    pub invocation_config_json: String,
    pub invocation_config_digest: String,
    /// Actual checkpoint row and source witnesses from the same pinned read.
    /// The Application verifies JSON identities; storage never invents aliases.
    pub core_checkpoint: Option<StoredExactRunCoreCheckpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredExactRunCoreCheckpoint {
    pub recovery: crate::remote_session::StoredLocalSessionRecovery,
    /// Optional read-only materialization previously validated for this exact
    /// pinned database snapshot. Never a persisted checkpoint or an execution
    /// shortcut. Application must still validate all Run/config/identity proofs.
    pub recovered_checkpoint: Option<crate::run_effect::StoredCheckpoint>,
    pub admission_turn_id: Option<String>,
    pub latest_started_turn_id: Option<String>,
    pub latest_started_session_id: Option<String>,
    pub committed_by_run: bool,
    /// First actual TurnStarted fact for this exact Run, never a Store revision
    /// convention or a value inferred from the currently installed checkpoint.
    pub initial_admission: Option<StoredInitialCoreAdmission>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredInitialCoreAdmission {
    pub session_id: String,
    pub turn_id: String,
    pub revision: u64,
    pub event_cursor: u64,
}

pub trait RunRecoveryInspectionRepository {
    type Error: std::fmt::Display;
    /// Keyset-paginated local unfinished Runs. Discovery grants no execution
    /// authority; each result still requires exact invocation validation.
    fn list_recoverable_runs(&self, after_run_id: Option<&str>, limit: usize)
        -> Result<Vec<RecoverableRunRef>, Self::Error>;
    /// Same-snapshot local Run ownership and only latest unknown attempts of
    /// pending effects. No body, credential, or historical attempt scan.
    fn inspect_unknown_effect_recovery(
        &self, conversation_id: &str, run_id: &str, assistant_message_id: &str,
    ) -> Result<Option<StoredUnknownEffectRecoveryInspection>, Self::Error>;
    fn inspect_exact_run(
        &self,
        run_id: &str,
    ) -> Result<Option<StoredRunRecoveryInspection>, Self::Error>;

    /// Reads one exact locally owned conversation Run in one snapshot. `None`
    /// means no Run row; mismatched identities, remote ownership, and missing
    /// frozen configuration are errors, never a request to use current settings.
    fn load_exact_run_invocation(
        &self,
        conversation_id: &str,
        run_id: &str,
        assistant_message_id: &str,
    ) -> Result<Option<StoredExactRunInvocation>, Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverableRunRef {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
}

pub struct StoredUnknownEffectRecoveryInspection {
    pub invocation_config_digest: String,
    pub targets: Vec<nola_domain::unknown_effect_recovery::UnknownEffectRecoveryTarget>,
    pub independent_pending_effects: u64,
    pub retry_allowed: bool,
}
