// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Executes individual Core effects using CLI provider and tool implementations.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use base64::Engine as _;
use nola_core::agent::capability::{
    mcp_model_tool_id, mcp_prompt_get_model_definition, mcp_resource_read_model_definition,
    mcp_resource_template_read_model_definition, CapabilityCatalog, McpCapabilityCatalog,
    McpPromptGetRequest, McpPromptReference, McpResourceReadRequest, McpResourceReference,
    McpResourceTemplateReadRequest, McpResourceTemplateReference, McpServerAvailability,
    McpServerDescriptor, McpTransportKind, RecoverableCapabilityError,
    MCP_PROMPT_GET_CAPABILITY_ID, MCP_RESOURCE_READ_CAPABILITY_ID,
    MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID,
};
#[cfg(test)]
use nola_core::agent::capability_projection::AgentModelToolProjection;
pub use nola_core::agent::host::HostPermissionResolution;
use nola_core::agent::host::{
    AgentHostDriver, CapabilityAvailability, DelegatedAgentEventV1, DelegatedAgentRequestV1,
    HostArtifact, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostError,
    HostInputNormalization, HostOutputPolicy, HostPermissionAuthorizationDecision,
    HostPermissionIntervention, HostReplaySafety, HostRequest, HostRequestEnvelope, HostResponse,
    HostResponseEnvelope, HostRootRelativePatternNormalization, HostToolOutcome, ModelDeltaChannel,
    ModelResponseMode,
};
use nola_core::agent::protocol::{
    AgentItem, AgentRole, AgentStepContextSnapshot, ItemId, ModelProviderContinuationState,
    ModelToolCall, ProviderContinuationCompactionPolicy, TokenUsage,
};
use nola_core::agent::ModelInvocationPlan;
use serde_json::{json, Value};
use sha2::Digest;

#[cfg(test)]
use crate::config::Config;
use crate::host_execution_settings::HostExecutionSettings;
use crate::execution_context::ExecutionContext;
use crate::external_executor::ExternalExecutionObserver;
use crate::runtime_control::RuntimeExecutionControl;
use crate::llm::{
    provider_continuation_namespace, stream_chat_checked,
    stream_chat_checked_with_presentation_ticks, LlmConfig, LlmEvent,
    ModelPresentationNotification, StreamedChoice, OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE,
};
use crate::model_output::{incomplete_model_output, IncompleteModelOutput};
use crate::tools::{
    bound_external_tool_output_in_store, current_device_bridge_runtime_tools, execute_tool,
    finish_tool_work_step_transaction, full_output_ref_from_outcome,
    prepare_effect_work_step_transaction, tool_is_read_only, tool_schemas_for_host_with_skills,
    ToolContext, ToolOutcome, ToolRequest, TOOL_NORMALIZATION_SCHEMA_KEY,
};

const MODEL_OUTPUT_BYTE_LIMIT_REACHED: &str = "model output byte backstop reached";
use nola_core::agent::model_tool_batch::{
    model_tool_batch_guidance, validate_model_tool_batch_size, MODEL_TOOL_BATCH_LIMIT_EXCEEDED,
};
const MAX_MODEL_OUTPUT_UTF8_BYTES: usize = 4 * 1024 * 1024;
const CONSERVATIVE_UTF8_BYTES_PER_TOKEN: usize = 16;

fn model_stream_admission_requires_abort(error: &str) -> bool {
    error != MODEL_OUTPUT_BYTE_LIMIT_REACHED && error != MODEL_TOOL_BATCH_LIMIT_EXCEEDED
}

fn model_output_byte_backstop(max_output_tokens: u64) -> usize {
    usize::try_from(max_output_tokens)
        .unwrap_or(usize::MAX)
        .saturating_mul(CONSERVATIVE_UTF8_BYTES_PER_TOKEN)
        .clamp(4_096, MAX_MODEL_OUTPUT_UTF8_BYTES)
}

#[derive(Debug)]
struct ModelOutputStreamBudget {
    byte_limit: usize,
    utf8_bytes: usize,
}

impl ModelOutputStreamBudget {
    fn new(max_output_tokens: u64) -> Self {
        Self {
            byte_limit: model_output_byte_backstop(max_output_tokens),
            utf8_bytes: 0,
        }
    }

    fn admit(&mut self, fragment: &str) -> bool {
        let Some(next_bytes) = self.utf8_bytes.checked_add(fragment.len()) else {
            return false;
        };
        // Only the provider's tokenizer can enforce max_output_tokens. A
        // character estimate is not evidence of token exhaustion and used to
        // cut valid tool arguments at ~64 KiB for a 16K-token request.
        if next_bytes > self.byte_limit {
            return false;
        }
        self.utf8_bytes = next_bytes;
        true
    }

    fn utf8_bytes(&self) -> usize {
        self.utf8_bytes
    }

    fn start_attempt(&mut self) {
        self.utf8_bytes = 0;
    }
}

fn model_response_provenance(
    llm: &LlmConfig,
    choice: &StreamedChoice,
    attempt: usize,
    cumulative_utf8_bytes: usize,
) -> Value {
    let configured_model = choice
        .provider_model
        .as_deref()
        .unwrap_or(llm.model.as_str());
    let response_model = choice.response_model.as_deref().unwrap_or("unknown");
    json!({
        "configured_model": configured_model,
        "response_model": response_model,
        "response_id": choice.response_id.as_deref(),
        "cache_usage": choice.usage.cache_usage(choice.provider_namespace.as_deref() == Some("anthropic.messages"), false),
        "attempt": attempt,
        "effective_max_output_tokens": llm.max_output_tokens,
        "reasoning_selection": llm.reasoning_selection,
        "output_token_parameter": crate::llm::output_token_parameter_name(llm),
        "finish_reason": choice.finish_reason.as_str(),
        "provider_usage_completion_tokens": choice.usage.output_tokens,
        "provider_usage_output_tokens": choice.usage.output_tokens,
        "cumulative_utf8_bytes": cumulative_utf8_bytes,
    })
}

// OpenRouter router aliases select a physical model per request. Freeze the
// actual model for the Turn so generation, review, and revision share one model.
// This is provider routing metadata, never a guess from the user's language.
fn openrouter_resolved_model(cfg: &LlmConfig, response_model: &str) -> Option<String> {
    let endpoint = reqwest::Url::parse(&cfg.endpoint).ok()?;
    if endpoint.host_str() != Some("openrouter.ai")
        || !cfg.model.starts_with("openrouter/")
        || response_model == cfg.model
        || response_model.len() > 256
        || response_model.chars().any(char::is_whitespace)
    {
        return None;
    }
    let (author, model) = response_model.split_once('/')?;
    (!author.is_empty() && !model.is_empty()).then(|| response_model.to_string())
}

fn restored_router_model(
    cfg: &LlmConfig,
    continuation: Option<&ModelProviderContinuationState>,
) -> Option<String> {
    let state = continuation?;
    if !matches!(
        state.provider_namespace.as_str(),
        OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE | "openai.responses"
    ) || !state.is_for_model(Some(cfg.model.as_str()))
        || state.state["route_identity"].as_str()
            != Some(crate::llm::model_route_identity(cfg).as_str())
    {
        return None;
    }
    openrouter_resolved_model(cfg, state.state["resolved_model_id"].as_str()?)
}

fn provider_tool_choice(has_tools: bool, response_mode: ModelResponseMode) -> &'static str {
    if !has_tools {
        "none"
    } else if response_mode == ModelResponseMode::RequiredTool {
        "required"
    } else {
        "auto"
    }
}

fn model_protocol_correction(has_tools: bool, response_mode: ModelResponseMode) -> (&'static str, &'static str, &'static str) {
    if response_mode == ModelResponseMode::StructuredAudit {
        (
            "structured_audit_protocol_retry",
            "The provider parser rejected the review response protocol; retrying under the existing JSON contract.",
            "Protocol correction: return only the review JSON required by this system contract. The user message is the unchanged serialized review evidence packet, not a task to execute or a conversation to continue. No tools are callable. Do not simulate execution, invent evidence, provide example code, or answer the end user's task; evaluate only the supplied candidate and evidence.",
        )
    } else if has_tools {
        (
            "native_tool_protocol_retry",
            "The provider parser rejected the response protocol; no rejected call was executed.",
            "The previous response could not be parsed as a valid provider response. No rejected call was executed. The advertised native schemas and existing policy remain unchanged; return a valid native call when needed, otherwise ordinary answer text.",
        )
    } else {
        (
            "text_only_protocol_retry",
            "The provider parser rejected the response protocol in a text-only step.",
            "Protocol correction: this request has no callable tool schemas and tool_choice is none. Native calls, tool discovery, file access, command execution, and permission requests are unavailable in this step. Do not simulate them or invent execution output. Answer from retained evidence or provide clearly labeled example code, without claiming it was executed. Explain any requested action that remains undone; do not imply that another permission confirmation can enable tools.",
        )
    }
}

/// Holds only the first short semantic phrase while a model transaction is
/// still ambiguous. Models often emit a conversational preamble immediately
/// before requesting a tool. Publishing that preamble as answer text forces
/// Core to reset the visible answer once the tool call arrives, which makes
/// every client flash and re-layout. A real final answer crosses this small
/// gate quickly and remains fully streaming after that.
#[derive(Default)]
struct ModelTextPresentationGate {
    pending: String,
    first_pending_at: Option<std::time::Instant>,
    stream: SemanticPresentationBuffer,
    is_answer_stream: bool,
    is_tool_round: bool,
    defer_until_core_accepts: bool,
    tool_calls_possible: bool,
}

/// Coalesces transport-sized provider deltas into durable presentation units.
///
/// Provider callbacks frequently contain one or two characters. Persisting
/// every callback as a Core transition both exposes transport details to every
/// client and serializes a full SQLite transaction per token. This buffer owns
/// the Host protocol boundary: Core receives lossless, ordered semantic units
/// while the provider transport remains free to choose any chunk size.
struct SemanticPresentationBuffer {
    pending: String,
    first_pending_at: Option<std::time::Instant>,
    has_emitted: bool,
}

impl Default for SemanticPresentationBuffer {
    fn default() -> Self {
        Self {
            pending: String::new(),
            first_pending_at: None,
            has_emitted: false,
        }
    }
}

impl SemanticPresentationBuffer {
    // Core durably accepts each visible unit in its own transaction. Keep the
    // stream responsive, but do not turn provider token-sized callbacks into
    // dozens of full checkpoint/receipt writes for one short paragraph.
    const MINIMUM_PHRASE_CHARACTERS: usize = 64;
    const MAXIMUM_PHRASE_CHARACTERS: usize = 256;
    const FIRST_PHRASE_CHARACTERS: usize = 16;
    const FIRST_CHUNK_MAXIMUM_CHARACTERS: usize = 48;

    fn push(&mut self, delta: String) -> Vec<String> {
        self.push_at(delta, std::time::Instant::now())
    }

    fn push_reasoning(&mut self, delta: String) -> Vec<String> {
        // Reasoning has an explicit protocol channel, so its first useful
        // fragment does not need the answer/tool-prefix presentation gate.
        // Release only that first fragment early; all later callbacks retain
        // semantic batching rather than becoming a transaction per token.
        let chunks = self.push(delta);
        if !self.has_emitted && !self.pending.trim().is_empty() {
            return self.drain(true);
        }
        chunks
    }

    fn push_at(&mut self, delta: String, now: std::time::Instant) -> Vec<String> {
        if delta.is_empty() {
            return Vec::new();
        }
        self.first_pending_at.get_or_insert(now);
        self.pending.push_str(&delta);
        self.drain(false)
    }

    fn flush_due(
        &mut self,
        now: std::time::Instant,
        maximum_wait: std::time::Duration,
    ) -> Vec<String> {
        if self
            .first_pending_at
            .is_some_and(|started| now.saturating_duration_since(started) >= maximum_wait)
        {
            self.drain(true)
        } else {
            Vec::new()
        }
    }

    fn finish(&mut self) -> Vec<String> {
        self.drain(true)
    }

    fn clear(&mut self) {
        self.pending.clear();
        self.first_pending_at = None;
        self.has_emitted = false;
    }

