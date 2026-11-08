// SPDX-License-Identifier: Apache-2.0
//! Transaction-facing reducer capability. No publication or effect execution.
use crate::agent::{AgentCore, AgentCoreConfig, AgentThreadState};
use crate::agent::protocol::{AgentReduction, AgentSubmission};
use crate::agent::recovery::{CheckpointReason, CoreCheckpoint};

/// An owned candidate is adopted only after Application commits its facts.
/// Failure consumes it, preventing accidental reuse of partially reduced state.
pub trait AgentReducerPort: Sized {
    fn recover(config: AgentCoreConfig, checkpoint: CoreCheckpoint<AgentThreadState>)
        -> Result<Self, String>;
    fn reduce_candidate(self, input: AgentSubmission, observed_at_ms: u64)
        -> Result<(Self, AgentReduction), String>;
    fn recovery_checkpoint(&self, reason: CheckpointReason)
        -> CoreCheckpoint<AgentThreadState>;
}

impl AgentReducerPort for AgentCore {
    fn recover(config: AgentCoreConfig, checkpoint: CoreCheckpoint<AgentThreadState>)
        -> Result<Self, String> {
        Self::restore(config, checkpoint)
    }
    fn reduce_candidate(self, input: AgentSubmission, observed_at_ms: u64)
        -> Result<(Self, AgentReduction), String> {
        self.reduce_owned_at(input, observed_at_ms)
    }
    fn recovery_checkpoint(&self, reason: CheckpointReason)
        -> CoreCheckpoint<AgentThreadState> {
        self.checkpoint(reason)
    }
}
