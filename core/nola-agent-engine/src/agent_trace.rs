// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the versioned, platform-neutral Agent Trace contract and pure reducer.
// - Security: Producers must redact sensitive values before creating a trace event.

use crate::agent::parts::{AgentPart, PartUpdate, ToolPartState};
use crate::agent::protocol::AgentEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const AGENT_TRACE_SCHEMA_VERSION: u32 = 1;
const MAX_TRACE_STRING_CHARS: usize = 12_000;
const MAX_TRACE_COLLECTION_ITEMS: usize = 128;
const MAX_TRACE_VALUE_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentTraceKind {
    #[serde(rename = "run.started")]
    RunStarted,
    #[serde(rename = "prompt.composed")]
    PromptComposed,
    #[serde(rename = "phase.changed")]
    PhaseChanged,
    #[serde(rename = "tool.started")]
    ToolStarted,
    #[serde(rename = "tool.progress")]
    ToolProgress,
    #[serde(rename = "tool.permission.requested")]
    ToolPermissionRequested,
    #[serde(rename = "tool.permission.resolved")]
    ToolPermissionResolved,
    #[serde(rename = "tool.completed")]
    ToolCompleted,
    #[serde(rename = "hook.started")]
    HookStarted,
    #[serde(rename = "hook.completed")]
    HookCompleted,
    #[serde(rename = "plan.updated")]
    PlanUpdated,
    #[serde(rename = "subagent.updated")]
    SubagentUpdated,
    #[serde(rename = "usage.snapshot")]
    UsageSnapshot,
    #[serde(rename = "memory.used")]
    MemoryUsed,
    #[serde(rename = "memory.updated")]
    MemoryUpdated,
    #[serde(rename = "context.compacted")]
    ContextCompacted,
    #[serde(rename = "validation.updated")]
    ValidationUpdated,
    #[serde(rename = "run.completed")]
    RunCompleted,
    #[serde(rename = "run.failed")]
    RunFailed,
    #[serde(rename = "run.cancelled")]
    RunCancelled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTraceStatus {
    Pending,
    #[default]
    Running,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
    Incomplete,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTraceVisibility {
    #[default]
    Summary,
    Detail,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTraceResourceOperation {
    Unknown,
    Read,
    Create,
    Update,
    Delete,
    Execute,
    Open,
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTraceResource {
    pub uri: String,
    pub operation: AgentTraceResourceOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

pub type AgentTraceUsage = crate::agent::context_window::ContextUsageSnapshot;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTraceValidation {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTracePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Stable identifier of the execution unit that owns an external session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_backend: Option<String>,
    /// Opaque session/thread identifier returned by an external execution unit.
    /// Core preserves this value but never interprets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Typed Host progress data used by clients for state and identity. Human
    /// `detail` text is presentation-only and must not be parsed as protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// True when the projector reconstructed a missing lifecycle boundary from
    /// canonical state. Consumers can keep strict start/completion pairing
    /// without presenting recovery as a fresh tool execution.
    #[serde(default, skip_serializing_if = "is_false")]
    pub recovered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AgentTraceUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_diagnostic: Option<crate::prompt_cache::PromptCacheDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<AgentTraceValidation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<AgentTraceResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<crate::memory::MemoryUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_update: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_identity: Option<AgentTracePromptIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_hook: Option<crate::lifecycle_hooks::LifecycleHookExecutionProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub source_item_count: usize,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub original_token_count: usize,
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

fn compaction_reason_name(reason: crate::agent::context_window::CompactionReason) -> &'static str {
    match reason {
        crate::agent::context_window::CompactionReason::Manual => "manual",
        crate::agent::context_window::CompactionReason::ThresholdReached => "threshold_reached",
        crate::agent::context_window::CompactionReason::ContextLimitRecovery => {
            "context_limit_recovery"
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTracePromptIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_prefix_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragment_ids: Vec<String>,
    pub fragment_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTraceEvent {
    pub schema_version: u32,
    pub trace_id: String,
    pub run_id: String,
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub turn_id: String,
    pub event_id: String,
    pub sequence: u64,
    pub emitted_at: String,
    pub kind: AgentTraceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub status: AgentTraceStatus,
    pub visibility: AgentTraceVisibility,
    #[serde(default)]
    pub payload: AgentTracePayload,
}

pub fn attach_agent_trace(value: &mut Value, trace: &AgentTraceEvent) {
    let Value::Object(object) = value else {
        return;
    };
    object.insert(
        "trace".to_string(),
        serde_json::to_value(trace).unwrap_or(Value::Null),
    );
    object.insert(
        "trace_id".to_string(),
        Value::String(trace.trace_id.clone()),
    );
    object.insert("run_id".to_string(), Value::String(trace.run_id.clone()));
    object.insert("turn_id".to_string(), Value::String(trace.turn_id.clone()));
    object.insert(
        "event_id".to_string(),
        Value::String(trace.event_id.clone()),
    );
    object.insert("sequence".to_string(), Value::from(trace.sequence));
    if let Some(message_id) = trace.message_id.as_ref() {
        object.insert("message_id".to_string(), Value::String(message_id.clone()));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTraceContext {
    pub trace_id: String,
    pub run_id: String,
    pub conversation_id: String,
    pub message_id: Option<String>,
    pub turn_id: String,
    #[serde(default)]
    next_message_id: Option<String>,
    next_sequence: u64,
    #[serde(default)]
    is_terminal: bool,
    /// Public tool lifecycle state is checkpointed with the trace cursor. A
    /// restored Host can therefore finish an interrupted call without emitting
    /// an orphan terminal event or replaying an already completed call.
    #[serde(default)]
    active_tool_calls: BTreeMap<String, TraceToolLifecycle>,
    #[serde(default)]
    completed_tool_calls: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceToolLifecycle {
    tool_name: Option<String>,
    summary: Option<String>,
    arguments: Option<Value>,
    resources: Vec<AgentTraceResource>,
}

impl AgentTraceContext {
    pub fn new(conversation_id: impl Into<String>, message_id: Option<String>) -> Self {
        let conversation_id = conversation_id.into();
        let identifier = format!(
            "context_{}",
            conversation_id
                .chars()
                .map(|character| if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '_'
                })
                .collect::<String>()
        );
        Self::with_identifier(conversation_id, message_id, identifier)
    }

    pub fn with_identifier(
        conversation_id: impl Into<String>,
        message_id: Option<String>,
        identifier: impl Into<String>,
    ) -> Self {
        let identifier = identifier.into();
        Self {
            trace_id: format!("trace_{identifier}"),
            run_id: format!("run_{identifier}"),
            conversation_id: conversation_id.into(),
            message_id: message_id.or_else(|| Some(format!("assistant_{identifier}"))),
            turn_id: format!("turn_{identifier}"),
            next_message_id: None,
            next_sequence: 1,
            is_terminal: false,
            active_tool_calls: BTreeMap::new(),
            completed_tool_calls: BTreeSet::new(),
        }
    }

    pub fn ephemeral(message_id: Option<String>) -> Self {
        Self::with_identifier("conversation_ephemeral", message_id, "ephemeral")
    }

    /// Binds a Host's pre-Core diagnostics to the admitted Turn identity.
    /// `with_identifier` creates a standalone external trace; a canonical Run
    /// must not gain its `run_`/`turn_` prefixes or a second trace namespace.
    pub fn for_turn(
        conversation_id: impl Into<String>,
        message_id: Option<String>,
        turn_id: impl Into<String>,
    ) -> Self {
        let turn_id = turn_id.into();
        let mut context = Self::with_identifier(conversation_id, None, turn_id.clone());
        context.set_next_message_id(message_id);
        context.begin_turn(turn_id);
        context
    }

    /// Starts the UI trace owned by one Core turn.
    ///
    /// A conversation keeps one durable Core event cursor, while each turn has
    /// an independent user-facing Trace sequence. Persisting this context in
    /// the session checkpoint lets a Host reconnect without regenerating or
    /// renumbering already published UI events.
    pub fn begin_turn(&mut self, turn_id: impl Into<String>) {
        let turn_id = turn_id.into();
        if !self.is_terminal && self.turn_id == turn_id {
            return;
        }
        let turn_identifier = turn_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        self.trace_id = format!("trace_{turn_identifier}");
        self.run_id = turn_id.clone();
        self.message_id = self
            .next_message_id
            .take()
            .or_else(|| Some(format!("{turn_id}:assistant")));
        self.turn_id = turn_id;
        self.next_sequence = 1;
        self.is_terminal = false;
        self.active_tool_calls.clear();
        self.completed_tool_calls.clear();
    }

    pub fn set_next_message_id(&mut self, message_id: Option<String>) {
        self.next_message_id = message_id;
    }

    pub fn project_external_event(&mut self, event: &Value) -> Option<AgentTraceEvent> {
        let event = project_external_event(
            event,
            self,
            self.next_sequence,
            format!("external_event_{}_{}", self.run_id, self.next_sequence),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
        if event.is_some() {
            self.next_sequence = self.next_sequence.saturating_add(1);
        }
        event
    }

    pub fn project_host_event(
        &mut self,
        event: &Value,
        event_id: impl Into<String>,
        emitted_at: impl Into<String>,
    ) -> Option<AgentTraceEvent> {
        let event = project_external_event(
            event,
            self,
            self.next_sequence,
            event_id.into(),
            emitted_at.into(),
        );
        if event.is_some() {
            self.next_sequence = self.next_sequence.saturating_add(1);
        }
        event
    }

    /// Projects the canonical Agent event directly into the UI trace stream.
    /// The Core cursor supplies stable identity while the trace sequence remains
    /// local to this process/run.
    pub fn project_agent_event(
        &mut self,
        event: &AgentEvent,
        event_cursor: u64,
    ) -> Option<AgentTraceEvent> {
        self.project_agent_event_at(
            event,
            event_cursor,
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )
    }

    /// Projects control diagnostics only. Canonical Parts and their journal
    /// own all assistant and reasoning content; traces never buffer that body.
    pub fn project_agent_events(
        &mut self,
        event: &AgentEvent,
        event_cursor: u64,
    ) -> Vec<AgentTraceEvent> {
        self.project_agent_events_at(
            event,
            event_cursor,
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )
    }

    pub fn project_agent_events_at(
        &mut self,
        event: &AgentEvent,
        event_cursor: u64,
        emitted_at: impl Into<String>,
    ) -> Vec<AgentTraceEvent> {
        let emitted_at = emitted_at.into();
        // A TurnStarted event is the hard boundary of a user-facing run. Reset
        // the projection before capturing the first public sequence number;
        // otherwise the new trace id is paired with the previous run's next
        // sequence and every strict client correctly reports a sequence gap.
        if let AgentEvent::TurnStarted { turn_id, .. } = event {
            self.begin_turn(turn_id.as_str().to_string());
        }
        let published_sequence = self.next_sequence;
        let mut traces = Vec::with_capacity(2);
        if let Some(trace) = self.project_agent_event_at(event, event_cursor, emitted_at) {
            let memory_trace = match event {
                AgentEvent::TurnContextCaptured { memory_usage, .. }
                    if trace.kind == AgentTraceKind::PromptComposed && !memory_usage.is_empty() =>
                {
                    let mut memory_trace = trace.clone();
                    memory_trace.event_id = format!("{}:memory", memory_trace.event_id);
                    memory_trace.kind = AgentTraceKind::MemoryUsed;
                    memory_trace.visibility = AgentTraceVisibility::Detail;
                    memory_trace.payload = AgentTracePayload {
                        summary: Some(format!("Using {} memory records", memory_usage.len())),
                        memories: memory_usage.clone(),
                        ..AgentTracePayload::default()
                    };
                    Some(memory_trace)
                }
                _ => None,
            };
            traces.extend(self.normalize_tool_lifecycle(trace));
            if let Some(memory_trace) = memory_trace {
                traces.push(memory_trace);
            }
        }
        // Presentation-invisible Core events must not leak as holes in the
        // public Trace sequence. Every client and replay reducer should be able
        // to consume the published stream without guessing whether a missing
        // sequence was hidden or lost.
        for (offset, trace) in traces.iter_mut().enumerate() {
            trace.sequence = published_sequence.saturating_add(offset as u64);
        }
        self.next_sequence = published_sequence.saturating_add(traces.len() as u64);
        traces
    }

    fn normalize_tool_lifecycle(&mut self, trace: AgentTraceEvent) -> Vec<AgentTraceEvent> {
        match trace.kind {
            AgentTraceKind::ToolStarted => {
                let Some(call_id) = trace.payload.call_id.clone() else {
                    return vec![trace];
                };
                // Canonical Part updates may be replayed after a response was
                // committed. Replaying Running must not open a second public
                // lifecycle for the same logical call.
                if self.active_tool_calls.contains_key(&call_id)
                    || self.completed_tool_calls.contains(&call_id)
                {
                    return Vec::new();
                }
                self.active_tool_calls.insert(
                    call_id,
                    TraceToolLifecycle {
                        tool_name: trace.payload.tool_name.clone(),
                        summary: trace.payload.summary.clone(),
                        arguments: trace.payload.arguments.clone(),
                        resources: trace.payload.resources.clone(),
                    },
                );
                vec![trace]
            }
            AgentTraceKind::ToolCompleted => {
                let Some(call_id) = trace.payload.call_id.clone() else {
                    return vec![trace];
                };
                if self.completed_tool_calls.contains(&call_id) {
                    return Vec::new();
                }
                let recovered_start =
                    self.active_tool_calls.remove(&call_id).is_none().then(|| {
                        let mut start = trace.clone();
                        start.event_id.push_str(":recovered_start");
                        start.kind = AgentTraceKind::ToolStarted;
                        start.status = AgentTraceStatus::Running;
                        start.payload.detail = None;
                        start.payload.result = None;
                        start.payload.reason = Some(
                            "Tool execution was resolved before a running state was published."
                                .to_string(),
                        );
                        start.payload.code = Some("recovered_tool_start".to_string());
                        start.payload.recovered = true;
                        start
                    });
                self.completed_tool_calls.insert(call_id);
                recovered_start.into_iter().chain([trace]).collect()
            }
            AgentTraceKind::RunCompleted
            | AgentTraceKind::RunFailed
            | AgentTraceKind::RunCancelled => {
                let interrupted_status = match trace.kind {
                    AgentTraceKind::RunCancelled => AgentTraceStatus::Cancelled,
                    _ => AgentTraceStatus::Failed,
                };
                let active = std::mem::take(&mut self.active_tool_calls);
                let mut normalized = Vec::with_capacity(active.len() + 1);
                for (index, (call_id, lifecycle)) in active.into_iter().enumerate() {
                    self.completed_tool_calls.insert(call_id.clone());
                    let mut completed = trace.clone();
                    completed.event_id = format!("{}:interrupted_tool_{index}", trace.event_id);
                    completed.kind = AgentTraceKind::ToolCompleted;
                    completed.span_id = Some(call_id.clone());
                    completed.status = interrupted_status;
                    completed.visibility = AgentTraceVisibility::Summary;
                    completed.payload = AgentTracePayload {
                        summary: lifecycle.summary,
                        tool_name: lifecycle.tool_name,
                        call_id: Some(call_id),
                        arguments: lifecycle.arguments,
                        reason: Some(
                            "Tool execution was interrupted by the terminal run state.".to_string(),
                        ),
                        code: Some("tool_interrupted_by_run_terminal".to_string()),
                        recovered: true,
                        resources: lifecycle.resources,
                        ..AgentTracePayload::default()
                    };
                    normalized.push(completed);
                }
                normalized.push(trace);
                normalized
            }
            _ => vec![trace],
        }
    }

    pub fn project_agent_event_at(
        &mut self,
        event: &AgentEvent,
        event_cursor: u64,
        emitted_at: impl Into<String>,
    ) -> Option<AgentTraceEvent> {
        if self.is_terminal && !matches!(event, AgentEvent::TurnStarted { .. }) {
            return None;
        }
        if let AgentEvent::TurnStarted { turn_id, .. } = event {
            self.begin_turn(turn_id.as_str().to_string());
        }
        let (kind, status, visibility, span_id, payload) = match event {
            AgentEvent::SessionStarted { session_id } => {
                self.conversation_id = session_id.0.clone();
                return None;
            }
            AgentEvent::TurnStarted { turn_id, .. } => {
                self.turn_id = turn_id.0.clone();
                (
                    AgentTraceKind::RunStarted,
                    AgentTraceStatus::Running,
                    AgentTraceVisibility::Summary,
                    None,
                    AgentTracePayload {
                        phase: Some("processing".to_string()),
                        summary: Some("Nola is processing".to_string()),
                        ..AgentTracePayload::default()
                    },
                )
            }
            AgentEvent::PartUpdated { update, part, .. } => match update {
                PartUpdate::TextDelta { .. }
                | PartUpdate::TextCommittedDelta { .. }
                | PartUpdate::TextReplaced { .. }
                | PartUpdate::ReasoningDelta { .. } => return None,
                PartUpdate::ReasoningReplaced { .. } | PartUpdate::ToolInputReplaced { .. } => {
                    return None
                }
                // Providers stream tool arguments as tiny JSON fragments. The
                // resolved arguments are emitted once by ToolState::Running, so
                // persisting every fragment only bloats long-session traces.
                PartUpdate::ToolInputDelta { .. } => return None,
                PartUpdate::ToolProgress { title, detail, .. } => {
                    let AgentPart::Tool(tool) = part else {
                        return None;
                    };
                    (
                        AgentTraceKind::ToolProgress,
                        AgentTraceStatus::Running,
                        AgentTraceVisibility::Summary,
                        Some(tool.call_id.clone()),
                        AgentTracePayload {
                            summary: title.clone().or_else(|| Some(tool.title.clone())),
                            detail: detail.clone(),
                            tool_name: Some(tool.capability_id.clone()),
                            call_id: Some(tool.call_id.clone()),
                            ..AgentTracePayload::default()
                        },
                    )
                }
                PartUpdate::ToolPermission { permission, .. } => {
                    let AgentPart::Tool(tool) = part else {
                        return None;
                    };
                    let resolved = permission.decision.is_some();
                    (
                        if resolved {
                            AgentTraceKind::ToolPermissionResolved
                        } else {
                            AgentTraceKind::ToolPermissionRequested
                        },
                        if permission.decision.as_deref() == Some("deny") {
                            AgentTraceStatus::Failed
                        } else if resolved {
                            AgentTraceStatus::Running
                        } else {
                            AgentTraceStatus::Waiting
                        },
                        AgentTraceVisibility::Summary,
                        Some(tool.call_id.clone()),
                        AgentTracePayload {
                            summary: Some(tool.title.clone()),
                            tool_name: Some(tool.capability_id.clone()),
                            call_id: Some(tool.call_id.clone()),
                            arguments: Some(sanitize_trace_value(&tool.input, 0)),
                            permission_request_id: Some(permission.request_id.clone()),
                            permission_scope: Some(permission.permission_domain.clone()),
                            permission_decision: permission.decision.clone(),
                            reason: Some(permission.rationale.clone()),
                            ..AgentTracePayload::default()
                        },
                    )
                }
                PartUpdate::Created { .. } => return None,
                PartUpdate::ToolState { .. } => {
                    let AgentPart::Tool(tool) = part else {
                        return None;
                    };
                    match tool.state {
                        ToolPartState::WaitingForInput => (
                            AgentTraceKind::ToolProgress, AgentTraceStatus::Waiting,
                            AgentTraceVisibility::Summary, Some(tool.call_id.clone()),
                            AgentTracePayload { summary: Some(tool.title.clone()), tool_name: Some(tool.capability_id.clone()),
                                call_id: Some(tool.call_id.clone()), ..AgentTracePayload::default() },
                        ),
                        ToolPartState::WaitingForPermission => (
                            AgentTraceKind::ToolPermissionRequested,
                            AgentTraceStatus::Waiting,
                            AgentTraceVisibility::Summary,
                            Some(tool.call_id.clone()),
                            AgentTracePayload {
                                summary: Some(tool.title.clone()),
                                tool_name: Some(tool.capability_id.clone()),
                                call_id: Some(tool.call_id.clone()),
                                arguments: Some(sanitize_trace_value(&tool.input, 0)),
                                permission_request_id: tool
                                    .permission
                                    .as_ref()
                                    .map(|permission| permission.request_id.clone()),
                                permission_scope: tool
                                    .permission
                                    .as_ref()
                                    .map(|permission| permission.permission_domain.clone()),
                                reason: tool
                                    .permission
                                    .as_ref()
                                    .map(|permission| permission.rationale.clone()),
                                ..AgentTracePayload::default()
                            },
                        ),
                        ToolPartState::Running => (
                            AgentTraceKind::ToolStarted,
                            AgentTraceStatus::Running,
                            AgentTraceVisibility::Summary,
                            Some(tool.call_id.clone()),
                            AgentTracePayload {
                                summary: tool.summary.clone(),
                                tool_name: Some(tool.capability_id.clone()),
                                call_id: Some(tool.call_id.clone()),
                                arguments: Some(sanitize_trace_value(&tool.input, 0)),
                                resources: extract_resources(
                                    &tool.input,
                                    Some(tool.capability_id.as_str()),
                                ),
                                ..AgentTracePayload::default()
                            },
                        ),
                        ToolPartState::Completed
                        | ToolPartState::Failed
                        | ToolPartState::Cancelled
                        | ToolPartState::Sealed => (
                            AgentTraceKind::ToolCompleted,
                            if tool.state == ToolPartState::Completed {
                                AgentTraceStatus::Succeeded
                            } else if tool.state == ToolPartState::Sealed {
                                AgentTraceStatus::Incomplete
                            } else if tool.state == ToolPartState::Cancelled {
                                AgentTraceStatus::Cancelled
                            } else {
                                AgentTraceStatus::Failed
                            },
                            AgentTraceVisibility::Summary,
                            Some(tool.call_id.clone()),
                            AgentTracePayload {
                                summary: tool.summary.clone(),
                                tool_name: Some(tool.capability_id.clone()),
                                call_id: Some(tool.call_id.clone()),
                                arguments: Some(sanitize_trace_value(&tool.input, 0)),
                                detail: tool.output_preview.clone(),
                                result: Some(serde_json::json!({
                                    "output_preview": tool.output_preview,
                                    "full_output_ref": tool.full_output_ref,
                                    "artifacts": tool.artifacts,
                                })),
                                resources: artifact_resources(
                                    &tool.artifacts,
                                    tool.capability_id.as_str(),
                                )
                                .into_iter()
                                .chain(extract_resources(
                                    &tool.input,
                                    Some(tool.capability_id.as_str()),
                                ))
                                .collect(),
                                ..AgentTracePayload::default()
                            },
                        ),
                        ToolPartState::Pending => return None,
                    }
                }
                PartUpdate::ToolInputResolved { .. }
                | PartUpdate::Completed { .. }
                | PartUpdate::TextReviewChanged { .. } => {
                    return None;
                }
            },
            AgentEvent::HostProgress {
                effect_id,
                title,
                detail,
                metadata,
                ..
            } => (
                AgentTraceKind::ToolProgress,
                AgentTraceStatus::Running,
                AgentTraceVisibility::Summary,
                Some(effect_id.clone()),
                AgentTracePayload {
                    phase: Some("working".to_string()),
                    summary: Some(title.clone()),
                    detail: detail.clone(),
                    metadata: (!metadata.is_null()).then(|| metadata.clone()),
                    ..AgentTracePayload::default()
                },
            ),
            AgentEvent::LifecycleHookUpdated { projection, .. } => {
                let (kind, status) = match projection.status {
                    crate::lifecycle_hooks::LifecycleHookExecutionStatus::Running => {
                        (AgentTraceKind::HookStarted, AgentTraceStatus::Running)
                    }
                    crate::lifecycle_hooks::LifecycleHookExecutionStatus::Succeeded => {
                        (AgentTraceKind::HookCompleted, AgentTraceStatus::Succeeded)
                    }
                    crate::lifecycle_hooks::LifecycleHookExecutionStatus::Failed
                    | crate::lifecycle_hooks::LifecycleHookExecutionStatus::TimedOut => {
                        (AgentTraceKind::HookCompleted, AgentTraceStatus::Failed)
                    }
                    crate::lifecycle_hooks::LifecycleHookExecutionStatus::Cancelled => {
                        (AgentTraceKind::HookCompleted, AgentTraceStatus::Cancelled)
                    }
                };
                (
                    kind,
                    status,
                    AgentTraceVisibility::Detail,
                    Some(projection.invocation_id.clone()),
                    AgentTracePayload {
                        summary: projection
                            .display_name
                            .clone()
                            .or_else(|| Some(projection.handler_id.clone())),
                        detail: projection.output_preview.clone(),
                        reason: projection.error_code.clone(),
                        lifecycle_hook: Some(projection.clone()),
                        ..AgentTracePayload::default()
                    },
                )
            }
            AgentEvent::EngineeringLoopUpdated { state, .. } => (
                AgentTraceKind::ValidationUpdated,
                if state.validation_evidence_insufficient() {
                    AgentTraceStatus::Incomplete
                } else {
                    AgentTraceStatus::Running
                },
                AgentTraceVisibility::Summary,
                None,
                AgentTracePayload {
                    phase: Some(state.phase.as_str().to_string()),
                    detail: state.last_failure.clone(),
                    validation: Some(AgentTraceValidation {
                        status: if state.validation_evidence_insufficient() {
                            "evidence_insufficient".to_string()
                        } else if state.validation_required {
                            "required".to_string()
                        } else {
                            // Absence of debt is not evidence that a check ran
                            // or that the user's requirements were accepted.
                            "not_required".to_string()
                        },
                        level: Some("targeted".to_string()),
                        summary: Some(format!(
                            "{} change(s), {} validation attempt(s), {} correction(s)",
                            state.mutation_count,
                            state.validation_attempt_count,
                            state.correction_count
                        )),
                        next_action: (state.validation_required
                            && !state.validation_evidence_insufficient()).then(|| {
                            if state.phase.as_str() == "correcting" {
                                "correct_then_validate".to_string()
                            } else {
                                "run_targeted_validation".to_string()
                            }
                        }),
                    }),
                    ..AgentTracePayload::default()
                },
            ),
            AgentEvent::HostDisconnected { reason, .. } => (
                AgentTraceKind::PhaseChanged,
                AgentTraceStatus::Failed,
                AgentTraceVisibility::Summary,
                None,
                AgentTracePayload {
                    phase: Some("host_disconnected".to_string()),
                    detail: Some(reason.clone()),
                    reason: Some(reason.clone()),
                    ..AgentTracePayload::default()
                },
            ),
            AgentEvent::ContextUpdated {
                used_tokens,
                context_window_tokens,
                usage_percent,
                usage_is_estimated,
                context_usage,
                prompt_cache_diagnostic,
                ..
            } => (
                AgentTraceKind::UsageSnapshot,
                AgentTraceStatus::Running,
                AgentTraceVisibility::Detail,
                None,
                AgentTracePayload {
                    usage: Some(if context_usage.context_window_tokens > 0 {
                        *context_usage
                    } else {
                        AgentTraceUsage {
                            used_tokens: *used_tokens,
                            context_window_tokens: *context_window_tokens,
                            prompt_token_limit: *context_window_tokens,
                            usage_percent: *usage_percent,
                            usage_is_estimated: *usage_is_estimated,
                            ..AgentTraceUsage::default()
                        }
                    }),
                    prompt_cache_diagnostic: prompt_cache_diagnostic.clone(),
                    ..AgentTracePayload::default()
                },
            ),
            AgentEvent::CompactionRequested {
                reason,
                item_count,
                estimated_tokens,
                source_fingerprint,
                source_item_count,
                original_token_count,
            } => (
                AgentTraceKind::PhaseChanged,
                AgentTraceStatus::Running,
                AgentTraceVisibility::Summary,
                None,
                AgentTracePayload {
                    phase: Some("compacting".to_string()),
                    reason: Some(compaction_reason_name(*reason).to_string()),
                    code: Some(compaction_reason_name(*reason).to_string()),
                    detail: Some(format!(
                        "Compacting {item_count} context items (~{estimated_tokens} tokens)"
                    )),
                    source_fingerprint: (!source_fingerprint.is_empty())
                        .then(|| source_fingerprint.clone()),
                    source_item_count: *source_item_count,
                    original_token_count: *original_token_count,
                    ..AgentTracePayload::default()
                },
            ),
            AgentEvent::CompactionCompleted {
                summary,
                replaced_item_count,
                remaining_item_count,
                reason,
                compaction_item_id,
                source_fingerprint,
                source_item_count,
                original_token_count,
            } => (
                AgentTraceKind::ContextCompacted,
                AgentTraceStatus::Succeeded,
                AgentTraceVisibility::Detail,
                None,
                AgentTracePayload {
                    phase: Some("compacted".to_string()),
                    reason: reason.map(|value| compaction_reason_name(value).to_string()),
                    summary: Some(summary.clone()),
                    detail: Some(format!(
                        "Compacted {replaced_item_count} items; {remaining_item_count} remain"
                    )),
                    compaction_item_id: compaction_item_id
                        .as_ref()
                        .map(|value| value.as_str().to_string()),
                    source_fingerprint: source_fingerprint.clone(),
                    source_item_count: *source_item_count,
                    original_token_count: *original_token_count,
                    ..AgentTracePayload::default()
                },
            ),
            AgentEvent::CompactionFailed { error } => (
                AgentTraceKind::PhaseChanged,
                AgentTraceStatus::Failed,
                AgentTraceVisibility::Summary,
                None,
                AgentTracePayload {
                    phase: Some("compaction_failed".to_string()),
                    detail: Some(error.clone()),
                    reason: Some(error.clone()),
                    ..AgentTracePayload::default()
                },
            ),
            AgentEvent::TurnCompleted {
                turn_id, outcome, ..
            } => {
                self.turn_id = turn_id.0.clone();
                // A completed execution slice is not necessarily a completed
                // task. Preserve Core's explicit outcome, including absence.
                let (status, reason) = match outcome {
                    Some(nola_domain::TaskCompletionOutcome::Answered) => (AgentTraceStatus::Succeeded, "answered"),
                    Some(nola_domain::TaskCompletionOutcome::ActionCompleted) => (AgentTraceStatus::Succeeded, "action_completed"),
                    Some(nola_domain::TaskCompletionOutcome::NeedsInput) => (AgentTraceStatus::Waiting, "needs_input"),
                    Some(nola_domain::TaskCompletionOutcome::Blocked) => (AgentTraceStatus::Incomplete, "blocked"),
                    Some(nola_domain::TaskCompletionOutcome::Incomplete) => (AgentTraceStatus::Incomplete, "incomplete"),
                    None => (AgentTraceStatus::Incomplete, "outcome_missing"),
                };
                (
                    AgentTraceKind::RunCompleted,
                    status,
                    AgentTraceVisibility::Summary,
                    None,
                    AgentTracePayload {
                        reason: Some(reason.to_string()),
                        ..AgentTracePayload::default()
                    },
                )
            }
            AgentEvent::TurnCancelled {
                turn_id, reason, ..
            } => {
                self.turn_id = turn_id.0.clone();
                (
                    AgentTraceKind::RunCancelled,
                    AgentTraceStatus::Cancelled,
                    AgentTraceVisibility::Summary,
                    None,
                    AgentTracePayload {
                        reason: Some(reason.clone()),
                        ..AgentTracePayload::default()
                    },
                )
            }
            AgentEvent::TurnFailed { turn_id, error, .. } => {
                self.turn_id = turn_id.0.clone();
                (
                    AgentTraceKind::RunFailed,
                    AgentTraceStatus::Failed,
                    AgentTraceVisibility::Summary,
                    None,
                    AgentTracePayload {
                        detail: Some(error.clone()),
                        reason: Some(error.clone()),
                        ..AgentTracePayload::default()
                    },
                )
            }
            AgentEvent::TurnContextCaptured {
                stable_prefix_digest,
                prompt_fragment_ids,
                memory_usage,
                ..
            } => match sanitize_prompt_identity(stable_prefix_digest, prompt_fragment_ids) {
                Some(prompt_identity) => (
                    AgentTraceKind::PromptComposed,
                    AgentTraceStatus::Succeeded,
                    AgentTraceVisibility::Internal,
                    None,
                    AgentTracePayload {
                        prompt_identity: Some(prompt_identity),
                        ..AgentTracePayload::default()
                    },
                ),
                None if !memory_usage.is_empty() => (
                    AgentTraceKind::MemoryUsed,
                    AgentTraceStatus::Succeeded,
                    AgentTraceVisibility::Detail,
                    None,
                    AgentTracePayload {
                        summary: Some(format!("Using {} memory records", memory_usage.len())),
                        memories: memory_usage.clone(),
                        ..AgentTracePayload::default()
                    },
                ),
                None => return None,
            },
            AgentEvent::MemoryUpdated {
                memory_projection_revision,
                update,
            } => (
                AgentTraceKind::MemoryUpdated,
                AgentTraceStatus::Succeeded,
                AgentTraceVisibility::Detail,
                None,
                AgentTracePayload {
                    summary: Some("Memory updated".to_string()),
                    detail: Some(format!(
                        "Memory projection revision {memory_projection_revision}"
                    )),
                    memory_update: serde_json::to_value(update)
                        .ok()
                        .map(|value| sanitize_trace_value(&value, 0)),
                    ..AgentTracePayload::default()
                },
            ),
            AgentEvent::InputQueued { .. }
            | AgentEvent::InputApplied { .. }
            | AgentEvent::AssistantSegmentStarted { .. }
            | AgentEvent::AssistantEffectBound { .. }
            | AgentEvent::AssistantActivityBound { .. }
            | AgentEvent::InputAccepted { .. }
            | AgentEvent::InputWithdrawn { .. }
            | AgentEvent::UserInputWaitChanged { .. }
            | AgentEvent::PermissionSettled { .. }
            | AgentEvent::ExecutionEvidenceObserved { .. }
            | AgentEvent::RuntimeDecisionFactObserved { .. }
            | AgentEvent::RequirementScopeChanged { .. }
            | AgentEvent::RequirementAssessed { .. }
            | AgentEvent::RunResultDecided { .. }
            | AgentEvent::CapabilitiesActivated { .. }
            | AgentEvent::TaskUpdated { .. }
            | AgentEvent::GoalUpdated { .. }
            | AgentEvent::SkillRuntimeUpdated { .. }
            | AgentEvent::HostCatalogUpdated { .. }
            | AgentEvent::HostLeaseRenewed { .. }
            | AgentEvent::SessionConfigurationUpdated { .. } => return None,
        };

        let trace = AgentTraceEvent {
            schema_version: AGENT_TRACE_SCHEMA_VERSION,
            trace_id: self.trace_id.clone(),
            run_id: self.run_id.clone(),
            conversation_id: self.conversation_id.clone(),
            message_id: self.message_id.clone(),
            turn_id: self.turn_id.clone(),
            event_id: format!("core_event_{}_{}", self.run_id, event_cursor),
            sequence: self.next_sequence,
            emitted_at: emitted_at.into(),
            kind,
            span_id,
            parent_span_id: Some(self.turn_id.clone()),
            status,
            visibility,
            payload,
        };
        if matches!(
            trace.kind,
            AgentTraceKind::RunCompleted | AgentTraceKind::RunFailed | AgentTraceKind::RunCancelled
        ) {
            self.is_terminal = true;
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        Some(trace)
    }
}

pub fn project_external_event(
    event: &Value,
    context: &AgentTraceContext,
    sequence: u64,
    event_id: String,
    emitted_at: String,
) -> Option<AgentTraceEvent> {
    let event_type = string_at(event, "type").unwrap_or("status");
    if matches!(
        event_type,
        "assistant_delta" | "assistant_reset" | "thinking_delta"
    ) {
        // Canonical journal content is never duplicated into diagnostics.
        return None;
    }
    let is_error = bool_at(event, "error")
        || bool_at(event, "is_error")
        || string_at(event, "reason")
            .map(|value| matches!(value, "failed" | "error" | "permission_denied"))
            .unwrap_or(false);
    let (kind, status, visibility) = match event_type {
        "run_started" => (
            AgentTraceKind::RunStarted,
            AgentTraceStatus::Running,
            AgentTraceVisibility::Summary,
        ),
        "tool_started" => (
            AgentTraceKind::ToolStarted,
            AgentTraceStatus::Running,
            AgentTraceVisibility::Summary,
        ),
        "tool_progress" => (
            AgentTraceKind::ToolProgress,
            AgentTraceStatus::Running,
            AgentTraceVisibility::Summary,
        ),
        "tool_finished" => (
            AgentTraceKind::ToolCompleted,
            if is_error {
                AgentTraceStatus::Failed
            } else {
                AgentTraceStatus::Succeeded
            },
            AgentTraceVisibility::Summary,
        ),
        "hook_started" => (
            AgentTraceKind::HookStarted,
            AgentTraceStatus::Running,
            AgentTraceVisibility::Detail,
        ),
        "hook_completed" => (
            AgentTraceKind::HookCompleted,
            if is_error {
                AgentTraceStatus::Failed
            } else {
                AgentTraceStatus::Succeeded
            },
            AgentTraceVisibility::Detail,
        ),
        "permission_request" => (
            AgentTraceKind::ToolPermissionRequested,
            AgentTraceStatus::Waiting,
            AgentTraceVisibility::Summary,
        ),
        "permission_resolved" => (
            AgentTraceKind::ToolPermissionResolved,
            if string_at(event, "decision") == Some("deny") {
                AgentTraceStatus::Failed
            } else {
                AgentTraceStatus::Running
            },
            AgentTraceVisibility::Summary,
        ),
        "context_usage" => (
            AgentTraceKind::UsageSnapshot,
            AgentTraceStatus::Running,
            AgentTraceVisibility::Detail,
        ),
        "plan_updated" => (
            AgentTraceKind::PlanUpdated,
            AgentTraceStatus::Running,
            AgentTraceVisibility::Detail,
        ),
        "validation_updated" => (
            AgentTraceKind::ValidationUpdated,
            AgentTraceStatus::Running,
            AgentTraceVisibility::Summary,
        ),
        "subagent_event" => (
            AgentTraceKind::SubagentUpdated,
            AgentTraceStatus::Running,
            AgentTraceVisibility::Summary,
        ),
        "done" => (
            AgentTraceKind::RunCompleted,
            AgentTraceStatus::Succeeded,
            AgentTraceVisibility::Summary,
        ),
        "error" => (
            AgentTraceKind::RunFailed,
            AgentTraceStatus::Failed,
            AgentTraceVisibility::Summary,
        ),
        "cancelled" => (
            AgentTraceKind::RunCancelled,
            AgentTraceStatus::Cancelled,
            AgentTraceVisibility::Summary,
        ),
        "status" | "external_session_started" => (
            AgentTraceKind::PhaseChanged,
            AgentTraceStatus::Running,
            AgentTraceVisibility::Summary,
        ),
        _ => (
            AgentTraceKind::RunFailed,
            AgentTraceStatus::Failed,
            AgentTraceVisibility::Summary,
        ),
    };
    let call_id = first_string(event, &["call_id"]);
    let tool_name = first_string(event, &["tool_name"]);
    let arguments = event
        .get("arguments")
        .map(|value| sanitize_trace_value(value, 0));
    let result =
        matches!(kind, AgentTraceKind::ToolCompleted).then(|| sanitize_trace_value(event, 0));
    let resources = extract_resources(arguments.as_ref().unwrap_or(event), tool_name.as_deref());
    let validation = validation_from_external_event(event);
    let status = validation.as_ref().filter(|_| kind == AgentTraceKind::ValidationUpdated).map_or(status, |validation| {
        match validation.status.to_ascii_lowercase().as_str() {
            "failed" | "error" => AgentTraceStatus::Failed,
            "incomplete" | "blocked" | "evidence_insufficient" => AgentTraceStatus::Incomplete,
            // This is diagnostic metadata, not authority to complete a Run
            // or to create a permission wait. Preserve the source status.
            _ => status,
        }
    });
    // Attached diagnostics never replace lifecycle facts. In particular a
    // permission wait, tool terminal, or Run terminal must remain observable.
    let payload = AgentTracePayload {
        phase: first_string(event, &["phase"]).or_else(|| {
            (event_type == "external_session_started")
                .then(|| "external_session_started".to_string())
        }),
        execution_backend: first_string(event, &["execution_backend"]),
        external_session_id: first_string(event, &["external_session_id"]),
        summary: first_string(event, &["summary"]),
        detail: first_string(event, &["detail"]),
        tool_name,
        call_id: call_id.clone(),
        arguments,
        result,
        permission_request_id: first_string(event, &["permission_request_id"]).or_else(|| {
            matches!(kind, AgentTraceKind::ToolPermissionRequested)
                .then(|| call_id.clone())
                .flatten()
        }),
        permission_scope: first_string(event, &["permission_scope"]),
        permission_decision: first_string(event, &["permission_decision"]),
        reason: first_string(event, &["reason"]),
        code: first_string(event, &["code"]),
        usage: usage_from_external_event(event),
        validation,
        resources,
        ..AgentTracePayload::default()
    };
    let span_id = call_id
        .clone()
        .or_else(|| first_string(event, &["span_id"]));

    Some(AgentTraceEvent {
        schema_version: AGENT_TRACE_SCHEMA_VERSION,
        trace_id: context.trace_id.clone(),
        run_id: context.run_id.clone(),
        conversation_id: context.conversation_id.clone(),
        message_id: context.message_id.clone(),
        turn_id: context.turn_id.clone(),
        event_id,
        sequence,
        emitted_at,
        kind,
        span_id,
        parent_span_id: Some(context.turn_id.clone()),
        status,
        visibility,
        payload,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTraceActivity {
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub status: AgentTraceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<AgentTraceResource>,
    pub started_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_sequence: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentRunProjection {
    pub trace_id: String,
    pub run_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub status: AgentTraceStatus,
    pub phase: Option<String>,
    pub activities: Vec<AgentTraceActivity>,
    pub usage: Option<AgentTraceUsage>,
    pub validation: Option<AgentTraceValidation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_updates: Vec<Value>,
    pub terminal_reason: Option<String>,
    pub last_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTraceApplyResult {
    Applied,
    Duplicate,
    Gap { expected: u64, received: u64 },
    OutOfOrder { last: u64, received: u64 },
    IdentityMismatch,
    UnsupportedSchema(u32),
    EventConflict,
    InvalidEvent,
    TerminalAlreadyObserved,
}

#[derive(Debug, Default)]
pub struct AgentTraceReducer {
    projection: AgentRunProjection,
    // Fixed-size payload digests avoid retaining another transcript copy.
    seen_event_ids: BTreeMap<String, [u8; 32]>,
    // Terminality comes from lifecycle kind, not task outcome/status.
    run_terminal_observed: bool,
}

impl AgentTraceReducer {
    pub fn projection(&self) -> &AgentRunProjection {
        &self.projection
    }

    pub fn apply(&mut self, event: &AgentTraceEvent) -> AgentTraceApplyResult {
        if event.schema_version != AGENT_TRACE_SCHEMA_VERSION {
            return AgentTraceApplyResult::UnsupportedSchema(event.schema_version);
        }
        // Bind on the first accepted event, not on an out-of-order arrival.
        // Check before event-id deduplication: another Run may reuse that ID.
        if self.projection.last_sequence > 0
            && (event.trace_id != self.projection.trace_id
                || event.run_id != self.projection.run_id
                || event.conversation_id != self.projection.conversation_id)
        {
            return AgentTraceApplyResult::IdentityMismatch;
        }
        let Ok(bytes) = serde_json::to_vec(event) else {
            return AgentTraceApplyResult::InvalidEvent;
        };
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        if let Some(previous) = self.seen_event_ids.get(&event.event_id) {
            return if previous == &digest {
                AgentTraceApplyResult::Duplicate
            } else {
                AgentTraceApplyResult::EventConflict
            };
        }
        let expected = self.projection.last_sequence.saturating_add(1);
        if event.sequence > expected {
            return AgentTraceApplyResult::Gap {
                expected,
                received: event.sequence,
            };
        }
        if event.sequence < expected {
            return AgentTraceApplyResult::OutOfOrder {
                last: self.projection.last_sequence,
                received: event.sequence,
            };
        }
        if self.run_terminal_observed && matches!(event.kind,
            AgentTraceKind::RunStarted | AgentTraceKind::RunCompleted
            | AgentTraceKind::RunFailed | AgentTraceKind::RunCancelled)
        {
            return AgentTraceApplyResult::TerminalAlreadyObserved;
        }
        self.seen_event_ids.insert(event.event_id.clone(), digest);
        self.projection.last_sequence = event.sequence;
        self.projection.trace_id = event.trace_id.clone();
        self.projection.run_id = event.run_id.clone();
        self.projection.conversation_id = event.conversation_id.clone();
        self.projection.turn_id = event.turn_id.clone();
        // Activity results and diagnostic success/failure are not Run results.
        // Once a Run terminal is observed, late telemetry cannot revive it.
        let run_was_terminal = self.run_terminal_observed;
        if !run_was_terminal {
            match event.kind {
                AgentTraceKind::RunCompleted
                | AgentTraceKind::RunFailed
                | AgentTraceKind::RunCancelled
                | AgentTraceKind::RunStarted => self.projection.status = event.status,
                AgentTraceKind::PhaseChanged
                    if matches!(event.status, AgentTraceStatus::Pending | AgentTraceStatus::Running | AgentTraceStatus::Waiting) => {
                        self.projection.status = event.status;
                    }
                _ => {}
            }
        }
        self.apply_payload(event, run_was_terminal);
        self.run_terminal_observed |= matches!(event.kind,
            AgentTraceKind::RunCompleted | AgentTraceKind::RunFailed | AgentTraceKind::RunCancelled);
        AgentTraceApplyResult::Applied
    }

    fn apply_payload(&mut self, event: &AgentTraceEvent, run_was_terminal: bool) {
        if let Some(validation) = &event.payload.validation {
            self.projection.validation = Some(validation.clone());
        }
        match event.kind {
            AgentTraceKind::ToolStarted
            | AgentTraceKind::ToolProgress
            | AgentTraceKind::ToolPermissionRequested
            | AgentTraceKind::ToolPermissionResolved
            | AgentTraceKind::ToolCompleted
            | AgentTraceKind::HookStarted
            | AgentTraceKind::HookCompleted => self.apply_activity(event),
            AgentTraceKind::UsageSnapshot => {
                self.projection.usage = event.payload.usage.clone();
            }
            AgentTraceKind::ValidationUpdated => {
                self.projection.validation = event.payload.validation.clone();
            }
            AgentTraceKind::MemoryUpdated => {
                if let Some(update) = event.payload.memory_update.clone() {
                    self.projection.memory_updates.push(update);
                }
            }
            AgentTraceKind::PhaseChanged => {
                if !run_was_terminal && event.payload.phase.is_some() {
                    self.projection.phase = event.payload.phase.clone();
                }
                if event.payload.validation.is_some() {
                    self.projection.validation = event.payload.validation.clone();
                }
            }
            AgentTraceKind::RunCompleted
            | AgentTraceKind::RunFailed
            | AgentTraceKind::RunCancelled => {
                if !run_was_terminal {
                    self.projection.terminal_reason = event.payload.reason.clone();
                }
            }
            AgentTraceKind::RunStarted
            | AgentTraceKind::PromptComposed
            | AgentTraceKind::ContextCompacted
            | AgentTraceKind::MemoryUsed
            | AgentTraceKind::PlanUpdated
            | AgentTraceKind::SubagentUpdated => {
                if !run_was_terminal && event.payload.phase.is_some() {
                    self.projection.phase = event.payload.phase.clone();
                }
            }
        }
    }

    fn apply_activity(&mut self, event: &AgentTraceEvent) {
        let span_id = event
            .span_id
            .clone()
            .or_else(|| event.payload.call_id.clone())
            .unwrap_or_else(|| event.event_id.clone());
        let index = self
            .projection
            .activities
            .iter()
            .position(|activity| activity.span_id == span_id);
        if let Some(index) = index {
            let activity = &mut self.projection.activities[index];
            activity.status = event.status;
            if event.payload.summary.is_some() {
                activity.summary = event.payload.summary.clone();
            }
            if event.payload.detail.is_some() {
                activity.detail = event.payload.detail.clone();
            }
            if event.payload.arguments.is_some() {
                activity.arguments = event.payload.arguments.clone();
            }
            if event.payload.result.is_some() {
                activity.result = event.payload.result.clone();
            }
            if !event.payload.resources.is_empty() {
                activity.resources = event.payload.resources.clone();
            }
            if matches!(
                event.status,
                AgentTraceStatus::Succeeded
                    | AgentTraceStatus::Failed
                    | AgentTraceStatus::Cancelled
            ) {
                activity.finished_sequence = Some(event.sequence);
            }
            return;
        }
        self.projection.activities.push(AgentTraceActivity {
            span_id,
            tool_name: event.payload.tool_name.clone(),
            status: event.status,
            summary: event.payload.summary.clone(),
            detail: event.payload.detail.clone(),
            arguments: event.payload.arguments.clone(),
            result: event.payload.result.clone(),
            resources: event.payload.resources.clone(),
            started_sequence: event.sequence,
            finished_sequence: event.status.is_terminal().then_some(event.sequence),
        });
    }
}

impl AgentTraceStatus {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

pub fn reduce_agent_trace(events: &[AgentTraceEvent]) -> Result<AgentRunProjection, AgentTraceApplyResult> {
    let mut ordered = events.to_vec();
    ordered.sort_by_key(|event| event.sequence);
    let mut reducer = AgentTraceReducer::default();
    for event in &ordered {
        match reducer.apply(event) {
            AgentTraceApplyResult::Applied | AgentTraceApplyResult::Duplicate => {}
            error => return Err(error),
        }
    }
    Ok(reducer.projection)
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn string_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn bool_at(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn usage_from_external_event(value: &Value) -> Option<AgentTraceUsage> {
    let used_tokens = usize_at(value, "used_tokens");
    let context_window_tokens =
        usize_at(value, "context_window_tokens").or_else(|| usize_at(value, "limit_tokens"));
    let prompt_token_limit = usize_at(value, "prompt_token_limit");
    if used_tokens.is_none() && context_window_tokens.is_none() && prompt_token_limit.is_none() {
        return None;
    }
    Some(AgentTraceUsage {
        used_tokens: used_tokens.unwrap_or_default(),
        context_window_tokens: context_window_tokens.unwrap_or_default(),
        prompt_token_limit: prompt_token_limit.unwrap_or_default(),
        usage_percent: value
            .get("usage_percent")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        usage_is_estimated: value
            .get("usage_is_estimated")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        ..AgentTraceUsage::default()
    })
}

fn usize_at(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map(|number| number as usize)
}

fn sanitize_trace_value(value: &Value, depth: usize) -> Value {
    if depth >= MAX_TRACE_VALUE_DEPTH {
        return Value::String("[truncated: maximum depth]".to_string());
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .take(MAX_TRACE_COLLECTION_ITEMS)
                .map(|(key, value)| {
                    let sanitized = if is_sensitive_trace_key(key) {
                        Value::String("[redacted]".to_string())
                    } else {
                        sanitize_trace_value(value, depth + 1)
                    };
                    (key.clone(), sanitized)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(MAX_TRACE_COLLECTION_ITEMS)
                .map(|value| sanitize_trace_value(value, depth + 1))
                .collect(),
        ),
        Value::String(value) => {
            if value.chars().count() <= MAX_TRACE_STRING_CHARS {
                Value::String(value.clone())
            } else {
                let prefix = value
                    .chars()
                    .take(MAX_TRACE_STRING_CHARS)
                    .collect::<String>();
                Value::String(format!(
                    "{prefix}\n[truncated: {} additional characters]",
                    value.chars().count().saturating_sub(MAX_TRACE_STRING_CHARS)
                ))
            }
        }
        _ => value.clone(),
    }
}

fn is_sensitive_trace_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "apikey"
            | "authorization"
            | "cookie"
            | "password"
            | "passwd"
            | "secret"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "clientsecret"
    )
}

fn validation_from_external_event(value: &Value) -> Option<AgentTraceValidation> {
    let meta = value.get("meta").unwrap_or(value);
    let status = first_string(meta, &["validation_status"])?;
    Some(AgentTraceValidation {
        status,
        level: first_string(meta, &["validation_level"]),
        summary: first_string(meta, &["validation_summary", "summary"]),
        next_action: first_string(meta, &["next_action"]),
    })
}

fn extract_resources(value: &Value, tool_name: Option<&str>) -> Vec<AgentTraceResource> {
    let operation = resource_operation(tool_name.unwrap_or_default());
    let mut resources = Vec::new();
    collect_resource_values(value, operation, &mut resources);
    resources.sort_by(|left, right| left.uri.cmp(&right.uri));
    resources.dedup_by(|left, right| left.uri == right.uri && left.operation == right.operation);
    resources.truncate(32);
    resources
}

fn artifact_resources(
    artifacts: &[crate::agent::parts::ArtifactReference],
    tool_name: &str,
) -> Vec<AgentTraceResource> {
    artifacts
        .iter()
        .filter(|artifact| !artifact.uri.trim().is_empty())
        .map(|artifact| AgentTraceResource {
            uri: artifact.uri.clone(),
            operation: artifact
                .operation
                .as_deref()
                .and_then(artifact_resource_operation)
                .unwrap_or_else(|| resource_operation(tool_name)),
            display_name: (!artifact.name.trim().is_empty()).then(|| artifact.name.clone()),
            mime_type: artifact.mime_type.clone(),
        })
        .collect()
}

fn artifact_resource_operation(value: &str) -> Option<AgentTraceResourceOperation> {
    match value.trim().to_ascii_lowercase().as_str() {
        "read" => Some(AgentTraceResourceOperation::Read),
        "create" => Some(AgentTraceResourceOperation::Create),
        "update" | "write" => Some(AgentTraceResourceOperation::Update),
        "delete" => Some(AgentTraceResourceOperation::Delete),
        "execute" => Some(AgentTraceResourceOperation::Execute),
        "open" => Some(AgentTraceResourceOperation::Open),
        "search" => Some(AgentTraceResourceOperation::Search),
        _ => None,
    }
}

fn collect_resource_values(
    value: &Value,
    operation: AgentTraceResourceOperation,
    output: &mut Vec<AgentTraceResource>,
) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let normalized = key.to_ascii_lowercase();
                if matches!(
                    normalized.as_str(),
                    "path" | "file" | "file_path" | "target_path" | "output_path" | "url" | "uri"
                ) {
                    if let Some(uri) = value.as_str().map(str::trim).filter(|uri| !uri.is_empty()) {
                        output.push(AgentTraceResource {
                            uri: uri.to_string(),
                            operation,
                            display_name: None,
                            mime_type: None,
                        });
                    }
                } else {
                    collect_resource_values(value, operation, output);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_resource_values(value, operation, output);
            }
        }
        _ => {}
    }
}

fn resource_operation(tool_name: &str) -> AgentTraceResourceOperation {
    let _tool_name = tool_name;
    AgentTraceResourceOperation::Unknown
}

const MAX_TRACE_PROMPT_FRAGMENT_ID_CHARS: usize = 256;

fn sanitize_prompt_identity(
    stable_prefix_digest: &str,
    prompt_fragment_ids: &[String],
) -> Option<AgentTracePromptIdentity> {
    let digest = stable_prefix_digest.trim();
    let stable_prefix_digest = (digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then(|| digest.to_ascii_lowercase());
    let fragment_count = prompt_fragment_ids.len();
    let fragment_ids = prompt_fragment_ids
        .iter()
        .take(MAX_TRACE_COLLECTION_ITEMS)
        .filter_map(|fragment_id| {
            let fragment_id = fragment_id.trim();
            (!fragment_id.is_empty()).then(|| {
                fragment_id
                    .chars()
                    .take(MAX_TRACE_PROMPT_FRAGMENT_ID_CHARS)
                    .collect::<String>()
            })
        })
        .collect::<Vec<_>>();
    if stable_prefix_digest.is_none() && fragment_ids.is_empty() {
        return None;
    }
    Some(AgentTracePromptIdentity {
        stable_prefix_digest,
        fragment_ids,
        fragment_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_host_trace_identity_survives_begin_turn_and_checkpoint_restore() {
        let turn_id = "76EDEDE8-713F-4010-A2D3-187A9D992AB6";
        let message_id = "canonical-assistant";
        let mut context =
            AgentTraceContext::for_turn("canonical-conversation", Some(message_id.into()), turn_id);
        let startup = context
            .project_external_event(&json!({"type":"status","phase":"prepare"}))
            .unwrap();
        assert_eq!(startup.run_id, turn_id);
        assert_eq!(startup.turn_id, turn_id);
        assert_eq!(
            startup.trace_id,
            "trace_76EDEDE8_713F_4010_A2D3_187A9D992AB6"
        );
        assert_eq!(startup.message_id.as_deref(), Some(message_id));
        assert_eq!(startup.sequence, 1);

        let mut restored: AgentTraceContext =
            serde_json::from_value(serde_json::to_value(&context).unwrap()).unwrap();
        restored.begin_turn(turn_id);
        let cancelled = restored
            .project_external_event(&json!({"type":"cancelled","reason":"cancelled"}))
            .unwrap();
        assert_eq!(cancelled.run_id, startup.run_id);
        assert_eq!(cancelled.turn_id, startup.turn_id);
        assert_eq!(cancelled.trace_id, startup.trace_id);
        assert_eq!(cancelled.message_id, startup.message_id);
        assert_eq!(cancelled.sequence, 2);

        restored.set_next_message_id(Some("next-assistant".into()));
        restored.begin_turn("next-turn");
        let next = restored
            .project_external_event(&json!({"type":"status","phase":"prepare"}))
            .unwrap();
        assert_eq!(next.run_id, "next-turn");
        assert_eq!(next.trace_id, "trace_next_turn");
        assert_eq!(next.message_id.as_deref(), Some("next-assistant"));
        assert_eq!(next.sequence, 1);
    }

    #[test]
    fn normalizes_and_reduces_a_complete_long_task_trace() {
        let mut context = AgentTraceContext::new("conversation-1", Some("message-1".to_string()));
        let event = [
            json!({"type":"status","phase":"prepare","message":"Preparing"}),
            json!({"type":"thinking_delta","delta":"Inspect the project."}),
            json!({"type":"tool_started","call_id":"call-1","tool_name":"read_file","arguments":{"path":"README.md"}}),
            json!({"type":"tool_finished","call_id":"call-1","tool_name":"read_file","summary":"Read 20 lines","error":false}),
            json!({"type":"assistant_delta","delta":"## Result\n\n"}),
            json!({"type":"assistant_delta","delta":"The project is ready."}),
            json!({"type":"done","reason":"completed"}),
        ];
        let events = event
            .iter()
            .filter_map(|event| context.project_external_event(event))
            .collect::<Vec<_>>();
        let projection = reduce_agent_trace(&events).unwrap();

        assert_eq!(projection.last_sequence, 4);
        assert_eq!(projection.activities.len(), 1);
        assert_eq!(projection.activities[0].status, AgentTraceStatus::Succeeded);
        assert_eq!(projection.activities[0].resources[0].uri, "README.md");
        assert_eq!(projection.status, AgentTraceStatus::Succeeded);
    }

    #[test]
    fn external_executor_session_identity_survives_trace_normalization() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let event = context
            .project_external_event(&json!({
                "protocol_version": 1,
                "type": "external_session_started",
                "execution_backend": "codex",
                "external_session_id": "thread-42",
                "source": "external_executor"
            }))
            .unwrap();

        assert_eq!(event.kind, AgentTraceKind::PhaseChanged);
        assert_eq!(
            event.payload.phase.as_deref(),
            Some("external_session_started")
        );
        assert_eq!(event.payload.execution_backend.as_deref(), Some("codex"));
        assert_eq!(
            event.payload.external_session_id.as_deref(),
            Some("thread-42")
        );
    }

    #[test]
    fn external_content_has_no_trace_or_sequence_and_old_body_schema_is_rejected() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        for kind in ["assistant_delta", "assistant_reset", "thinking_delta"] {
            assert!(context
                .project_external_event(&json!({"type":kind,"delta":"private body"}))
                .is_none());
        }
        let control = context
            .project_external_event(&json!({"type":"status","phase":"prepare"}))
            .unwrap();
        assert_eq!(control.sequence, 1);
        for kind in ["assistant.delta", "assistant.reset", "thinking.delta"] {
            assert!(serde_json::from_value::<AgentTraceKind>(json!(kind)).is_err());
        }
        assert!(serde_json::from_value::<AgentTracePayload>(
            json!({"delta":"retired body channel"})
        )
        .is_err());
    }

    #[test]
    fn completed_artifacts_are_projected_as_trace_resources() {
        let resources = artifact_resources(
            &[crate::agent::parts::ArtifactReference {
                uri: "file:///tmp/report.docx".to_string(),
                name: "report.docx".to_string(),
                mime_type: Some(
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                        .to_string(),
                ),
                operation: Some("create".to_string()),
            }],
            "device_write_file",
        );

        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].uri, "file:///tmp/report.docx");
        assert_eq!(resources[0].operation, AgentTraceResourceOperation::Create);
        assert_eq!(resources[0].display_name.as_deref(), Some("report.docx"));
    }

    #[test]
    fn untyped_resource_discovery_does_not_infer_operation_from_tool_name() {
        let resources = extract_resources(
            &json!({"path": "/tmp/report.docx"}),
            Some("device_write_file"),
        );

        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].operation, AgentTraceResourceOperation::Unknown);
    }

    #[test]
    fn reducer_rejects_duplicate_gap_and_out_of_order_events() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let first = context
            .project_external_event(&json!({"type":"status","phase":"A"}))
            .unwrap();
        let second = context
            .project_external_event(&json!({"type":"status","phase":"B"}))
            .unwrap();
        let third = context
            .project_external_event(&json!({"type":"status","phase":"C"}))
            .unwrap();
        let mut reducer = AgentTraceReducer::default();

        assert_eq!(reducer.apply(&first), AgentTraceApplyResult::Applied);
        assert_eq!(reducer.apply(&first), AgentTraceApplyResult::Duplicate);
        assert_eq!(
            reducer.apply(&third),
            AgentTraceApplyResult::Gap {
                expected: 2,
                received: 3
            }
        );
        assert_eq!(reducer.apply(&second), AgentTraceApplyResult::Applied);
        assert_eq!(reducer.apply(&third), AgentTraceApplyResult::Applied);

        let mut unseen_old = first.clone();
        unseen_old.event_id = "different-event".to_string();
        assert_eq!(
            reducer.apply(&unseen_old),
            AgentTraceApplyResult::OutOfOrder {
                last: 3,
                received: 1
            }
        );
        assert_eq!(reducer.projection().phase.as_deref(), Some("C"));
    }

    #[test]
    fn streamed_tool_argument_fragments_are_not_durable_trace_events() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let event = AgentEvent::PartUpdated {
            turn_id: crate::agent::protocol::TurnId::from_string("turn-1".to_string()),
            projection_revision: 1,
            update: PartUpdate::ToolInputDelta {
                part_id: "tool-part-1".to_string(),
                delta: "{\"path\":".to_string(),
            },
            part: AgentPart::Reasoning(crate::agent::parts::ReasoningPart {
                id: "reasoning-part-1".to_string(),
                ownership: crate::agent::parts::PartOwnership::for_turn("turn-1"),
                title: "Thinking".to_string(),
                detail: String::new(),
                is_complete: false,
            }),
        };

        assert!(context
            .project_agent_events_at(&event, 1, "2026-07-21T00:00:00.000Z")
            .is_empty());
    }

    #[test]
    fn host_progress_metadata_survives_as_typed_trace_payload() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let event = AgentEvent::HostProgress {
            turn_id: Some(crate::agent::protocol::TurnId::from_string(
                "turn-1".to_string(),
            )),
            effect_id: "effect-1".to_string(),
            title: "model_retry".to_string(),
            detail: Some("primary/model → fallback/model · quota exhausted".to_string()),
            metadata: json!({
                "attempt": 2,
                "failure_code": "quota_exhausted",
                "failure_message": "The provider quota is exhausted."
            }),
        };

        let projected = context.project_agent_events_at(&event, 1, "2026-07-21T00:00:00.000Z");

        assert_eq!(projected.len(), 1);
        assert_eq!(
            projected[0]
                .payload
                .metadata
                .as_ref()
                .and_then(|value| value["attempt"].as_u64()),
            Some(2)
        );
        assert_eq!(
            projected[0]
                .payload
                .metadata
                .as_ref()
                .and_then(|value| value["failure_code"].as_str()),
            Some("quota_exhausted")
        );
    }

    #[test]
    fn media_preparation_projects_the_exporter_wire_contract() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let event = AgentEvent::HostProgress {
            turn_id: None,
            effect_id: "effect-1".to_string(),
            title: "tool_media.prepared".to_string(),
            detail: None,
            metadata: json!({"request_id":"request-1", "effect_id":"effect-1",
                "artifacts":[{"tool_call_id":"call-1", "status":"prepared"}],
                "model_receipt_proven":false}),
        };
        let projected = context.project_agent_events_at(&event, 1, "2026-07-21T00:00:00.000Z");
        assert_eq!(projected.len(), 1);
        let wire = serde_json::to_value(&projected[0]).unwrap();
        assert_eq!(wire["kind"], "tool.progress");
        assert_eq!(wire["payload"]["summary"], "tool_media.prepared");
        assert!(wire["payload"].get("title").is_none());
        assert_eq!(wire["payload"]["metadata"]["request_id"], "request-1");
        assert_eq!(wire["payload"]["metadata"]["model_receipt_proven"], false);
    }

    fn tool_state_event(call_id: &str, state: ToolPartState) -> AgentEvent {
        use crate::agent::host::HostCapabilityOrigin;
        use crate::agent::parts::{PartOwnership, ToolPart};
        use crate::agent::protocol::TurnId;

        let part_id = format!("tool-{call_id}");
        AgentEvent::PartUpdated {
            turn_id: TurnId::from_string("turn-1".to_string()),
            projection_revision: 1,
            update: PartUpdate::ToolState {
                part_id: part_id.clone(),
                state,
                title: Some("Read file".to_string()),
                summary: Some("Read file".to_string()),
                output_preview: state.is_terminal().then(|| "result".to_string()),
                structured_output: None,
                full_output_ref: None,
                artifacts: Vec::new(),
                timestamp_ms: Some(1),
            },
            part: AgentPart::Tool(ToolPart {
                execution_source: Default::default(),
                id: part_id,
                ownership: PartOwnership::for_turn("turn-1"),
                call_id: call_id.to_string(),
                capability_id: "read_file".to_string(),
                title: "Read file".to_string(),
                summary: Some("Read file".to_string()),
                origin: HostCapabilityOrigin::default(),
                state,
                input: json!({"path":"README.md"}),
                raw_input: String::new(),
                permission: None,
                output_preview: state.is_terminal().then(|| "result".to_string()),
                structured_output: None,
                full_output_ref: None,
                artifacts: Vec::new(),
                started_at_ms: (state != ToolPartState::Pending).then_some(1),
                finished_at_ms: state.is_terminal().then_some(2),
            }),
        }
    }

    #[test]
    fn direct_tool_terminal_state_recovers_a_paired_start() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let traces = context.project_agent_events_at(
            &tool_state_event("call-guarded", ToolPartState::Failed),
            1,
            "2026-09-11T00:00:00.000Z",
        );

        assert_eq!(traces.len(), 2);
        assert_eq!(traces[0].kind, AgentTraceKind::ToolStarted);
        assert_eq!(traces[0].status, AgentTraceStatus::Running);
        assert!(traces[0].payload.recovered);
        assert_eq!(
            traces[0].payload.code.as_deref(),
            Some("recovered_tool_start")
        );
        assert_eq!(traces[1].kind, AgentTraceKind::ToolCompleted);
        assert_eq!(traces[1].status, AgentTraceStatus::Failed);
        assert_eq!(traces[0].payload.call_id, traces[1].payload.call_id);
        assert_eq!(traces[1].sequence, traces[0].sequence + 1);

        // Replaying the same canonical terminal update does not publish a
        // second completion for the already closed logical call.
        assert!(context
            .project_agent_events_at(
                &tool_state_event("call-guarded", ToolPartState::Failed),
                2,
                "2026-09-11T00:00:01.000Z",
            )
            .is_empty());
    }

    #[test]
    fn checkpointed_running_tool_is_closed_before_failed_run() {
        use crate::agent::protocol::{SessionId, TurnId};

        let mut context = AgentTraceContext::new("conversation-1", None);
        let started = context.project_agent_events_at(
            &tool_state_event("call-interrupted", ToolPartState::Running),
            1,
            "2026-09-11T00:00:00.000Z",
        );
        assert_eq!(started.len(), 1);

        // Exercise the actual cold-recovery boundary: lifecycle ownership is
        // serialized with the trace cursor, not rebuilt from UI state.
        let checkpoint = serde_json::to_string(&context).unwrap();
        let mut recovered: AgentTraceContext = serde_json::from_str(&checkpoint).unwrap();
        let terminal = recovered.project_agent_events_at(
            &AgentEvent::TurnFailed {
                session_id: SessionId::from_string("conversation-1".to_string()),
                turn_id: TurnId::from_string("turn-1".to_string()),
                error: "host disconnected".to_string(),
            },
            2,
            "2026-09-11T00:00:01.000Z",
        );

        assert_eq!(terminal.len(), 2);
        assert_eq!(terminal[0].kind, AgentTraceKind::ToolCompleted);
        assert_eq!(terminal[0].status, AgentTraceStatus::Failed);
        assert!(terminal[0].payload.recovered);
        assert_eq!(
            terminal[0].payload.code.as_deref(),
            Some("tool_interrupted_by_run_terminal")
        );
        assert_eq!(terminal[1].kind, AgentTraceKind::RunFailed);
        assert_eq!(terminal[1].sequence, terminal[0].sequence + 1);
    }

    #[test]
    fn canonical_body_is_never_buffered_or_republished_by_control_traces() {
        use crate::agent::parts::{PartOwnership, ReasoningPart, TextPart};
        use crate::agent::protocol::{SessionId, TurnId};
        let mut context = AgentTraceContext::new("conversation-1", None);
        let turn = TurnId::from_string("turn-1".into());
        let body = "## 完整正文🙂\n\n**Markdown**\n".repeat(65536);
        for (index, update) in [
            PartUpdate::TextDelta {
                part_id: "text".into(),
                delta: body.clone(),
            },
            PartUpdate::TextReplaced {
                part_id: "text".into(),
                text: body.clone(),
                is_complete: true,
            },
            PartUpdate::TextCommittedDelta {
                part_id: "text".into(),
                delta: body.clone(),
            },
            PartUpdate::Completed {
                part_id: "text".into(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            let part = AgentPart::Text(TextPart {
                id: "text".into(),
                ownership: PartOwnership::for_turn("turn-1"),
                text: body.clone(),
                is_complete: true,
                review: None,
            });
            let event = AgentEvent::PartUpdated {
                turn_id: turn.clone(),
                projection_revision: 1,
                update,
                part,
            };
            assert!(context
                .project_agent_events_at(&event, index as u64 + 1, "2026-08-22T00:00:00Z")
                .is_empty());
            let encoded = serde_json::to_string(&context).unwrap();
            assert!(encoded.len() < 2048);
            assert!(
                !encoded.contains("pending_assistant") && !encoded.contains("pending_thinking")
            );
            // The source Part remains intact for canonical publication.
            let AgentEvent::PartUpdated {
                part: AgentPart::Text(part),
                ..
            } = event
            else {
                unreachable!()
            };
            assert_eq!(part.text, body);
        }
        let reasoning = AgentEvent::PartUpdated {
            turn_id: turn.clone(),
            projection_revision: 2,
            update: PartUpdate::ReasoningDelta {
                part_id: "reasoning".into(),
                title: None,
                detail_delta: body.clone(),
            },
            part: AgentPart::Reasoning(ReasoningPart {
                id: "reasoning".into(),
                ownership: PartOwnership::for_turn("turn-1"),
                title: "Thinking".into(),
                detail: body.clone(),
                is_complete: true,
            }),
        };
        assert!(context
            .project_agent_events_at(&reasoning, 5, "2026-08-22T00:00:00Z")
            .is_empty());
        let mut restored: AgentTraceContext =
            serde_json::from_str(&serde_json::to_string(&context).unwrap()).unwrap();
        let terminal = restored.project_agent_events_at(
            &AgentEvent::TurnCompleted {
                session_id: SessionId::from_string("conversation-1".into()),
                turn_id: turn,
                outcome: Some(nola_domain::TaskCompletionOutcome::ActionCompleted),
            },
            6,
            "2026-08-22T00:00:01Z",
        );
        assert_eq!(terminal.len(), 1);
        assert_eq!(terminal[0].sequence, 1);
        assert_eq!(terminal[0].kind, AgentTraceKind::RunCompleted);
        assert!(serde_json::to_string(&terminal).unwrap().len() < 2048);
    }

    #[test]
    fn admitted_shared_root_survives_trace_checkpoint_and_next_run() {
        use crate::agent::protocol::{SessionId, TurnId};
        let mut context = AgentTraceContext::new("conversation", None);
        context.set_next_message_id(Some("canonical-root".into()));
        let first = context.project_agent_events_at(&AgentEvent::TurnStarted {
            session_id:SessionId::from_string("conversation".into()),
            turn_id:TurnId::from_string("first".into()),assistant_message_id:"first:assistant".into(),
        },1,"2026-10-27T00:00:00Z");
        assert_eq!(first[0].message_id.as_deref(),Some("canonical-root"));
        context.project_agent_events_at(&AgentEvent::TurnCompleted {
            outcome:None,session_id:SessionId::from_string("conversation".into()),
            turn_id:TurnId::from_string("first".into()),
        },2,"2026-10-27T00:00:01Z");
        let mut cold: AgentTraceContext = serde_json::from_value(serde_json::to_value(context).unwrap()).unwrap();
        cold.set_next_message_id(Some("canonical-root".into()));
        let second = cold.project_agent_events_at(&AgentEvent::TurnStarted {
            session_id:SessionId::from_string("conversation".into()),
            turn_id:TurnId::from_string("second".into()),assistant_message_id:"second:assistant".into(),
        },3,"2026-10-27T00:00:02Z");
        assert_eq!(second[0].message_id.as_deref(),Some("canonical-root"));
        assert_eq!(second[0].run_id,"second");
        assert_eq!(second[0].sequence,1);
        assert_ne!(first[0].trace_id,second[0].trace_id);
    }

    #[test]
    fn consecutive_turns_start_independent_trace_sequences() {
        use crate::agent::protocol::{SessionId, TurnId};

        let mut context = AgentTraceContext::new("conversation-1", None);
        let first_turn = TurnId::from_string("conversation-1:turn:1".to_string());
        let first_started = context.project_agent_events_at(
            &AgentEvent::TurnStarted {
                session_id: SessionId::from_string("conversation-1".to_string()),
                turn_id: first_turn.clone(),
                assistant_message_id: "conversation-1:turn:1:assistant".into(),
            },
            1,
            "2026-07-28T00:00:00.000Z",
        );
        let first_completed = context.project_agent_events_at(
            &AgentEvent::TurnCompleted {
                outcome: None,
                session_id: SessionId::from_string("conversation-1".to_string()),
                turn_id: first_turn,
            },
            2,
            "2026-07-28T00:00:01.000Z",
        );
        let first_trace_id = first_started[0].trace_id.clone();

        assert_eq!(first_started[0].sequence, 1);
        assert_eq!(first_completed[0].sequence, 2);

        let second_started = context.project_agent_events_at(
            &AgentEvent::TurnStarted {
                session_id: SessionId::from_string("conversation-1".to_string()),
                turn_id: TurnId::from_string("conversation-1:turn:2".to_string()),
                assistant_message_id: "conversation-1:turn:2:assistant".into(),
            },
            3,
            "2026-07-28T00:00:02.000Z",
        );

        assert_eq!(second_started.len(), 1);
        assert_eq!(second_started[0].kind, AgentTraceKind::RunStarted);
        assert_eq!(second_started[0].sequence, 1);
        assert_ne!(second_started[0].trace_id, first_trace_id);
        assert_eq!(second_started[0].turn_id, "conversation-1:turn:2");
    }

    #[test]
    fn compaction_trace_links_overflow_recovery_to_canonical_source_identity() {
        use crate::agent::context_window::CompactionReason;
        use crate::agent::protocol::{ItemId, SessionId, TurnId};

        let mut context = AgentTraceContext::new("conversation-1", None);
        let turn_id = TurnId::from_string("conversation-1:turn:1".to_string());
        let _ = context.project_agent_events_at(
            &AgentEvent::TurnStarted {
                session_id: SessionId::from_string("conversation-1".to_string()),
                turn_id,
                assistant_message_id: "conversation-1:turn:1:assistant".into(),
            },
            1,
            "2026-09-11T00:00:00.000Z",
        );
        let requested = context.project_agent_events_at(
            &AgentEvent::CompactionRequested {
                reason: CompactionReason::ContextLimitRecovery,
                item_count: 5,
                estimated_tokens: 12_000,
                source_fingerprint: "source-digest".to_string(),
                source_item_count: 3,
                original_token_count: 9_500,
            },
            2,
            "2026-09-11T00:00:01.000Z",
        );
        let completed = context.project_agent_events_at(
            &AgentEvent::CompactionCompleted {
                summary: "bounded summary".to_string(),
                replaced_item_count: 3,
                remaining_item_count: 2,
                reason: Some(CompactionReason::ContextLimitRecovery),
                compaction_item_id: Some(ItemId::from_string("compaction-1".to_string())),
                source_fingerprint: Some("source-digest".to_string()),
                source_item_count: 3,
                original_token_count: 9_500,
            },
            3,
            "2026-09-11T00:00:02.000Z",
        );

        assert_eq!(
            requested[0].payload.code.as_deref(),
            Some("context_limit_recovery")
        );
        assert_eq!(
            requested[0].payload.source_fingerprint.as_deref(),
            Some("source-digest")
        );
        assert_eq!(completed[0].kind, AgentTraceKind::ContextCompacted);
        assert_eq!(
            completed[0].payload.compaction_item_id.as_deref(),
            Some("compaction-1")
        );
        assert_eq!(completed[0].payload.source_item_count, 3);
        assert_eq!(completed[0].payload.original_token_count, 9_500);
    }

    #[test]
    fn missing_scope_or_version_does_not_request_repeated_validation() {
        for (scope_count, version, expected) in [
            (1, Some("v1"), true),
            (0, None, true),
            (0, Some("v1"), false),
        ] {
            for phase in ["verifying", "correcting"] {
                let mut state = serde_json::to_value(
                    crate::agent::AgentEngineeringLoopState::default(),
                ).unwrap();
                state["validation_required"] = json!(true);
                state["phase"] = json!(phase);
                state["unscoped_mutation_count"] = json!(scope_count);
                state["pending_validation_versions"] = json!({"file:///fixture":version});
                let event = AgentEvent::EngineeringLoopUpdated {
                    turn_id: crate::agent::protocol::TurnId::from_string("turn-1".into()),
                    state: serde_json::from_value(state).unwrap(),
                };
                let mut context = AgentTraceContext::new("conversation-1", None);
                let projected = context.project_agent_events_at(&event, 1, "2026-10-12T00:00:00Z");
                let trace = projected.iter().find(|trace| trace.kind == AgentTraceKind::ValidationUpdated).unwrap();
                let validation = trace.payload.validation.as_ref().unwrap();
                if expected {
                    assert_eq!(validation.status, "evidence_insufficient");
                    assert_eq!(trace.status, AgentTraceStatus::Incomplete);
                    assert!(validation.next_action.is_none());
                } else {
                    assert_eq!(validation.status, "required");
                    assert_eq!(trace.status, AgentTraceStatus::Running);
                    assert_eq!(validation.next_action.as_deref(), Some(if phase == "correcting" {
                        "correct_then_validate"
                    } else { "run_targeted_validation" }));
                }
            }
        }
    }

    #[test]
    fn engineering_debt_absence_does_not_claim_validation_succeeded() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let event = AgentEvent::EngineeringLoopUpdated {
            turn_id: crate::agent::protocol::TurnId::from_string("turn-1".into()),
            state: crate::agent::AgentEngineeringLoopState::default(),
        };
        let projected = context.project_agent_events_at(&event, 1, "2026-10-12T00:00:00Z");
        let trace = projected.iter().find(|trace| trace.kind == AgentTraceKind::ValidationUpdated)
            .expect("engineering diagnostic projection");
        assert_eq!(trace.payload.validation.as_ref().unwrap().status, "not_required");
        assert_ne!(trace.status, AgentTraceStatus::Succeeded);
    }

    #[test]
    fn trace_rejects_schema_payload_conflicts_and_second_terminal_without_mutation() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let first = context.project_external_event(&json!({"type":"run_started"})).unwrap();
        let end = context.project_external_event(&json!({"type":"done"})).unwrap();
        let mut reducer = AgentTraceReducer::default();
        let mut invalid = first.clone();
        invalid.schema_version += 1;
        assert_eq!(reducer.apply(&invalid), AgentTraceApplyResult::UnsupportedSchema(2));
        assert_eq!(reducer.projection().last_sequence, 0);
        assert_eq!(reducer.apply(&first), AgentTraceApplyResult::Applied);
        let before = reducer.projection().clone();
        let mut conflict = first.clone();
        conflict.payload.summary = Some("different payload".into());
        assert_eq!(reducer.apply(&conflict), AgentTraceApplyResult::EventConflict);
        assert_eq!(reducer.projection(), &before);
        assert_eq!(reducer.apply(&first), AgentTraceApplyResult::Duplicate);
        assert_eq!(reducer.apply(&end), AgentTraceApplyResult::Applied);
        let sealed = reducer.projection().clone();
        let mut second = end.clone();
        second.sequence += 1;
        second.event_id.push_str("-conflict");
        second.kind = AgentTraceKind::RunFailed;
        second.status = AgentTraceStatus::Failed;
        assert_eq!(reducer.apply(&second), AgentTraceApplyResult::TerminalAlreadyObserved);
        assert_eq!(reducer.projection(), &sealed);
        assert_eq!(reducer.apply(&end), AgentTraceApplyResult::Duplicate);
        // A rejected input must not reserve either its sequence or event ID.
        second.kind = AgentTraceKind::UsageSnapshot;
        assert_eq!(reducer.apply(&second), AgentTraceApplyResult::Applied);
        assert_eq!(reducer.projection().status, AgentTraceStatus::Succeeded);
        assert_eq!(reduce_agent_trace(&[first, conflict]), Err(AgentTraceApplyResult::EventConflict));
    }

    #[test]
    fn completed_turn_preserves_task_outcome_and_terminal_boundary() {
        use nola_domain::TaskCompletionOutcome as Outcome;
        for (outcome, status, reason) in [
            (Some(Outcome::Answered), AgentTraceStatus::Succeeded, "answered"),
            (Some(Outcome::ActionCompleted), AgentTraceStatus::Succeeded, "action_completed"),
            (Some(Outcome::NeedsInput), AgentTraceStatus::Waiting, "needs_input"),
            (Some(Outcome::Blocked), AgentTraceStatus::Incomplete, "blocked"),
            (Some(Outcome::Incomplete), AgentTraceStatus::Incomplete, "incomplete"),
            (None, AgentTraceStatus::Incomplete, "outcome_missing"),
        ] {
            let mut context = AgentTraceContext::new("conversation-1", None);
            let events = context.project_agent_events_at(&AgentEvent::TurnCompleted {
                session_id: crate::agent::protocol::SessionId::from_string("conversation-1".into()),
                turn_id: crate::agent::protocol::TurnId::from_string("turn-1".into()),
                outcome,
            }, 1, "2026-10-12T00:00:00Z");
            let end = events.last().unwrap();
            assert_eq!(end.kind, AgentTraceKind::RunCompleted);
            assert_eq!(end.status, status);
            assert_eq!(end.payload.reason.as_deref(), Some(reason));
            let mut reducer = AgentTraceReducer::default();
            assert_eq!(reducer.apply(end), AgentTraceApplyResult::Applied);
            let mut late = end.clone();
            late.event_id.push_str("-late");
            late.sequence += 1;
            late.kind = AgentTraceKind::PhaseChanged;
            late.status = AgentTraceStatus::Running;
            late.payload.phase = Some("late".into());
            assert_eq!(reducer.apply(&late), AgentTraceApplyResult::Applied);
            assert_eq!(reducer.projection().status, status);
            assert_eq!(reducer.projection().terminal_reason.as_deref(), Some(reason));
            assert!(reducer.projection().phase.is_none());
        }
    }

    #[test]
    fn trace_identity_rejection_is_atomic_and_precedes_duplicate_detection() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let first = context.project_external_event(&json!({"type":"run_started"})).unwrap();
        let second = context.project_external_event(&json!({"type":"done"})).unwrap();
        for field in ["trace", "run", "conversation"] {
            for reuse_event_id in [false, true] {
                let mut reducer = AgentTraceReducer::default();
                assert_eq!(reducer.apply(&first), AgentTraceApplyResult::Applied);
                let before = reducer.projection().clone();
                let mut foreign = second.clone();
                match field {
                    "trace" => foreign.trace_id = "foreign".into(),
                    "run" => foreign.run_id = "foreign".into(),
                    _ => foreign.conversation_id = "foreign".into(),
                }
                if reuse_event_id { foreign.event_id = first.event_id.clone(); }
                assert_eq!(reducer.apply(&foreign), AgentTraceApplyResult::IdentityMismatch);
                assert_eq!(reducer.projection(), &before);
                assert_eq!(reducer.apply(&second), AgentTraceApplyResult::Applied);
                assert_eq!(reduce_agent_trace(&[first.clone(), foreign]), Err(AgentTraceApplyResult::IdentityMismatch));
            }
        }
        // Batch reconstruction must report missing events rather than return
        // a plausible-looking partial success projection.
        assert_eq!(reduce_agent_trace(&[second]), Err(AgentTraceApplyResult::Gap { expected: 1, received: 2 }));
    }

    #[test]
    fn activity_results_and_late_telemetry_do_not_complete_or_revive_run() {
        for terminal in ["done", "error", "cancelled"] {
            let mut context = AgentTraceContext::new("conversation-1", None);
            let mut reducer = AgentTraceReducer::default();
            for value in [
                json!({"type":"tool_finished", "call_id":"a", "error":false}),
                json!({"type":"tool_finished", "call_id":"b", "error":true}),
                json!({"type":"hook_completed", "error":true}),
                json!({"type":"context_usage"}),
            ] {
                let event = context.project_external_event(&value).unwrap();
                assert_eq!(reducer.apply(&event), AgentTraceApplyResult::Applied);
                assert_eq!(reducer.projection().status, AgentTraceStatus::Running);
            }
            assert_eq!(reducer.projection().activities[0].status, AgentTraceStatus::Succeeded);
            assert_eq!(reducer.projection().activities[1].status, AgentTraceStatus::Failed);
            let end = context.project_external_event(&json!({"type":terminal, "reason":"original"})).unwrap();
            assert_eq!(reducer.apply(&end), AgentTraceApplyResult::Applied);
            let terminal_status = reducer.projection().status;
            for value in [
                json!({"type":"context_usage"}),
                json!({"type":"status", "phase":"prepare"}),
                json!({"type":"tool_finished", "call_id":"c", "error":false}),
                json!({"type":"validation_updated", "validation_status":"incomplete"}),
            ] {
                let event = context.project_external_event(&value).unwrap();
                assert_eq!(reducer.apply(&event), AgentTraceApplyResult::Applied);
                assert_eq!(reducer.projection().status, terminal_status);
                assert_eq!(reducer.projection().terminal_reason.as_deref(), Some("original"));
                assert!(reducer.projection().phase.is_none());
            }
        }
    }

    #[test]
    fn attached_validation_cannot_replace_lifecycle_or_permission_events() {
        for (source, kind, status) in [
            ("tool_finished", AgentTraceKind::ToolCompleted, AgentTraceStatus::Succeeded),
            ("permission_request", AgentTraceKind::ToolPermissionRequested, AgentTraceStatus::Waiting),
            ("done", AgentTraceKind::RunCompleted, AgentTraceStatus::Succeeded),
            ("error", AgentTraceKind::RunFailed, AgentTraceStatus::Failed),
            ("cancelled", AgentTraceKind::RunCancelled, AgentTraceStatus::Cancelled),
        ] {
            for diagnostic in ["failed", "incomplete", "blocked", "evidence_insufficient"] {
                let mut context = AgentTraceContext::new("conversation-1", None);
                let event = context.project_external_event(&json!({
                    "type":source, "call_id":"call", "tool_name":"bash",
                    "meta":{"validation_status":diagnostic}
                })).unwrap();
                assert_eq!(event.kind, kind);
                assert_eq!(event.status, status);
                let mut reducer = AgentTraceReducer::default();
                assert_eq!(reducer.apply(&event), AgentTraceApplyResult::Applied);
                let run_status = if matches!(kind, AgentTraceKind::ToolCompleted | AgentTraceKind::ToolPermissionRequested) {
                    AgentTraceStatus::Running
                } else { status };
                assert_eq!(reducer.projection().status, run_status);
                assert_eq!(reducer.projection().validation.as_ref().unwrap().status, diagnostic);
                let diagnostic_event = context.project_external_event(&json!({
                    "type":"validation_updated", "validation_status":diagnostic
                })).unwrap();
                assert_eq!(reducer.apply(&diagnostic_event), AgentTraceApplyResult::Applied);
                assert_eq!(reducer.projection().status, run_status);
            }
        }
    }

    #[test]
    fn external_validation_failure_remains_distinct_from_unfinished_work() {
        for (validation_status, expected) in [
            ("failed", AgentTraceStatus::Failed),
            ("error", AgentTraceStatus::Failed),
            ("incomplete", AgentTraceStatus::Incomplete),
            ("blocked", AgentTraceStatus::Incomplete),
            ("evidence_insufficient", AgentTraceStatus::Incomplete),
        ] {
            let mut context = AgentTraceContext::new("conversation-1", None);
            let trace = context.project_external_event(&json!({
                "type":"validation_updated", "call_id":"call-1", "tool_name":"bash",
                "error":false, "meta":{"validation_status":validation_status}
            })).unwrap();
            assert_eq!(trace.kind, AgentTraceKind::ValidationUpdated);
            assert_eq!(trace.status, expected);
            assert_eq!(trace.payload.validation.unwrap().status, validation_status);
        }
    }

    #[test]
    fn validation_metadata_becomes_an_incomplete_validation_event() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let event = context
            .project_external_event(&json!({
                "type":"validation_updated",
                "call_id":"call-1",
                "tool_name":"bash",
                "error":false,
                "meta":{
                    "validation_status":"incomplete",
                    "validation_level":"compile",
                    "next_action":"rerun tests"
                }
            }))
            .unwrap();

        assert_eq!(event.kind, AgentTraceKind::ValidationUpdated);
        assert_eq!(event.status, AgentTraceStatus::Incomplete);
        assert_eq!(
            event.payload.validation.unwrap().next_action.as_deref(),
            Some("rerun tests")
        );
    }

    #[test]
    fn tool_payload_is_redacted_and_bounded_before_it_enters_the_trace() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let event = context
            .project_external_event(&json!({
                "type":"tool_started",
                "call_id":"call-1",
                "tool_name":"web_fetch",
                "arguments":{
                    "url":"https://example.com",
                    "authorization":"Bearer private",
                    "nested":{"api_key":"private-key"},
                    "body":"x".repeat(MAX_TRACE_STRING_CHARS + 50)
                }
            }))
            .unwrap();
        let arguments = event.payload.arguments.expect("tool arguments");

        assert_eq!(arguments["authorization"], "[redacted]");
        assert_eq!(arguments["nested"]["api_key"], "[redacted]");
        assert!(!arguments["body"]
            .as_str()
            .unwrap_or_default()
            .contains(&"x".repeat(MAX_TRACE_STRING_CHARS + 1)));
        assert!(arguments["body"]
            .as_str()
            .unwrap_or_default()
            .contains("[truncated: 50 additional characters]"));
    }

    #[test]
    fn native_permission_challenge_is_a_waiting_permission_span_not_a_failed_tool() {
        let mut context = AgentTraceContext::new("conversation-1", None);
        let event = context
            .project_external_event(&json!({
                "type":"permission_request",
                "call_id":"call-write",
                "tool_name":"write_file",
                "permission_request_id":"call-write",
                "summary":"permission_required"
            }))
            .unwrap();

        assert_eq!(event.kind, AgentTraceKind::ToolPermissionRequested);
        assert_eq!(event.status, AgentTraceStatus::Waiting);
        assert_eq!(event.span_id.as_deref(), Some("call-write"));
        assert_eq!(
            event.payload.permission_request_id.as_deref(),
            Some("call-write")
        );
    }

    #[test]
    fn prompt_identity_is_redacted_bounded_and_digest_validated() {
        let mut fragment_ids = vec!["policy.base".to_string()];
        fragment_ids.extend((0..140).map(|index| format!("fragment.{index}")));
        fragment_ids.push("x".repeat(400));

        let identity = sanitize_prompt_identity(&"AB".repeat(32), &fragment_ids).unwrap();
        let encoded = serde_json::to_string(&AgentTracePayload {
            prompt_identity: Some(identity.clone()),
            ..AgentTracePayload::default()
        })
        .unwrap();

        assert_eq!(identity.stable_prefix_digest, Some("ab".repeat(32)));
        assert_eq!(identity.fragment_count, 142);
        assert_eq!(identity.fragment_ids.len(), MAX_TRACE_COLLECTION_ITEMS);
        assert!(identity.fragment_ids.iter().all(|fragment_id| {
            fragment_id.chars().count() <= MAX_TRACE_PROMPT_FRAGMENT_ID_CHARS
        }));
        assert!(!encoded.contains("system prompt body"));
        assert!(sanitize_prompt_identity("not-a-digest", &[]).is_none());
    }
}
