// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns persistent session state independent of any client UI.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::agent_trace::AgentTraceContext;

use super::capability::SkillRuntimeState;
use super::context_window::{ContextWindowConfig, ContextWindowState};
use super::goal::GoalProjection;
use super::host::{HostCapabilityCatalog, HostCapabilityRoute};
use super::input_queue::AgentInputQueue;
use super::memory_runtime::MemoryRuntimeProjection;
use super::parts::ConversationProjection;
use super::permissions::PermissionRuleSet;
use super::protocol::{
    AgentCoreConfig, AgentItem, AgentRole, AgentSessionConfigurationPatch, AgentTurnIdentity,
    ModelProviderContinuationState, SessionId, TurnId,
};
use super::recovery::HostEffectJournal;
use super::task::TaskProjection;
use super::turn::AgentTurnState;
use crate::memory::MemoryProjection;

/// Configuration captured when a conversation thread is created.
///
/// This snapshot is checkpointed with the session. Execution configuration may
/// be replaced only while the thread is idle and has no pending Host work.
/// Durable conversation history survives replacement; in-flight effects never do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionConfiguration {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_model_iterations: usize,
    /// Required frozen allowance; checkpoint restore must not invent a budget.
    pub max_strategy_recovery_attempts: u32,
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
    pub context_window: ContextWindowConfig,
    #[serde(default)]
    pub memory_policy: crate::memory::MemoryPolicy,
    #[serde(default)]
    pub turn_identity: AgentTurnIdentity,
}

impl AgentSessionConfiguration {
    pub fn from_runtime(config: &AgentCoreConfig) -> Self {
        Self {
            model: config.model.clone(),
            max_model_iterations: config.max_model_iterations,
            max_strategy_recovery_attempts: config.max_strategy_recovery_attempts,
            tools: config.tools.clone(),
            context_window: config.context_window.clone().normalized(),
            memory_policy: config.memory_policy.clone(),
            turn_identity: config.turn_identity.clone(),
        }
    }

    pub fn matches_runtime(&self, config: &AgentCoreConfig) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(Self::from_runtime(config)).ok()
    }

    pub fn matches_runtime_except_tools(&self, config: &AgentCoreConfig) -> bool {
        let mut comparable = self.clone();
        comparable.tools = config.tools.clone();
        comparable.matches_runtime(config)
    }

    pub fn apply_patch(&mut self, patch: AgentSessionConfigurationPatch) {
        if let Some(model) = patch.model {
            self.model = model;
        }
        if let Some(max_model_iterations) = patch.max_model_iterations {
            self.max_model_iterations = max_model_iterations;
        }
        if let Some(max_strategy_recovery_attempts) = patch.max_strategy_recovery_attempts {
            self.max_strategy_recovery_attempts = max_strategy_recovery_attempts;
        }
        if let Some(tools) = patch.tools {
            self.tools = tools;
        }
        if let Some(context_window) = patch.context_window {
            self.context_window = context_window.normalized();
        }
        if let Some(memory_policy) = patch.memory_policy {
            self.memory_policy = memory_policy;
        }
        if let Some(turn_identity) = patch.turn_identity {
            self.turn_identity = turn_identity;
        }
    }
}

/// Durable conversation state. It contains only data that participates in
/// model context or stable UI projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConversationState {
    pub transcript: Vec<AgentItem>,
    pub model_context: Vec<AgentItem>,
    /// Last provider continuation committed by a successful model transaction.
    /// It is checkpointed but intentionally absent from the UI projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_continuation: Option<ModelProviderContinuationState>,
    pub context_window: ContextWindowState,
    pub projection: ConversationProjection,
    /// Skills explicitly pinned by the user for the conversation. Normal
    /// invocations live on AgentTurnState and never leak into later turns.
    #[serde(default)]
    pub pinned_skills: BTreeMap<String, AgentInvokedSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentInvokedSkill {
    pub skill_id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub instructions_ref: String,
    #[serde(default)]
    pub instructions_digest: String,
    #[serde(default)]
    pub base_directory: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    pub instructions: String,
    #[serde(default)]
    pub required_tools: Vec<String>,
    #[serde(default)]
    pub validation_steps: Vec<String>,
    #[serde(default)]
    pub activation: AgentSkillActivation,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentSkillActivation {
    #[default]
    Turn,
    Session,
}

/// Mutable execution state for one Thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExecutionState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_worktree: Option<nola_domain::task_workspace::PrimaryWorktreeBinding>,
    /// Read-only hot observations from the immediately preceding Turn only.
    /// Older cells are queried through Application's conversation-owned store;
    /// checkpoints must not accumulate every script/output in a long session.
    pub retired_code_cells: BTreeMap<String, super::code_mode::CodeCellState>,
    pub requirements: super::runtime::requirements::RequirementExecutionState,
    pub user_input_waits: std::collections::BTreeMap<String, nola_domain::user_input_wait::UserInputWait>,
    /// Heap ownership keeps checkpoint visitors from inlining a whole Turn.
    #[serde(default)]
    pub active_turn: Option<Box<AgentTurnState>>,
    #[serde(default)]
    pub input_queue: AgentInputQueue,
    pub permission_rules: PermissionRuleSet,
    pub active_permission_policy: Option<nola_domain::ActivePermissionPolicy>,
    /// Canonical permission UI state, persisted independently from Host
    /// presentation so reconnects do not duplicate or lose prompts.
    #[serde(default)]
    pub permissions: super::permissions::PermissionProjection,
}

/// Concrete Host lifetime and its outstanding effects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHostState {
    #[serde(rename = "host_catalog")]
    pub catalog: HostCapabilityCatalog,
    #[serde(rename = "pending_host_effects")]
    pub pending_effects: HostEffectJournal,
}

/// Event sourcing, idempotency, and recovery metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentContextUsageFingerprint {
    pub measurement: Option<nola_domain::ContextUsageMeasurement>,
    pub prompt_cache_measurement: Option<nola_domain::ContextUsageMeasurement>,
    #[serde(default)]
    pub cache_usage: Option<nola_domain::PromptCacheUsage>,
    pub used_tokens: usize,
    pub context_window_tokens: usize,
    pub usage_is_estimated: bool,
    #[serde(default)]
    pub cached_input_tokens: usize,
    #[serde(default)]
    pub cached_input_tokens_reported: bool,
    #[serde(default)]
    pub cache_write_input_tokens: usize,
    #[serde(default)]
    pub cache_write_input_tokens_reported: bool,
    #[serde(default)]
    pub prompt_cache_diagnostic: Option<crate::prompt_cache::PromptCacheDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentJournalState {
    pub trace_context: AgentTraceContext,
    pub revision: u64,
    pub next_event_sequence: u64,
    #[serde(default)]
    pub next_id_sequence: u64,
    #[serde(default)]
    pub next_turn_sequence: u64,
    #[serde(default)]
    pub accepted_submission_ids: BTreeSet<String>,
    #[serde(default)]
    pub accepted_submission_order: std::collections::VecDeque<String>,
    pub accepted_input_fingerprints: std::collections::BTreeMap<String, String>,
    pub session_started_emitted: bool,
    #[serde(default)]
    pub session_closing: bool,
    #[serde(default)]
    pub session_closed: bool,
    #[serde(default)]
    pub last_context_usage_fingerprint: Option<AgentContextUsageFingerprint>,
}

impl AgentConversationState {
    fn new(context_window: ContextWindowConfig) -> Self {
        Self {
            transcript: Vec::new(),
            model_context: Vec::new(),
            provider_continuation: None,
            context_window: ContextWindowState::new(context_window),
            projection: ConversationProjection::default(),
            pinned_skills: BTreeMap::new(),
        }
    }
}

impl AgentExecutionState {
    fn new() -> Self {
        Self {
            primary_worktree: None,
            retired_code_cells: BTreeMap::new(),
            user_input_waits: Default::default(),
            requirements: Default::default(),
            active_turn: None,
            input_queue: AgentInputQueue::default(),
            permission_rules: PermissionRuleSet::default(),
            active_permission_policy: None,
            permissions: super::permissions::PermissionProjection::default(),
        }
    }

    pub(crate) fn retire_code_cells(&mut self) -> Result<(), String> {
        let Some(turn) = self.active_turn.as_ref() else { return Ok(()); };
        for (id, cell) in &turn.code_cells {
            cell.validate()?;
            if !cell.is_settled() {
                return Err("cannot retire a code cell with unreconciled effects".into());
            }
            if self.retired_code_cells.get(id).is_some_and(|previous| previous != cell) {
                return Err("retired code cell identity conflict".into());
            }
        }
        // Validate the entire transfer before moving anything. Never discard a
        // live isolate simply because its owning conversation is being rebased.
        let cells = std::mem::take(&mut self.active_turn.as_mut().expect("checked Turn").code_cells);
        self.retired_code_cells = cells;
        Ok(())
    }