    fn drain(&mut self, flush: bool) -> Vec<String> {
        let mut chunks = Vec::new();
        loop {
            if self.pending.is_empty() {
                break;
            }
            let mut count = 0usize;
            let mut cut = None;
            let minimum_characters = if self.has_emitted {
                Self::MINIMUM_PHRASE_CHARACTERS
            } else {
                Self::FIRST_PHRASE_CHARACTERS
            };
            let maximum_characters = if self.has_emitted {
                Self::MAXIMUM_PHRASE_CHARACTERS
            } else {
                Self::FIRST_CHUNK_MAXIMUM_CHARACTERS
            };
            for (offset, character) in self.pending.char_indices() {
                count += 1;
                let boundary = "\n。！？；.!?;".contains(character);
                if count >= maximum_characters || (count >= minimum_characters && boundary) {
                    cut = Some(offset + character.len_utf8());
                    break;
                }
            }
            if cut.is_none() && flush {
                cut = Some(self.pending.len());
            }
            let Some(cut) = cut else {
                break;
            };
            let tail = self.pending.split_off(cut);
            let chunk = std::mem::replace(&mut self.pending, tail);
            if !chunk.is_empty() {
                self.has_emitted = true;
                chunks.push(chunk);
            }
        }
        if self.pending.is_empty() {
            self.first_pending_at = None;
        }
        chunks
    }
}

impl ModelTextPresentationGate {
    const MINIMUM_READABLE_PREFIX_CHARACTERS: usize = 24;
    const FORCED_RELEASE_CHARACTERS: usize = 128;
    const MAXIMUM_PREFIX_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

    fn for_request(defer_until_core_accepts: bool, tool_calls_possible: bool) -> Self {
        Self {
            defer_until_core_accepts,
            tool_calls_possible,
            is_answer_stream: !defer_until_core_accepts && !tool_calls_possible,
            ..Self::default()
        }
    }

    fn push_text_against_reasoning(
        &mut self,
        delta: String,
        accumulated_reasoning: &str,
    ) -> Vec<String> {
        if delta.is_empty() || self.is_tool_round {
            return Vec::new();
        }
        let candidate = format!("{}{}", self.pending, delta);
        if !accumulated_reasoning.is_empty() && accumulated_reasoning.starts_with(&candidate) {
            self.pending = candidate;
            return Vec::new();
        }
        if !self.pending.is_empty() && accumulated_reasoning.starts_with(&self.pending) {
            self.pending.clear();
        }
        self.push_text(delta)
    }

    fn push_text(&mut self, delta: String) -> Vec<String> {
        self.push_text_at(delta, std::time::Instant::now())
    }

    fn push_text_at(&mut self, delta: String, now: std::time::Instant) -> Vec<String> {
        if delta.is_empty() || self.is_tool_round {
            return Vec::new();
        }
        if self.defer_until_core_accepts {
            self.pending.push_str(&delta);
            return Vec::new();
        }
        if self.is_answer_stream {
            let ends_at_semantic_boundary = delta
                .chars()
                .last()
                .is_some_and(|character| "\n。！？；.!?;".contains(character));
            let mut chunks = self.stream.push_at(delta, now);
            // A tool-free synthesis cannot turn into a tool round. Settle a
            // complete trailing sentence without waiting for another provider
            // callback while retaining the larger persistence batches for
            // incomplete transport fragments.
            if !self.tool_calls_possible && ends_at_semantic_boundary {
                chunks.extend(self.stream.finish());
            }
            return chunks;
        }
        let first_pending_at = *self.first_pending_at.get_or_insert(now);
        self.pending.push_str(&delta);
        // Hold only a short semantic prefix. Waiting for the model's terminal
        // tool decision makes every ordinary answer appear non-streaming when
        // tools are available, even though providers delivered useful deltas.
        let pending_characters = self.pending.chars().count();
        let has_semantic_boundary = self
            .pending
            .chars()
            .any(|character| "\n。！？；.!?;".contains(character));
        // Character thresholds alone made slow tool-capable models withhold
        // several seconds of ordinary prose. Once a readable phrase exists,
        // bound the speculative prefix wait on the next provider callback.
        // Explicitly deferred/tool rounds still
        // take their existing paths above; no prompt classification is involved.
        let prefix_wait_expired = now.saturating_duration_since(first_pending_at)
            >= Self::MAXIMUM_PREFIX_WAIT
            && pending_characters >= Self::MINIMUM_READABLE_PREFIX_CHARACTERS
            && has_semantic_boundary;
        if prefix_wait_expired
            || (pending_characters >= Self::FORCED_RELEASE_CHARACTERS
                && (!has_semantic_boundary || pending_characters >= 256))
        {
            self.is_answer_stream = true;
            if prefix_wait_expired {
                return vec![std::mem::take(&mut self.pending)];
            }
            return semantic_presentation_chunks(std::mem::take(&mut self.pending));
        }
        Vec::new()
    }

    fn poll_at(&mut self, now: std::time::Instant, accumulated_reasoning: &str) -> Vec<String> {
        // This local timer never invents provider output or a completion. It
        // releases only already-received text, still provisional until Core's
        // real text/tool/terminal boundary. A paused provider must not need a
        // second callback merely to make the first phrase visible.
        if self.defer_until_core_accepts || self.is_tool_round {
            return Vec::new();
        }
        if !self.pending.is_empty()
            && !accumulated_reasoning.is_empty()
            && accumulated_reasoning.starts_with(&self.pending)
        {
            return Vec::new();
        }
        if self.is_answer_stream {
            return self.stream.flush_due(now, Self::MAXIMUM_PREFIX_WAIT);
        }
        if !self.pending.is_empty()
            && self.first_pending_at.is_some_and(|started| {
                now.saturating_duration_since(started) >= Self::MAXIMUM_PREFIX_WAIT
            })
        {
            self.is_answer_stream = true;
            self.first_pending_at = None;
            return semantic_presentation_chunks(std::mem::take(&mut self.pending));
        }
        Vec::new()
    }

    fn observe_tool_call(&mut self) {
        self.is_tool_round = true;
        self.pending.clear();
    }

    fn finish_answer(&mut self) -> Vec<String> {
        if self.is_tool_round || self.defer_until_core_accepts {
            // Private candidates/audits travel once in ModelCompleted. They
            // have no presentation deltas to durably replay and then discard.
            self.pending.clear();
            self.stream.clear();
            return Vec::new();
        }
        self.is_answer_stream = true;
        let mut chunks = semantic_presentation_chunks(std::mem::take(&mut self.pending));
        chunks.extend(self.stream.finish());
        chunks
    }

    fn discard_for_tool_round(&mut self) {
        self.observe_tool_call();
        self.stream.clear();
    }

    fn reset_after_transport_retry(&mut self) {
        *self = Self::for_request(self.defer_until_core_accepts, self.tool_calls_possible);
    }
}

struct ModelRecoveryBudget {
    protocol_corrections_remaining: u8,
}

impl ModelRecoveryBudget {
    const MAX_PROTOCOL_CORRECTIONS: u8 = 2;

    fn new() -> Self {
        Self {
            protocol_corrections_remaining: Self::MAX_PROTOCOL_CORRECTIONS,
        }
    }

    fn take_protocol_correction(&mut self) -> bool {
        if self.protocol_corrections_remaining == 0 {
            return false;
        }
        self.protocol_corrections_remaining -= 1;
        true
    }
}

fn semantic_presentation_chunks(text: String) -> Vec<String> {
    const MINIMUM_PHRASE_CHARACTERS: usize = SemanticPresentationBuffer::MINIMUM_PHRASE_CHARACTERS;
    const MAXIMUM_PHRASE_CHARACTERS: usize = SemanticPresentationBuffer::MAXIMUM_PHRASE_CHARACTERS;

    if text.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut character_count = 0usize;
    for (offset, character) in text.char_indices() {
        character_count += 1;
        let semantic_boundary = "\n。！？；.!?;".contains(character);
        if character_count >= MAXIMUM_PHRASE_CHARACTERS
            || (character_count >= MINIMUM_PHRASE_CHARACTERS && semantic_boundary)
        {
            let end = offset + character.len_utf8();
            chunks.push(text[start..end].to_string());
            start = end;
            character_count = 0;
        }
    }
    if start < text.len() {
        chunks.push(text[start..].to_string());
    }
    chunks
}

#[cfg(test)]
include!("orchestration_tests.rs");








#[test]
fn tool_round_narration_remains_provisional_until_the_call_is_known() {
    let mut gate = ModelTextPresentationGate::for_request(false, true);
    let narration = "The command succeeded and produced valid output. I will inspect that output before writing the final answer so the result is grounded in evidence.";
    assert!(gate.push_text(narration.to_string()).is_empty());
    gate.observe_tool_call();
    assert!(gate.finish_answer().is_empty());
}

#[test]
fn tiny_provider_deltas_are_coalesced_across_callbacks_without_data_loss() {
    let mut gate = ModelTextPresentationGate::for_request(false, true);
    let text = "第一段说明已经完整，可以稳定显示。第二段继续补充必要信息，并保持 Markdown 内容连续。第三段给出最终结论，避免整篇回答突然出现。".repeat(3);
    let mut chunks = Vec::new();
    for character in text.chars() {
        chunks.extend(gate.push_text(character.to_string()));
    }
    chunks.extend(gate.finish_answer());

    assert_eq!(chunks.concat(), text);
    assert!(chunks.len() < text.chars().count() / 8);
    assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 256));
}

#[test]
fn short_tool_preamble_does_not_flash_at_a_newline_boundary() {
    let mut gate = ModelTextPresentationGate::for_request(false, true);
    let preamble = format!(
        "{}\n",
        "I will inspect the repository before answering. ".repeat(2)
    );

    assert!(preamble.chars().count() < ModelTextPresentationGate::FORCED_RELEASE_CHARACTERS);
    assert!(gate.push_text(preamble).is_empty());
    gate.observe_tool_call();
    assert!(gate.finish_answer().is_empty());
}

#[test]
fn tool_free_final_synthesis_streams_semantic_phrases_immediately() {
    let mut gate = ModelTextPresentationGate::for_request(false, false);
    let phrase = "审计已经完成。这里是基于工具证据得到的结论，并会持续呈现给用户。".repeat(3);

    let chunks = gate.push_text(phrase.clone());

    assert!(!chunks.is_empty());
    assert_eq!(chunks.concat(), phrase);
}

#[test]
fn mirrored_reasoning_prefix_is_not_presented_as_answer_before_a_tool_call() {
    let mut gate = ModelTextPresentationGate::for_request(false, true);
    let reasoning = "Inspect the repository state before deciding what to change.";

    assert!(gate
        .push_text_against_reasoning(reasoning.to_string(), reasoning)
        .is_empty());
    gate.observe_tool_call();
    assert!(gate.finish_answer().is_empty());
}

#[test]
fn final_answer_diverging_from_reasoning_still_streams_without_the_mirrored_prefix() {
    let mut gate = ModelTextPresentationGate::for_request(false, true);
    let reasoning = "Inspect the repository state before deciding what to change.";
    assert!(gate
        .push_text_against_reasoning(reasoning.to_string(), reasoning)
        .is_empty());

    let answer = "The repository audit is complete. The durable fix keeps every committed paragraph visible while later tool rounds continue.";
    let mut visible = gate.push_text_against_reasoning(answer.to_string(), reasoning);
    visible.extend(gate.finish_answer());
    assert_eq!(visible.concat(), answer);
}

#[test]
fn reasoning_transport_deltas_use_the_same_semantic_commit_boundary() {
    let mut buffer = SemanticPresentationBuffer::default();
    let text = "先读取项目结构并确认入口。然后核对协议状态与持久化投影。最后根据真实证据给出结论。"
        .repeat(3);
    let mut chunks = Vec::new();
    for character in text.chars() {
        chunks.extend(buffer.push_reasoning(character.to_string()));
    }
    chunks.extend(buffer.finish());

    assert_eq!(chunks.concat(), text);
    assert!(chunks.len() < text.chars().count() / 6);
}

#[test]
fn reasoning_first_meaningful_delta_is_immediate_then_batches_resume() {
    let mut buffer = SemanticPresentationBuffer::default();
    assert!(buffer.push_reasoning(String::new()).is_empty());
    assert!(buffer.push_reasoning(" \n".into()).is_empty());
    assert_eq!(buffer.push_reasoning("思".into()), vec![" \n思"]);
    assert!(buffer.push_reasoning("考".into()).is_empty());
    assert!(buffer.push_reasoning("中".into()).is_empty());
    let started = buffer.first_pending_at.unwrap();
    let wait = ModelTextPresentationGate::MAXIMUM_PREFIX_WAIT;
    assert!(buffer.flush_due(started + wait / 2, wait).is_empty());
    assert_eq!(buffer.flush_due(started + wait, wait), vec!["考中"]);
    assert!(buffer.finish().is_empty());
}

