// SPDX-License-Identifier: Apache-2.0
//! Code Mode identities and bounded requests. These contracts never grant tool
//! authority: every nested invocation must enter ordinary Core admission.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeCellPhase {
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellCompletionDetails {
    pub error: Option<String>,
    pub full_output_ref: Option<String>,
}

/// Read-only model observation. Internal worker identities and pending tool
/// arguments are deliberately absent; the cell ID is not an execution grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellView {
    pub cell_id: String,
    pub parent_call_id: String,
    pub phase: CodeCellPhase,
    pub terminal: bool,
    pub settled: bool,
    pub next_output_sequence: u64,
    pub has_more_output: bool,
    pub output: Vec<CodeOutputChunk>,
    pub unsettled_calls: usize,
    pub completion: CodeCellCompletionDetails,
}

impl CodeCellView {
    /// Shared by persisted observations and authenticated historical pages.
    pub fn validate_state(&self) -> Result<(), String> {
        if !valid_id(&self.cell_id) || !valid_id(&self.parent_call_id) {
            return Err("invalid code cell observation identity".into());
        }
        let terminal = matches!(self.phase, CodeCellPhase::Completed | CodeCellPhase::Failed
            | CodeCellPhase::Cancelled | CodeCellPhase::Interrupted);
        if self.terminal != terminal || (self.settled && (!terminal || self.unsettled_calls != 0)) {
            return Err("inconsistent code cell observation state".into());
        }
        if matches!(self.phase, CodeCellPhase::Completed | CodeCellPhase::Failed | CodeCellPhase::Cancelled)
            && !self.settled {
            return Err("finished code cell still has unresolved effects".into());
        }
        if !terminal && self.completion != Default::default() {
            return Err("live code cell carries terminal completion details".into());
        }
        Ok(())
    }

    /// Page a complete committed observation. Never apply this to a page:
    /// its indices no longer describe the complete output sequence.
    pub fn page(mut self, cursor: u64) -> Result<Self, String> {
        self.validate_state()?;
        if self.has_more_output || self.next_output_sequence != self.output.len() as u64
            || self.output.iter().enumerate().any(|(index, chunk)| chunk.sequence != index as u64 + 1) {
            return Err("code_cell_observation_is_not_complete".into());
        }
        let start = usize::try_from(cursor).map_err(|_| "invalid_code_cell_output_cursor")?;
        let remaining = self.output.get(start..).ok_or("invalid_code_cell_output_cursor")?;
        let mut bytes = 0usize;
        let mut count = 0usize;
        for chunk in remaining {
            if count > 0 && (count >= 64 || bytes.saturating_add(chunk.text.len()) > 16 * 1024) { break; }
            bytes = bytes.saturating_add(chunk.text.len());
            count += 1;
        }
        self.has_more_output = count < remaining.len();
        self.next_output_sequence = cursor + count as u64;
        self.output = remaining[..count].to_vec();
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeToolDescription {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Durable provenance of a tool record. Code-cell intermediates remain in the
/// canonical transcript but never become independent model-context messages.
/// Historical provenance is not an authorization credential.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodeItemSource {
    #[default]
    Model,
    CodeCell { cell: CodeCellIdentity },
}

impl CodeItemSource {
    pub fn is_model_visible(&self) -> bool { matches!(self, Self::Model) }
}

fn valid_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellIdentity {
    pub host_id: String,
    pub conversation_id: String,
    /// Core Turn identity. Application binds this to the canonical Run; replay
    /// Runs must not be assumed to have the same ID as the original Turn.
    pub turn_id: String,
    pub parent_call_id: String,
    pub cell_id: String,
    /// A replacement worker cannot submit callbacks for the previous isolate.
    pub generation: String,
}

impl CodeCellIdentity {
    pub fn validate(&self) -> Result<(), String> {
        if [&self.host_id, &self.conversation_id, &self.turn_id, &self.parent_call_id,
            &self.cell_id, &self.generation].iter().all(|value| valid_id(value)) {
            Ok(())
        } else {
            Err("invalid_code_cell_identity".into())
        }
    }
}

/// Effective limits selected by the Host/Core configuration, not a grant the
/// model or JavaScript can enlarge. All sizes are UTF-8 bytes, not token guesses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellLimits {
    pub source_bytes: u32,
    pub argument_bytes: u32,
    pub result_bytes: u32,
    pub output_bytes: u32,
    pub heap_bytes: u64,
    pub wall_time_ms: u64,
    pub max_calls: u32,
    pub max_in_flight: u32,
}

impl Default for CodeCellLimits {
    fn default() -> Self {
        Self { source_bytes: 64 * 1024, argument_bytes: 256 * 1024,
            result_bytes: 1024 * 1024, output_bytes: 256 * 1024,
            heap_bytes: 32 * 1024 * 1024, wall_time_ms: 120_000,
            max_calls: 64, max_in_flight: 8 }
    }
}

impl CodeCellLimits {
    pub fn validate_within(&self, ceiling: &Self) -> Result<(), String> {
        self.validate()?;
        ceiling.validate()?;
        if self.source_bytes > ceiling.source_bytes || self.argument_bytes > ceiling.argument_bytes
            || self.result_bytes > ceiling.result_bytes || self.output_bytes > ceiling.output_bytes
            || self.heap_bytes > ceiling.heap_bytes || self.wall_time_ms > ceiling.wall_time_ms
            || self.max_calls > ceiling.max_calls || self.max_in_flight > ceiling.max_in_flight {
            return Err("code_cell_budget_exceeds_host".into());
        }
        Ok(())
    }