    pub fn code_cell(&self, id: &str) -> Option<&super::code_mode::CodeCellState> {
        self.active_turn.as_ref().and_then(|turn| turn.code_cells.get(id))
            .or_else(|| self.retired_code_cells.get(id))
    }
}

impl AgentHostState {
    fn new() -> Self {
        Self {
            catalog: HostCapabilityCatalog::empty("local"),
            pending_effects: HostEffectJournal::default(),
        }
    }
}

impl AgentJournalState {
    fn new(trace_context: AgentTraceContext) -> Self {
        Self {
            trace_context,
            revision: 0,
            next_event_sequence: 1,
            next_id_sequence: 0,
            next_turn_sequence: 0,
            accepted_submission_ids: BTreeSet::new(),
            accepted_submission_order: std::collections::VecDeque::new(),
            accepted_input_fingerprints: std::collections::BTreeMap::new(),
            session_started_emitted: false,
            session_closing: false,
            session_closed: false,
            last_context_usage_fingerprint: None,
        }
    }

    pub(crate) fn has_accepted_submission(&self, submission_id: &str) -> bool {
        self.accepted_submission_ids.contains(submission_id)
    }

    pub(crate) fn record_accepted_submission(&mut self, submission_id: String) {
        const MAX_ACCEPTED_SUBMISSIONS: usize = 4_096;

        let order_matches_set = self.accepted_submission_order.len()
            == self.accepted_submission_ids.len()
            && self
                .accepted_submission_order
                .iter()
                .all(|value| self.accepted_submission_ids.contains(value));
        if !order_matches_set {
            self.accepted_submission_order = self
                .accepted_submission_ids
                .iter()
                .take(MAX_ACCEPTED_SUBMISSIONS)
                .cloned()
                .collect();
            self.accepted_submission_ids = self.accepted_submission_order.iter().cloned().collect();
        }
        if !self.accepted_submission_ids.insert(submission_id.clone()) {
            return;
        }
        self.accepted_submission_order.push_back(submission_id);
        while self.accepted_submission_order.len() > MAX_ACCEPTED_SUBMISSIONS {
            if let Some(expired) = self.accepted_submission_order.pop_front() {
                self.accepted_submission_ids.remove(&expired);
            }
        }
    }
}