#[test]
fn reasoning_immediate_first_delta_remains_bounded_and_lossless() {
    let mut buffer = SemanticPresentationBuffer::default();
    let text = "思考🙂".repeat(400);
    let mut chunks = buffer.push_reasoning(text.clone());
    chunks.extend(buffer.finish());
    assert_eq!(chunks.concat(), text);
    assert!(
        chunks[0].chars().count() <= SemanticPresentationBuffer::FIRST_CHUNK_MAXIMUM_CHARACTERS
    );
    assert!(chunks.iter().all(|chunk| {
        chunk.chars().count() <= SemanticPresentationBuffer::MAXIMUM_PHRASE_CHARACTERS
    }));
}

#[test]
fn semantic_stream_releases_a_small_first_chunk_then_returns_to_durable_batches() {
    let mut buffer = SemanticPresentationBuffer::default();
    let first = "首个可读片段应尽快进入气泡，后续仍保持较大的持久化批次。";
    let released = buffer.push(first.to_string());
    assert!(!released.is_empty());
    assert!(released[0].chars().count() <= 48);

    let short_tail = "短尾巴";
    assert!(buffer.push(short_tail.to_string()).is_empty());
    assert_eq!(buffer.finish(), vec![short_tail]);
}

fn host_runtime_fact(id: &str, facts: Value) -> Value {
    json!({"role":"user", "content":format!(
        "<runtime_state id=\"{id}\">\nHost-observed execution facts only; not a new user request, permission grant, or replacement for the existing policy and tool schemas.\n{facts}\n</runtime_state>"
    )})
}

fn model_response_requires_protocol_recovery(finish_reason: &str, has_native_calls: bool) -> bool {
    !has_native_calls && finish_reason == "protocol_error"
}

fn core_scoped_model_call_id(effect_id: &str, provider_call_id: &str) -> String {
    // Provider call IDs are transaction-local. Core persists tool calls across
    // turns, so using the provider value directly can associate a later call
    // with an older result when the provider reuses identifiers such as
    // `call_text_0`. The model effect is unique for each transaction and keeps
    // the assistant tool call and its eventual tool result paired end-to-end.
    format!("{effect_id}::{provider_call_id}")
}

#[cfg(test)]
fn has_tool_result_since_latest_user(messages: &[Value]) -> bool {
    messages
        .iter()
        .rev()
        .take_while(|message| !is_original_user_message(message))
        .any(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
}

fn has_web_tool_result_since_latest_user(messages: &[Value]) -> bool {
    messages
        .iter()
        .rev()
        .take_while(|message| !is_original_user_message(message))
        .any(|message| {
            message.get("role").and_then(Value::as_str) == Some("tool")
                && matches!(
                    message.get("name").and_then(Value::as_str),
                    Some("web_search" | "web_fetch")
                )
        })
}


fn project_overview_file_count_since_latest_user(messages: &[Value]) -> Option<usize> {
    messages
        .iter()
        .rev()
        .take_while(|message| !is_original_user_message(message))
        .find_map(|message| {
            if message.get("role").and_then(Value::as_str) != Some("tool")
                || message.get("name").and_then(Value::as_str) != Some("project_overview")
            {
                return None;
            }
            let content = message.get("content").and_then(Value::as_str)?;
            serde_json::from_str::<Value>(content)
                .ok()
                .and_then(|value| project_overview_file_count_from_value(&value))
                .or_else(|| project_overview_file_count_from_text(content))
        })
}

fn project_overview_file_count_from_value(value: &Value) -> Option<usize> {
    match value {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|nested| project_overview_file_count_from_value(&nested))
            .or_else(|| project_overview_file_count_from_text(text)),
        Value::Array(values) => values
            .iter()
            .find_map(project_overview_file_count_from_value),
        Value::Object(object) => object
            .get("scanned_files")
            .and_then(Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .or_else(|| {
                object
                    .get("text")
                    .and_then(project_overview_file_count_from_value)
            })
            .or_else(|| {
                object
                    .get("output_preview")
                    .and_then(project_overview_file_count_from_value)
            }),
        _ => None,
    }
}

fn project_overview_file_count_from_text(text: &str) -> Option<usize> {
    let suffix = text.split_once("scanned_files:")?.1.trim_start();
    let digits = suffix
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse::<usize>().ok())?
}

fn merge_into_leading_system_message(messages: &mut Vec<Value>, content: &str) {
    if let Some(system_message) = messages
        .iter_mut()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("system"))
    {
        let existing = system_message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        system_message["content"] = Value::String(format!("{existing}\n\n{content}"));
        return;
    }
    messages.insert(0, json!({"role": "system", "content": content}));
}

pub struct HostLifecycleHookResult {
    pub status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus,
    pub output_preview: Option<String>,
    pub output_truncated: bool,
    pub error_code: Option<String>,
}

#[derive(Debug)]
pub enum PermissionHandlerError {
    Failed(String),
    TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason),
    /// Stop intent is already persisted. End this callback without inventing
    /// a user decision or acknowledging the separate cancellation effect.
    StopRequested,
    /// A committed Core settlement already consumed the pending occurrence.
    Settled,
}

impl From<String> for PermissionHandlerError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

type PermissionHandler = Box<
    dyn FnMut(
            &HostRequestEnvelope,
            &str,
            &str,
            &str,
            &Value,
        ) -> Result<HostPermissionResolution, PermissionHandlerError>
        + Send,
>;
type ContextHandler =
    Box<dyn FnMut(&HostRequestEnvelope, &str, &Value) -> Result<Vec<Value>, String> + Send>;
type RuntimeToolHandler = Box<
    dyn FnMut(
            &HostRequestEnvelope,
            &str,
            &str,
            &serde_json::Map<String, Value>,
            &ToolContext,
        ) -> Option<RuntimeToolResult>
        + Send,
>;
type RuntimeToolReplayHandler = Box<
    dyn FnMut(
            &HostRequestEnvelope,
            &str,
            &str,
            &serde_json::Map<String, Value>,
            &ToolContext,
        ) -> Option<RuntimeTaskUpdate>
        + Send,
>;

#[derive(Debug, Clone)]
pub struct RuntimeTaskPromotion {
    pub task_id: String,
    pub input_watermark: Option<i64>,
    pub kind: nola_core::agent::TaskKind,
    pub state: nola_core::agent::TaskState,
    pub progress: Option<f64>,
    pub current_step: Option<String>,
    pub lineage: nola_core::agent::TaskLineage,
}

