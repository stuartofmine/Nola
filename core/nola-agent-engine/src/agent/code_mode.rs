// SPDX-License-Identifier: Apache-2.0
//! Deterministic cell callback sequencing. Registration is NOT permission or
//! dispatch; callers must feed registered calls into ordinary tool admission.
//! This state belongs inside the Core checkpoint, not a separate task database.

use nola_domain::code_mode::{CodeCellIdentity, CodeCellLimits, CodeNestedCall};
pub use nola_domain::code_mode::{CodeCellPhase, CodeCellCompletionDetails, CodeCellView};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A capability-aware discovery affordance, not a requirement to use cells.
pub const DISCOVERY_HINT: &str = "For loops, dependent calls or parallel tool-result processing, you may discover Code Mode with tool_search query select:code_mode_exec,code_mode_wait. It runs bounded JavaScript over authorized tools, not native processes or filesystem APIs. Each nested call still needs its ordinary authorization. Use ordinary tools or bash directly when simpler; Code Mode is optional.";

/// One declaration shared by concrete Hosts. The outer computation owns no
/// file/network grant; every child still enters normal Core admission.
pub fn capability_descriptors(limits: &CodeCellLimits)
    -> Result<Vec<super::host::HostCapabilityDescriptor>, String>
{
    use super::host::*;
    limits.validate()?;
    let mut exec = limits.exec_input_schema()?;
    exec["description"] = serde_json::json!("Run bounded JavaScript for loops, dependent calls or parallel processing. ALL_TOOLS is a read-only ARRAY of {name, description, input_schema}, not a name-keyed object; use ALL_TOOLS.find(t => t.name === name) or filter/map. Call await tools[entry.name](arguments) matching entry.input_schema. Tool results are structured JSON: inspect their actual fields before parsing file content; do not assume a string or content property. Emit selected results with text(value). Each nested call retains ordinary permissions. No native filesystem, network, process or modules. A running result is not completion: use code_mode_wait on its cell_id, never rerun source just because observation timed out.");
    let mut wait = nola_domain::code_mode::CodeCellWaitRequest::input_schema();
    wait["description"] = serde_json::json!("Observe an existing code cell in this conversation without rerunning its source. Pass next_output_sequence as the next cursor; drain has_more_output even after terminal. Repeat a cursor to retry a lost response. settled means child effects are reconciled, not that execution succeeded. terminate requests cancellation, not rollback.");
    Ok([("code_mode_exec", exec), ("code_mode_wait", wait)].into_iter().map(|(id, input_schema)| {
        HostCapabilityDescriptor {
            id: id.into(), title: id.into(), route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Deferred,
            permission_domain: None,
            input_schema,
            input_normalization: HostInputNormalization {
                code_mode_limits: (id == "code_mode_exec").then(|| limits.clone()),
                transaction_boundary: HostCapabilityTransactionBoundary::HostState,
                origin: HostCapabilityOrigin {
                    kind: HostCapabilityOriginKind::Builtin, provider_id: Some("nola".into()),
                    provider_title: Some("Nola".into()), namespace: Some("code_mode".into()),
                },
                ..Default::default()
            },
            output_policy: HostOutputPolicy {
                preview_char_limit: 8_000, persist_full_output: true, allow_artifacts: false,
            },
            supports_streaming: true, supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: if id == "code_mode_exec" { HostReplaySafety::RequiresRecoveryDecision }
                else { HostReplaySafety::Idempotent },
        }
    }).collect())
}