pub use nola_domain::code_mode::CodeItemSource as AgentItemSource;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentItemOwnership {
    pub source: AgentItemSource,
    pub turn_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionLineage {
    pub parent_session_id: String,
    pub through_item_id: String,
    pub parent_revision: u64,
    pub parent_event_cursor: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentThreadState {
    pub session_id: SessionId,
    /// Serde preserves the configuration object while its owner stays small.
    pub configuration: Box<AgentSessionConfiguration>,
    pub conversation: AgentConversationState,
    pub execution: AgentExecutionState,
    pub host: AgentHostState,
    pub journal: AgentJournalState,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub item_ownership: BTreeMap<String, AgentItemOwnership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<AgentSessionLineage>,
    #[serde(default)]
    pub task_projection: TaskProjection,
    /// Original objective, audited requirements, and resumable round state.
    /// This is intentionally independent from the growing conversation.
    /// Required even when empty: absence must not reset semantic operation IDs.
    pub goal_projection: GoalProjection,
    #[serde(default)]
    pub memory_projection: MemoryProjection,
    /// Durable extraction/consolidation jobs and usage accounting. This lives
    /// beside the records so a restarted Host resumes the same owned work
    /// instead of manufacturing a platform-private queue.
    #[serde(default)]
    pub memory_runtime: MemoryRuntimeProjection,
    #[serde(default)]
    pub skill_runtime: SkillRuntimeState,
}

impl AgentThreadState {
    /// Bounded read-only admissibility for replacing idle execution metadata.
    /// Callers still need their own Store/Run admission transaction; this fact
    /// exposes neither queued input nor frozen delivery/permission authority.
    pub fn permits_new_turn_execution_configuration(&self) -> bool {
        self.execution
            .active_turn
            .as_ref()
            .is_none_or(|turn| turn.status.is_terminal())
            && self.host.pending_effects.is_quiescent()
            && self.execution.input_queue.len() == 0
    }

    pub fn new(
        session_id: SessionId,
        configuration: impl Into<Box<AgentSessionConfiguration>>,
    ) -> Self {
        let configuration = configuration.into();
        let context_window = configuration.context_window.clone();
        let trace_context = AgentTraceContext::new(session_id.as_str().to_string(), None);
        Self {
            session_id,
            configuration,
            conversation: AgentConversationState::new(context_window),
            execution: AgentExecutionState::new(),
            host: AgentHostState::new(),
            journal: AgentJournalState::new(trace_context),
            item_ownership: BTreeMap::new(),
            lineage: None,
            task_projection: TaskProjection::default(),
            goal_projection: GoalProjection::default(),
            memory_projection: MemoryProjection::default(),
            memory_runtime: MemoryRuntimeProjection::default(),
            skill_runtime: SkillRuntimeState::default(),
        }
    }

    pub fn allocate_id(&mut self, kind: &str) -> String {
        // The numeric journal sequence is only monotonic inside one restored
        // Runtime. A completed Runtime may be rebuilt from the canonical
        // transcript, so session-scoped IDs would collide with Parts and Host
        // effects from an earlier Run. Turn IDs are durable product identities
        // and therefore provide the correct namespace for all Turn-owned IDs.
        let namespace = self
            .execution
            .active_turn
            .as_ref()
            .map(|turn| turn.turn_id.as_str().to_string())
            .unwrap_or_else(|| self.session_id.as_str().to_string());
        self.journal.next_id_sequence = self.journal.next_id_sequence.saturating_add(1);
        format!("{namespace}:{kind}:{}", self.journal.next_id_sequence)
    }

    pub fn allocate_turn_id(&mut self) -> String {
        // Older checkpoints did not persist a separate turn counter. Seed it
        // from durable user messages once so restored conversations continue
        // with human-scale turn numbers without reusing earlier identifiers.
        if self.journal.next_turn_sequence == 0 {
            self.journal.next_turn_sequence = self
                .conversation
                .transcript
                .iter()
                .filter(|item| {
                    item.is_user_input()
                })
                .count() as u64;
        }
        self.journal.next_turn_sequence = self.journal.next_turn_sequence.saturating_add(1);
        format!(
            "{}:turn:{}",
            self.session_id.as_str(),
            self.journal.next_turn_sequence
        )
    }

    pub fn record_item(&mut self, item: AgentItem) {
        let turn_id = self
            .execution
            .active_turn
            .as_ref()
            .map(|turn| turn.turn_id.clone());
        self.record_item_for_turn(item, turn_id.as_ref());
    }

    pub(crate) fn record_turn_items(
        &mut self,
        turn_id: &TurnId,
        items: impl IntoIterator<Item = AgentItem>,
    ) {
        for item in items {
            self.record_item_for_turn(item, Some(turn_id));
        }
    }

    fn record_item_for_turn(&mut self, item: AgentItem, turn_id: Option<&TurnId>) {
        let nested = self.is_code_nested_item(&item, turn_id);
        if let Some(turn_id) = turn_id {
            let mut owner = self.item_owner_for_turn(&item,turn_id);
            owner.source = self.item_source(&item, Some(turn_id));
            self.item_ownership.insert(item.id().as_str().to_owned(),owner);
        }
        if let AgentItem::Reasoning {
            provider_continuation: Some(continuation),
            ..
        } = &item
        {
            self.conversation.provider_continuation = Some(continuation.clone());
        }
        self.conversation.transcript.push(item.clone());
        if !nested { self.conversation.model_context.push(item); }
    }

    fn is_code_nested_item(&self, item: &AgentItem, turn_id: Option<&TurnId>) -> bool {
        !self.item_source(item, turn_id).is_model_visible()
    }

    fn item_source(&self, item: &AgentItem, turn_id: Option<&TurnId>) -> AgentItemSource {
        let call_id = match item {
            AgentItem::ToolCall { call_id, .. } | AgentItem::ToolResult { call_id, .. } => call_id,
            _ => return AgentItemSource::Model,
        };
        self.tool_record_source(turn_id, call_id.as_str())
    }

    pub(crate) fn tool_record_source(&self, turn_id: Option<&TurnId>, call_id: &str) -> AgentItemSource {
        self.execution.active_turn.as_ref()
            .filter(|turn| turn_id == Some(&turn.turn_id))
            .and_then(|turn| turn.code_cells.values()
                .find(|cell| cell.owns_call_id(call_id)))
            .map(|cell| AgentItemSource::CodeCell { cell: cell.identity().clone() })
            .unwrap_or_default()
    }

    pub fn record_items(&mut self, items: impl IntoIterator<Item = AgentItem>) {
        for item in items {
            self.record_item(item);
        }
    }

    fn item_owner_for_turn(&self,item:&AgentItem,turn_id:&TurnId)->AgentItemOwnership {
        // Results stay with their original call even if guidance opened a new
        // response segment while the tool was running.
        let tool_message = match item {
            AgentItem::ToolCall {call_id,..} | AgentItem::ToolResult {call_id,..} =>
                self.conversation.projection.parts.values().find_map(|part| match part {
                    super::parts::AgentPart::Tool(tool) if tool.call_id == call_id.as_str()
                        && tool.ownership.turn_id == turn_id.as_str() => Some(tool.ownership.message_id.clone()),
                    _ => None,
                }),
            _ => None,
        };
        AgentItemOwnership {
            source: Default::default(),
            turn_id:turn_id.as_str().to_owned(),
            message_id:match item {
                AgentItem::UiAction { .. } | AgentItem::Message {role:AgentRole::User,..}=>item.id().as_str().to_owned(),
                _=>tool_message.or_else(|| self.conversation.projection.active_message_segments
                    .get(turn_id.as_str()).cloned()).unwrap_or_else(|| format!("{}:assistant",turn_id.as_str())),
            },
        }
    }

    /// The original result and its bounded model view are one logical item.
    /// The caller supplies the validated effect Turn, not an ambient/latest
    /// owner. Commit provenance together with both views, including failures.
    pub(crate) fn record_item_views(&mut self, turn_id:&TurnId, transcript_item: AgentItem, model_context_item: AgentItem) -> Result<(),String> {
        let same_identity=matches!((&transcript_item,&model_context_item),
            (AgentItem::ToolResult {id,call_id,name,is_error,..},AgentItem::ToolResult {id:model_id,call_id:model_call,name:model_name,is_error:model_error,..})
                if id==model_id && call_id==model_call && name==model_name && is_error==model_error);
        if !same_identity || !self.execution.active_turn.as_ref().is_some_and(|turn|&turn.turn_id==turn_id)
            || self.item_ownership.contains_key(transcript_item.id().as_str()) {
            return Err("tool result views lack one exact original Turn/item/call identity".into());
        }
        let mut owner = self.item_owner_for_turn(&transcript_item,turn_id);
        owner.source = self.item_source(&transcript_item, Some(turn_id));
        self.item_ownership.insert(transcript_item.id().as_str().to_owned(),owner);
        let nested = self.is_code_nested_item(&transcript_item, Some(turn_id));
        self.conversation.transcript.push(transcript_item);
        if !nested { self.conversation.model_context.push(model_context_item); }
        Ok(())
    }

    /// Close a turn without rewriting its already-sent model prefix.
    /// Tool results are bounded on admission; only committed compaction may
    /// replace history. Keep protocol call/result pairs and reasoning intact.
    pub(crate) fn seal_completed_turn_model_context(&mut self, _turn_id: &TurnId) {
        self.conversation
            .context_window
            .refresh_estimate(&self.conversation.model_context);
    }

    pub fn record_invoked_skill(&mut self, skill: AgentInvokedSkill) -> Vec<String> {
        let activation_source = format!("skill:{}", skill.skill_id);
        let required_tools = skill.required_tools.clone();
        if skill.activation == AgentSkillActivation::Session {
            self.conversation
                .pinned_skills
                .entry(skill.skill_id.clone())
                .or_insert(skill);
        } else if let Some(turn) = self.execution.active_turn.as_mut() {
            turn.invoked_skills
                .entry(skill.skill_id.clone())
                .or_insert(skill);
        }
        if let Some(turn) = self.execution.active_turn.as_mut() {
            return turn.activate_capabilities(activation_source, required_tools);
        }
        Vec::new()
    }

    pub fn invoked_skill_context_items(&self) -> Vec<AgentItem> {
        let turn_skills = self
            .execution
            .active_turn
            .iter()
            .flat_map(|turn| turn.invoked_skills.values())
            .filter(|skill| {
                !self
                    .conversation
                    .pinned_skills
                    .contains_key(&skill.skill_id)
            });
        self.conversation
            .pinned_skills
            .values()
            .chain(turn_skills)
            .map(|skill| {
                let rendered_arguments = match &skill.arguments {
                    serde_json::Value::String(value) => value.clone(),
                    value => serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
                };
                AgentItem::Message {
                    id: super::protocol::ItemId::from_string(format!(
                        "{}:invoked-skill:{}",
                        self.session_id.as_str(),
                        skill.skill_id
                    )),
                    role: AgentRole::System,
                    content: format!(
                    "<invoked-skill name={:?}>\nSkill version: {}\nImmutable instructions ref: {}\nInstructions SHA-256: {}\nSkill resource directory: {}\nInvocation arguments: {}\n{}\n\n{}\n</invoked-skill>\nContinue following this Skill for the {}. Resolve only Skill-owned scripts, templates, and reference files from the Skill resource directory. User-provided paths, workspace files, and deliverable destinations remain relative to the active workspace. Do not invoke it again during that scope.",
                    skill.skill_id,
                    skill.version,
                    skill.instructions_ref,
                    skill.instructions_digest,
                    skill.base_directory,
                    rendered_arguments,
                    if skill.validation_steps.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "Required final validation steps from the Skill manifest:\n{}",
                            skill.validation_steps
                                .iter()
                                .map(|step| format!("- {step}"))
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    },
                    skill.instructions,
                    if skill.activation == AgentSkillActivation::Session { "conversation" } else { "active turn" }
                    ),
                }
            })
            .collect()
    }

    pub fn invoked_skill_validation_steps(&self) -> Vec<String> {
        let turn_skills = self
            .execution
            .active_turn
            .iter()
            .flat_map(|turn| turn.invoked_skills.values())
            .filter(|skill| {
                !self
                    .conversation
                    .pinned_skills
                    .contains_key(&skill.skill_id)
            });
        self.conversation
            .pinned_skills
            .values()
            .chain(turn_skills)
            .flat_map(|skill| skill.validation_steps.iter().cloned())
            .collect()
    }

    /// Returns the concrete, still-unsatisfied completion records for Skills
    /// owned by the active Turn. Unlike the static manifest prose, this is
    /// derived from Core's SkillRuntime state and therefore becomes empty as
    /// progress, artifacts, and verifier evidence are recorded.
    pub fn pending_skill_completion_steps(&self) -> Vec<String> {
        let Some(turn) = self.execution.active_turn.as_ref() else {
            return Vec::new();
        };
        self.skill_runtime
            .runs
            .values()
            .filter(|run| {
                run.owner.parent_turn_id == turn.turn_id.as_str() && !run.status.is_terminal()
            })
            .flat_map(|run| {
                let contract = &run.descriptor.execution_contract;
                let mut pending = Vec::new();
                pending.extend(
                    contract
                        .progress_steps
                        .iter()
                        .filter(|step| {
                            step.required && !run.completed_progress_steps.contains(&step.id)
                        })
                        .map(|step| {
                            format!(
                                "skill_update run_id={} action=progress step_id={} completed=true ({})",
                                run.descriptor.skill_id, step.id, step.title
                            )
                        }),
                );
                pending.extend(
                    contract
                        .artifacts
                        .iter()
                        .filter(|artifact| {
                            artifact.required && !run.artifacts.contains_key(&artifact.id)
                        })
                        .map(|artifact| {
                            format!(
                                "skill_update run_id={} action=artifact artifact_id={} artifact_ref=<canonical validated URI>",
                                run.descriptor.skill_id, artifact.id
                            )
                        }),
                );
                pending.extend(
                    contract
                        .verifiers
                        .iter()
                        .filter(|verifier| {
                            verifier.required
                                && !run
                                    .verifications
                                    .get(&verifier.id)
                                    .is_some_and(|result| result.passed)
                        })
                        .map(|verifier| {
                            format!(
                                "skill_update run_id={} action=verification verifier_id={} passed=true evidence=<structured {}; cite real execution_evidence or tool_call IDs, not invented refs> ({})",
                                run.descriptor.skill_id, verifier.id, verifier.method.as_str(), verifier.description
                            )
                        }),
                );
                pending
            })
            .collect()
    }

    /// Returns whether every required artifact for each nonterminal Skill
    /// owned by the active Turn has already been registered from a real Host
    /// result. This lets the final tool-capable round settle only internal
    /// completion records after durable outputs exist, without forcing a
    /// freshly invoked Skill to report progress before doing its work.
    pub fn active_skill_required_artifacts_satisfied(&self) -> bool {
        let Some(turn) = self.execution.active_turn.as_ref() else {
            return false;
        };
        let runs = self
            .skill_runtime
            .runs
            .values()
            .filter(|run| {
                run.owner.parent_turn_id == turn.turn_id.as_str() && !run.status.is_terminal()
            })
            .collect::<Vec<_>>();
        !runs.is_empty()
            && runs.iter().all(|run| {
                run.descriptor
                    .execution_contract
                    .artifacts
                    .iter()
                    .all(|artifact| !artifact.required || run.artifacts.contains_key(&artifact.id))
            })
    }

    /// Returns whether active Skills have crossed the activation boundary and
    /// can truthfully be asked to report completion records. A freshly loaded
    /// Skill has not yet had a model decision made under its instructions, so
    /// presenting its progress/verifier checklist at that point competes with
    /// the actual work and can make weaker models report completion first.
    pub fn active_skill_completion_reporting_ready(&self) -> bool {
        let Some(turn) = self.execution.active_turn.as_ref() else {
            return false;
        };
        let runs = self
            .skill_runtime
            .runs
            .values()
            .filter(|run| {
                run.owner.parent_turn_id == turn.turn_id.as_str() && !run.status.is_terminal()
            })
            .collect::<Vec<_>>();
        !runs.is_empty()
            && runs.iter().all(|run| run.usage.model_turns > 0)
            && runs.iter().all(|run| {
                run.descriptor
                    .execution_contract
                    .artifacts
                    .iter()
                    .all(|artifact| !artifact.required || run.artifacts.contains_key(&artifact.id))
            })
    }

    /// Returns the durable representation of a Thread.
    ///
    /// An active Turn must retain its frozen typed context until the Turn is
    /// terminal. A recovered provider response can immediately produce another
    /// model request or tool continuation; clearing the context here makes that
    /// continuation observe a different world than the request it resumes.
    /// The frozen pending request is part of the recovery contract: Core needs
    /// it to validate the response family before advancing the state machine.
    pub fn checkpoint_projection(&self) -> Self {
        self.clone().into_checkpoint_projection()
    }

    /// Live recovery state already has the durable shape. Only terminal
    /// history requires an owned projection to remove discovery-only data.
    pub fn checkpoint_projection_view(&self) -> std::borrow::Cow<'_, Self> {
        if self.execution.active_turn.as_ref().is_some_and(|turn| {
            turn.status.is_terminal() && self.permits_new_turn_execution_configuration()
        }) {
            std::borrow::Cow::Owned(self.checkpoint_projection())
        } else {
            std::borrow::Cow::Borrowed(self)
        }
    }

    /// Projects an owned Thread without copying its transcript or frozen requests.
    /// Callers retaining a live Thread use `checkpoint_projection` instead.
    pub fn into_checkpoint_projection(mut self) -> Self {
        // The owned Host journal already contains only external effects. Keep
        // every frozen request, delivery cursor, and completed-effect identity.
        let terminal_history = self.execution.active_turn.as_ref().is_some_and(|turn| {
            turn.status.is_terminal() && self.permits_new_turn_execution_configuration()
        });
        if terminal_history {
            // Calls/results are the durable historical record. Tool schemas and
            // activation projections are live discovery state, including for a
            // tool that happened to be called, and are rebuilt from the current
            // Host before a successor Turn.
            self.configuration.tools.clear();
            self.host
                .catalog
                .capabilities
                .retain(|_, descriptor| descriptor.route != HostCapabilityRoute::Tool);
            self.skill_runtime.redact_catalog_for_history();
            let turn = self.execution.active_turn.as_mut().unwrap();
            // Skill and SOP catalogs are live Host discovery data. Persisting
            // their full contents on every Turn makes every unselected entry
            // part of terminal conversation history. A selected Skill is already
            // represented by `invoked_skills` and `skill_runtime.runs`; an
            // invoked SOP is represented by its durable run and active-run
            // context. Nonterminal recovery retains the full frozen Turn;
            // terminal history needs identities and actual run evidence only.
            turn.context.components.retain(|component| {
                component.kind != crate::context_engine::ContextComponentKind::Skill
                    && !is_ephemeral_sop_catalog_component(&component.id)
            });
            turn.context.items.retain(|item| {
                let id = item.id().as_str();
                !is_ephemeral_skill_context_item(id) && !is_ephemeral_sop_catalog_item(id)
            });
            turn.context.configuration.tools.clear();
            turn.context
                .capability_projection
                .redact_tool_catalog_for_history();
            turn.context.runtime_snapshot.enabled_capability_ids.clear();
            turn.activated_capabilities.clear();
            turn.model_host_tool_working_set.clear();
            if let Some(projection) = turn.last_model_tool_projection.as_mut() {
                projection.capability_ids.clear();
            }
        }
        self
    }
}