    pub fn exec_input_schema(&self) -> Result<Value, String> {
        self.validate()?;
        Ok(serde_json::json!({
            "type":"object", "additionalProperties":false, "required":["source"],
            "properties":{
                "initial_wait_ms":{"type":"integer","minimum":0,"maximum":60000,"default":1000,
                    "description":"Foreground observation budget, not execution timeout. Returns a cell reference if still running."},
                "source":{"type":"string","minLength":1,"maxLength":self.source_bytes,
                    "description":"JavaScript. await tools[name](arguments); use ALL_TOOLS for schemas and text(value) for explicit output. Nested calls use ordinary authorization. No filesystem, network, process or module APIs."},
                "budget":{"type":"object","additionalProperties":false,"properties":{
                    "max_calls":{"type":"integer","minimum":1,"maximum":self.max_calls},
                    "max_in_flight":{"type":"integer","minimum":1,"maximum":self.max_in_flight},
                    "wall_time_ms":{"type":"integer","minimum":1,"maximum":self.wall_time_ms}
                }}
            }
        }))
    }

    pub fn restrict(&self, request: &CodeCellBudgetRequest) -> Result<Self, String> {
        self.validate()?;
        let mut effective = self.clone();
        if let Some(value) = request.max_calls {
            if value > self.max_calls { return Err("code_cell_budget_exceeds_host".into()); }
            effective.max_calls = value;
            effective.max_in_flight = effective.max_in_flight.min(value);
        }
        if let Some(value) = request.max_in_flight {
            if value > self.max_in_flight { return Err("code_cell_budget_exceeds_host".into()); }
            effective.max_in_flight = value;
        }
        if let Some(value) = request.wall_time_ms {
            if value > self.wall_time_ms { return Err("code_cell_budget_exceeds_host".into()); }
            effective.wall_time_ms = value;
        }
        effective.validate()?;
        Ok(effective)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.source_bytes == 0 || self.argument_bytes == 0 || self.result_bytes == 0
            || self.output_bytes == 0 || self.heap_bytes == 0 || self.wall_time_ms == 0
            || self.max_calls == 0 || self.max_in_flight == 0
            || self.max_in_flight > self.max_calls {
            return Err("invalid_code_cell_limits".into());
        }
        Ok(())
    }