#[derive(Debug, Clone)]
pub struct RuntimeTaskUpdate {
    pub task_id: String,
    pub state: nola_core::agent::TaskState,
    pub progress: Option<f64>,
    pub current_step: Option<String>,
    pub result_ref: Option<String>,
    pub cancel_requested: bool,
    pub ownership_token: Option<String>,
    pub completion_watermark: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RuntimeToolResult {
    pub outcome: ToolOutcome,
    pub task_promotion: Option<RuntimeTaskPromotion>,
    pub task_update: Option<RuntimeTaskUpdate>,
}

fn coarse_code_intelligence_fallback(
    operation: nola_core::code_intelligence::CodeIntelligenceOperation,
    source_args: &serde_json::Map<String, Value>,
    context: &ToolContext,
    index_started: bool,
    fallback_ids: Vec<String>,
) -> ToolOutcome {
    let Some(root) = context.allowed_dirs.first() else {
        return ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
            text: json!({
                "mode": "coarse_fallback",
                "error": "workspace_unavailable",
                "message": "The graph is not indexed and no allowed workspace is available for native exploration.",
                "background_fast_index_started": index_started,
            })
            .to_string(),
            display: "Code intelligence fallback unavailable".to_string(),
            error: true,
            artifacts: Vec::new(),
            activation: Some(nola_core::agent::host::HostCapabilityActivation {
                source: "code_intelligence_coarse_fallback".to_string(),
                capability_ids: fallback_ids,
                capability_descriptors: Vec::new(),
            }),
        };
    };
    let query = source_args
        .get("query")
        .or_else(|| source_args.get("name_pattern"))
        .or_else(|| source_args.get("qn_pattern"))
        .or_else(|| source_args.get("qualified_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (tool_name, args) = match operation {
        nola_core::code_intelligence::CodeIntelligenceOperation::Search
        | nola_core::code_intelligence::CodeIntelligenceOperation::TextSearch
            if query.is_some() =>
        {
            (
                "code_search",
                json!({
                    "root": root,
                    "query": query.unwrap_or_default(),
                    "mode": "text",
                    "max_results": 80,
                })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            )
        }
        nola_core::code_intelligence::CodeIntelligenceOperation::ReadSymbol
        | nola_core::code_intelligence::CodeIntelligenceOperation::Trace
            if query.is_some() =>
        {
            (
                "symbol_search",
                json!({
                    "root": root,
                    "symbol": query.unwrap_or_default(),
                    "include_references": true,
                    "max_matches": 80,
                })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            )
        }
        _ => (
            "repo_map",
            json!({
                "root": root,
                "max_files": 240,
                "max_symbols_per_file": 24,
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        ),
    };
    let mut outcome = crate::tools::execute_internal_read_only_tool(
        &ToolRequest {
            id: "code-intelligence-coarse-fallback".to_string(),
            name: tool_name.to_string(),
            args,
        },
        context,
    );
    outcome.text = json!({
        "mode": "coarse_fallback",
        "native_tool": tool_name,
        "background_fast_index_started": index_started,
        "result": outcome.text,
    })
    .to_string();
    outcome.display = format!("Initial code map / {tool_name}");
    outcome.activation = Some(nola_core::agent::host::HostCapabilityActivation {
        source: "code_intelligence_coarse_fallback".to_string(),
        capability_ids: fallback_ids,
        capability_descriptors: Vec::new(),
    });
    outcome
}

impl RuntimeToolResult {
    pub fn completed(outcome: ToolOutcome) -> Self {
        Self {
            outcome,
            task_promotion: None,
            task_update: None,
        }
    }

    pub fn task_updated(outcome: ToolOutcome, task_update: RuntimeTaskUpdate) -> Self {
        Self {
            outcome,
            task_promotion: None,
            task_update: Some(task_update),
        }
    }
}

fn successful_tool_completion(_capability_id: &str, outcome: &ToolOutcome) -> HostToolOutcome {
    if outcome.execution.as_ref().is_some_and(|receipt| {
        receipt.state == nola_host_protocol::tool_result::ExecutionState::Deferred
    }) {
        HostToolOutcome::Deferred
    } else {
        HostToolOutcome::Succeeded
    }
}

#[test]
fn deferred_receipt_is_typed_shell_output_not_a_phrase_or_file_payload() {
    let mut outcome = ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: json!({"type":"deferred_tool_task", "task_id":"task-1", "effects_completed":false})
            .to_string(),
        display: "queued".into(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    };
    assert_eq!(
        successful_tool_completion("bash", &outcome),
        HostToolOutcome::Succeeded
    );
    assert_eq!(
        successful_tool_completion("read_file", &outcome),
        HostToolOutcome::Succeeded
    );
    outcome.data =
        Some(json!({"type":"deferred_tool_task", "task_id":"task-1", "effects_completed":false}));
    outcome.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
        state: nola_host_protocol::tool_result::ExecutionState::Deferred,
        ..Default::default()
    });
    assert_eq!(
        successful_tool_completion("bash", &outcome),
        HostToolOutcome::Deferred
    );
    assert_eq!(
        canonical_tool_output("bash", &outcome, None)["task_id"],
        "task-1"
    );
    outcome.text = "Queued background bash task task-1".into();
    outcome.execution = None;
    assert_eq!(
        successful_tool_completion("bash", &outcome),
        HostToolOutcome::Succeeded
    );
}

#[test]
fn saved_output_search_errors_reach_core_as_protocol_fields() {
    let outcome = ToolOutcome {
        full_output_ref: None,
        data: Some(
            json!({"error":"tool_output_outside_store", "tool_execution_skipped":true,
                    "recovery":{"capability":"tool_output_search"}}),
        ),
        execution: None,
        text: json!({"error":"tool_output_outside_store", "tool_execution_skipped":true,
                    "recovery":{"capability":"tool_output_search"}})
        .to_string(),
        display: "unavailable".into(),
        error: true,
        artifacts: Vec::new(),
        activation: None,
    };
    for capability in [
        "tool_output_read",
        "tool_output_search",
        "tool_output_find",
        "tool_output_list",
    ] {
        let output = canonical_tool_output(capability, &outcome, None);
        assert_eq!(output["error"], "tool_output_outside_store");
        assert_eq!(output["tool_execution_skipped"], true);
        assert!(output.get("recovery").is_some());
    }
}

#[test]
fn queued_task_observation_error_preserves_unknown_receipt_at_core_boundary() {
    use nola_host_protocol::tool_result::{ExecutionReceipt, ExecutionState};
    for phase in ["queue_submit", "registry", "observe", "cancel_submit", "cancel_observe"] {
        let outcome = ToolOutcome {
            full_output_ref: None,
            data: Some(json!({
                "type": "deferred_tool_task", "task_id": "original-task",
                "state": "unknown", "effects_completed": false,
                "observation_error": {"phase": phase, "detail": "store unavailable"},
                "_nola_execution": {"state": "completed"}
            })),
            execution: Some(ExecutionReceipt { state: ExecutionState::Unknown, ..Default::default() }),
            text: "Task outcome could not be confirmed".into(),
            display: "Observation unavailable".into(),
            error: true,
            artifacts: Vec::new(),
            activation: None,
        };
        let result = tool_result_contract(&outcome, None);
        assert_eq!(result.execution.as_ref().unwrap().state, ExecutionState::Unknown);
        let output = result.into_core_output();
        assert_eq!(output["task_id"], "original-task");
        assert_eq!(output["observation_error"]["phase"], phase);
        assert_eq!(output["state"], "unknown");
        assert_eq!(output["effects_completed"], false);
        let receipt: ExecutionReceipt = serde_json::from_value(output["_nola_execution"].clone()).unwrap();
        assert_eq!(receipt.state, ExecutionState::Unknown);
        assert!(receipt.effects.is_empty());
        assert!(receipt.checks.is_empty());
    }
}

#[test]
fn tool_metadata_survives_canonical_projection_without_parsing_presentation() {
    let metadata = json!({
        "tool_discovery_metrics": {"duration_us": 17, "failed": false},
        "code_evidence": {"backend": "source_scan", "build_or_tests_executed": false}
    });
    let outcome = ToolOutcome {
        full_output_ref: Some("tool-evidence.txt".into()),
        data: Some(metadata.clone()),
        execution: None,
        text: "{\"build_or_tests_executed\":true}".into(),
        display: "Evidence".into(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    };
    let contract = tool_result_contract(&outcome, None);
    assert_eq!(contract.data, Some(metadata.clone()));
    assert!(contract.execution.is_none());
    let output = canonical_tool_output("tool_search", &outcome, None);
    assert_eq!(output["tool_discovery_metrics"], metadata["tool_discovery_metrics"]);
    assert_eq!(output["code_evidence"], metadata["code_evidence"]);
    assert!(output.get("build_or_tests_executed").is_none());
}

fn canonical_tool_output(
    _capability_id: &str,
    outcome: &ToolOutcome,
    memory_usage: Option<Value>,
) -> Value {
    tool_result_contract(outcome, memory_usage).into_core_output()
}

fn external_tool_result(
    data: Value,
    state: nola_host_protocol::tool_result::ExecutionState,
) -> Value {
    use nola_host_protocol::tool_result::*;
    serde_json::to_value(ToolResult {
        contract: ToolResultContract::V1,
        presentation: ToolPresentation {
            text: data.to_string(),
            summary: data.get("_nola_display").and_then(Value::as_str)
                .filter(|summary| !summary.trim().is_empty())
                .or_else(|| data.get("message").and_then(Value::as_str))
                .unwrap_or_default().to_string(),
        },
        data: Some(data),
        execution: Some(ExecutionReceipt {
            state,
            ..Default::default()
        }),
        full_output_ref: None,
        memory_usage: None,
    })
    .expect("tool result contains JSON-compatible fields")
}

fn tool_result_contract(
    outcome: &ToolOutcome,
    memory_usage: Option<Value>,
) -> nola_host_protocol::tool_result::ToolResult {
    use nola_host_protocol::tool_result::*;
    ToolResult {
        contract: ToolResultContract::V1,
        data: outcome.data.clone(),
        presentation: ToolPresentation {
            text: outcome.text.clone(),
            summary: outcome.display.clone(),
        },
        execution: outcome.execution.clone(),
        full_output_ref: full_output_ref_from_outcome(outcome),
        memory_usage,
    }
}

/// Local completion evidence is narrower than protocol-terminal delivery.
/// Never interpret business data, error wording or a joined Rust call as a
/// proof that an explicitly uncertain physical operation has stopped.
fn terminal_response_has_current_local_stop(
    response: &HostResponse,
    expected: Option<&nola_domain::execution_evidence::LocalExecutionIdentity>,
) -> bool {
    terminal_response_current_local_stop(response, expected).is_some()
}

fn terminal_response_current_local_stop(
    response: &HostResponse,
    expected: Option<&nola_domain::execution_evidence::LocalExecutionIdentity>,
) -> Option<nola_domain::execution_evidence::LocalExecutionStop> {
    let expected = expected?;
    let HostResponse::ToolCompleted { call_id, output, .. } = response else { return None; };
    if call_id != &expected.call_id { return None; }
    let Ok(result) = serde_json::from_value::<nola_host_protocol::tool_result::ToolResult>(output.clone()) else {
        return None;
    };
    result.validate().ok()?;
    result.execution?.local_stop.filter(|stop| &stop.identity == expected)
}

fn terminal_response_proves_local_stop(
    response: &HostResponse,
    ordinary_investigation: bool,
    expected: Option<&nola_domain::execution_evidence::LocalExecutionIdentity>,
) -> bool {
    if !response.is_terminal() { return false; }
    match response {
        HostResponse::Failed { .. } => false,
        HostResponse::ToolCompleted { output, .. } => {
            let Ok(result) = serde_json::from_value::<nola_host_protocol::tool_result::ToolResult>(output.clone()) else {
                return false;
            };
            if result.validate().is_err() { return false; }
            match result.execution.as_ref() {
                None => ordinary_investigation,
                Some(receipt) if receipt.local_stop.is_some() => {
                    terminal_response_has_current_local_stop(response, expected)
                }
                Some(receipt) => matches!(receipt.state,
                    nola_host_protocol::tool_result::ExecutionState::Completed
                        | nola_host_protocol::tool_result::ExecutionState::NotStarted),
            }
        }
        _ => true,
    }
}

impl CliAgentHostDriver {
    fn has_admitted_not_started_target(&self, request: &HostRequestEnvelope) -> bool {
        let HostRequest::Cancel { target_effect_id, target_not_started: Some(proof) } = &request.payload else { return false; };
        // Only the exact committed dispatch owner may consume Application's
        // frozen revocation. Arguments/data and an unleased synthetic request
        // cannot confer this authority.
        request.delivery.is_some()
            && request.host_id == self.catalog.host_id
            && request.catalog_revision == self.catalog.revision
            && request.availability_epoch == self.catalog.availability_epoch
            && self.boundary_session_id.as_deref() == Some(proof.conversation_id.as_str())
            && self.bound_dispatch_run_id.as_deref() == Some(proof.run_id.as_str())
            && proof.matches_control(&request.host_id.0,&request.session_id,request.turn_id.as_deref(),
                &request.effect_id.0,&request.request_id,&target_effect_id.0)
    }

    fn fresh_local_execution_identity(
        &self,
        request: &HostRequestEnvelope,
    ) -> Option<nola_domain::execution_evidence::LocalExecutionIdentity> {
        let HostRequest::ExecuteTool { capability_id, call_id, .. } = &request.payload else { return None; };
        let capability = self.catalog.resolve_available(capability_id)?;
        if !matches!(capability.input_normalization.origin.kind,
            nola_core::agent::host::HostCapabilityOriginKind::Builtin
                | nola_core::agent::host::HostCapabilityOriginKind::Internal)
            || request.host_id != self.catalog.host_id {
            return None;
        }
        let turn_id = request.turn_id.as_ref().filter(|turn| !turn.trim().is_empty())?;
        Some(nola_domain::execution_evidence::LocalExecutionIdentity {
            execution_id: uuid::Uuid::new_v4().to_string(),
            host_id: request.host_id.0.clone(), session_id: request.session_id.clone(),
            turn_id: turn_id.clone(), effect_id: request.effect_id.0.clone(),
            request_id: request.request_id.clone(), call_id: call_id.clone(),
            delivery_attempt_id: request.delivery.as_ref().map(|delivery| delivery.attempt_id.clone()),
        })
    }
}

/// Retain the observed target fence, including a typed stop's invocation nonce.
/// The map key is an index, not permission to apply this fact to another
/// request or physical delivery. Any new dispatch removes the entire witness.
struct ObservedHostStop {
    request_id: String,
    delivery_attempt_id: Option<String>,
    delivery: Option<nola_core::agent::host::EffectDeliveryLease>,
    requires_current_target_validation: bool,
    local_stop: Option<nola_domain::execution_evidence::LocalExecutionStop>,
}

impl ObservedHostStop {
    fn from_request(request: &HostRequestEnvelope,
        local_stop: Option<nola_domain::execution_evidence::LocalExecutionStop>) -> Self {
        Self { request_id: request.request_id.clone(),
            delivery_attempt_id: request.delivery.as_ref().map(|delivery| delivery.attempt_id.clone()),
            delivery: request.delivery.clone(), requires_current_target_validation: false,
            local_stop }
    }

    fn matches_owner(&self, owner: &(String, String, Option<String>, String)) -> bool {
        !self.request_id.is_empty() && self.local_stop.as_ref().is_none_or(|stop| {
            stop.validate().is_ok()
                && stop.identity.host_id == owner.0 && stop.identity.session_id == owner.1
                && Some(stop.identity.turn_id.as_str()) == owner.2.as_deref()
                && stop.identity.effect_id == owner.3
                && stop.identity.request_id == self.request_id
                && stop.identity.delivery_attempt_id == self.delivery_attempt_id
        })
    }
}

#[test]
fn missing_tool_receipt_stays_absent_without_changing_success_outcome() {
    use nola_host_protocol::tool_result::{ExecutionReceipt, ExecutionState};
    let mut outcome = ToolOutcome {
        full_output_ref: None, data: Some(json!({"_nola_execution":{"state":"completed"}})),
        execution: None, text: "completed and verified".into(), display: "Done".into(),
        error: false, artifacts: vec![], activation: None,
    };
    assert_eq!(successful_tool_completion("write_file", &outcome), HostToolOutcome::Succeeded);
    for error in [false, true] {
        outcome.error = error;
        let result = tool_result_contract(&outcome, None);
        assert!(result.execution.is_none());
        let wire = serde_json::to_value(&result).unwrap();
        nola_host_protocol::tool_result::validate_completion(&wire,
            if error { HostToolOutcome::Failed } else { HostToolOutcome::Succeeded }).unwrap();
        assert!(result.into_core_output().get("_nola_execution").is_none());
    }
    for state in [ExecutionState::NotStarted, ExecutionState::Completed,
        ExecutionState::Deferred, ExecutionState::Partial, ExecutionState::Unknown] {
        let receipt = ExecutionReceipt { state, ..Default::default() };
        outcome.execution = Some(receipt.clone());
        assert_eq!(tool_result_contract(&outcome, None).execution, Some(receipt));
    }
}

fn deterministic_tool_failure_key(
    capability_id: &str,
    args: &serde_json::Map<String, Value>,
) -> String {
    if capability_id != "apply_patch" {
        return format!(
            "{capability_id}:{}",
            serde_json::to_string(args).unwrap_or_default()
        );
    }
    let root = args.get("root").and_then(Value::as_str).unwrap_or_default();
    let mut targets = BTreeSet::new();
    if let Some(patch) = args.get("patch").and_then(Value::as_str) {
        for line in patch.lines() {
            for prefix in ["--- a/", "+++ b/", "*** Update File: "] {
                if let Some(path) = line.strip_prefix(prefix) {
                    let path = path.trim();
                    if !path.is_empty() && path != "/dev/null" {
                        targets.insert(path.to_string());
                    }
                }
            }
        }
    }
    format!(
        "apply_patch:{root}:{}",
        targets.into_iter().collect::<Vec<_>>().join(",")
    )
}


fn goal_complete_protocol_payload(text: &str) -> Option<Value> {
    let mut status = None;
    let mut summary = None;
    let mut next_action = None;
    let mut evidence = Vec::new();
    let mut blockers = Vec::new();
    enum ListKind {
        Evidence,
        Blockers,
    }
    let mut list = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("goal_complete: ") {
            status = Some(value.trim().to_string());
            list = None;
        } else if let Some(value) = line.strip_prefix("goal_summary: ") {
            summary = Some(value.trim().to_string());
            list = None;
        } else if let Some(value) = line.strip_prefix("goal_next_action: ") {
            next_action = Some(value.trim().to_string());
            list = None;
        } else if line == "goal_evidence:" {
            list = Some(ListKind::Evidence);
        } else if line == "goal_blockers:" {
            list = Some(ListKind::Blockers);
        } else if let Some((_, value)) = line.split_once(". ") {
            match list {
                Some(ListKind::Evidence) => evidence.push(value.trim().to_string()),
                Some(ListKind::Blockers) => blockers.push(value.trim().to_string()),
                None => {}
            }
        }
    }
    status.as_ref()?;
    Some(json!({
        "type": "goal_completion",
        "goal_status": status.unwrap_or_default(),
        "summary": summary.unwrap_or_default(),
        "evidence": evidence,
        "blockers": blockers,
        "next_action": next_action,
    }))
}


/// Skill instructions become part of the model context only after the Skill
/// call completes. Reject a mixed batch atomically and request a new decision;
/// never silently remove provider call IDs from an otherwise accepted batch.
fn tool_batch_requires_activation_replan(tool_calls: &[crate::llm::ToolCall]) -> bool {
    tool_calls.len() > 1
        && tool_calls.iter().any(|call| {
            matches!(
                call.name.as_str(),
                "Skill" | "skill_invoke" | "SOP" | "sop_invoke"
            )
        })
}


type LifecycleHookHandler = Box<
    dyn FnMut(
            &nola_core::lifecycle_hooks::LifecycleHookInvocation,
            &nola_core::lifecycle_hooks::LifecycleHookEvent,
        ) -> Result<HostLifecycleHookResult, String>
        + Send,
>;

fn validate_model_tool_projection(
    settings: &Value,
    tools: &[Value],
) -> Result<AgentStepContextSnapshot, String> {
    let value = settings
        .get("step_context")
        .cloned()
        .ok_or_else(|| "Core model request is missing StepContext".to_string())?;
    let step_context = serde_json::from_value::<AgentStepContextSnapshot>(value)
        .map_err(|error| format!("Core model request has invalid StepContext: {error}"))?;
    if step_context.schema_version != 1 {
        return Err(format!(
            "Core model request uses unsupported StepContext schema_version {}",
            step_context.schema_version
        ));
    }
    if step_context.step_index == 0 {
        return Err("Core model request StepContext step_index must be positive".to_string());
    }
    if step_context.step_index != step_context.tool_projection.revision {
        return Err(
            "Core model request StepContext revision does not match its tool projection"
                .to_string(),
        );
    }
    for (name, digest) in [
        (
            "invocation_policy_digest",
            &step_context.invocation_policy_digest,
        ),
        ("stable_prefix_digest", &step_context.stable_prefix_digest),
        ("model_prefix_digest", &step_context.model_prefix_digest),
        (
            "request_snapshot_digest",
            &step_context.request_snapshot_digest,
        ),
    ] {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!(
                "Core model request StepContext {name} is not a SHA-256 digest"
            ));
        }
    }
    let expected_model_prefix = step_context
        .tool_projection
        .model_prefix_digest_with_invocation_policy(
            &step_context.stable_prefix_digest,
            &step_context.invocation_policy_digest,
        );
    if step_context.model_prefix_digest != expected_model_prefix {
        return Err(
            "Core model request StepContext model prefix identity is inconsistent".to_string(),
        );
    }
    let expected_request_snapshot = step_context.tool_projection.request_snapshot_digest(
        &step_context.turn_context,
        &step_context.invocation_policy_digest,
    );
    if step_context.request_snapshot_digest != expected_request_snapshot {
        return Err(
            "Core model request StepContext request snapshot identity is inconsistent".to_string(),
        );
    }
    if !step_context
        .tool_projection
        .matches_model_definitions(tools)
    {
        return Err(
            "Core model request tools do not match the frozen Step tool projection".to_string(),
        );
    }
    Ok(step_context)
}