fn is_ephemeral_skill_context_item(id: &str) -> bool {
    id.starts_with("nola-context-component-skill-")
}

fn is_ephemeral_sop_catalog_component(id: &str) -> bool {
    id == "nola-available-sops-v1" || id.starts_with("nola-sop-catalog-")
}

fn is_ephemeral_sop_catalog_item(id: &str) -> bool {
    id == "nola-context-component-system-nola-available-sops-v1"
        || id.starts_with("nola-context-component-system-nola-sop-catalog-")
}

#[cfg(test)]
mod tests {
    use super::{
        AgentInvokedSkill, AgentItemOwnership, AgentSessionConfiguration, AgentSkillActivation, AgentThreadState,
    };
    use crate::agent::host::{
        CapabilityAvailability, HostCancellationMode, HostCapabilityDescriptor,
        HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostOutputPolicy,
        HostReplaySafety,
    };
    use crate::agent::{
        AgentItem, AgentRole, AgentTurnContext, AgentTurnIdentity, AgentTurnState, AgentTurnStatus,
        CallId, ContextWindowConfig, ItemId, SessionId, TurnId,
    };
    use crate::context_engine::{ContextComponent, ContextComponentKind};

    fn configuration() -> AgentSessionConfiguration {
        AgentSessionConfiguration {
            model: None,
            max_model_iterations: 0,
            max_strategy_recovery_attempts: 3,
            tools: Vec::new(),
            context_window: ContextWindowConfig::default(),
            memory_policy: crate::memory::MemoryPolicy::default(),
            turn_identity: AgentTurnIdentity::default(),
        }
    }

    #[test]
    fn recovery_allowance_is_frozen_patchable_and_required_on_restore() {
        let runtime = crate::agent::AgentCoreConfig::default();
        assert_eq!(runtime.max_strategy_recovery_attempts, 3);
        let mut frozen = AgentSessionConfiguration::from_runtime(&runtime);
        assert!(frozen.matches_runtime(&runtime));
        frozen.apply_patch(crate::agent::AgentSessionConfigurationPatch {
            max_strategy_recovery_attempts: Some(0),
            ..Default::default()
        });
        assert_eq!(frozen.max_strategy_recovery_attempts, 0);
        assert!(!frozen.matches_runtime(&runtime));
        let mut value = serde_json::to_value(&frozen).unwrap();
        assert_eq!(serde_json::from_value::<AgentSessionConfiguration>(value.clone())
            .unwrap().max_strategy_recovery_attempts, 0);
        value.as_object_mut().unwrap().remove("max_strategy_recovery_attempts");
        assert!(serde_json::from_value::<AgentSessionConfiguration>(value).is_err());
    }

    fn start_turn(state: &mut AgentThreadState, id: &str) {
        state.execution.active_turn = Some(Box::new(AgentTurnState::new(
            TurnId::from_string(id.to_string()),
            AgentTurnContext {
                input_origin: Default::default(),
                configuration: state.configuration.clone(),
                platform: Default::default(),
                components: Vec::new(),
                component_metrics: Vec::new(),
                component_payload_digest: String::new(),
                items: Vec::new(),
                host_id: "test-host".to_string(),
                host_catalog_revision: 1,
                host_availability_epoch: 1,
                capability_projection: Default::default(),
                runtime_snapshot: Default::default(),
            },
        )));
    }

    fn result_view(id:&str,call:&str,error:bool,output:serde_json::Value)->AgentItem {
        AgentItem::ToolResult {id:ItemId::from_string(id.into()),call_id:CallId::from_string(call.into()),
            name:"read_file".into(),output,is_error:error,full_output_ref:None,artifacts:vec![]}
    }

