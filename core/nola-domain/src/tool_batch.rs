// SPDX-License-Identifier: Apache-2.0
//! Minimal facts for tool admission; no Host transport or executable payload.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolBatchCandidateState {
    Pending,
    Running,
    AwaitingPermission,
    /// Includes ready and terminal calls, neither of which is newly admitted.
    Inactive,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolBatchCapabilityFacts {
    pub available: bool,
    pub is_tool: bool,
    pub concurrent_safe: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolBatchCandidate {
    pub call_id: String,
    pub state: ToolBatchCandidateState,
    pub capability: ToolBatchCapabilityFacts,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolBatchSelection {
    pub call_ids: Vec<String>,
}