/// Concrete CLI Host. It executes one effect at a time and never owns an
/// Agent/tool loop.
struct CompactionRequestWitness {
    request: std::sync::Arc<crate::llm::PreparedModelRequest>,
    session_id: String,
    configuration_identity: String,
    context_window: usize,
    usage: crate::llm::LlmUsage,
}

pub struct CliAgentHostDriver {
    ui_publisher: Option<std::sync::Arc<dyn Fn(nola_application::generative_ui::ExecuteUiTool) -> Result<serde_json::Value, nola_service_client::ApplicationServiceCommandError> + Send + Sync>>,
    code_cells: BTreeMap<String, HostCodeCell>,
    code_mode_limits: nola_domain::code_mode::CodeCellLimits,
    catalog: HostCapabilityCatalog,
    capability_catalog: CapabilityCatalog,
    boundary_session_id: Option<String>,
    /// Exact Run adopted by the committed dispatcher, not inferred from a queue.
    bound_dispatch_run_id: Option<String>,
    boundary_submission_state: std::sync::Arc<std::sync::Mutex<DurableBoundarySubmissionState>>,
    mcp_tool_routes: BTreeMap<String, McpToolRoute>,
    mcp_server_registry: Vec<Value>,
    plugin_mcp_session: Option<crate::plugin_runtime::SharedPluginSession>,
    // Already issued model requests may precede a concurrent Skill catalog
    // refresh. Retain their exact model capability, never old tool authority.
    retained_model_catalogs: BTreeMap<u64, (u64, HostCapabilityDescriptor)>,
    revoked_plugin_capabilities: HashSet<String>,
    code_intelligence_provider:
        Option<crate::code_intelligence_provider::SharedCodeIntelligenceProvider>,
    code_memory_workspace: Option<PathBuf>,
    current_turn_code_memory_assessment: Option<Value>,
    code_memory_initialization: crate::code_intelligence_provider::CodeMemoryInitialization,
    llm: LlmConfig,
    compaction_request_witness: Option<CompactionRequestWitness>,
    tool_context: ToolContext,
    permission_handler: PermissionHandler,
    context_handler: ContextHandler,
    runtime_tool_replay_handler: RuntimeToolReplayHandler,
    runtime_tool_handler: RuntimeToolHandler,
    runtime_tool_ids: HashSet<String>,
    /// Optional model-surface boundary supplied by the owning Core runtime.
    /// Delegated allowlists remain authoritative. Native authorization-only
    /// Hosts admit dynamic MCP descriptors only through their frozen Core policy.
    configured_tool_allowlist: Option<HashSet<String>>,
    mcp_extension_policy: Option<nola_core::tool_policy::ToolPolicyInput>,
    turn_policy: HostTurnPolicy,
    current_turn_project_overview_file_count: Option<usize>,
    current_turn_project_overview_attempted: bool,
    current_turn_skill_decided: bool,
    current_turn_deterministic_tool_failures: BTreeMap<String, u8>,
    current_turn_available_tool_ids: HashSet<String>,
    current_turn_instruction_contexts: HashSet<String>,
    current_turn_unavailable_model_targets: HashSet<String>,
    lifecycle: LifecycleHookExecutor,
    active_effect_cancellations:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, ExecutionContext>>>,
    execution_settings: Option<HostExecutionSettings>,
    delegated_agent_sessions: BTreeMap<String, DelegatedAgentWorkerSession>,
    /// Delivery attempts whose execution-start barrier was durably accepted by
    /// Application. A duplicate dispatch must not emit another start event or
    /// cross the physical side-effect boundary a second time.
    started_effect_deliveries: HashSet<String>,
    /// Positive, owner-scoped local completion or queued NotStarted evidence.
    /// An absent cancellation handle is never itself a quiescence proof.
    cancellation_stop_validator: fn(&str, &str, &HostRequestEnvelope, &str,
        Option<&nola_core::agent::host::EffectDeliveryLease>) -> Result<(), String>,
    quiescent_effects: BTreeMap<(String, String, Option<String>, String), ObservedHostStop>,
    base_allowed_dirs: Vec<std::path::PathBuf>,
}

#[derive(Clone)]
enum DelegatedAgentWorkerControl {
    ResolvePermission {
        request_id: String,
        decision: HostPermissionAuthorizationDecision,
        envelope: HostRequestEnvelope,
        target: nola_host_protocol::DelegatedPermissionDeliveryTarget,
        occurrence: nola_host_protocol::DelegatedPermissionOccurrence,
        backend_id: String,
        backend_session_id: Option<String>,
    },
    Cancel,
}

enum DelegatedAgentWorkerOutput {
    Control {
        envelope: HostRequestEnvelope,
        sequence: u64,
        response: HostResponse,
        acknowledgement: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    Event {
        event: DelegatedAgentEventV1,
        acknowledgement: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    // Positive physical process-stop observation, independent of result commit ACK.
    PhysicalStopObserved,
    Suspended,
    Finished,
}

struct DelegatedAgentWorkerSession {
    effect_id: String,
    permission_delivery_ids: std::collections::BTreeSet<String>,
    request: HostRequestEnvelope,
    next_sequence: u64,
    control_tx: std::sync::mpsc::Sender<DelegatedAgentWorkerControl>,
    output_rx: std::sync::mpsc::Receiver<DelegatedAgentWorkerOutput>,
    cancellation: RuntimeExecutionControl,
    worker: Option<std::thread::JoinHandle<()>>,
    admission_failure: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    cancellation_registry:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, ExecutionContext>>>,
}

impl Drop for DelegatedAgentWorkerSession {
    fn drop(&mut self) {
        if let Ok(mut cancellations) = self.cancellation_registry.lock() {
            cancellations.remove(&self.effect_id);
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            if let Ok(mut failure) = self.admission_failure.lock() {
                if failure.is_none() {
                    *failure = Some("delegated consumer closed".into());
                }
            }
            // This is process cleanup, not a user cancellation decision. The
            // worker checks admission_failure before emitting any terminal event.
            self.cancellation.request();
        }
        let (control_tx, _) = std::sync::mpsc::channel();
        drop(std::mem::replace(&mut self.control_tx, control_tx));
        let (_, output_rx) = std::sync::mpsc::sync_channel(1);
        drop(std::mem::replace(&mut self.output_rx, output_rx));
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                eprintln!("delegated worker panicked during cleanup");
            }
        }
    }
}

struct DelegatedAgentWorkerObserver {
    physical_stop_confirmed: bool,
    accounting: Option<nola_domain::delegated_invocation::ExternalUsageSnapshot>,
    worker_generation: String,
    invocation_id: String,
    pending_delivery: Option<(
        HostRequestEnvelope,
        nola_host_protocol::DelegatedPermissionDeliveryTarget,
        String,
        String,
        u64,
    )>,
    backend_id: String,
    backend_session_id: Option<String>,
    parent_effect_id: String,
    text_part_id: String,
    reasoning_part_id: String,
    output_tx: std::sync::mpsc::SyncSender<DelegatedAgentWorkerOutput>,
    admission_failure: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    control_rx: std::sync::mpsc::Receiver<DelegatedAgentWorkerControl>,
    tool_context: ToolContext,
    work_step_transactions: BTreeMap<String, nola_application::WorkStepTransaction>,
    work_step_transaction_failures: BTreeMap<String, ToolOutcome>,
    text: String,
    reasoning: String,
    used_tokens: u64,
    context_window_tokens: u64,
}