    #[test]
    fn settled_code_output_survives_turn_replacement_without_resuming_execution() {
        use crate::agent::code_mode::{CodeCellState, CodeCellPhase};
        use nola_domain::code_mode::{CodeCellIdentity, CodeCellLimits, CodeOutputFrame, CodeOutputChunk};
        let mut state = AgentThreadState::new(SessionId::from_string("session".into()), configuration());
        start_turn(&mut state, "first");
        let owner = CodeCellIdentity {
            host_id:"h".into(), conversation_id:"session".into(), turn_id:"first".into(),
            parent_call_id:"p".into(), cell_id:"c".into(), generation:"g".into(),
        };
        let mut cell = CodeCellState::new(owner.clone(), CodeCellLimits::default(), "text(1)").unwrap();
        cell.append_output(CodeOutputFrame { cell:owner.clone(), chunk:CodeOutputChunk {
            sequence:1, text:"saved result".into(),
        }}).unwrap();
        state.execution.active_turn.as_mut().unwrap().code_cells.insert("c".into(), cell);
        assert!(state.execution.retire_code_cells().is_err());
        assert!(state.execution.retired_code_cells.is_empty());
        state.execution.active_turn.as_mut().unwrap().code_cells.get_mut("c").unwrap()
            .finish(&owner, CodeCellPhase::Completed).unwrap();
        state.execution.retire_code_cells().unwrap();
        start_turn(&mut state, "second");
        let restored: AgentThreadState = serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        let cell = restored.execution.code_cell("c").unwrap();
        assert_eq!(cell.phase(), &CodeCellPhase::Completed);
        assert_eq!(cell.output_after(&owner, 0).unwrap()[0].text, "saved result");
        assert!(cell.output_after(&owner, 1).unwrap().is_empty());
        assert!(restored.execution.active_turn.as_ref().unwrap().code_cells.is_empty());
        // An empty successor still rotates the hot observations. The old
        // result remains in Application history, not an ever-growing Core map.
        state.execution.retire_code_cells().unwrap();
        start_turn(&mut state, "third");
        assert!(state.execution.retired_code_cells.is_empty());
        assert!(state.execution.code_cell("c").is_none());
    }

    #[test]
    fn code_nested_transcript_is_complete_without_orphan_model_results() {
        use crate::agent::code_mode::CodeCellState;
        use nola_domain::code_mode::{CodeCellIdentity, CodeCellLimits, CodeNestedCall};
        let mut state = AgentThreadState::new(SessionId::from_string("session".into()), configuration());
        start_turn(&mut state, "turn");
        let turn_id = TurnId::from_string("turn".into());
        let owner = CodeCellIdentity { host_id:"test-host".into(), conversation_id:"session".into(),
            turn_id:"turn".into(), parent_call_id:"parent".into(), cell_id:"cell".into(), generation:"g".into() };
        let mut cell = CodeCellState::new(owner.clone(), CodeCellLimits::default(), "text(1)").unwrap();
        cell.register_call(CodeNestedCall { cell:owner.clone(), sequence:1,
            tool_name:"read_file".into(), arguments:serde_json::json!({"path":"source"}) }).unwrap();
        let nested = cell.nested_call_id(1);
        state.execution.active_turn.as_mut().unwrap().code_cells.insert("cell".into(), cell);
        state.record_turn_items(&turn_id, [AgentItem::ToolCall {
            id:ItemId::from_string("nested-call".into()), call_id:CallId::from_string(nested.clone()),
            name:"read_file".into(), arguments:serde_json::json!({"path":"source"}),
        }]);
        // Ownership must remain known even after delivery removed the waiter.
        state.execution.active_turn.as_mut().unwrap().code_cells.get_mut("cell").unwrap()
            .settle_call(&owner, 1).unwrap();
        let result = result_view("nested-result", &nested, false, serde_json::json!({"text":"private intermediate"}));
        state.record_item_views(&turn_id, result.clone(), result).unwrap();
        assert_eq!(state.conversation.transcript.len(), 2);
        assert!(state.conversation.model_context.is_empty());
        let restored: AgentThreadState = serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(restored.conversation.transcript, state.conversation.transcript);
        assert!(restored.conversation.model_context.is_empty());
        for id in ["nested-call", "nested-result"] {
            assert_eq!(restored.item_ownership[id].source,
                super::AgentItemSource::CodeCell { cell: owner.clone() });
        }
        // The boundary survives removal of the active execution, not merely
        // serialization while the cell still exists.
        let mut historical = restored;
        historical.execution.active_turn = None;
        assert!(historical.conversation.transcript.iter().all(|item|
            !historical.item_ownership[item.id().as_str()].source.is_model_visible()));
        let parent = result_view("parent-result", "parent", false, serde_json::json!({"text":"explicit output"}));
        state.record_item_views(&turn_id, parent.clone(), parent).unwrap();
        assert_eq!(state.conversation.model_context.len(), 1);
    }

    #[test]
    fn split_tool_views_bind_original_turn_for_success_error_and_reverse_parallel_results() {
        for error in [false,true] {
            let mut state=AgentThreadState::new(SessionId::from_string("session".into()),configuration());
            start_turn(&mut state,"turn");
            let turn=TurnId::from_string("turn".into());
            // Complete the second request first, as buffered parallel tools do.
            for index in [2,1] {
                let id=format!("result-{index}");
                let original=result_view(&id,&format!("call-{index}"),error,serde_json::json!({"text":"原文".repeat(2048)}));
                let model=result_view(&id,&format!("call-{index}"),error,serde_json::json!({"preview":"bounded"}));
                state.record_item_views(&turn,original.clone(),model.clone()).unwrap();
                assert_eq!(state.conversation.transcript.last(),Some(&original));
                assert_eq!(state.conversation.model_context.last(),Some(&model));
                assert_eq!(state.item_ownership[&id],AgentItemOwnership {source: Default::default(),turn_id:"turn".into(),message_id:"turn:assistant".into()});
            }
            let restored:AgentThreadState=serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
            assert_eq!(restored.item_ownership,state.item_ownership);
            assert_eq!(restored.conversation.transcript,state.conversation.transcript);
            assert_eq!(restored.conversation.model_context,state.conversation.model_context);
        }
    }

    #[test]
    fn split_tool_views_reject_foreign_turn_duplicate_or_changed_logical_identity_atomically() {
        for mutation in ["item","call","name","error","turn","absent_turn","duplicate"] {
            let mut state=AgentThreadState::new(SessionId::from_string("session".into()),configuration());
            start_turn(&mut state,"turn");
            let original=result_view("result","call",false,serde_json::json!({"original":true}));
            let mut model=result_view("result","call",false,serde_json::json!({"preview":true}));
            let mut turn=TurnId::from_string("turn".into());
            match mutation {
                "item"=>if let AgentItem::ToolResult {id,..}=&mut model {*id=ItemId::from_string("foreign-result".into());},
                "call"=>if let AgentItem::ToolResult {call_id,..}=&mut model {*call_id=CallId::from_string("foreign-call".into());},
                "name"=>if let AgentItem::ToolResult {name,..}=&mut model {*name="foreign-tool".into();},
                "error"=>if let AgentItem::ToolResult {is_error,..}=&mut model {*is_error=true;},
                "turn"=>turn=TurnId::from_string("foreign-turn".into()),
                "absent_turn"=>state.execution.active_turn=None,
                "duplicate"=>state.record_item_views(&turn,original.clone(),model.clone()).unwrap(),
                _=>unreachable!(),
            }
            let before=serde_json::to_value(&state).unwrap();
            assert!(state.record_item_views(&turn,original,model).is_err(),"{mutation}");
            assert_eq!(serde_json::to_value(&state).unwrap(),before,"{mutation} wrote a partial view or owner");
        }
    }

    #[test]
    fn new_turn_configuration_requires_quiescent_execution_and_queued_input() {
        use crate::agent::AgentInputDelivery;
        let mut state =
            AgentThreadState::new(SessionId::from_string("quiescence".into()), configuration());
        assert!(state.permits_new_turn_execution_configuration());
        start_turn(&mut state, "turn-1");
        assert!(
            !state.permits_new_turn_execution_configuration(),
            "an active Turn owns its frozen configuration"
        );
        for status in [
            AgentTurnStatus::Completed,
            AgentTurnStatus::Failed,
            AgentTurnStatus::Cancelled,
        ] {
            state.execution.active_turn.as_mut().unwrap().status = status;
            assert!(state.permits_new_turn_execution_configuration());
        }
        state.configuration.tools = vec![serde_json::json!({"name":"owned.schema"})];
        for delivery in [
            AgentInputDelivery::CurrentTurn,
            AgentInputDelivery::NextTurn,
        ] {
            let mut queued = state.clone();
            queued
                .execution
                .input_queue
                .enqueue(
                    delivery,
                    [AgentItem::user_message(
                        ItemId::from_string("queued-user".into()),
                        "Original queued request",
                    )],
                )
                .unwrap();
            let before = serde_json::to_value(&queued).unwrap();
            assert!(!queued.permits_new_turn_execution_configuration());
            assert_eq!(
                serde_json::to_value(&queued).unwrap(),
                before,
                "predicate is read-only"
            );
            assert!(
                !queued
                    .checkpoint_projection()
                    .configuration
                    .tools
                    .is_empty(),
                "queued work is frozen recovery state, not closed terminal discovery history"
            );
        }
        assert!(
            state.checkpoint_projection().configuration.tools.is_empty(),
            "quiescent terminal history remains redacted"
        );
    }