/// Foreground delivery is independent from worker completion. A yielded
/// parent gets one model result; later cell output is read by distinct waits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodeCellForeground {
    AwaitingReturn,
    Yielded { output_cursor: u64 },
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_pages_are_retryable_and_never_skip_emissions() {
        let mut state = cell();
        let owner = state.identity().clone();
        for sequence in 1..=70 {
            state.append_output(nola_domain::code_mode::CodeOutputFrame {
                cell: owner.clone(), chunk: nola_domain::code_mode::CodeOutputChunk {
                    sequence, text: String::new(),
                },
            }).unwrap();
        }
        let first = state.view(&owner, 0).unwrap();
        assert_eq!(first.output.len(), 64);
        assert_eq!(first.next_output_sequence, 64);
        assert!(first.has_more_output);
        assert_eq!(state.view(&owner, 0).unwrap(), first);
        let second = state.view(&owner, first.next_output_sequence).unwrap();
        assert_eq!(second.output.len(), 6);
        assert_eq!(second.next_output_sequence, 70);
        assert!(!second.has_more_output);
        assert!(state.view(&owner, 70).unwrap().output.is_empty());
    }

    fn cell() -> CodeCellState {
        CodeCellState::new(CodeCellIdentity { host_id: "h".into(), conversation_id: "s".into(),
            turn_id: "t".into(), parent_call_id: "p".into(), cell_id: "c".into(), generation: "g".into() },
            CodeCellLimits { source_bytes: 1024, argument_bytes: 1024, result_bytes: 1024,
                output_bytes: 1024, heap_bytes: 1024 * 1024, wall_time_ms: 1000,
                max_calls: 3, max_in_flight: 1 }, "text(1)").unwrap()
    }

    #[test]
    fn failed_cell_observation_retains_error_and_archive_after_reload() {
        let mut state = cell();
        let owner = state.identity().clone();
        assert!(state.record_completion_details(&serde_json::json!({"error":"failed"}), None).is_err());
        state.finish(&owner, CodeCellPhase::Failed).unwrap();
        state.record_completion_details(&serde_json::json!({"error":"execution deadline exceeded"}),
            Some("artifact:code-output")).unwrap();
        let encoded = serde_json::to_vec(&state).unwrap();
        let mut restored: CodeCellState = serde_json::from_slice(&encoded).unwrap();
        restored.validate().unwrap();
        let view = restored.view(&owner, 0).unwrap();
        assert_eq!(view.completion.error.as_deref(), Some("execution deadline exceeded"));
        assert_eq!(view.completion.full_output_ref.as_deref(), Some("artifact:code-output"));
        assert!(restored.record_completion_details(&serde_json::json!({"error":"x".repeat(2048)}), None).is_err());
        assert_eq!(restored.view(&owner, 0).unwrap(), view);
    }

    #[test]
    fn yield_is_durable_once_only_and_does_not_finish_the_worker() {
        let mut state = cell();
        let owner = state.identity().clone();
        assert!(state.record_yield(&owner, 1).is_err());
        assert!(!state.has_yielded());
        assert!(state.record_yield(&owner, 0).unwrap());
        assert!(!state.record_yield(&owner, 0).unwrap());
        assert!(state.record_yield(&owner, 1).is_err());
        assert_eq!(state.phase(), &CodeCellPhase::Running);
        let mut state: CodeCellState = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        state.validate().unwrap();
        assert!(state.has_yielded());
        state.register_call(CodeNestedCall { cell:owner.clone(), sequence:1,
            tool_name:"read_file".into(), arguments:serde_json::json!({}) }).unwrap();
        state.settle_call(&owner, 1).unwrap();
        state.finish(&owner, CodeCellPhase::Completed).unwrap();
        assert!(!state.record_yield(&owner, 0).unwrap());
        let mut fresh = cell();
        fresh.finish(&owner, CodeCellPhase::Completed).unwrap();
        assert!(fresh.record_yield(&owner, 0).is_err(), "terminal-first cannot yield later");
        let mut wrong_owner = owner;
        wrong_owner.conversation_id = "another-session".into();
        assert!(state.record_yield(&wrong_owner, 0).is_err());
    }

    #[test]
    fn cancellation_receipts_survive_reload_and_require_every_exact_target() {
        let mut state = cell();
        let owner = state.identity().clone();
        state.track_cancellation(&owner, &["parent-effect".into(), "child-effect".into()]).unwrap();
        assert!(!state.cancellation_confirmed());
        let bytes = serde_json::to_vec(&state).unwrap();
        let mut state: CodeCellState = serde_json::from_slice(&bytes).unwrap();
        state.validate().unwrap();
        assert!(!state.acknowledge_cancelled_effect("sibling-effect"));
        assert!(state.acknowledge_cancelled_effect("parent-effect"));
        assert!(!state.acknowledge_cancelled_effect("parent-effect"));
        assert!(state.finish(&owner, CodeCellPhase::Cancelled).is_err());
        assert!(state.acknowledge_cancelled_effect("child-effect"));
        assert!(state.cancellation_confirmed());
        state.finish(&owner, CodeCellPhase::Cancelled).unwrap();
        state.validate().unwrap();
    }

    #[test]
    fn cancellation_scope_does_not_capture_sibling_or_observer_calls() {
        let mut state = cell();
        let owner = state.identity().clone();
        state.register_call(CodeNestedCall { cell:owner.clone(), sequence:1,
            tool_name:"read_file".into(), arguments:serde_json::json!({}) }).unwrap();
        let child = state.nested_call_id(1);
        assert!(state.owns_execution_call(&owner.parent_call_id));
        assert!(state.owns_execution_call(&child));
        assert!(!state.owns_execution_call("wait-observer"));
        assert!(!state.owns_execution_call("another-parent"));
        let mut sibling_owner = owner.clone();
        sibling_owner.cell_id = "sibling".into();
        let sibling = CodeCellState::new(sibling_owner, CodeCellLimits::default(), "text(1)").unwrap();
        assert!(!state.owns_execution_call(&sibling.nested_call_id(1)));
        state.settle_call(&owner, 1).unwrap();
        assert!(state.owns_execution_call(&child));
    }

    #[test]
    fn catalog_declares_frozen_budget_without_outer_resource_authority() {
        use super::super::host::{HostCapabilityTransactionBoundary, HostReplaySafety, HostCapabilityExposure};
        let limits = CodeCellLimits::default();
        let descriptors = capability_descriptors(&limits).unwrap();
        let exec = &descriptors[0];
        let wait = &descriptors[1];
        assert_eq!(exec.id, "code_mode_exec");
        assert_eq!(wait.id, "code_mode_wait");
        assert_eq!(exec.input_normalization.code_mode_limits.as_ref(), Some(&limits));
        assert!(wait.input_normalization.code_mode_limits.is_none());
        assert!(exec.permission_domain.is_none());
        assert_eq!(exec.exposure, HostCapabilityExposure::Deferred);
        assert_eq!(exec.input_normalization.transaction_boundary, HostCapabilityTransactionBoundary::HostState);
        assert_eq!(exec.replay_safety, HostReplaySafety::RequiresRecoveryDecision);
        assert_eq!(wait.replay_safety, HostReplaySafety::Idempotent);
        assert!(exec.validate_input(&serde_json::json!({"source":"text(1)"})).is_ok());
        assert!(wait.validate_input(&serde_json::json!({"cell_id":"c","after_output_sequence":0,"wait_ms":0,"terminate":false})).is_ok());
        assert!(wait.validate_input(&serde_json::json!({"cell_id":"c","source":"rerun()"})).is_err());
    }

    #[test]
    fn explicit_output_is_ordered_bounded_idempotent_and_cold_readable() {
        use nola_domain::code_mode::{CodeOutputFrame, CodeOutputChunk};
        let mut state = cell();
        let owner = state.identity().clone();
        let frame = CodeOutputFrame { cell: owner.clone(), chunk: CodeOutputChunk {
            sequence: 1, text: "结果🙂".into(),
        }};
        state.append_output(frame.clone()).unwrap();
        state.append_output(frame.clone()).unwrap();
        assert_eq!(state.output_after(&owner, 0).unwrap().len(), 1);
        assert!(state.output_after(&owner, 1).unwrap().is_empty());
        assert!(state.output_after(&owner, 2).is_err());
        let before = state.clone();
        let mut conflict = frame.clone();
        conflict.chunk.text = "different".into();
        assert!(state.append_output(conflict).is_err());
        let mut gap = frame.clone();
        gap.chunk.sequence = 3;
        assert!(state.append_output(gap).is_err());
        let mut foreign = frame.clone();
        foreign.cell.generation = "replacement".into();
        assert!(state.append_output(foreign.clone()).is_err());
        assert!(state.output_after(&foreign.cell, 0).is_err());
        let mut oversized = frame.clone();
        oversized.chunk.sequence = 2;
        oversized.chunk.text = "x".repeat(1024);
        assert!(state.append_output(oversized).is_err());
        assert_eq!(state, before);
        state.finish(&owner, CodeCellPhase::Completed).unwrap();
        let restored: CodeCellState = serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.output_after(&owner, 0).unwrap(), &[frame.chunk]);
        assert_eq!(restored.output_after(&owner, 0).unwrap(), state.output_after(&owner, 0).unwrap());
        let mut corrupt = serde_json::to_value(&restored).unwrap();
        corrupt["output_bytes"] = serde_json::json!(0);
        assert!(serde_json::from_value::<CodeCellState>(corrupt).unwrap().validate().is_err());
    }

    #[test]
    fn callbacks_do_not_replay_or_finish_with_unsettled_effects() {
        let mut state = cell();
        let owner = state.identity().clone();
        let call = CodeNestedCall { cell: owner.clone(), sequence: 1,
            tool_name: "read_file".into(), arguments: serde_json::json!({"path":"file"}) };
        state.register_call(call.clone()).unwrap();
        let before = state.clone();
        assert!(state.register_call(call).is_err());
        assert!(state.finish(&owner, CodeCellPhase::Completed).is_err());
        assert_eq!(state, before);
        state.request_cancel(&owner).unwrap();
        assert!(state.finish(&owner, CodeCellPhase::Cancelled).is_err());
        state.settle_call(&owner, 1).unwrap();
        state.finish(&owner, CodeCellPhase::Cancelled).unwrap();
        assert!(state.is_terminal());
        assert!(state.validate().is_ok());
        assert!(state.finish(&owner, CodeCellPhase::Completed).is_err());
    }

    #[test]
    fn interrupted_checkpoint_retains_unresolved_calls_without_restart() {
        let mut state = cell();
        let owner = state.identity().clone();
        state.register_call(CodeNestedCall { cell: owner.clone(), sequence: 1,
            tool_name: "write_file".into(), arguments: serde_json::json!({}) }).unwrap();
        state.finish(&owner, CodeCellPhase::Interrupted).unwrap();
        let encoded = serde_json::to_value(&state).unwrap();
        let restored: CodeCellState = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(restored, state);
        assert!(restored.validate().is_ok());
        assert!(restored.is_terminal());
        assert!(!restored.is_settled());
        assert!(!restored.view(&owner, 0).unwrap().settled);
        let mut reconciled = restored.clone();
        let mut foreign = owner.clone();
        foreign.generation = "foreign".into();
        assert!(reconciled.reconcile_interrupted_call(&foreign, 1).is_err());
        assert_eq!(reconciled, restored);
        reconciled.reconcile_interrupted_call(&owner, 1).unwrap();
        assert_eq!(reconciled.phase(), &CodeCellPhase::Interrupted);
        assert!(reconciled.is_settled());
        assert!(reconciled.view(&owner, 0).unwrap().settled);
        assert!(reconciled.pending_calls().next().is_none());
        assert!(reconciled.reconcile_interrupted_call(&owner, 1).is_err());
        let mut corrupt = encoded;
        corrupt["phase"] = serde_json::json!("completed");
        let corrupt: CodeCellState = serde_json::from_value(corrupt).unwrap();
        assert!(corrupt.validate().is_err());
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellState {
    identity: CodeCellIdentity,
    limits: CodeCellLimits,
    phase: CodeCellPhase,
    foreground: CodeCellForeground,
    completion: CodeCellCompletionDetails,
    last_sequence: u32,
    /// Call identifiers/arguments only. The existing tool lifecycle and effect
    /// journal remain the authority for permission, execution and results.
    pending: BTreeMap<u32, CodeNestedCall>,
    output: Vec<nola_domain::code_mode::CodeOutputChunk>,
    output_bytes: u64,
    /// Exact effect receipts still required before cancellation is confirmed.
    /// This is durable coordination, never evidence that effects rolled back.
    cancellation_effects: BTreeSet<String>,
}

impl CodeCellState {
    pub fn new(identity: CodeCellIdentity, limits: CodeCellLimits, source: &str) -> Result<Self, String> {
        identity.validate()?;
        limits.validate_source(source)?;
        Ok(Self { identity, limits, phase: CodeCellPhase::Running,
            foreground: CodeCellForeground::AwaitingReturn,
            completion: CodeCellCompletionDetails::default(),
            last_sequence: 0, pending: BTreeMap::new(), output: Vec::new(), output_bytes: 0,
            cancellation_effects: BTreeSet::new() })
    }

    pub fn identity(&self) -> &CodeCellIdentity { &self.identity }
    pub fn phase(&self) -> &CodeCellPhase { &self.phase }

    pub fn has_yielded(&self) -> bool {
        matches!(self.foreground, CodeCellForeground::Yielded { .. })
    }

    pub fn record_completion_details(&mut self, output: &serde_json::Value, full_output_ref: Option<&str>) -> Result<(), String> {
        if !self.is_terminal() { return Err("code_cell_completion_requires_terminal_state".into()); }
        let error = output.get("error").filter(|value| !value.is_null()).map(|value|
            value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string()));
        let details = CodeCellCompletionDetails { error, full_output_ref:full_output_ref.map(str::to_owned) };
        let size = serde_json::to_vec(&details).map_err(|error| error.to_string())?.len();
        if details != CodeCellCompletionDetails::default() && size > self.limits.result_bytes as usize {
            return Err("code_cell_completion_budget_exceeded".into());
        }
        self.completion = details;
        Ok(())
    }

    /// Returns true exactly once. Core must commit the foreground ToolResult
    /// in the same transaction; Host callers cannot write this state directly.
    pub fn record_yield(&mut self, owner: &CodeCellIdentity, cursor: u64) -> Result<bool, String> {
        if owner != &self.identity { return Err("code_cell_owner_mismatch".into()); }
        if let CodeCellForeground::Yielded { output_cursor } = self.foreground {
            return if output_cursor == cursor { Ok(false) } else { Err("code_cell_yield_conflict".into()) };
        }
        if self.phase != CodeCellPhase::Running { return Err("code_cell_not_running".into()); }
        if cursor != self.output.len() as u64 { return Err("code_cell_yield_cursor_mismatch".into()); }
        self.foreground = CodeCellForeground::Yielded { output_cursor: cursor };
        Ok(true)
    }

    /// Commit explicit JS text only, never implicit nested tool results.
    /// Repeated delivery of the exact chunk is idempotent; conflicting replay
    /// and sequence gaps cannot change a previously committed prefix.
    pub fn append_output(&mut self, frame: nola_domain::code_mode::CodeOutputFrame) -> Result<(), String> {
        if frame.cell != self.identity { return Err("code_cell_owner_mismatch".into()); }
        if frame.chunk.sequence == 0 { return Err("code_cell_output_sequence_mismatch".into()); }
        if let Some(previous) = self.output.iter().find(|chunk| chunk.sequence == frame.chunk.sequence) {
            return if previous == &frame.chunk { Ok(()) } else { Err("code_cell_output_conflict".into()) };
        }
        if self.is_terminal() { return Err("code_cell_already_terminal".into()); }
        if frame.chunk.sequence != self.output.len() as u64 + 1 {
            return Err("code_cell_output_sequence_mismatch".into());
        }
        let bytes = self.output_bytes.checked_add(frame.chunk.text.len().max(1) as u64)
            .ok_or("code_cell_output_budget_exceeded")?;
        if bytes > u64::from(self.limits.output_bytes) { return Err("code_cell_output_budget_exceeded".into()); }
        self.output.push(frame.chunk);
        self.output_bytes = bytes;
        Ok(())
    }

    /// Non-destructive cold-readable output. No worker or source replay is
    /// needed, including for interrupted cells with uncertain tool effects.
    pub fn output_after(&self, owner: &CodeCellIdentity, cursor: u64)
        -> Result<&[nola_domain::code_mode::CodeOutputChunk], String>
    {
        if owner != &self.identity { return Err("code_cell_owner_mismatch".into()); }
        let index = usize::try_from(cursor).map_err(|_| "invalid_code_cell_output_cursor")?;
        self.output.get(index..).ok_or_else(|| "invalid_code_cell_output_cursor".into())
    }

    pub fn view(&self, owner: &CodeCellIdentity, cursor: u64) -> Result<CodeCellView, String> {
        let remaining = self.output_after(owner, cursor)?;
        // Keep text emissions atomic: a single large emission is bounded by
        // the cell output budget. Between emissions, limit each observation
        // by both bytes and count (including zero-length emissions).
        const PAGE_BYTES: usize = 16 * 1024;
        const PAGE_CHUNKS: usize = 64;
        let mut bytes = 0usize;
        let mut output = Vec::new();
        for chunk in remaining {
            let size = chunk.text.len();
            if !output.is_empty() && (output.len() >= PAGE_CHUNKS
                || bytes.saturating_add(size) > PAGE_BYTES) { break; }
            bytes = bytes.saturating_add(size);
            output.push(chunk.clone());
        }
        let next_output_sequence = output.last().map_or(cursor, |chunk| chunk.sequence);
        Ok(CodeCellView {
            cell_id: self.identity.cell_id.clone(),
            parent_call_id: self.identity.parent_call_id.clone(),
            phase: self.phase.clone(), terminal: self.is_terminal(),
            settled: self.is_settled(),
            next_output_sequence,
            has_more_output: output.len() < remaining.len(), output,
            unsettled_calls: self.pending.len(),
            completion: self.completion.clone(),
        })
    }

    /// Transactional history publication only; model observers use view().
    pub fn committed_observation(&self) -> Result<CodeCellView, String> {
        self.validate()?;
        let mut view = self.view(&self.identity, 0)?;
        view.output = self.output.clone();
        view.next_output_sequence = self.output.len() as u64;
        view.has_more_output = false;
        Ok(view)
    }

    pub fn nested_call_id(&self, sequence: u32) -> String {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(&(&self.identity, sequence)).expect("serializable code cell identity");
        format!("code-call-{:x}", Sha256::digest(bytes))
    }

    pub fn pending_calls(&self) -> impl Iterator<Item = &CodeNestedCall> {
        self.pending.values()
    }

    /// Ownership survives settlement; pending alone cannot classify a result
    /// once its receipt has been delivered to JavaScript.
    pub fn owns_call_id(&self, call_id: &str) -> bool {
        (1..=self.last_sequence).any(|sequence| self.nested_call_id(sequence) == call_id)
    }

    /// Exact cancellation scope, derived only from admitted identities. A
    /// sibling cell, the observing wait call and unrelated Turn tools are not
    /// members. Completed children remain members for audit, not re-execution.
    pub fn owns_execution_call(&self, call_id: &str) -> bool {
        call_id == self.identity.parent_call_id || self.owns_call_id(call_id)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.phase, CodeCellPhase::Completed | CodeCellPhase::Failed
            | CodeCellPhase::Cancelled | CodeCellPhase::Interrupted)
    }

    /// Worker termination and effect settlement are distinct. Interrupted
    /// workers can retain child operations whose outcome is still unknown.
    pub fn is_settled(&self) -> bool {
        self.is_terminal() && self.pending.is_empty() && self.cancellation_effects.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        self.identity.validate()?;
        self.limits.validate()?;
        if (!self.is_terminal() && self.completion != CodeCellCompletionDetails::default())
            || (self.completion != CodeCellCompletionDetails::default()
                && serde_json::to_vec(&self.completion).map_err(|error| error.to_string())?.len() > self.limits.result_bytes as usize) {
            return Err("invalid_code_cell_completion_details".into());
        }
        if let CodeCellForeground::Yielded { output_cursor } = self.foreground {
            if output_cursor > self.output.len() as u64 {
                return Err("code_cell_yield_cursor_mismatch".into());
            }
        }
        if !self.cancellation_effects.is_empty()
            && !matches!(self.phase, CodeCellPhase::Cancelling | CodeCellPhase::Interrupted) {
            return Err("code_cell_has_unconfirmed_cancellation".into());
        }
        if self.cancellation_effects.iter().any(|id| id.trim().is_empty()) {
            return Err("invalid_code_cell_cancellation_effect".into());
        }
        let mut bytes = 0u64;
        for (index, chunk) in self.output.iter().enumerate() {
            if chunk.sequence != index as u64 + 1 { return Err("code_cell_output_sequence_mismatch".into()); }
            bytes = bytes.checked_add(chunk.text.len().max(1) as u64).ok_or("code_cell_output_budget_exceeded")?;
        }
        if bytes != self.output_bytes || bytes > u64::from(self.limits.output_bytes) {
            return Err("invalid_code_cell_output_accounting".into());
        }
        if self.last_sequence > self.limits.max_calls
            || self.pending.len() > self.limits.max_in_flight as usize {
            return Err("invalid_code_cell_call_accounting".into());
        }
        for (sequence, call) in &self.pending {
            call.validate(&self.identity, &self.limits)?;
            if *sequence != call.sequence || *sequence > self.last_sequence {
                return Err("invalid_code_cell_pending_identity".into());
            }
        }
        if self.is_terminal() && self.phase != CodeCellPhase::Interrupted && !self.pending.is_empty() {
            return Err("code_cell_terminal_has_unsettled_calls".into());
        }
        Ok(())
    }

    /// The caller persists this transition before planning permission/resource
    /// effects. Duplicate callbacks cannot cause a second tool dispatch.
    pub fn register_call(&mut self, call: CodeNestedCall) -> Result<(), String> {
        call.validate(&self.identity, &self.limits)?;
        if self.phase != CodeCellPhase::Running { return Err("code_cell_not_running".into()); }
        if self.last_sequence.checked_add(1) != Some(call.sequence) {
            return Err("code_cell_call_sequence_mismatch".into());
        }
        if self.pending.len() >= self.limits.max_in_flight as usize {
            return Err("code_cell_concurrency_exceeded".into());
        }
        self.last_sequence = call.sequence;
        self.pending.insert(call.sequence, call);
        Ok(())
    }

    /// Called only after the ordinary tool lifecycle accepts its exact result.
    /// A raw JS/Host message is not sufficient evidence for this transition.
    pub fn settle_call(&mut self, owner: &CodeCellIdentity, sequence: u32) -> Result<(), String> {
        if owner != &self.identity { return Err("code_cell_owner_mismatch".into()); }
        if !matches!(self.phase, CodeCellPhase::Running | CodeCellPhase::Cancelling) {
            return Err("code_cell_not_running".into());
        }
        if self.pending.remove(&sequence).is_none() { return Err("unknown_code_cell_call".into()); }
        Ok(())
    }

    /// Core reconciliation only: the worker remains Interrupted and receives
    /// no callback. The caller must verify the child lifecycle and Host journal.
    pub(crate) fn reconcile_interrupted_call(&mut self, owner: &CodeCellIdentity, sequence: u32) -> Result<(), String> {
        if owner != &self.identity { return Err("code_cell_owner_mismatch".into()); }
        if self.phase != CodeCellPhase::Interrupted { return Err("code_cell_not_interrupted".into()); }
        if self.pending.remove(&sequence).is_none() { return Err("unknown_code_cell_call".into()); }
        Ok(())
    }

    pub fn request_cancel(&mut self, owner: &CodeCellIdentity) -> Result<Vec<CodeNestedCall>, String> {
        if owner != &self.identity { return Err("code_cell_owner_mismatch".into()); }
        match self.phase {
            CodeCellPhase::Running => self.phase = CodeCellPhase::Cancelling,
            CodeCellPhase::Cancelling => {},
            _ => return Err("code_cell_already_terminal".into()),
        }
        Ok(self.pending.values().cloned().collect())
    }

    /// Called with Core-selected journal targets in the same transaction as
    /// the cancellation outbox. Retries may add targets but never forget them.
    pub fn track_cancellation(&mut self, owner: &CodeCellIdentity, targets: &[String]) -> Result<(), String> {
        if targets.iter().any(|id| id.trim().is_empty()) {
            return Err("invalid_code_cell_cancellation_effect".into());
        }
        self.request_cancel(owner)?;
        self.cancellation_effects.extend(targets.iter().cloned());
        Ok(())
    }

    /// Only the validated effect-journal ACK handler may consume a receipt.
    /// Unknown receipts do not advance this cell's state.
    pub fn acknowledge_cancelled_effect(&mut self, target: &str) -> bool {
        self.cancellation_effects.remove(target)
    }

    pub fn cancellation_confirmed(&self) -> bool {
        self.phase == CodeCellPhase::Cancelling && self.cancellation_effects.is_empty()
    }

    pub fn pending_cancellation_effects(&self) -> impl Iterator<Item = &str> {
        self.cancellation_effects.iter().map(String::as_str)
    }

    pub fn finish(&mut self, owner: &CodeCellIdentity, terminal: CodeCellPhase) -> Result<(), String> {
        if owner != &self.identity { return Err("code_cell_owner_mismatch".into()); }
        let valid = match (&self.phase, &terminal) {
            (CodeCellPhase::Running, CodeCellPhase::Completed | CodeCellPhase::Failed) => self.pending.is_empty(),
            (CodeCellPhase::Cancelling, CodeCellPhase::Cancelled) => self.pending.is_empty()
                && self.cancellation_effects.is_empty(),
            (CodeCellPhase::Running | CodeCellPhase::Cancelling, CodeCellPhase::Interrupted) => true,
            _ => false,
        };
        if !valid { return Err("invalid_code_cell_terminal_transition".into()); }
        // Interrupted cells retain pending identities for reconciliation. Never
        // turn them back into Running or replay their source after restart.
        self.phase = terminal;
        Ok(())
    }
}