fn send_delegated_event(
    output: &std::sync::mpsc::SyncSender<DelegatedAgentWorkerOutput>,
    failure: &std::sync::Arc<std::sync::Mutex<Option<String>>>,
    event: DelegatedAgentEventV1,
) -> Result<(), String> {
    if let Some(error) = failure
        .lock()
        .map_err(|_| "delegated admission state poisoned")?
        .clone()
    {
        return Err(error);
    }
    let (acknowledgement, received) = std::sync::mpsc::sync_channel(1);
    let result = output
        .send(DelegatedAgentWorkerOutput::Event {
            event,
            acknowledgement,
        })
        .map_err(|_| "delegated event consumer disconnected".to_string())
        .and_then(|_| {
            received
                .recv()
                .map_err(|_| "delegated commit acknowledgement disconnected".to_string())
        })
        .and_then(|result| result);
    if let Err(error) = &result {
        if let Ok(mut failure) = failure.lock() {
            *failure = Some(error.clone());
        }
    }
    result
}

impl DelegatedAgentWorkerObserver {
    fn send_control(
        &self,
        envelope: &HostRequestEnvelope,
        sequence: u64,
        response: HostResponse,
    ) -> Result<(), String> {
        let (acknowledgement, received) = std::sync::mpsc::sync_channel(1);
        let result = self
            .output_tx
            .send(DelegatedAgentWorkerOutput::Control {
                envelope: envelope.clone(),
                sequence,
                response,
                acknowledgement,
            })
            .map_err(|_| "delegated control consumer disconnected".to_string())
            .and_then(|_| {
                received
                    .recv()
                    .map_err(|_| "delegated control acknowledgement disconnected".to_string())
            })
            .and_then(|result| result);
        if let Err(error) = &result {
            if let Ok(mut failure) = self.admission_failure.lock() {
                *failure = Some(error.clone());
            }
        }
        result
    }

    fn send(&self, event: DelegatedAgentEventV1) -> Result<(), String> {
        send_delegated_event(&self.output_tx, &self.admission_failure, event)
    }

    fn transaction_call_id(&self, call_id: &str) -> String {
        let execution_identity = self
            .backend_session_id
            .as_deref()
            .unwrap_or(&self.parent_effect_id);
        format!(
            "delegated:{}:{execution_identity}:{call_id}",
            self.backend_id
        )
    }
}