    fn host_capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
        HostCapabilityDescriptor {
            id: id.to_string(),
            title: id.to_string(),
            route,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Hidden,
            permission_domain: None,
            input_schema: serde_json::json!({"type":"object"}),
            input_normalization: Default::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 1_000,
                persist_full_output: true,
                allow_artifacts: false,
            },
            supports_streaming: route == HostCapabilityRoute::Model,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        }
    }

    #[test]
    fn owned_checkpoint_projection_preserves_bodies_and_borrowed_semantics() {
        use crate::agent::host::{
            HostEffectId, HostId, HostRequest, HostRequestEnvelope, ModelResponseMode,
        };
        use crate::agent::parts::{AgentPart, PartOwnership, TextPart};
        use crate::agent::recovery::{PendingHostEffect, PendingHostEffectState};

        fn message_body(item: &AgentItem) -> &str {
            let AgentItem::Message { content, .. } = item else {
                panic!("fixture must retain its message body")
            };
            content
        }

        fn projected_body(state: &AgentThreadState) -> &str {
            let Some(AgentPart::Text(part)) = state.conversation.projection.parts.get("text-1")
            else {
                panic!("fixture must retain its Text Part")
            };
            &part.text
        }

        fn request_body(state: &AgentThreadState) -> Option<&str> {
            state
                .host
                .pending_effects
                .get("effect-1")?
                .request
                .as_ref()?
                .get("payload")?
                .get("messages")?
                .get(0)?
                .get("content")?
                .as_str()
        }

        for (status, has_pending_effect) in [
            (AgentTurnStatus::AwaitingModel, false),
            (AgentTurnStatus::Compacting, false),
            (AgentTurnStatus::Completed, false),
            (AgentTurnStatus::Cancelled, false),
            (AgentTurnStatus::Failed, false),
            (AgentTurnStatus::Completed, true),
        ] {
            let body = "原始正文🙂\\path\n".repeat(1_024);
            let mut state = AgentThreadState::new(
                SessionId::from_string("conversation".to_string()),
                configuration(),
            );
            state.configuration.tools.push(serde_json::json!({
                "type": "function",
                "function": {"name": "read_file", "description": "FROZEN_TOOL_CATALOG"}
            }));
            start_turn(&mut state, "turn-1");
            let turn = state.execution.active_turn.as_mut().unwrap();
            turn.status = status;
            turn.context.components.push(ContextComponent {
                kind: ContextComponentKind::Skill,
                id: "discovery-skill".to_string(),
                content: serde_json::json!("FROZEN_SKILL_CATALOG"),
                source: None,
            });
            turn.context.items.push(AgentItem::Message {
                id: ItemId::from_string("frozen-user-context".to_string()),
                role: AgentRole::User,
                content: body.clone(),
            });
            state.record_item(AgentItem::Message {
                id: ItemId::from_string("user-message".to_string()),
                role: AgentRole::User,
                content: body.clone(),
            });
            state
                .conversation
                .projection
                .order
                .push("text-1".to_string());
            state.conversation.projection.parts.insert(
                "text-1".to_string(),
                AgentPart::Text(TextPart {
                    review: None,
                    id: "text-1".to_string(),
                    ownership: PartOwnership::for_turn("turn-1"),
                    text: body.clone(),
                    is_complete: status.is_terminal(),
                }),
            );
            if has_pending_effect {
                let request = HostRequestEnvelope {
                    attempt_started: false,
                    attempt_id: None,
                    delegated_invocation_id: None,
                    request_id: "request-1".to_string(),
                    effect_id: HostEffectId("effect-1".to_string()),
                    host_id: HostId("test-host".to_string()),
                    lease_id: None,
                    session_id: "conversation".to_string(),
                    turn_id: Some("turn-1".to_string()),
                    catalog_revision: 1,
                    availability_epoch: 1,
                    created_at_ms: 1,
                    deadline_ms: None,
                    batch: None,
                    resume_after_sequence: Some(7),
                    delivery: None,
                    payload: HostRequest::ModelStream {
                        model_id: "model".to_string(),
                        text_part_id: "text-1".to_string(),
                        reasoning_part_id: "reasoning-1".to_string(),
                        response_mode: ModelResponseMode::Conversational,
                        messages: vec![
                            serde_json::to_value(&state.conversation.model_context[0]).unwrap()
                        ],
                        tools: state.configuration.tools.clone(),
                        settings: serde_json::json!({}),
                    },
                };
                assert!(state.host.pending_effects.register(PendingHostEffect {
                    control: Default::default(),
                    effect_id: request.effect_id.0.clone(),
                    request_id: request.request_id.clone(),
                    host_id: request.host_id.0.clone(),
                    lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision,
                    availability_epoch: request.availability_epoch,
                    capability_id: "model.stream".to_string(),
                    call_id: None,
                    task_id: None,
                    replay_safety: HostReplaySafety::Recoverable,
                    state: PendingHostEffectState::Streaming,
                    target_effect_id: None,
                    cancel_effect_id: None,
                    last_response_sequence: Some(7),
                    request: Some(serde_json::to_value(request).unwrap()),
                }));
            }

            let original = serde_json::to_vec(&state).unwrap();
            let borrowed = state.checkpoint_projection();
            assert_eq!(serde_json::to_vec(&state).unwrap(), original);
            let expected = serde_json::to_vec(&borrowed).unwrap();
            {
                let view = state.checkpoint_projection_view();
                assert_eq!(serde_json::to_vec(&view).unwrap(), expected);
                assert_eq!(
                    matches!(&view, std::borrow::Cow::Owned(_)),
                    status.is_terminal() && !has_pending_effect
                );
                if let std::borrow::Cow::Borrowed(live) = view {
                    assert!(std::ptr::eq(live, &state));
                }
            }
            let transcript_pointer = message_body(&state.conversation.transcript[0]).as_ptr();
            let model_context_pointer = message_body(&state.conversation.model_context[0]).as_ptr();
            let projected_pointer = projected_body(&state).as_ptr();
            let frozen_context_pointer =
                message_body(&state.execution.active_turn.as_ref().unwrap().context.items[0])
                    .as_ptr();
            let pending_pointer = request_body(&state).map(str::as_ptr);
            assert_eq!(pending_pointer.is_some(), has_pending_effect);

            let owned = state.into_checkpoint_projection();
            assert_eq!(serde_json::to_vec(&owned).unwrap(), expected);
            assert_eq!(
                message_body(&owned.conversation.transcript[0]).as_ptr(),
                transcript_pointer
            );
            assert_eq!(
                message_body(&owned.conversation.model_context[0]).as_ptr(),
                model_context_pointer
            );
            assert_eq!(projected_body(&owned).as_ptr(), projected_pointer);
            let frozen_context = &owned.execution.active_turn.as_ref().unwrap().context.items[0];
            assert_eq!(
                message_body(frozen_context).as_ptr(),
                frozen_context_pointer
            );
            assert_eq!(request_body(&owned).map(str::as_ptr), pending_pointer);
            let retain_catalog = !status.is_terminal() || has_pending_effect;
            assert_eq!(!owned.configuration.tools.is_empty(), retain_catalog);
            assert_eq!(
                !owned
                    .execution
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .context
                    .components
                    .is_empty(),
                retain_catalog
            );
        }
    }

    #[test]
    fn turn_numbers_do_not_inherit_internal_object_id_churn() {
        let mut state = AgentThreadState::new(
            SessionId::from_string("conversation".to_string()),
            configuration(),
        );
        for _ in 0..100 {
            state.allocate_id("model_request");
        }
        assert_eq!(state.allocate_turn_id(), "conversation:turn:1");
        for _ in 0..75 {
            state.allocate_id("tool-result");
        }
        assert_eq!(state.allocate_turn_id(), "conversation:turn:2");
    }

    #[test]
    fn invoked_skill_is_visible_only_for_the_active_turn_by_default() {
        let mut state = AgentThreadState::new(
            SessionId::from_string("conversation".to_string()),
            configuration(),
        );
        start_turn(&mut state, "turn-1");
        state.record_invoked_skill(AgentInvokedSkill {
            skill_id: "extract-image-text".to_string(),
            version: "1.0.0".to_string(),
            instructions_ref: "skill://extract-image-text/1.0.0/SKILL.md#sha256=fixture"
                .to_string(),
            instructions_digest: "fixture".to_string(),
            base_directory: "/skills/extract-image-text".to_string(),
            arguments: serde_json::json!({
                "path": "receipt.png",
                "language": "auto"
            }),
            instructions: "Run extract_text.swift and validate the artifact.".to_string(),
            required_tools: vec!["read_file".to_string(), "bash".to_string()],
            validation_steps: vec![
                "extract_text.swift --validate-only --output receipt.txt".to_string()
            ],
            activation: AgentSkillActivation::Turn,
        });
        let items = state.invoked_skill_context_items();
        assert_eq!(items.len(), 1);
        let AgentItem::Message { content, .. } = &items[0] else {
            panic!("invoked Skill must be model-visible system context")
        };
        assert!(content.contains("extract-image-text"));
        assert!(content.contains("workspace files"));
        assert!(content.contains("active workspace"));
        assert!(content.contains("extract_text.swift"));
        assert!(content.contains(r#""path":"receipt.png""#));
        assert!(content.contains("Required final validation steps"));
        assert_eq!(
            state.invoked_skill_validation_steps(),
            vec!["extract_text.swift --validate-only --output receipt.txt"]
        );
        assert!(content.contains("Do not invoke it again"));
        state.execution.active_turn = None;
        start_turn(&mut state, "turn-2");
        assert!(state.invoked_skill_context_items().is_empty());
    }

    #[test]
    fn explicitly_pinned_skill_survives_turn_boundaries() {
        let mut state = AgentThreadState::new(
            SessionId::from_string("conversation".to_string()),
            configuration(),
        );
        start_turn(&mut state, "turn-1");
        state.record_invoked_skill(AgentInvokedSkill {
            skill_id: "house-style".to_string(),
            version: "1.0.0".to_string(),
            instructions_ref: "skill://house-style/1.0.0/SKILL.md#sha256=fixture".to_string(),
            instructions_digest: "fixture".to_string(),
            base_directory: "/skills/house-style".to_string(),
            arguments: serde_json::Value::String(String::new()),
            instructions: "Use the approved house style.".to_string(),
            required_tools: Vec::new(),
            validation_steps: Vec::new(),
            activation: AgentSkillActivation::Session,
        });
        state.execution.active_turn = None;
        start_turn(&mut state, "turn-2");
        let items = state.invoked_skill_context_items();
        assert_eq!(items.len(), 1);
        let AgentItem::Message { content, .. } = &items[0] else {
            panic!("pinned Skill must remain model-visible")
        };
        assert!(content.contains("conversation"));
    }

    #[test]
    fn checkpoint_omits_unselected_skill_and_sop_discovery_content() {
        let mut state = AgentThreadState::new(
            SessionId::from_string("conversation".to_string()),
            configuration(),
        );
        start_turn(&mut state, "turn-1");
        let turn = state.execution.active_turn.as_mut().unwrap();
        turn.context.components.extend([
            ContextComponent {
                kind: ContextComponentKind::Skill,
                id: "skill-catalog".to_string(),
                content: serde_json::json!("UNSELECTED_SKILL_SENTINEL"),
                source: None,
            },
            ContextComponent {
                kind: ContextComponentKind::System,
                id: "nola-available-sops-v1".to_string(),
                content: serde_json::json!("UNSTARTED_SOP_SENTINEL"),
                source: None,
            },
            ContextComponent {
                kind: ContextComponentKind::System,
                id: "nola-active-sop-runs-v2".to_string(),
                content: serde_json::json!("STARTED_SOP_EVIDENCE"),
                source: None,
            },
            ContextComponent {
                kind: ContextComponentKind::Skill,
                id: "selected-skill-v2.1.0".to_string(),
                content: serde_json::json!("SELECTED_SKILL_COMPONENT"),
                source: None,
            },
        ]);
        turn.context.items.extend([
            AgentItem::Message {
                id: ItemId::from_string("nola-context-component-skill-skill-catalog".to_string()),
                role: AgentRole::System,
                content: "UNSELECTED_SKILL_SENTINEL".to_string(),
            },
            AgentItem::Message {
                id: ItemId::from_string(
                    "nola-context-component-system-nola-available-sops-v1".to_string(),
                ),
                role: AgentRole::System,
                content: "UNSTARTED_SOP_SENTINEL".to_string(),
            },
            AgentItem::Message {
                id: ItemId::from_string(
                    "nola-context-component-system-nola-active-sop-runs-v2".to_string(),
                ),
                role: AgentRole::System,
                content: "STARTED_SOP_EVIDENCE".to_string(),
            },
            AgentItem::Message {
                id: ItemId::from_string(
                    "nola-context-component-skill-selected-skill-v2.1.0".to_string(),
                ),
                role: AgentRole::System,
                content: "SELECTED_SKILL_COMPONENT".to_string(),
            },
        ]);
        state.record_invoked_skill(AgentInvokedSkill {
            skill_id: "selected-skill".to_string(),
            version: "2.1.0".to_string(),
            instructions_ref: "skill://selected-skill/2.1.0#sha256=selected".to_string(),
            instructions_digest: "selected".to_string(),
            base_directory: "/skills/selected-skill".to_string(),
            arguments: serde_json::json!({"input":"ACTUAL_SKILL_ARGUMENT"}),
            instructions: "SELECTED_SKILL_INSTRUCTIONS".to_string(),
            required_tools: vec!["read_file".to_string()],
            validation_steps: vec!["verify selected result".to_string()],
            activation: AgentSkillActivation::Turn,
        });

        let active = serde_json::to_string(&state.checkpoint_projection()).unwrap();
        assert!(active.contains("UNSELECTED_SKILL_SENTINEL"));
        assert!(active.contains("UNSTARTED_SOP_SENTINEL"));
        let restored: AgentThreadState = serde_json::from_str(&active).unwrap();
        let restored_skill_context = restored.invoked_skill_context_items();
        assert_eq!(restored_skill_context.len(), 1);
        let AgentItem::Message { content, .. } = &restored_skill_context[0] else {
            panic!("selected Skill must remain usable after checkpoint recovery")
        };
        assert!(content.contains("selected-skill"));
        assert!(content.contains("2.1.0"));
        assert!(content.contains("SELECTED_SKILL_INSTRUCTIONS"));
        assert!(content.contains("ACTUAL_SKILL_ARGUMENT"));

        state.execution.active_turn.as_mut().unwrap().status = AgentTurnStatus::Completed;
        let terminal = serde_json::to_string(&state.checkpoint_projection()).unwrap();
        assert!(!terminal.contains("UNSELECTED_SKILL_SENTINEL"));
        assert!(!terminal.contains("UNSTARTED_SOP_SENTINEL"));
        assert!(terminal.contains("STARTED_SOP_EVIDENCE"));
        assert!(!terminal.contains("SELECTED_SKILL_COMPONENT"));
        assert!(terminal.contains("selected-skill"));
        assert!(terminal.contains("2.1.0"));
        assert!(terminal.contains("ACTUAL_SKILL_ARGUMENT"));
    }

    #[test]
    fn terminal_checkpoint_keeps_calls_but_discards_the_entire_tool_catalog() {
        let used = serde_json::json!({
            "type":"function",
            "function":{"name":"read_file","description":"USED_TOOL_SCHEMA"}
        });
        let unused = serde_json::json!({
            "type":"function",
            "function":{"name":"web_search","description":"UNUSED_TOOL_SCHEMA"}
        });
        let mut state = AgentThreadState::new(
            SessionId::from_string("conversation".to_string()),
            configuration(),
        );
        state.configuration.tools = vec![used.clone(), unused.clone()];
        state.skill_runtime.catalog.revision = 7;
        state
            .skill_runtime
            .catalog
            .builtin_tools
            .extend(["read_file".to_string(), "UNUSED_SKILL_TOOL".to_string()]);
        state.skill_runtime.accepted_operation_ids.extend([
            "host_catalog:test-host:7:1".to_string(),
            "skill-run-start:kept-for-idempotency".to_string(),
        ]);
        start_turn(&mut state, "turn-1");
        state.host.catalog.capabilities.extend([
            (
                "model.stream".to_string(),
                host_capability("model.stream", HostCapabilityRoute::Model),
            ),
            (
                "read_file".to_string(),
                host_capability("read_file", HostCapabilityRoute::Tool),
            ),
            (
                "web_search".to_string(),
                host_capability("web_search", HostCapabilityRoute::Tool),
            ),
        ]);
        state
            .execution
            .active_turn
            .as_mut()
            .unwrap()
            .context
            .configuration
            .tools = vec![used, unused];
        {
            let turn = state.execution.active_turn.as_mut().unwrap();
            turn.context.capability_projection.available_capabilities =
                state.host.catalog.capabilities.clone();
            turn.context.runtime_snapshot.enabled_capability_ids =
                vec!["read_file".to_string(), "web_search".to_string()];
            turn.activated_capabilities.extend([
                ("read_file".to_string(), "skill:selected".to_string()),
                ("web_search".to_string(), "tool_search".to_string()),
            ]);
        }
        state.record_item(AgentItem::ToolCall {
            id: ItemId::from_string("tool-call".to_string()),
            call_id: CallId::from_string("call-1".to_string()),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path":"README.md"}),
        });

        let active = serde_json::to_string(&state.checkpoint_projection()).unwrap();
        assert!(active.contains("UNUSED_TOOL_SCHEMA"));
        assert!(active.contains("UNUSED_SKILL_TOOL"));

        state.execution.active_turn.as_mut().unwrap().status = AgentTurnStatus::Completed;
        let terminal = serde_json::to_string(&state.checkpoint_projection()).unwrap();
        assert!(!terminal.contains("USED_TOOL_SCHEMA"));
        assert!(!terminal.contains("UNUSED_TOOL_SCHEMA"));
        assert!(!terminal.contains("UNUSED_SKILL_TOOL"));
        assert!(terminal.contains("read_file"));
        let restored: AgentThreadState = serde_json::from_str(&terminal).unwrap();
        assert!(restored
            .host
            .catalog
            .capabilities
            .contains_key("model.stream"));
        assert!(!restored.host.catalog.capabilities.contains_key("read_file"));
        assert!(!restored
            .host
            .catalog
            .capabilities
            .contains_key("web_search"));
        assert_eq!(restored.skill_runtime.catalog.revision, 0);
        assert!(restored.skill_runtime.catalog.builtin_tools.is_empty());
        assert!(!restored
            .skill_runtime
            .accepted_operation_ids
            .contains("host_catalog:test-host:7:1"));
        assert!(restored
            .skill_runtime
            .accepted_operation_ids
            .contains("skill-run-start:kept-for-idempotency"));
        let restored_turn = restored.execution.active_turn.unwrap();
        assert!(restored_turn
            .context
            .capability_projection
            .available_capabilities
            .contains_key("model.stream"));
        assert!(restored_turn
            .context
            .runtime_snapshot
            .enabled_capability_ids
            .is_empty());
        assert!(restored_turn.activated_capabilities.is_empty());
        assert!(restored
            .conversation
            .transcript
            .iter()
            .any(|item| matches!(item, AgentItem::ToolCall { name, .. } if name == "read_file")));
    }

    #[test]
    fn cancellation_and_failure_redact_catalogs_while_compaction_keeps_recovery_state() {
        fn state_with_catalog(status: AgentTurnStatus) -> AgentThreadState {
            let mut state = AgentThreadState::new(
                SessionId::from_string("terminal-redaction".to_string()),
                configuration(),
            );
            let tool = serde_json::json!({
                "type":"function",
                "function":{"name":"read_file","description":"OPTIONAL_TOOL_SENTINEL"}
            });
            state.configuration.tools.push(tool.clone());
            start_turn(&mut state, "turn-1");
            let turn = state.execution.active_turn.as_mut().unwrap();
            turn.status = status;
            turn.context.configuration.tools.push(tool);
            turn.context.components.push(ContextComponent {
                kind: ContextComponentKind::Skill,
                id: "future-catalog-id".to_string(),
                content: serde_json::json!("OPTIONAL_SKILL_SENTINEL"),
                source: None,
            });
            state
        }

        for status in [AgentTurnStatus::Cancelled, AgentTurnStatus::Failed] {
            let checkpoint =
                serde_json::to_string(&state_with_catalog(status).checkpoint_projection()).unwrap();
            assert!(!checkpoint.contains("OPTIONAL_TOOL_SENTINEL"), "{status:?}");
            assert!(
                !checkpoint.contains("OPTIONAL_SKILL_SENTINEL"),
                "{status:?}"
            );
        }
        let compacting = serde_json::to_string(
            &state_with_catalog(AgentTurnStatus::Compacting).checkpoint_projection(),
        )
        .unwrap();
        assert!(compacting.contains("OPTIONAL_TOOL_SENTINEL"));
        assert!(compacting.contains("OPTIONAL_SKILL_SENTINEL"));
    }

    #[test]
    fn completed_turn_preserves_already_sent_context_and_tool_pairs() {
        let mut state = AgentThreadState::new(
            SessionId::from_string("conversation".to_string()),
            configuration(),
        );
        let turn_id = TurnId::from_string("turn-1".to_string());
        start_turn(&mut state, turn_id.as_str());
        state.record_items([
            AgentItem::Message {
                id: ItemId::from_string("user".to_string()),
                role: AgentRole::User,
                content: "Find current competitions".to_string(),
            },
            AgentItem::Reasoning {
                id: ItemId::from_string("reasoning".to_string()),
                summary: "Search several sources".to_string(),
                content: "Transient reasoning".to_string(),
                provider_continuation: None,
            },
            AgentItem::ToolCall {
                id: ItemId::from_string("call".to_string()),
                call_id: CallId::from_string("call-1".to_string()),
                name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "/workspace"}),
            },
            AgentItem::ToolResult {
                id: ItemId::from_string("result".to_string()),
                call_id: CallId::from_string("call-1".to_string()),
                name: "read_file".to_string(),
                output: serde_json::json!({"error": "Not a file"}),
                is_error: true,
                full_output_ref: None,
                artifacts: Vec::new(),
            },
            AgentItem::Message {
                id: ItemId::from_string("draft".to_string()),
                role: AgentRole::Assistant,
                content: "I am still checking.".to_string(),
            },
            AgentItem::Message {
                id: ItemId::from_string("final".to_string()),
                role: AgentRole::Assistant,
                content: "No eligible competitions were verified.".to_string(),
            },
        ]);

        let prefix = state.conversation.model_context.clone();
        state.seal_completed_turn_model_context(&turn_id);
        assert_eq!(state.conversation.model_context, prefix);

        assert_eq!(state.conversation.transcript.len(), 6);
        assert_eq!(state.conversation.model_context.len(), 6);
        assert!(matches!(
            &state.conversation.model_context[0],
            AgentItem::Message {
                role: AgentRole::User,
                ..
            }
        ));
        assert!(matches!(
            &state.conversation.model_context[5],
            AgentItem::Message { id, role: AgentRole::Assistant, content }
                if id.as_str() == "final" && content == "No eligible competitions were verified."
        ));
    }

    #[test]
    fn completed_turn_retains_the_compaction_checkpoint_created_during_that_turn() {
        let mut state = AgentThreadState::new(
            SessionId::from_string("conversation".to_string()),
            configuration(),
        );
        let turn_id = TurnId::from_string("turn-1".to_string());
        start_turn(&mut state, turn_id.as_str());
        state.record_items([
            AgentItem::Message {
                id: ItemId::from_string("user".to_string()),
                role: AgentRole::User,
                content: "Continue the task".to_string(),
            },
            AgentItem::Compaction {
                id: ItemId::from_string("compaction".to_string()),
                summary: "Critical fact: ALPHA-31415".to_string(),
                replaced_item_count: 8,
                source_item_ids: vec![],
                source_fingerprint: "fixture".to_string(),
                original_token_count: 8,
            },
            AgentItem::Reasoning {
                id: ItemId::from_string("reasoning".to_string()),
                summary: "Transient".to_string(),
                content: "Transient".to_string(),
                provider_continuation: None,
            },
            AgentItem::Message {
                id: ItemId::from_string("final".to_string()),
                role: AgentRole::Assistant,
                content: "Done".to_string(),
            },
        ]);

        state.seal_completed_turn_model_context(&turn_id);

        assert_eq!(state.conversation.model_context.len(), 4);
        assert!(state.conversation.model_context.iter().any(|item| matches!(
            item,
            AgentItem::Compaction { summary, .. }
                if summary.contains("ALPHA-31415")
        )));
        assert!(state
            .conversation
            .model_context
            .iter()
            .any(|item| matches!(item, AgentItem::Reasoning { .. })));
    }
}
