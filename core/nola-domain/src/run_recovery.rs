// SPDX-License-Identifier: Apache-2.0
//! Validated current Run facts for startup recovery, independent of storage.

use crate::AgentTurnStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunRecoveryFacts {
    pub has_session_lease: bool,
    pub has_canonical_run: bool,
    /// A remote Run with no local invocation or locally owned effects.
    pub external_run: bool,
    /// Application verified owner, configuration, current Turn and effect mappings.
    pub checkpoint_verified: bool,
    pub turn_status: Option<AgentTurnStatus>,
    pub pending_effect_count: usize,
    /// Open questions verified against original Store sources and checkpoint.
    /// These require user input, not an executable Host effect.
    pub open_user_input_wait_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunRecoveryRejection {
    UnverifiedCheckpoint,
    MissingCurrentTurn,
    IdleCurrentTurn,
    MissingPendingEffect,
    /// A terminal checkpoint is not a committed Run result/seal.
    MissingTerminalCommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunRecoveryAction {
    /// Keep durable state. This does not authorize dispatch or retry.
    Preserve,
    /// Report inconsistent recovery facts without synthesizing a terminal state.
    Reject(RunRecoveryRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceRecoveryFacts {
    pub has_session_lease: bool,
    pub external_run: bool,
    /// Exact current Core Turn mapping for a verified, preserved canonical Run.
    pub matches_preserved_current_turn: bool,
}