fn settle_recovered_delegated_transaction(
    transaction: nola_application::WorkStepTransaction,
    fallback: ToolOutcome,
) -> ToolOutcome {
    use nola_application::WorkStepTransactionPhase;

    match transaction.phase {
        WorkStepTransactionPhase::Checkpointed => {
            finish_tool_work_step_transaction(Some(transaction), fallback)
        }
        WorkStepTransactionPhase::EffectObserved => {
            if crate::work_step_port::reconcile_observed_work_step_transactions().is_err() {
                return ToolOutcome {
                    full_output_ref: None,
                    data: None,
                    execution: None,
                    text: "The delegated result was observed, but it could not be committed."
                        .to_string(),
                    display: "Delegated result requires recovery".to_string(),
                    error: true,
                    artifacts: Vec::new(),
                    activation: None,
                };
            }
            crate::work_step_port::work_step_transaction(&transaction.id)
                .ok()
                .flatten()
                .map(|resolved| settle_recovered_delegated_transaction(resolved, fallback.clone()))
                .unwrap_or(fallback)
        }
        WorkStepTransactionPhase::Committed | WorkStepTransactionPhase::Failed => {
            let result = transaction.result.as_ref();
            ToolOutcome {
                full_output_ref: result
                    .and_then(|v| v.get("full_output_ref"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                data: result
                    .and_then(|v| v.get("data"))
                    .filter(|v| !v.is_null())
                    .cloned(),
                execution: result
                    .and_then(|v| v.get("execution"))
                    .filter(|v| !v.is_null())
                    .and_then(|v| serde_json::from_value(v.clone()).ok()),
                text: result
                    .and_then(|value| value.get("text"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(transaction.error)
                    .unwrap_or(fallback.text),
                display: result
                    .and_then(|value| value.get("display"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or(fallback.display),
                error: transaction.phase == WorkStepTransactionPhase::Failed
                    || result
                        .and_then(|value| value.get("error"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                artifacts: result
                    .and_then(|value| value.get("artifacts"))
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or(fallback.artifacts),
                activation: None,
            }
        }
        WorkStepTransactionPhase::Prepared => ToolOutcome {
            full_output_ref: None,
            data: None,
            execution: None,
            text:
                "The delegated side effect was never claimed and its completion cannot be accepted."
                    .to_string(),
            display: "Delegated result rejected".to_string(),
            error: true,
            artifacts: Vec::new(),
            activation: None,
        },
    }
}

fn delegated_file_change_artifacts(arguments: &Value, context: &ToolContext) -> Vec<HostArtifact> {
    let Some(root) = context.allowed_dirs.first() else {
        return Vec::new();
    };
    let root = root.canonicalize().unwrap_or_else(|_| root.clone());
    let changes = arguments
        .as_array()
        .or_else(|| arguments.get("changes").and_then(Value::as_array));
    let Some(changes) = changes else {
        return Vec::new();
    };
    changes
        .iter()
        .filter_map(|change| {
            let raw_path = change
                .get("path")
                .or_else(|| change.get("file_path"))
                .or_else(|| change.get("filePath"))
                .and_then(Value::as_str)?;
            let path = std::path::PathBuf::from(raw_path);
            if path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return None;
            }
            let candidate = if path.is_absolute() {
                path
            } else {
                root.join(path)
            };
            let resolved = if candidate.exists() {
                candidate.canonicalize().ok()?
            } else {
                let parent = candidate.parent()?.canonicalize().ok()?;
                parent.join(candidate.file_name()?)
            };
            if !resolved.starts_with(&root) {
                return None;
            }
            let raw_operation = change
                .get("kind")
                .or_else(|| change.get("type"))
                .or_else(|| change.get("operation"))
                .and_then(Value::as_str)
                .unwrap_or("edit")
                .to_ascii_lowercase();
            let operation = if matches!(
                raw_operation.as_str(),
                "add" | "added" | "create" | "created"
            ) {
                "create"
            } else if matches!(
                raw_operation.as_str(),
                "delete" | "deleted" | "remove" | "removed"
            ) {
                "delete"
            } else {
                "edit"
            };
            let artifact = HostArtifact {
                uri: url::Url::from_file_path(&resolved).ok()?.to_string(),
                name: resolved.file_name()?.to_string_lossy().into_owned(),
                mime_type: None,
                operation: Some(operation.to_string()),
            };
            artifact.validate().ok()?;
            Some(artifact)
        })
        .collect()
}

fn delegated_tool_artifacts(
    name: &str,
    arguments: &Value,
    output: &str,
    context: &ToolContext,
) -> Result<Vec<HostArtifact>, String> {
    let mut artifacts = if name == "file_change" {
        delegated_file_change_artifacts(arguments, context)
    } else {
        Vec::new()
    };
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        artifacts.extend(crate::tools::explicit_host_artifacts_from_value(&value)?);
    }
    let mut seen = std::collections::HashSet::new();
    artifacts.retain(|artifact| seen.insert(artifact.uri.clone()));
    Ok(artifacts)
}

impl ExternalExecutionObserver for DelegatedAgentWorkerObserver {
    fn external_text(&mut self, text: &str) -> Result<(), String> {
        self.text.push_str(text);
        self.send(DelegatedAgentEventV1::TextDelta {
            part_id: self.text_part_id.clone(),
            delta: text.to_string(),
        })?;
        Ok(())
    }

    fn external_reasoning(&mut self, text: &str) -> Result<(), String> {
        self.reasoning.push_str(text);
        self.send(DelegatedAgentEventV1::ReasoningDelta {
            part_id: self.reasoning_part_id.clone(),
            delta: text.to_string(),
        })?;
        Ok(())
    }

    fn external_session_started(&mut self, backend: &str, session_id: &str) -> Result<(), String> {
        self.backend_session_id = Some(session_id.to_string());
        self.send(DelegatedAgentEventV1::SessionStarted {
            backend_id: backend.to_string(),
            backend_session_id: session_id.to_string(),
        })?;
        Ok(())
    }

    fn external_permission_requested(
        &mut self,
        request_id: &str,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        reason: &str,
    ) -> Result<Option<bool>, String> {
        let resource = nola_core::agent::permissions::permission_resource_for_input(
            tool_name, None, arguments,
        );
        let occurrence = nola_host_protocol::DelegatedPermissionOccurrence {
            occurrence_id: uuid::Uuid::new_v4().to_string(),
            worker_generation: self.worker_generation.clone(),
        };
        self.send(DelegatedAgentEventV1::PermissionRequested {
            occurrence: occurrence.clone(),
            request_id: request_id.to_string(),
            call_id: call_id.to_string(),
            capability_id: tool_name.to_string(),
            resource,
            reason: reason.to_string(),
            input: arguments.clone(),
        })?;
        self.output_tx
            .send(DelegatedAgentWorkerOutput::Suspended)
            .map_err(|_| "delegated suspension consumer disconnected".to_string())?;
        loop {
            match self
                .control_rx
                .recv()
                .map_err(|_| "delegated permission consumer disconnected".to_string())?
            {
                DelegatedAgentWorkerControl::ResolvePermission {
                    request_id: resolved_request_id,
                    decision,
                    envelope,
                    target,
                    occurrence: resolved_occurrence,
                    backend_id,
                    backend_session_id,
                } => {
                    use sha2::{Digest, Sha256};
                    let digest = format!(
                        "{:x}",
                        Sha256::digest(
                            serde_json::to_vec(arguments).map_err(|error| error.to_string())?
                        )
                    );
                    if resolved_request_id != request_id
                        || resolved_occurrence != occurrence
                        || target.worker_generation != self.worker_generation
                        || target.occurrence_id != occurrence.occurrence_id
                        || target.invocation_id != self.invocation_id
                        || target.call_id != call_id
                        || target.request_digest != digest
                        || backend_id != self.backend_id
                        || backend_session_id != self.backend_session_id
                        || self.pending_delivery.is_some()
                    {
                        let error = "delegated permission delivery target mismatch".to_string();
                        *self
                            .admission_failure
                            .lock()
                            .map_err(|_| "delegated admission state poisoned")? =
                            Some(error.clone());
                        return Err(error);
                    }
                    let sequence = CliAgentHostDriver::response_sequence(&envelope);
                    self.send_control(
                        &envelope,
                        sequence,
                        HostResponse::DelegatedPermissionClaimed {
                            delivery_id: target.delivery_id.clone(),
                        },
                    )?;
                    self.pending_delivery = Some((
                        envelope,
                        target,
                        request_id.to_string(),
                        call_id.to_string(),
                        sequence + 1,
                    ));
                    return Ok(Some(decision != HostPermissionAuthorizationDecision::Deny));
                }
                DelegatedAgentWorkerControl::Cancel => {
                    return Err("delegated execution cancelled".into())
                }
            }
        }
    }

    fn external_permission_written(
        &mut self,
        request_id: &str,
        call_id: &str,
    ) -> Result<(), String> {
        let (envelope, target, expected_request, expected_call, sequence) = self
            .pending_delivery
            .take()
            .ok_or("permission write has no committed claim")?;
        if request_id != expected_request || call_id != expected_call {
            return Err("permission write target mismatch".into());
        }
        self.send_control(
            &envelope,
            sequence,
            HostResponse::DelegatedPermissionTransportWritten {
                delivery_id: target.delivery_id,
            },
        )
    }

    fn external_tool_started(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: &Value,
    ) -> Result<(), String> {
        let args = arguments.as_object().cloned().unwrap_or_else(|| {
            let mut args = serde_json::Map::new();
            args.insert("input".to_string(), arguments.clone());
            args
        });
        let transaction_request = ToolRequest {
            id: self.transaction_call_id(call_id),
            name: format!("external::{}::{name}", self.backend_id),
            args,
        };
        match prepare_effect_work_step_transaction(
            &transaction_request,
            &self.tool_context,
            nola_core::agent::host::HostCapabilityEffect::Execute,
            nola_core::agent::host::HostCapabilityTransactionBoundary::External,
        ) {
            Ok(Some(transaction)) => {
                self.work_step_transactions
                    .insert(call_id.to_string(), transaction);
            }
            Ok(None) => {}
            Err(outcome) => {
                self.work_step_transaction_failures
                    .insert(call_id.to_string(), outcome);
            }
        }
        self.send(DelegatedAgentEventV1::ToolStarted {
            call_id: call_id.to_string(),
            capability_id: name.to_string(),
            input: arguments.clone(),
        })?;
        Ok(())
    }

    fn external_tool_finished(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: &Value,
        output: &str,
        failed: bool,
    ) -> Result<(), String> {
        let artifact_result = delegated_tool_artifacts(name, arguments, output, &self.tool_context);
        let mut outcome = ToolOutcome {
            full_output_ref: None,
            data: None,
            execution: None,
            text: output.to_string(),
            display: format!("{} / {name}", self.backend_id),
            error: failed || artifact_result.is_err(),
            artifacts: artifact_result.unwrap_or_default(),
            activation: None,
        };
        let recovered_transaction = if self.work_step_transactions.contains_key(call_id) {
            None
        } else {
            crate::work_step_port::work_step_transaction(&format!(
                "work-step:{}:{}",
                self.tool_context
                    .permission_scope
                    .owner_session_id()
                    .unwrap_or_else(|| format!(
                        "workspace:{}",
                        self.tool_context
                            .allowed_dirs
                            .first()
                            .map(|path| path.to_string_lossy().into_owned())
                            .unwrap_or_else(|| ".".to_string())
                    )),
                self.transaction_call_id(call_id),
            ))
            .ok()
            .flatten()
        };
        if let Some(transaction) = self
            .work_step_transactions
            .remove(call_id)
            .or(recovered_transaction)
        {
            outcome = settle_recovered_delegated_transaction(transaction, outcome);
        } else if let Some(transaction_failure) =
            self.work_step_transaction_failures.remove(call_id)
        {
            outcome = transaction_failure;
        }
        let artifacts = outcome.artifacts.clone();
        self.send(DelegatedAgentEventV1::ToolCompleted {
            call_id: call_id.to_string(),
            capability_id: name.to_string(),
            output: serde_json::to_value(tool_result_contract(&outcome, None))
                .map_err(|error| error.to_string())?,
            outcome: if outcome.error {
                HostToolOutcome::Failed
            } else {
                HostToolOutcome::Succeeded
            },
            full_output_ref: outcome.full_output_ref.clone(),
            artifacts,
        })?;
        Ok(())
    }

    fn external_accounting(&mut self, mut snapshot: nola_domain::delegated_invocation::ExternalUsageSnapshot) -> Result<(), String> {
        if snapshot.basis != nola_domain::delegated_invocation::ExternalUsageBasis::SessionCumulative {
            snapshot.scope_id = self.invocation_id.clone();
        }
        snapshot.validate().map_err(str::to_string)?;
        if self.accounting.as_ref() == Some(&snapshot) { return Ok(()); }
        self.send(DelegatedAgentEventV1::Accounting { snapshot: snapshot.clone() })?;
        self.accounting = Some(snapshot);
        Ok(())
    }

    fn external_context_usage(
        &mut self,
        used_tokens: usize,
        context_window_tokens: usize,
    ) -> Result<(), String> {
        self.used_tokens = used_tokens as u64;
        self.context_window_tokens = context_window_tokens as u64;
        self.send(DelegatedAgentEventV1::Usage {
            used_tokens: self.used_tokens,
            context_window_tokens: self.context_window_tokens,
            input_tokens: None,
            output_tokens: None,
        })?;
        Ok(())
    }

    fn external_execution_cancelled(&mut self, _backend: &str) -> Result<(), String> {
        self.physical_stop_confirmed = true;
        self.output_tx.send(DelegatedAgentWorkerOutput::PhysicalStopObserved)
            .map_err(|_| "physical stop consumer disconnected".to_string())

    }

    fn external_execution_completed(&mut self, _backend: &str) -> Result<(), String> {
        let mut items = Vec::new();
        if !self.reasoning.is_empty() {
            items.push(
                serde_json::to_value(AgentItem::Reasoning {
                    id: ItemId::from_string(self.reasoning_part_id.clone()),
                    summary: String::new(),
                    content: self.reasoning.clone(),
                    provider_continuation: None,
                })
                .unwrap_or(Value::Null),
            );
        }
        if !self.text.is_empty() {
            items.push(
                serde_json::to_value(AgentItem::Message {
                    id: ItemId::from_string(self.text_part_id.clone()),
                    role: AgentRole::Assistant,
                    content: self.text.clone(),
                })
                .unwrap_or(Value::Null),
            );
        }
        let mut usage = TokenUsage::default();
        if let Some(snapshot) = self.accounting.as_ref().filter(|snapshot|
            snapshot.basis == nola_domain::delegated_invocation::ExternalUsageBasis::InvocationCumulative) {
            usage.input_tokens = snapshot.input_tokens.map(|value| value as usize);
            usage.output_tokens = snapshot.output_tokens.map(|value| value as usize);
            usage.cached_input_tokens = snapshot.cache_read_tokens.map(|value| value as usize);
            usage.cache_write_input_tokens = snapshot.cache_write_tokens.map(|value| value as usize);
        }
        self.send(DelegatedAgentEventV1::Completed {
            items, usage: serde_json::to_value(usage).map_err(|error|error.to_string())?,
        })?;
        Ok(())
    }
}

#[derive(Clone)]
struct CliBoundarySubmissionSource {
    cancellation: RuntimeExecutionControl,
    session_id: Option<String>,
    run_id: Option<String>,
    durable_lease_owner: String,
    durable_state: std::sync::Arc<std::sync::Mutex<DurableBoundarySubmissionState>>,
}

#[derive(Default)]
struct DurableBoundarySubmissionState {
    cancellation_id: Option<String>,
    cancellation_acknowledged: bool,
    in_flight: std::collections::BTreeMap<String, (String, String)>,
    requeued: std::collections::VecDeque<nola_core::agent::host::AgentBoundaryInput>,
}

fn durable_submission_key(submission: &nola_core::agent::host::AgentBoundaryInput) -> Option<String> {
    serde_json::to_string(submission).ok()
}


impl nola_core::agent::host::AgentBoundarySubmissionSource<nola_core::agent::host::AgentBoundaryInput> for CliBoundarySubmissionSource {
    fn take_pending(&self) -> Result<Vec<nola_core::agent::host::AgentBoundaryInput>, String> {
        // A user interrupt also wakes/cancels transport workers. Admit its
        // Core control command at this boundary before accepting their terminal
        // response; waiting for the outer run loop would seal a false failure.
        {
            let mut state = self.durable_state.lock()
                .map_err(|_| "boundary cancellation state poisoned")?;
            if self.cancellation.is_requested() && state.cancellation_id.is_none() {
                let run_id = self.run_id.as_deref()
                    .filter(|run_id| !run_id.trim().is_empty())
                    .ok_or("foreground cancellation has no exact dispatched Run")?;
                let cancellation_id = format!("execution-interrupt:{}:{run_id}", self.cancellation.execution_id());
                state.cancellation_id = Some(cancellation_id.clone());
                state.requeued.push_front(nola_core::agent::host::AgentBoundaryInput::Submission {
                    submission: nola_core::agent::AgentSubmission::Interrupt {
                        goal_control: None, cancellation_id,
                        reason: Some("cancelled by user".into()),
                    },
                });
            }
        }
        let mut submissions = self
            .durable_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .requeued
            .drain(..)
            .collect::<Vec<_>>();
        submissions.extend(
            self.cancellation
                .take_boundary_user_inputs()
                .into_iter()
                .map(|content| nola_core::agent::host::AgentBoundaryInput::Submission { submission: nola_core::agent::AgentSubmission::UserInput {
                    items: vec![nola_core::agent::AgentItem::user_message(
                        nola_core::agent::ItemId::from_string(format!(
                            "turn-steer:{}",
                            uuid::Uuid::new_v4()
                        )),
                        content,
                    )],
                    turn_context_items: Vec::new(),
                    turn_context_platform: nola_core::context_engine::ContextPlatform::Cli,
                    turn_context_components: Vec::new(),
                    submission_id: Some(format!("turn-steer:{}", uuid::Uuid::new_v4())),
                    submitted_at_epoch_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64),
                    // Composer input submitted while an active run is executing is a
                    // queued follow-up, not steering for the model request that is
                    // currently finishing. Core promotes NextTurn input only after the
                    // active Turn commits its final assistant item, preserving both
                    // answers and their user -> assistant ordering.
                    delivery: nola_core::agent::AgentInputDelivery::NextTurn,
                }}),
        );
        if let Some(session_id) = self
            .session_id
            .as_deref()
            .filter(|_| submissions.is_empty())
        {
            // A poll is a scheduling slice, not a drain-to-empty barrier.
            // Durable FIFO commands remain in the ledger for later polls;
            // a heartbeat producer must not delay the current Host response
            // behind an arbitrarily large batch of checkpoint commits.
            for _ in 0..1 {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .min(i64::MAX as u128) as i64;
                let lease_until_ms = now_ms.saturating_add(30_000);
                let command = match crate::agent_command_port::claim_next_agent_command(
                    session_id,
                    &self.durable_lease_owner,
                    now_ms,
                    lease_until_ms,
                ) {
                    Ok(Some(command)) => command,
                    Ok(None) => break,
                    Err(error) => return Err(error),
                };
                let submission = crate::agent_command_port::decode_claimed(&command)?;
                let Some(key) = durable_submission_key(&submission) else {
                    crate::agent_command_port::release_agent_command_lease(session_id,&command.command_id,&self.durable_lease_owner)?;
                    return Err("durable boundary input could not be encoded".into());
                };
                self.durable_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .in_flight
                    .insert(key, (command.command_id, self.durable_lease_owner.clone()));
                submissions.push(submission);
            }
        }
        Ok(submissions)
    }

    fn did_apply(&self, submission: &nola_core::agent::host::AgentBoundaryInput) -> Result<(), String> {
        if let (Some(session_id), Some(key)) = (
            self.session_id.as_deref(),
            durable_submission_key(submission),
        ) {
            let receipt = self
                .durable_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .in_flight
                .get(&key).cloned();
            if let Some((command_id, lease_owner)) = receipt {
                if !crate::agent_command_port::acknowledge_agent_command(
                    session_id,
                    &command_id,
                    &lease_owner,
                )? {
                    return Err(format!(
                        "durable boundary input {command_id} disappeared before acknowledgement"
                    ));
                }
                self.durable_state.lock().unwrap_or_else(|poisoned|poisoned.into_inner()).in_flight.remove(&key);
            }
        }
        if let nola_core::agent::host::AgentBoundaryInput::Submission {
            submission: nola_core::agent::AgentSubmission::Interrupt { cancellation_id, .. },
        } = submission {
            let mut state = self.durable_state.lock()
                .map_err(|_| "boundary cancellation state poisoned")?;
            if state.cancellation_id.as_ref() == Some(cancellation_id) {
                state.cancellation_acknowledged = true;
            }
        }
        Ok(())
    }

    fn requeue(&self, submission: &nola_core::agent::host::AgentBoundaryInput) -> Result<(), String> {
        let key=durable_submission_key(submission).ok_or("boundary input cannot be encoded")?;
        let receipt=self.durable_state.lock().unwrap_or_else(|poisoned|poisoned.into_inner())
            .in_flight.remove(&key);
        if let Some((command_id,lease_owner))=receipt {
            let session=self.session_id.as_deref().ok_or("durable boundary has no session")?;
            crate::agent_command_port::release_agent_command_lease(session,&command_id,&lease_owner)?;
        } else {
            self.durable_state.lock().unwrap_or_else(|poisoned|poisoned.into_inner()).requeued.push_back(submission.clone());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpToolRoute {
    transport: McpTransportKind,
    connection_ref: String,
    server_id: String,
    server_name: String,
    tool_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpResourceExecutionRoute {
    transport: McpTransportKind,
    connection_ref: String,
    server_id: String,
    resource_id: String,
    uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpResourceTemplateExecutionRoute {
    transport: McpTransportKind,
    connection_ref: String,
    server_id: String,
    template_id: String,
    uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpPromptExecutionRoute {
    transport: McpTransportKind,
    connection_ref: String,
    server_id: String,
    prompt_name: String,
    arguments: BTreeMap<String, String>,
}

fn mcp_catalog_access_descriptor(
    id: &str,
    title: &str,
    input_schema: Value,
) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: title.to_string(),
        route: HostCapabilityRoute::Tool,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: Some("mcp_resource".into()),
        input_schema,
        input_normalization: HostInputNormalization {
            origin: nola_core::agent::host::HostCapabilityOrigin {
                kind: nola_core::agent::host::HostCapabilityOriginKind::Mcp,
                provider_id: None,
                provider_title: None,
                namespace: Some("mcp".to_string()),
            },
            effect: nola_core::agent::host::HostCapabilityEffectDeclaration::fixed(
                nola_core::agent::host::HostCapabilityEffect::Investigate,
            ),
            permission_resource:
                nola_core::agent::host::HostPermissionResourceDeclaration::FullInput,
            ..Default::default()
        },
        output_policy: HostOutputPolicy {
            preview_char_limit: 8_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: false,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::ConcurrentSafe,
        cancellation: HostCancellationMode::BestEffort,
        replay_safety: HostReplaySafety::Idempotent,
    }
}

fn resolve_mcp_resource_route(
    catalog: &McpCapabilityCatalog,
    request: &McpResourceReadRequest,
) -> Result<McpResourceExecutionRoute, Value> {
    validate_mcp_text("server_id", &request.server_id, 512)?;
    validate_mcp_text("resource_id", &request.resource_id, 2_048)?;
    let reference = McpResourceReference {
        server_id: request.server_id.clone(),
        resource_id: request.resource_id.clone(),
    };
    let resource = catalog
        .resolve_resource(&reference)
        .map_err(mcp_recovery_output)?;
    let server = catalog.servers.get(&request.server_id).ok_or_else(|| {
        mcp_invalid_input("mcp_catalog_inconsistent", "resolved MCP server is missing")
    })?;
    Ok(McpResourceExecutionRoute {
        transport: server.transport,
        connection_ref: server.connection_ref.clone(),
        server_id: server.server_id.clone(),
        resource_id: resource.resource_id.clone(),
        uri: resource.uri.clone(),
    })
}

fn resolve_mcp_resource_template_route(
    catalog: &McpCapabilityCatalog,
    request: McpResourceTemplateReadRequest,
) -> Result<McpResourceTemplateExecutionRoute, Value> {
    validate_mcp_text("server_id", &request.server_id, 512)?;
    validate_mcp_text("template_id", &request.template_id, 4_096)?;
    if request.variables.len() > 64
        || request.variables.iter().any(|(name, value)| {
            name.is_empty() || name.len() > 512 || value.chars().count() > 16_384
        })
    {
        return Err(mcp_invalid_input(
            "mcp_resource_template_variables_invalid",
            "MCP resource template variables exceed the bounded schema",
        ));
    }
    let reference = McpResourceTemplateReference {
        server_id: request.server_id.clone(),
        template_id: request.template_id.clone(),
    };
    let template = catalog
        .resolve_resource_template(&reference)
        .map_err(mcp_recovery_output)?;
    let uri = expand_mcp_resource_uri_template(&template.uri_template, &request.variables)?;
    let server = catalog.servers.get(&request.server_id).ok_or_else(|| {
        mcp_invalid_input("mcp_catalog_inconsistent", "resolved MCP server is missing")
    })?;
    Ok(McpResourceTemplateExecutionRoute {
        transport: server.transport,
        connection_ref: server.connection_ref.clone(),
        server_id: server.server_id.clone(),
        template_id: template.template_id.clone(),
        uri,
    })
}

fn expand_mcp_resource_uri_template(
    template: &str,
    variables: &BTreeMap<String, String>,
) -> Result<String, Value> {
    if template.is_empty() || template.chars().count() > 32_768 {
        return Err(mcp_invalid_input(
            "mcp_resource_template_invalid",
            "MCP resource URI template must contain 1 to 32768 characters",
        ));
    }
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    let mut used = BTreeSet::new();
    while let Some(open) = remaining.find('{') {
        let prefix = &remaining[..open];
        if prefix.contains('}') {
            return Err(mcp_invalid_input(
                "mcp_resource_template_invalid",
                "MCP resource URI template contains an unmatched closing brace",
            ));
        }
        output.push_str(prefix);
        let expression = &remaining[open + 1..];
        let Some(close) = expression.find('}') else {
            return Err(mcp_invalid_input(
                "mcp_resource_template_invalid",
                "MCP resource URI template contains an unclosed expression",
            ));
        };
        let name = &expression[..close];
        if name.is_empty()
            || name.chars().next().is_some_and(|character| {
                matches!(character, '+' | '#' | '.' | '/' | ';' | '?' | '&')
            })
            || name
                .chars()
                .any(|character| matches!(character, ',' | ':' | '*' | '{'))
            || !name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            })
        {
            return Err(mcp_invalid_input(
                "mcp_resource_template_operator_unsupported",
                "This Host supports bounded simple {variable} MCP resource templates only",
            ));
        }
        let Some(value) = variables.get(name) else {
            return Err(mcp_invalid_input(
                "mcp_resource_template_variable_required",
                format!("MCP resource template requires variable `{name}`"),
            ));
        };
        used.insert(name.to_string());
        output.push_str(&percent_encode_uri_template_value(value));
        remaining = &expression[close + 1..];
    }
    if remaining
        .chars()
        .any(|character| matches!(character, '{' | '}'))
    {
        return Err(mcp_invalid_input(
            "mcp_resource_template_invalid",
            "MCP resource URI template contains unmatched braces",
        ));
    }
    output.push_str(remaining);
    if let Some(name) = variables.keys().find(|name| !used.contains(*name)) {
        return Err(mcp_invalid_input(
            "mcp_resource_template_variable_unknown",
            format!("MCP resource template does not declare variable `{name}`"),
        ));
    }
    if output.chars().count() > 32_768 {
        return Err(mcp_invalid_input(
            "mcp_resource_template_uri_too_large",
            "Expanded MCP resource URI exceeds 32768 characters",
        ));
    }
    Ok(output)
}

fn percent_encode_uri_template_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

fn resolve_mcp_prompt_route(
    catalog: &McpCapabilityCatalog,
    request: McpPromptGetRequest,
) -> Result<McpPromptExecutionRoute, Value> {
    validate_mcp_text("server_id", &request.server_id, 512)?;
    validate_mcp_text("prompt_name", &request.prompt_name, 512)?;
    if request.arguments.len() > 64
        || request
            .arguments
            .values()
            .any(|value| value.chars().count() > 16_384)
    {
        return Err(mcp_invalid_input(
            "mcp_prompt_arguments_too_large",
            "MCP prompt arguments exceed the bounded schema",
        ));
    }
    let reference = McpPromptReference {
        server_id: request.server_id.clone(),
        prompt_name: request.prompt_name.clone(),
    };
    let prompt = catalog
        .resolve_prompt(&reference)
        .map_err(mcp_recovery_output)?;
    let declared = prompt
        .arguments
        .iter()
        .map(|argument| argument.name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(name) = request
        .arguments
        .keys()
        .find(|name| !declared.contains(name.as_str()))
    {
        return Err(mcp_invalid_input(
            "mcp_prompt_argument_unknown",
            format!("MCP prompt does not declare argument `{name}`"),
        ));
    }
    if let Some(argument) = prompt
        .arguments
        .iter()
        .find(|argument| argument.required && !request.arguments.contains_key(&argument.name))
    {
        return Err(mcp_invalid_input(
            "mcp_prompt_argument_required",
            format!("MCP prompt requires argument `{}`", argument.name),
        ));
    }
    let server = catalog.servers.get(&request.server_id).ok_or_else(|| {
        mcp_invalid_input("mcp_catalog_inconsistent", "resolved MCP server is missing")
    })?;
    Ok(McpPromptExecutionRoute {
        transport: server.transport,
        connection_ref: server.connection_ref.clone(),
        server_id: server.server_id.clone(),
        prompt_name: prompt.name.clone(),
        arguments: request.arguments,
    })
}

fn validate_mcp_text(field: &str, value: &str, maximum: usize) -> Result<(), Value> {
    let length = value.chars().count();
    if length == 0 || length > maximum {
        return Err(mcp_invalid_input(
            "mcp_request_identifier_invalid",
            format!("MCP `{field}` must contain 1 to {maximum} characters"),
        ));
    }
    Ok(())
}

fn mcp_invalid_input(code: &str, message: impl Into<String>) -> Value {
    json!({"error": code, "message": message.into(), "retryable": false})
}

fn mcp_recovery_output(error: RecoverableCapabilityError) -> Value {
    serde_json::to_value(error).unwrap_or_else(|encode_error| {
        mcp_invalid_input("mcp_recovery_encode_failed", encode_error.to_string())
    })
}

fn emit_mcp_catalog_completion(
    request: &HostRequestEnvelope,
    sequence: &mut u64,
    call_id: &str,
    execution: Result<crate::mcp_host::McpToolExecution, Value>,
    output_store: &crate::tools::ToolOutputStore,
    emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
) -> Result<(), String> {
    let (output, is_error, full_output_ref, artifacts) = match execution {
        Ok(execution) => {
            let artifacts = execution.artifacts;
            let (output, full_output_ref) =
                bound_external_tool_output_in_store(execution.output, output_store);
            (output, execution.is_error, full_output_ref, artifacts)
        }
        Err(output) => (output, true, None, Vec::new()),
    };
    CliAgentHostDriver::emit(
        request,
        sequence,
        HostResponse::ToolCompleted {
            call_id: call_id.to_string(),
            output: external_tool_result(
                output,
                if is_error {
                    nola_host_protocol::tool_result::ExecutionState::Unknown
                } else {
                    nola_host_protocol::tool_result::ExecutionState::Completed
                },
            ),
            outcome: if is_error {
                HostToolOutcome::Failed
            } else {
                HostToolOutcome::Succeeded
            },
            activation: None,
            full_output_ref,
            artifacts,
        },
        emit,
    )?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct HostTurnPolicy;

struct IsolatedToolJob {
    request: HostRequestEnvelope,
    capability_id: String,
    call_id: String,
    args: serde_json::Map<String, Value>,
    context: ToolContext,
    failure_key: String,
    permission_contract: nola_domain::permission_resource_plan::PermissionOperationContract,
    local_execution_identity: Option<nola_domain::execution_evidence::LocalExecutionIdentity>,
}

struct IsolatedToolCompletion {
    job: IsolatedToolJob,
    outcome: ToolOutcome,
}

struct ActiveEffectCancellationGuard {
    effect_id: String,
    registry: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, ExecutionContext>>>,
}

impl Drop for ActiveEffectCancellationGuard {
    fn drop(&mut self) {
        if let Ok(mut cancellations) = self.registry.lock() {
            cancellations.remove(&self.effect_id);
        }
    }
}

include!("orchestration_execution.rs");

/// Indexing writes only Nola's rebuildable derived cache, so it is an
/// investigative Host effect even though the engine serializes database writes.
/// Deleting an index remains a user-visible mutation. This classification is
/// capability metadata and never depends on prompt wording.
fn code_intelligence_host_effect(
    operation: nola_core::code_intelligence::CodeIntelligenceOperation,
) -> nola_core::agent::host::HostCapabilityEffect {
    if operation.is_read_only()
        || operation == nola_core::code_intelligence::CodeIntelligenceOperation::Index
    {
        nola_core::agent::host::HostCapabilityEffect::Investigate
    } else {
        nola_core::agent::host::HostCapabilityEffect::Mutate
    }
}

/// A step may restrict an explicit preference, but cannot invent one.
fn constrained_parallel_tool_preference(
    preference: Option<bool>,
    step_allows: bool,
) -> Option<bool> {
    preference.map(|requested| requested && step_allows)
}


/// Provider annotations remain authoritative for ordinary MCP servers. Nola's
/// built-in code-intelligence provider has a stable Core contract, so its
/// known query operations use Core semantics even when an upstream catalog
/// incorrectly labels those operations as destructive mutations.
fn effective_mcp_tool_semantics(
    server_id: &str,
    tool_name: &str,
    provider_read_only: bool,
    provider_destructive: bool,
) -> (bool, bool) {
    let read_only = nola_core::code_intelligence::stable_capability_id(server_id, tool_name)
        .and_then(nola_core::code_intelligence::operation_for_capability_id)
        .map(nola_core::code_intelligence::CodeIntelligenceOperation::is_read_only)
        .unwrap_or(provider_read_only);
    (read_only, !read_only && provider_destructive)
}