    pub fn validate_source(&self, source: &str) -> Result<(), String> {
        self.validate()?;
        if source.trim().is_empty() || source.len() > self.source_bytes as usize {
            return Err("invalid_code_cell_source_size".into());
        }
        Ok(())
    }
}

/// A model may request less work, never more authority than the Host ceiling.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellBudgetRequest {
    pub max_calls: Option<u32>,
    pub max_in_flight: Option<u32>,
    pub wall_time_ms: Option<u64>,
}

/// Business arguments after ordinary Core admission removes protocol-owned
/// lineage. All callers share this decoder; ignored model fields are errors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellExecRequest {
    pub source: String,
    pub budget: Option<CodeCellBudgetRequest>,
    pub initial_wait_ms: Option<u32>,
}

impl CodeCellExecRequest {
    pub fn initial_wait_ms(&self) -> Result<u32, String> {
        let wait = self.initial_wait_ms.unwrap_or(1000);
        if wait > 60_000 { return Err("code_cell_observation_wait_exceeded".into()); }
        Ok(wait)
    }

    pub fn effective_limits(&self, ceiling: &CodeCellLimits) -> Result<CodeCellLimits, String> {
        self.initial_wait_ms()?;
        let limits = ceiling.restrict(self.budget.as_ref().unwrap_or(&CodeCellBudgetRequest::default()))?;
        limits.validate_source(&self.source)?;
        Ok(limits)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeNestedCall {
    pub cell: CodeCellIdentity,
    /// Starts at one, monotonically assigned before dispatch. It identifies a
    /// request, not its effects; the ordinary Host effect journal owns effects.
    pub sequence: u32,
    pub tool_name: String,
    pub arguments: Value,
}

impl CodeNestedCall {
    pub fn validate(&self, owner: &CodeCellIdentity, limits: &CodeCellLimits) -> Result<(), String> {
        self.cell.validate()?;
        limits.validate()?;
        if &self.cell != owner { return Err("code_cell_owner_mismatch".into()); }
        if self.sequence == 0 || self.sequence > limits.max_calls {
            return Err("code_cell_call_budget_exceeded".into());
        }
        if !valid_id(&self.tool_name) || !self.arguments.is_object() {
            return Err("invalid_code_nested_call".into());
        }
        if matches!(self.tool_name.as_str(), "code_mode_exec" | "code_mode_wait") {
            return Err("recursive_code_mode_not_allowed".into());
        }
        if serde_json::to_vec(&self.arguments).map_err(|error| error.to_string())?.len()
            > limits.argument_bytes as usize {
            return Err("code_cell_argument_budget_exceeded".into());
        }
        Ok(())
    }
}

/// Observation is non-destructive: the same cursor can be retried after a lost
/// response. Store/Runtime must verify the owner before looking up the cell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellObservation {
    pub cell: CodeCellIdentity,
    pub after_output_sequence: u64,
    pub wait_ms: u32,
    pub terminate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeOutputChunk {
    pub sequence: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeOutputFrame {
    pub cell: CodeCellIdentity,
    pub chunk: CodeOutputChunk,
}

/// Model-facing observation arguments. The model names a cell, but never
/// supplies its authority (Host, conversation, parent, or worker generation).
/// Resolve the persisted owner in the authenticated conversation before bind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellWaitRequest {
    pub cell_id: String,
    pub after_output_sequence: u64,
    pub wait_ms: u32,
    pub terminate: bool,
}

impl CodeCellWaitRequest {
    pub fn input_schema() -> Value {
        serde_json::json!({
            "type":"object", "additionalProperties":false,
            "required":["cell_id","after_output_sequence","wait_ms","terminate"],
            "properties":{
                "cell_id":{"type":"string","minLength":1,"maxLength":512,
                    "description":"Exact cell ID returned by code_mode_exec in this conversation."},
                "after_output_sequence":{"type":"integer","minimum":0,
                    "description":"Last received output cursor; zero reads from the beginning. Reading never consumes output."},
                "wait_ms":{"type":"integer","minimum":0,"maximum":60000,
                    "description":"Bounded observation wait, not an execution deadline. Zero polls immediately."},
                "terminate":{"type":"boolean",
                    "description":"Request cancellation of this cell. Submitted tool effects still require confirmed cancellation or reconciliation."}
            }
        })
    }

    pub fn bind(
        &self,
        owner: &CodeCellIdentity,
        authenticated_host_id: &str,
        authenticated_conversation_id: &str,
    ) -> Result<CodeCellObservation, String> {
        owner.validate()?;
        if self.cell_id != owner.cell_id
            || authenticated_host_id != owner.host_id
            || authenticated_conversation_id != owner.conversation_id
        {
            return Err("code_cell_owner_mismatch".into());
        }
        let observation = CodeCellObservation {
            cell: owner.clone(),
            after_output_sequence: self.after_output_sequence,
            wait_ms: self.wait_ms,
            terminate: self.terminate,
        };
        observation.validate(owner)?;
        Ok(observation)
    }
}

/// A committed canonical tool result returned to the exact waiting isolate.
/// This is a receipt, not a second invocation or permission grant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeToolResult {
    pub cell: CodeCellIdentity,
    pub sequence: u32,
    pub output: Value,
    pub is_error: bool,
}

impl CodeToolResult {
    pub fn acknowledgement(&self) -> Value {
        serde_json::json!({"cell": self.cell, "sequence": self.sequence})
    }
}

impl CodeCellObservation {
    pub fn validate(&self, owner: &CodeCellIdentity) -> Result<(), String> {
        self.cell.validate()?;
        if &self.cell != owner { return Err("code_cell_owner_mismatch".into()); }
        if self.wait_ms > 60_000 { return Err("code_cell_observation_wait_exceeded".into()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn historical_observation_rejects_contradictory_states() {
        use super::*;
        let completed = CodeCellView {
            cell_id: "cell".into(), parent_call_id: "parent".into(),
            phase: CodeCellPhase::Completed, terminal: true, settled: true,
            next_output_sequence: 0, has_more_output: false, output: vec![],
            unsettled_calls: 0, completion: CodeCellCompletionDetails::default(),
        };
        assert!(completed.validate_state().is_ok());
        let mut invalid = completed.clone();
        invalid.terminal = false;
        assert!(invalid.validate_state().is_err());
        invalid = completed.clone();
        invalid.settled = false;
        assert!(invalid.validate_state().is_err());
        invalid = completed.clone();
        invalid.unsettled_calls = 1;
        assert!(invalid.validate_state().is_err());
        let mut interrupted = completed.clone();
        interrupted.phase = CodeCellPhase::Interrupted;
        interrupted.settled = false;
        interrupted.unsettled_calls = 1;
        assert!(interrupted.validate_state().is_ok(), "interruption is not settlement");
        let mut live = completed;
        live.phase = CodeCellPhase::Running;
        live.terminal = false;
        live.settled = false;
        assert!(live.validate_state().is_ok());
        live.completion.error = Some("not a terminal result".into());
        assert!(live.validate_state().is_err());
    }

    #[test]
    fn historical_pages_are_bounded_repeatable_and_keep_atomic_emissions() {
        use super::*;
        let mut view = CodeCellView {
            cell_id: "cell".into(), parent_call_id: "parent".into(),
            phase: CodeCellPhase::Completed, terminal: true, settled: true,
            next_output_sequence: 70, has_more_output: false,
            output: (1..=70).map(|sequence| CodeOutputChunk { sequence, text: String::new() }).collect(),
            unsettled_calls: 0, completion: CodeCellCompletionDetails::default(),
        };
        let first = view.clone().page(0).unwrap();
        assert_eq!(first.output.len(), 64);
        assert!(first.has_more_output);
        assert_eq!(first, view.clone().page(0).unwrap());
        let last = view.clone().page(first.next_output_sequence).unwrap();
        assert_eq!(last.output.len(), 6);
        assert!(!last.has_more_output);
        assert!(view.clone().page(71).is_err());
        assert!(first.page(0).is_err(), "a partial page is not a complete observation");
        view.output[0].text = "x".repeat(20 * 1024);
        let atomic = view.page(0).unwrap();
        assert_eq!(atomic.output.len(), 1);
        assert_eq!(atomic.output[0].text.len(), 20 * 1024);
        assert!(atomic.has_more_output);
    }

    use super::*;

    #[test]
    fn observation_receipt_belongs_to_wait_call_not_original_parent() {
        let request: crate::host_effect::HostRequest = crate::host_effect::HostRequest::ObserveCodeCell {
            call_id:"wait-call".into(), observation:CodeCellObservation {
                cell:identity(), after_output_sequence:0, wait_ms:1, terminate:false,
            },
        };
        let policy = crate::host_control::HostResponsePolicy::from_request(&request);
        let response = |call_id: &str| -> crate::host_effect::HostResponse {
            crate::host_effect::HostResponse::ToolCompleted {
                call_id:call_id.into(), output:serde_json::json!({"cell_id":"cell"}),
                outcome:crate::host_effect::HostToolOutcome::Succeeded,
                activation:None, full_output_ref:None, artifacts:vec![],
            }
        };
        assert!(policy.accepts_response(&response("wait-call")));
        assert!(!policy.accepts_response(&response("parent")));
    }

    #[test]
    fn initial_observation_budget_does_not_change_execution_budget() {
        let ceiling = CodeCellLimits::default();
        let mut request = CodeCellExecRequest {
            source:"text(1)".into(), budget:None, initial_wait_ms:None,
        };
        assert_eq!(request.initial_wait_ms().unwrap(), 1000);
        for wait in [0, 1, 60_000] {
            request.initial_wait_ms = Some(wait);
            assert_eq!(request.initial_wait_ms().unwrap(), wait);
            assert_eq!(request.effective_limits(&ceiling).unwrap(), ceiling);
        }
        request.initial_wait_ms = Some(60_001);
        assert!(request.effective_limits(&ceiling).is_err());
    }

    #[test]
    fn execution_business_request_is_closed_and_uses_utf8_budget() {
        let request: CodeCellExecRequest = serde_json::from_value(serde_json::json!({
            "source":"text('结果')", "budget":{"max_calls":2}
        })).unwrap();
        let effective = request.effective_limits(&CodeCellLimits::default()).unwrap();
        assert_eq!(effective.max_calls, 2);
        assert_eq!(effective.max_in_flight, 2);
        for value in [
            serde_json::json!({"source":"text(1)","timeout":10}),
            serde_json::json!({"source":"text(1)","host_id":"forged"}),
            serde_json::json!({"source":"text(1)","budget":{"heap_bytes":1}}),
            serde_json::json!({"code":"text(1)"}),
        ] {
            assert!(serde_json::from_value::<CodeCellExecRequest>(value).is_err());
        }
        let mut ceiling = CodeCellLimits::default();
        ceiling.source_bytes = 4;
        let request = CodeCellExecRequest { source:"中文".into(), budget:None, initial_wait_ms:None };
        assert!(request.effective_limits(&ceiling).is_err());
        assert_eq!(ceiling.source_bytes, 4);
    }

    #[test]
    fn wait_resolves_authority_from_persisted_owner_not_model_arguments() {
        let owner = identity();
        let mut request = CodeCellWaitRequest {
            cell_id: owner.cell_id.clone(), after_output_sequence: 7,
            wait_ms: 0, terminate: false,
        };
        let bound = request.bind(&owner, "host", "conversation").unwrap();
        assert_eq!(bound.cell, owner);
        assert_eq!(bound.after_output_sequence, 7);
        assert!(request.bind(&owner, "other", "conversation").is_err());
        assert!(request.bind(&owner, "host", "other").is_err());
        request.cell_id = "other".into();
        assert!(request.bind(&owner, "host", "conversation").is_err());
        request.cell_id = owner.cell_id.clone();
        request.wait_ms = 60_001;
        assert!(request.bind(&owner, "host", "conversation").is_err());
        request.wait_ms = 60_000;
        request.terminate = true;
        assert!(request.bind(&owner, "host", "conversation").unwrap().terminate);
        let mut wire = serde_json::to_value(&request).unwrap();
        wire["generation"] = serde_json::json!("forged");
        assert!(serde_json::from_value::<CodeCellWaitRequest>(wire).is_err());
    }

    #[test]
    fn requested_budget_only_reduces_the_frozen_host_ceiling() {
        let ceiling = CodeCellLimits::default();
        let small = ceiling.restrict(&CodeCellBudgetRequest { max_calls: Some(2),
            max_in_flight: None, wall_time_ms: Some(100) }).unwrap();
        assert_eq!(small.max_calls, 2);
        assert_eq!(small.max_in_flight, 2);
        assert_eq!(small.wall_time_ms, 100);
        assert_eq!(small.heap_bytes, ceiling.heap_bytes);
        assert!(ceiling.restrict(&CodeCellBudgetRequest { max_calls: Some(0),
            ..Default::default() }).is_err());
        assert!(ceiling.restrict(&CodeCellBudgetRequest { max_calls: Some(ceiling.max_calls + 1),
            ..Default::default() }).is_err());
        assert!(ceiling.restrict(&CodeCellBudgetRequest { max_calls: Some(1), max_in_flight: Some(2),
            ..Default::default() }).is_err());
        assert!(serde_json::from_value::<CodeCellBudgetRequest>(serde_json::json!({"heap_bytes":1})).is_err());
    }

    fn identity() -> CodeCellIdentity {
        CodeCellIdentity { host_id: "host".into(), conversation_id: "conversation".into(),
            turn_id: "turn".into(), parent_call_id: "parent".into(), cell_id: "cell".into(),
            generation: "worker".into() }
    }

    #[test]
    fn result_receipt_acknowledgement_is_bound_to_cell_and_sequence() {
        let result = CodeToolResult { cell: identity(), sequence: 2,
            output: serde_json::json!({"value":3}), is_error: false };
        let request: crate::host_effect::HostRequest =
            crate::host_effect::HostRequest::ResolveCodeTool { result: result.clone() };
        let policy = crate::host_control::HostResponsePolicy::from_request(&request);
        let mut response: crate::host_effect::HostResponse = crate::host_effect::HostResponse::Completed {
            output: result.acknowledgement(), usage: None, full_output_ref: None, artifacts: vec![],
        };
        assert!(policy.accepts_response(&response));
        if let crate::host_effect::HostResponse::Completed { output, .. } = &mut response {
            output["sequence"] = serde_json::json!(1);
        }
        assert!(!policy.accepts_response(&response));
        if let crate::host_effect::HostResponse::Completed { output, .. } = &mut response {
            *output = result.acknowledgement();
            output["cell"]["generation"] = serde_json::json!("replacement");
        }
        assert!(!policy.accepts_response(&response));
    }

    fn limits() -> CodeCellLimits {
        CodeCellLimits { source_bytes: 1024, argument_bytes: 128, result_bytes: 1024,
            output_bytes: 1024, heap_bytes: 1024 * 1024, wall_time_ms: 1000,
            max_calls: 8, max_in_flight: 2 }
    }

    #[test]
    fn nested_call_requires_exact_owner_and_generation() {
        let owner = identity();
        let call = CodeNestedCall { cell: owner.clone(), sequence: 1,
            tool_name: "read_file".into(), arguments: serde_json::json!({"path":"a"}) };
        assert!(call.validate(&owner, &limits()).is_ok());
        for index in 0..6 {
            let mut foreign = owner.clone();
            let field = match index { 0 => &mut foreign.host_id, 1 => &mut foreign.conversation_id,
                2 => &mut foreign.turn_id, 3 => &mut foreign.parent_call_id,
                4 => &mut foreign.cell_id, _ => &mut foreign.generation };
            *field = "other".into();
            assert!(call.validate(&foreign, &limits()).is_err());
        }
    }

    #[test]
    fn rejects_recursion_oversized_arguments_and_invalid_sequence() {
        let owner = identity();
        let mut call = CodeNestedCall { cell: owner.clone(), sequence: 1,
            tool_name: "code_mode_exec".into(), arguments: serde_json::json!({}) };
        assert!(call.validate(&owner, &limits()).is_err());
        call.tool_name = "read_file".into();
        for sequence in [0, 9] {
            call.sequence = sequence;
            assert!(call.validate(&owner, &limits()).is_err());
        }
        call.sequence = 1;
        call.arguments = serde_json::json!({"path":"中".repeat(128)});
        assert!(call.validate(&owner, &limits()).is_err());
        call.arguments = Value::Null;
        assert!(call.validate(&owner, &limits()).is_err());
    }

    #[test]
    fn current_wire_contract_rejects_unknown_fields() {
        let mut value = serde_json::to_value(identity()).unwrap();
        value["session_id"] = serde_json::json!("old");
        assert!(serde_json::from_value::<CodeCellIdentity>(value).is_err());
        assert!(limits().validate_source(" ").is_err());
        assert!(limits().validate_source(&"中".repeat(400)).is_err());
        assert!(limits().validate_source("text(1 + 2)").is_ok());
    }

    #[test]
    fn yielded_receipt_keeps_parent_effect_live_and_rejects_other_calls() {
        use crate::host_control::HostResponsePolicy;
        struct TestTaskState;
        impl crate::host_effect::HostTaskState for TestTaskState {
            fn is_terminal(&self) -> bool { false }
        }
        let yielded: crate::host_effect::HostResponse<Value, Value, TestTaskState> = crate::host_effect::HostResponse::CodeModeYielded {
            cell: identity(), output_cursor: 0,
        };
        assert!(!yielded.is_terminal());
        assert!(HostResponsePolicy::ExecuteTool { call_id:"parent".into() }.accepts_response(&yielded));
        assert!(!HostResponsePolicy::ExecuteTool { call_id:"wait".into() }.accepts_response(&yielded));
        assert!(!HostResponsePolicy::ModelStream {
            response_mode:crate::host_effect::ModelResponseMode::Conversational,
        }.accepts_response(&yielded));
        let suspended: crate::host_effect::HostResponse<Value, Value, TestTaskState> =
            crate::host_effect::HostResponse::CodeModeObservationSuspended {
                cell: identity(), call_id: "wait".into(),
            };
        assert!(!suspended.is_terminal(), "scheduler yield must keep the observer live");
        assert!(HostResponsePolicy::ExecuteTool { call_id:"wait".into() }.accepts_response(&suspended));
        assert!(!HostResponsePolicy::ExecuteTool { call_id:"parent".into() }.accepts_response(&suspended));
        assert!(!HostResponsePolicy::ModelStream {
            response_mode:crate::host_effect::ModelResponseMode::Conversational,
        }.accepts_response(&suspended));
    }

    #[test]
    fn callback_wire_is_bound_to_the_parent_tool_not_model_or_other_tool() {
        use crate::host_control::HostResponsePolicy;
        let callback: crate::host_effect::HostResponse = crate::host_effect::HostResponse::CodeModeToolRequested {
            request: CodeNestedCall { cell: identity(), sequence: 1,
                tool_name: "read_file".into(), arguments: serde_json::json!({}) },
        };
        assert!(HostResponsePolicy::ExecuteTool { call_id: "parent".into() }.accepts_response(&callback));
        assert!(!HostResponsePolicy::ExecuteTool { call_id: "other".into() }.accepts_response(&callback));
        assert!(!HostResponsePolicy::ModelStream {
            response_mode: crate::host_effect::ModelResponseMode::Conversational,
        }.accepts_response(&callback));
    }
}
