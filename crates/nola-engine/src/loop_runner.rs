// SPDX-License-Identifier: Apache-2.0
//! Core Agent Loop Runner:
//! - Immutable `frozen_system_prefix` built ONCE on Turn 0
//! - Prefix-stable tool catalog with tail-only deferred tool activation
//! - Parallel read-only tool batches (up to 12) + sequential mutating tools
//! - Automatic `nola-store` session + event + `--resume` policy persistence
//! - Clean `OutputLimit` continuation + cancellation + `--json-schema` validation.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nola_llm::{LlmClient, LlmError};
use nola_store::{EventPage, Store, StoreError, DEFAULT_MAX_EVENTS_PER_SCOPE};
use nola_tools::{ToolContext, ToolRegistry};
use nola_types::{
    estimate_request_tokens, new_id, now_ms, sha256_hex, ContextBudget, FrozenSystemBlock,
    HistoryMessage, HookDecision, HookEvent, HostCapabilities, MediaAttachment, MemoryItem,
    ModelRouteConfig, ModelStopReason, ModelStreamDelta, NormalizedModelResponse, ResourceEffect,
    RuleScope, RuntimeReminder, Session, StructuredModelRequest, TokenUsage, ToolExecutionResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::catalog::{ForceToolSet, ToolCatalog};
use crate::compaction::{compact_history, compact_history_force};
use crate::hooks::HookRunner;
use crate::loop_guard::LoopGuard;
use crate::middleware::{ApprovalCallback, MiddlewareOutcome, ToolExecutionMiddleware};
use crate::policy::EffectivePolicy;

const MAX_PARALLEL_READ_TOOLS: usize = 12;
const MAX_CONSECUTIVE_OUTPUT_LIMITS: usize = 5;
const MAX_CONSECUTIVE_EMPTY_RESPONSES: usize = 3;
const MAX_ACTIVE_TOOL_MEDIA_ATTACHMENTS: usize = 3;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("llm error: {0}")]
    Llm(#[from] LlmError),
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("schema validation error: {0}")]
    SchemaValidation(String),
    #[error("cancelled")]
    Cancelled,
}

/// Trait abstracting LLM execution (production uses `HttpModelCaller`; tests inject mocks).
pub trait ModelCaller: Send + Sync {
    fn call(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError>;
}

/// Production `ModelCaller` backed by `nola-llm::LlmClient`.
pub struct HttpModelCaller {
    client: LlmClient,
}

impl HttpModelCaller {
    pub fn new(timeout_secs: u64) -> Result<Self, LlmError> {
        Ok(Self {
            client: LlmClient::new(timeout_secs)?,
        })
    }
}

impl ModelCaller for HttpModelCaller {
    fn call(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        self.client.complete(route, req, stream, on_delta)
    }
}

/// Live NDJSON-serializable event emitted during an agent turn (for CLI `--json`, TUI, Web SSE).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    SessionStarted {
        session_id: String,
        model: String,
    },
    TextDelta {
        delta: String,
    },
    ReasoningDelta {
        delta: String,
    },
    ToolStarted {
        call_id: String,
        name: String,
        arguments: Value,
    },
    ToolFinished {
        call_id: String,
        name: String,
        is_error: bool,
        summary: String,
        effects: Vec<ResourceEffect>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        full_output_ref: Option<String>,
    },
    Compacted {
        tokens_before: usize,
        tokens_after: usize,
        micro_tools: usize,
        summary_messages: usize,
    },
    TurnCompleted {
        session_id: String,
        steps: usize,
        usage: TokenUsage,
        final_text: String,
    },
}

/// Configuration for a single `AgentEngine::run` execution.
pub struct AgentRunConfig {
    pub session_id: Option<String>,
    pub resume: bool,
    pub route: ModelRouteConfig,
    pub frozen_system_blocks: Vec<FrozenSystemBlock>,
    pub host_capabilities: HostCapabilities,
    pub policy: EffectivePolicy,
    pub hooks: HookRunner,
    pub approval: Option<ApprovalCallback>,
    /// 0 means unlimited iterations (supports Harbor/Terminal-Bench unlimited-step runs!).
    pub max_steps: usize,
    pub force_tool_set: ForceToolSet,
    pub stream: bool,
    pub json_schema: Option<Value>,
    pub cancel: Arc<AtomicBool>,
    pub auto_recall_memory: bool,
    pub inbox: Option<Arc<Mutex<Vec<String>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunResult {
    pub session_id: String,
    pub final_text: String,
    pub steps: usize,
    pub usage: TokenUsage,
    pub effects: Vec<ResourceEffect>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSessionState {
    history: Vec<HistoryMessage>,
    policy: EffectivePolicy,
    activated_tools: Vec<String>,
    #[serde(default)]
    usage: TokenUsage,
}

/// Deterministic workspace-scoped memory namespace (`ws:<16-hex-sha256>`).
pub fn workspace_memory_namespace(workspace_root: &Path) -> String {
    let digest = sha256_hex(workspace_root.to_string_lossy().as_bytes());
    format!("ws:{}", &digest[..16])
}

/// Load cumulative persisted `TokenUsage` for a conversation/session.
pub fn load_session_usage(store: &Store, session_id: &str) -> TokenUsage {
    let state_key = format!("session_state:{session_id}");
    store
        .get_kv(&state_key)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<PersistedSessionState>(v).ok())
        .map(|s| s.usage)
        .unwrap_or_default()
}

/// Immediately compact `session_state:{session_id}` on disk (using an LLM summarizer when
/// `llm` & `route` are provided, falling back to deterministic ledger compaction otherwise),
/// persist the compacted state, and append a `compaction_marker` event.
pub fn compact_session_now(
    store: &Store,
    llm: Option<&LlmClient>,
    route: Option<&ModelRouteConfig>,
    session_id: &str,
) -> Result<Value, String> {
    let state_key = format!("session_state:{session_id}");
    let Some(raw_state) = store.get_kv(&state_key).map_err(|e| e.to_string())? else {
        let summary = "No persisted messages to compact.".to_string();
        let _ = store.append_event(
            session_id,
            "compaction_marker",
            json!({
                "conversation_id": session_id,
                "before_messages": 0,
                "after_messages": 0,
                "summary": &summary,
            }),
            DEFAULT_MAX_EVENTS_PER_SCOPE,
        );
        return Ok(json!({
            "ok": true,
            "conversation_id": session_id,
            "before_messages": 0,
            "after_messages": 0,
            "summary": summary,
        }));
    };

    let mut state: PersistedSessionState = serde_json::from_value(raw_state)
        .map_err(|e| format!("invalid persisted session state for '{session_id}': {e}"))?;
    let before_messages = state.history.len();
    let budget = match route {
        Some(r) => ContextBudget::for_window(r.context_window, r.capabilities.reasoning)
            .with_explicit_output_limit(r.max_output_tokens),
        None => ContextBudget::for_window(128_000, false),
    };

    let report = if let (Some(client), Some(r)) = (llm, route) {
        let summarizer = |slice: &[HistoryMessage]| -> Option<String> {
            summarize_history_with_llm_client(client, r, slice)
        };
        compact_history_force(&mut state.history, budget, 0, Some(&summarizer))
    } else {
        compact_history_force(&mut state.history, budget, 0, None)
    };

    let after_messages = state.history.len();
    let summary = extract_compacted_summary(&state.history).unwrap_or_else(|| {
        format!(
            "Compacted {before_messages} message(s) into {after_messages} message(s) (saved {} tokens).",
            report.saved_tokens()
        )
    });

    let persisted = serde_json::to_value(&state)
        .map_err(|e| format!("failed to serialize compacted session state: {e}"))?;
    store
        .set_kv(&state_key, &persisted)
        .map_err(|e| e.to_string())?;
    let _ = store.append_event(
        session_id,
        "compaction_marker",
        json!({
            "conversation_id": session_id,
            "before_messages": before_messages,
            "after_messages": after_messages,
            "tokens_before": report.tokens_before,
            "tokens_after": report.tokens_after,
            "summary": &summary,
        }),
        DEFAULT_MAX_EVENTS_PER_SCOPE,
    );

    Ok(json!({
        "ok": true,
        "conversation_id": session_id,
        "before_messages": before_messages,
        "after_messages": after_messages,
        "tokens_before": report.tokens_before,
        "tokens_after": report.tokens_after,
        "summary": summary,
    }))
}

pub struct AgentEngine {
    store: Store,
    registry: Arc<ToolRegistry>,
    caller: Arc<dyn ModelCaller>,
}

impl AgentEngine {
    pub fn new(store: Store, registry: Arc<ToolRegistry>, caller: Arc<dyn ModelCaller>) -> Self {
        Self {
            store,
            registry,
            caller,
        }
    }

    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn run(
        &self,
        ctx: &ToolContext,
        user_prompt: &str,
        cfg: AgentRunConfig,
        on_event: impl FnMut(AgentEvent),
    ) -> Result<AgentRunResult, AgentError> {
        self.run_with_media(ctx, user_prompt, Vec::new(), cfg, on_event)
    }

    pub fn run_with_media(
        &self,
        ctx: &ToolContext,
        user_prompt: &str,
        user_media: Vec<MediaAttachment>,
        cfg: AgentRunConfig,
        mut on_event: impl FnMut(AgentEvent),
    ) -> Result<AgentRunResult, AgentError> {
        let session_id = cfg.session_id.clone().unwrap_or_else(new_id);
        let run_ctx = ctx.clone().with_session_id(session_id.clone());
        let now = now_ms();

        // Restore history + EffectivePolicy + activated tools + cumulative usage when resuming!
        // Preserve stricter User/CliOverride rules and network restrictions from the new invocation.
        let state_key = format!("session_state:{session_id}");
        let mut active_policy = cfg.policy.clone();
        let mut catalog = ToolCatalog::build_initial(
            &self.registry,
            cfg.host_capabilities,
            cfg.force_tool_set,
        );
        let mut session_usage = TokenUsage::default();
        let mut history: Vec<HistoryMessage> = if cfg.resume {
            if let Some(val) = self.store.get_kv(&state_key)? {
                if let Ok(saved) = serde_json::from_value::<PersistedSessionState>(val) {
                    let mut merged_policy = saved.policy;
                    // Never silently escalate on resume, while always honoring stricter overrides
                    // (e.g. tightening a session from AcceptEdits -> Cautious).
                    let profile_rank = |p: nola_types::PermissionProfile| match p {
                        nola_types::PermissionProfile::Cautious => 0u8,
                        nola_types::PermissionProfile::AcceptEdits => 1u8,
                        nola_types::PermissionProfile::AllowAll => 2u8,
                    };
                    if profile_rank(cfg.policy.profile) < profile_rank(merged_policy.profile) {
                        merged_policy.profile = cfg.policy.profile;
                    }
                    merged_policy.network_allowed =
                        merged_policy.network_allowed && cfg.policy.network_allowed;
                    for r in &cfg.policy.rules {
                        if matches!(r.scope, RuleScope::User | RuleScope::CliOverride)
                            && !merged_policy.rules.contains(r)
                        {
                            merged_policy.rules.push(r.clone());
                        }
                    }
                    active_policy = merged_policy;
                    catalog.restore_activated(&saved.activated_tools, &self.registry);
                    session_usage = saved.usage;
                    saved.history
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let existing_created = self
            .store
            .get_session(&session_id)?
            .map(|s| s.created_at_ms)
            .unwrap_or(now);
        let title: String = user_prompt
            .lines()
            .next()
            .unwrap_or("New Session")
            .chars()
            .take(80)
            .collect();
        self.store.upsert_session(&Session {
            id: session_id.clone(),
            title,
            workspace_root: run_ctx.workspace_root.display().to_string(),
            model_connection: Some(cfg.route.connection_id.clone()),
            model_id: Some(cfg.route.model.clone()),
            created_at_ms: existing_created,
            updated_at_ms: now,
        })?;

        self.emit_and_persist(
            &session_id,
            AgentEvent::SessionStarted {
                session_id: session_id.clone(),
                model: cfg.route.model.clone(),
            },
            &mut on_event,
        );

        // Append new user message (preserving any multimodal image/media attachments)
        history.push(HistoryMessage::User {
            content: user_prompt.to_string(),
            media: user_media,
        });

        let middleware = ToolExecutionMiddleware {
            policy: active_policy.clone(),
            hooks: cfg.hooks,
            approval: cfg.approval,
        };
        let budget = ContextBudget::for_window(
            cfg.route.context_window,
            cfg.route.capabilities.reasoning,
        )
        .with_explicit_output_limit(cfg.route.max_output_tokens);

        let mut loop_guard = LoopGuard::default();
        let mut pending_reminders: Vec<RuntimeReminder> = Vec::new();
        // Run SessionStart lifecycle hooks
        let start_payload = json!({
            "session_id": session_id,
            "model": cfg.route.model,
            "resume": cfg.resume,
        });
        for dec in middleware
            .hooks
            .run_event(HookEvent::SessionStart, "session", &start_payload)
        {
            match dec {
                HookDecision::InjectContext { text } => {
                    pending_reminders.push(RuntimeReminder {
                        tag: "session_start_hook".into(),
                        text,
                    });
                }
                HookDecision::Block { reason } => {
                    return Err(AgentError::SchemaValidation(format!(
                        "SessionStart hook blocked run: {reason}"
                    )));
                }
                HookDecision::Allow | HookDecision::ModifyInput { .. } => {}
            }
        }
        // Automatic FTS5 memory recall injected ONLY as an ephemeral tail reminder (0 KV prefix drift!)
        // Strictly scoped to workspace/session/default namespaces — NEVER recalls `code_memory` or
        // items belonging to other workspaces.
        if cfg.auto_recall_memory {
            let recall_query = extract_recall_keywords(user_prompt);
            if !recall_query.is_empty() {
                let recalled = recall_scoped_memories(
                    &self.store,
                    &recall_query,
                    &run_ctx.workspace_root,
                    &session_id,
                    5,
                );
                if !recalled.is_empty() {
                    let body = recalled
                        .iter()
                        .map(|m| format!("- [{}] {}: {}", m.namespace, m.title, m.content))
                        .collect::<Vec<_>>()
                        .join("\n");
                    pending_reminders.push(RuntimeReminder {
                        tag: "recalled_memory".into(),
                        text: format!("Relevant persisted memories:\n{body}"),
                    });
                }
            }
        }
        let mut total_usage = TokenUsage::default();
        let mut all_effects: Vec<ResourceEffect> = Vec::new();
        let mut final_text = String::new();
        let mut steps = 0usize;
        let mut consecutive_output_limits = 0usize;
        let mut consecutive_empty_responses = 0usize;
        let mut cancelled = false;
        // Real server-token calibration ratio (actual_input / estimated_input), clamped to [1.0, 2.5].
        let mut token_calibration: f64 = 1.0;
        // Optional execution deadline (e.g. NOLA_EXECUTION_DEADLINE_UNIX_MS from supervisor) for
        // one-shot ephemeral tail wrap-up reminders (0 KV prefix drift!).
        let execution_deadline_ms: Option<i64> = std::env::var("NOLA_EXECUTION_DEADLINE_UNIX_MS")
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .filter(|&v| v > 0);
        let mut deadline_warn_180_sent = false;
        let mut deadline_warn_60_sent = false;

        loop {
            if cfg.cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            // Inject at most two one-shot ephemeral wrap-up reminders as execution deadline nears.
            if let Some(deadline_ms) = execution_deadline_ms {
                let remaining_secs = (deadline_ms.saturating_sub(now_ms())) / 1_000;
                if remaining_secs <= 60 && !deadline_warn_60_sent {
                    deadline_warn_60_sent = true;
                    deadline_warn_180_sent = true;
                    pending_reminders.push(RuntimeReminder {
                        tag: "time_budget_final".into(),
                        text: format!(
                            "Time budget warning: ~{}s remaining before hard timeout. Immediately write/save your best working deliverable(s), run a quick verification check, and conclude.",
                            remaining_secs.max(0)
                        ),
                    });
                } else if remaining_secs <= 180 && !deadline_warn_180_sent {
                    deadline_warn_180_sent = true;
                    pending_reminders.push(RuntimeReminder {
                        tag: "time_budget".into(),
                        text: format!(
                            "Time budget notice: ~{remaining_secs}s remaining. Stop broad exploration, converge on your solution, ensure all required output files are written to disk, and verify them."
                        ),
                    });
                }
            }
            // Drain any asynchronous messages delivered to this agent's inbox (e.g. from parent task_control).
            if let Some(inbox) = &cfg.inbox {
                if let Ok(mut q) = inbox.lock() {
                    for msg in q.drain(..) {
                        if !msg.trim().is_empty() {
                            history.push(HistoryMessage::User {
                                content: msg,
                                media: Vec::new(),
                            });
                        }
                    }
                }
            }
            if cfg.max_steps > 0 && steps >= cfg.max_steps {
                break;
            }
            steps += 1;

            // Prefix-stable tools & calibrated overhead estimation
            let tools = catalog.compile_preserving_prefix_order(&self.registry);
            let probe_req = StructuredModelRequest {
                frozen_system_prefix: cfg.frozen_system_blocks.clone(),
                prefix_stable_tools: tools.clone(),
                history: Vec::new(),
                ephemeral_tail_reminders: pending_reminders.clone(),
                max_output_tokens: budget.reserved_output_tokens,
                json_schema: cfg.json_schema.clone(),
            };
            let raw_overhead = estimate_request_tokens(&probe_req);
            // Scale overhead upward by calibration factor when provider tokenizer counts higher
            let calibrated_budget = ContextBudget {
                context_window_tokens: ((budget.context_window_tokens as f64) / token_calibration)
                    .round()
                    .max(4_096.0) as usize,
                ..budget
            };

            // Real LLM-assisted Tier-2 summarizer closure with automatic deterministic fallback
            // if the summarization call fails or returns empty content.
            let llm_summarizer = |slice: &[HistoryMessage]| -> Option<String> {
                summarize_history_with_caller(self.caller.as_ref(), &cfg.route, slice)
            };

            // Three-tier compaction (Tier-1 stale-read/reasoning/micro -> Tier-2 LLM/ledger -> Tier-3 hard limit!)
            let comp = compact_history(
                &mut history,
                calibrated_budget,
                raw_overhead,
                Some(&llm_summarizer),
            );
            if comp.saved_tokens() > 0 {
                self.emit_and_persist(
                    &session_id,
                    AgentEvent::Compacted {
                        tokens_before: comp.tokens_before,
                        tokens_after: comp.tokens_after,
                        micro_tools: comp.micro_compacted_tools,
                        summary_messages: comp.summary_compacted_messages,
                    },
                    &mut on_event,
                );
            }

            let step_reminders = std::mem::take(&mut pending_reminders);
            let mut request = StructuredModelRequest {
                frozen_system_prefix: cfg.frozen_system_blocks.clone(),
                prefix_stable_tools: tools.clone(),
                history: history.clone(),
                ephemeral_tail_reminders: step_reminders.clone(),
                max_output_tokens: budget.reserved_output_tokens,
                json_schema: cfg.json_schema.clone(),
            };
            let estimated_req_tokens = estimate_request_tokens(&request);

            let resp = match self.caller.call(&cfg.route, &request, cfg.stream, &mut |delta| {
                match delta {
                    ModelStreamDelta::Text(t) => on_event(AgentEvent::TextDelta { delta: t }),
                    ModelStreamDelta::Reasoning(r) => {
                        on_event(AgentEvent::ReasoningDelta { delta: r })
                    }
                    ModelStreamDelta::ToolCallStart { .. }
                    | ModelStreamDelta::ToolCallArgsDelta { .. } => {}
                }
            }) {
                Ok(r) => r,
                Err(err) if is_context_overflow_error(&err) => {
                    // Reactive context-overflow recovery: force-compact history (with LLM summarizer
                    // + deterministic fallback) and retry once cleanly!
                    let forced = compact_history_force(
                        &mut history,
                        calibrated_budget,
                        raw_overhead,
                        Some(&llm_summarizer),
                    );
                    if forced.saved_tokens() == 0 {
                        return Err(AgentError::Llm(err));
                    }
                    self.emit_and_persist(
                        &session_id,
                        AgentEvent::Compacted {
                            tokens_before: forced.tokens_before,
                            tokens_after: forced.tokens_after,
                            micro_tools: forced.micro_compacted_tools,
                            summary_messages: forced.summary_compacted_messages,
                        },
                        &mut on_event,
                    );
                    token_calibration = (token_calibration * 1.25).clamp(1.0, 2.5);
                    request.history = history.clone();
                    self.caller.call(&cfg.route, &request, cfg.stream, &mut |delta| {
                        match delta {
                            ModelStreamDelta::Text(t) => {
                                on_event(AgentEvent::TextDelta { delta: t })
                            }
                            ModelStreamDelta::Reasoning(r) => {
                                on_event(AgentEvent::ReasoningDelta { delta: r })
                            }
                            ModelStreamDelta::ToolCallStart { .. }
                            | ModelStreamDelta::ToolCallArgsDelta { .. } => {}
                        }
                    })?
                }
                Err(err) => return Err(AgentError::Llm(err)),
            };

            // Calibrate local estimator using real server-reported input_tokens
            if resp.usage.input_tokens > 0 && estimated_req_tokens > 0 {
                let ratio = (resp.usage.input_tokens as f64) / (estimated_req_tokens as f64);
                let clamped = ratio.clamp(1.0, 2.5);
                token_calibration = (token_calibration * 0.6 + clamped * 0.4).clamp(1.0, 2.5);
            }

            accumulate_usage(&mut total_usage, &resp.usage);
            accumulate_usage(&mut session_usage, &resp.usage);

            // Record assistant turn verbatim (including provider continuation!)
            history.push(HistoryMessage::Assistant {
                content: resp.content.clone(),
                reasoning: resp.reasoning.clone(),
                tool_calls: resp.tool_calls.clone(),
                continuation: resp.continuation.clone(),
            });

            // Handle OutputLimit cleanly (truncated tool calls were already atomically discarded!)
            if resp.stop_reason == ModelStopReason::OutputLimit {
                consecutive_output_limits += 1;
                if consecutive_output_limits >= MAX_CONSECUTIVE_OUTPUT_LIMITS {
                    final_text = resp.content;
                    self.persist_state(
                        &state_key,
                        &history,
                        &active_policy,
                        catalog.active_names(),
                        &session_usage,
                    )?;
                    break;
                }
                pending_reminders.push(RuntimeReminder {
                    tag: "output_limit".into(),
                    text: "Your previous response reached max_output_tokens and was truncated. Any partial tool call was safely discarded. Continue cleanly using smaller, focused edits.".into(),
                });
                self.persist_state(
                    &state_key,
                    &history,
                    &active_policy,
                    catalog.active_names(),
                    &session_usage,
                )?;
                continue;
            }
            consecutive_output_limits = 0;

            // Final assistant response (no tool calls)
            if resp.tool_calls.is_empty() {
                // Guard against reasoning-only empty turns (e.g. model streamed thousands of
                // reasoning tokens then stopped with empty content & no tool calls): nudge to continue!
                if resp.content.trim().is_empty() {
                    consecutive_empty_responses += 1;
                    if consecutive_empty_responses <= MAX_CONSECUTIVE_EMPTY_RESPONSES {
                        pending_reminders.push(RuntimeReminder {
                            tag: "empty_response".into(),
                            text: "Your previous turn produced only internal reasoning with empty visible output and no tool calls. Continue directly by invoking the next tool call or providing your final answer.".into(),
                        });
                        self.persist_state(
                            &state_key,
                            &history,
                            &active_policy,
                            catalog.active_names(),
                            &session_usage,
                        )?;
                        continue;
                    }
                }
                if let Some(schema) = &cfg.json_schema {
                    validate_json_output(&resp.content, schema)?;
                }
                final_text = resp.content;
                self.persist_state(
                    &state_key,
                    &history,
                    &active_policy,
                    catalog.active_names(),
                    &session_usage,
                )?;
                break;
            }
            consecutive_empty_responses = 0;

            for tc in &resp.tool_calls {
                self.emit_and_persist(
                    &session_id,
                    AgentEvent::ToolStarted {
                        call_id: tc.call_id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                    },
                    &mut on_event,
                );
            }

            // Skill batch hygiene: `skill_load` MUST be invoked alone first before calling other
            // tools so the model actually reads the skill instructions before acting.
            let has_skill_load = resp.tool_calls.iter().any(|tc| tc.name == "skill_load");
            let has_non_skill_load = resp.tool_calls.iter().any(|tc| tc.name != "skill_load");
            let mixed_skill_batch = has_skill_load && has_non_skill_load;

            // Execute tool batch: read-only batches (including read-only git actions) run in parallel!
            let all_read_only = !mixed_skill_batch
                && resp.tool_calls.len() > 1
                && resp.tool_calls.iter().all(|tc| {
                    if tc.name == "git" {
                        return matches!(
                            tc.arguments.get("action").and_then(|v| v.as_str()).unwrap_or(""),
                            "status" | "diff" | "log" | "show" | "blame" | "worktree_list"
                        );
                    }
                    self.registry
                        .get(&tc.name)
                        .map(|h| h.spec().read_only)
                        .unwrap_or(false)
                });

            let outcomes: Vec<MiddlewareOutcome> = if mixed_skill_batch {
                resp.tool_calls
                    .iter()
                    .map(|_| MiddlewareOutcome {
                        result: ToolExecutionResult::err_structured(
                            "mixed_skill_load_batch",
                            "skill_load cannot be mixed with other tools in the same batch; invoke skill_load alone first so you can read and follow its instructions before running other tools.",
                            None,
                        ),
                        reminders: Vec::new(),
                    })
                    .collect()
            } else if all_read_only {
                let mut collected = Vec::with_capacity(resp.tool_calls.len());
                for chunk in resp.tool_calls.chunks(MAX_PARALLEL_READ_TOOLS) {
                    if cfg.cancel.load(Ordering::Relaxed) {
                        cancelled = true;
                        for _ in chunk {
                            collected.push(MiddlewareOutcome {
                                result: ToolExecutionResult::err_structured(
                                    "cancelled",
                                    "execution cancelled",
                                    None,
                                ),
                                reminders: Vec::new(),
                            });
                        }
                        continue;
                    }
                    let mw = &middleware;
                    let run_ctx_ref = &run_ctx;
                    let chunk_out: Vec<_> = std::thread::scope(|s| {
                        let handles: Vec<_> = chunk
                            .iter()
                            .map(|tc| {
                                let name = tc.name.as_str();
                                let args = tc.arguments.clone();
                                s.spawn(move || mw.execute(&self.registry, run_ctx_ref, name, args))
                            })
                            .collect();
                        handles
                            .into_iter()
                            .map(|h| {
                                h.join().unwrap_or_else(|_| MiddlewareOutcome {
                                    result: ToolExecutionResult::err_structured(
                                        "tool_panic",
                                        "parallel read-only tool thread panicked",
                                        None,
                                    ),
                                    reminders: Vec::new(),
                                })
                            })
                            .collect()
                    });
                    collected.extend(chunk_out);
                }
                collected
            } else {
                let mut collected = Vec::with_capacity(resp.tool_calls.len());
                for tc in &resp.tool_calls {
                    if cfg.cancel.load(Ordering::Relaxed) {
                        cancelled = true;
                        collected.push(MiddlewareOutcome {
                            result: ToolExecutionResult::err_structured(
                                "cancelled",
                                "execution cancelled",
                                None,
                            ),
                            reminders: Vec::new(),
                        });
                        continue;
                    }
                    collected.push(middleware.execute(
                        &self.registry,
                        &run_ctx,
                        &tc.name,
                        tc.arguments.clone(),
                    ));
                }
                collected
            };

            let mut guard_batch = Vec::with_capacity(resp.tool_calls.len());
            for (tc, out) in resp.tool_calls.iter().zip(outcomes.into_iter()) {
                // Auto-activate ONLY explicitly requested deferred tools returned by `tool_search`
                // strictly at tail (never blindly activating all 40 tools on an empty browse query!).
                if tc.name == "tool_search" && !out.result.is_error {
                    let val = out.result.payload.to_model_value();
                    if let Some(arr) = val.get("activate_names").and_then(|v| v.as_array()) {
                        let names: Vec<String> = arr
                            .iter()
                            .filter_map(|v| v.as_str())
                            .take(8)
                            .map(ToString::to_string)
                            .collect();
                        catalog.activate_tail(&names, &self.registry);
                    }
                }
                // Reference loaded skill concisely as an ephemeral tail reminder without duplicating
                // the full skill body (which is already present in the tool result above!).
                if tc.name == "skill_load" && !out.result.is_error {
                    let val = out.result.payload.to_model_value();
                    if val.get("body").and_then(|v| v.as_str()).is_some() {
                        let skill_name = val
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("skill");
                        pending_reminders.push(RuntimeReminder {
                            tag: format!("skill:{skill_name}"),
                            text: format!(
                                "Follow the loaded '{skill_name}' skill instructions from the tool result above."
                            ),
                        });
                    }
                }

                pending_reminders.extend(out.reminders);
                all_effects.extend(out.result.effects.clone());
                // Auto-spill oversized tool payloads (>24 KiB) to disk OutputStore
                // so huge logs/diffs never blow up the LLM context window, and auto-activate
                // `tool_output` at the catalog tail so the model can immediately inspect slices!
                let mut full_output_ref: Option<String> = None;
                let effective_payload = if tc.name != "tool_output" {
                    if let Some((compact, meta)) = run_ctx.output_store.maybe_spill_payload(
                        &tc.name,
                        &out.result.payload,
                        nola_tools::DEFAULT_SPILL_THRESHOLD_BYTES,
                    ) {
                        full_output_ref = Some(meta.path.display().to_string());
                        catalog.activate_tail(&["tool_output".to_string()], &self.registry);
                        compact
                    } else {
                        out.result.payload.clone()
                    }
                } else {
                    out.result.payload.clone()
                };
                self.emit_and_persist(
                    &session_id,
                    AgentEvent::ToolFinished {
                        call_id: tc.call_id.clone(),
                        name: tc.name.clone(),
                        is_error: out.result.is_error,
                        summary: effective_payload.ui_summary().to_string(),
                        effects: out.result.effects.clone(),
                        full_output_ref,
                    },
                    &mut on_event,
                );
                history.push(HistoryMessage::Tool {
                    call_id: tc.call_id.clone(),
                    tool_name: tc.name.clone(),
                    payload: effective_payload,
                    is_error: out.result.is_error,
                    media: out.result.media.clone(),
                });
                guard_batch.push((tc.name.clone(), tc.arguments.clone(), out.result));
            }

            // Cap active tool media attachments in history (keeping the newest 3) so long
            // vision sessions do not re-prefill dozens of stale base64 images every turn.
            evict_stale_tool_media(&mut history, MAX_ACTIVE_TOOL_MEDIA_ATTACHMENTS);

            if let Some(reminder) = loop_guard.observe_batch(&guard_batch) {
                pending_reminders.push(reminder);
            }

            self.persist_state(
                &state_key,
                &history,
                &active_policy,
                catalog.active_names(),
                &session_usage,
            )?;
        }

        let stop_payload = json!({
            "session_id": session_id,
            "steps": steps,
            "cancelled": cancelled,
            "final_text_len": final_text.len(),
        });
        let _ = middleware
            .hooks
            .run_event(HookEvent::Stop, "session", &stop_payload);

        self.emit_and_persist(
            &session_id,
            AgentEvent::TurnCompleted {
                session_id: session_id.clone(),
                steps,
                usage: total_usage.clone(),
                final_text: final_text.clone(),
            },
            &mut on_event,
        );

        Ok(AgentRunResult {
            session_id,
            final_text,
            steps,
            usage: total_usage,
            effects: all_effects,
            cancelled,
        })
    }

    pub fn load_session_events(&self, session_id: &str, after_seq: i64) -> Result<EventPage, StoreError> {
        self.store.events_after(session_id, after_seq, 500)
    }

    fn persist_state(
        &self,
        key: &str,
        history: &[HistoryMessage],
        policy: &EffectivePolicy,
        activated_tools: &[String],
        usage: &TokenUsage,
    ) -> Result<(), StoreError> {
        let val = serde_json::to_value(PersistedSessionState {
            history: history.to_vec(),
            policy: policy.clone(),
            activated_tools: activated_tools.to_vec(),
            usage: usage.clone(),
        })
        .unwrap_or(json!({}));
        self.store.set_kv(key, &val)
    }

    fn emit_and_persist(
        &self,
        session_id: &str,
        event: AgentEvent,
        on_event: &mut impl FnMut(AgentEvent),
    ) {
        if let Ok(val) = serde_json::to_value(&event) {
            let kind = val
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("event")
                .to_string();
            let _ = self
                .store
                .append_event(session_id, &kind, val, DEFAULT_MAX_EVENTS_PER_SCOPE);
        }
        on_event(event);
    }
}

pub(crate) fn evict_stale_tool_media(history: &mut [HistoryMessage], max_active: usize) {
    let total_media: usize = history
        .iter()
        .map(|m| match m {
            HistoryMessage::Tool { media, .. } => media.len(),
            _ => 0,
        })
        .sum();
    if total_media <= max_active {
        return;
    }
    let mut to_drop = total_media - max_active;
    for msg in history.iter_mut() {
        if to_drop == 0 {
            break;
        }
        if let HistoryMessage::Tool { media, .. } = msg {
            if !media.is_empty() {
                if media.len() <= to_drop {
                    to_drop -= media.len();
                    media.clear();
                } else {
                    media.drain(..to_drop);
                    to_drop = 0;
                }
            }
        }
    }
}

fn is_context_overflow_error(err: &LlmError) -> bool {
    let body = match err {
        LlmError::Status {
            status: 400 | 413,
            body,
        } => body.as_str(),
        LlmError::Wire(msg) => msg.as_str(),
        _ => return false,
    };
    let lower = body.to_ascii_lowercase();
    lower.contains("context length")
        || lower.contains("maximum context")
        || lower.contains("too many tokens")
        || lower.contains("prompt is too long")
        || lower.contains("context_length_exceeded")
        || lower.contains("exceeds the available context")
}

fn extract_recall_keywords(prompt: &str) -> String {
    const STOP_WORDS: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "into", "your", "have", "will",
        "what", "when", "where", "which", "about", "please", "could", "would", "should", "there",
        "their", "they", "them", "then", "than", "make", "just", "some", "more", "also", "been",
    ];
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for raw in prompt.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        let tok = raw.trim().to_ascii_lowercase();
        if tok.chars().count() < 3 || STOP_WORDS.contains(&tok.as_str()) {
            continue;
        }
        if seen.insert(tok.clone()) {
            out.push(tok);
            if out.len() >= 20 {
                break;
            }
        }
    }
    out.join(" ")
}

fn accumulate_usage(dst: &mut TokenUsage, src: &TokenUsage) {
    dst.input_tokens = dst.input_tokens.saturating_add(src.input_tokens);
    dst.output_tokens = dst.output_tokens.saturating_add(src.output_tokens);
    dst.cache_read_tokens = dst.cache_read_tokens.saturating_add(src.cache_read_tokens);
    dst.cache_write_tokens = dst.cache_write_tokens.saturating_add(src.cache_write_tokens);
    dst.reasoning_tokens = dst.reasoning_tokens.saturating_add(src.reasoning_tokens);
    dst.usage_reported |= src.usage_reported;
}

fn recall_scoped_memories(
    store: &Store,
    query: &str,
    workspace_root: &Path,
    session_id: &str,
    limit: usize,
) -> Vec<MemoryItem> {
    let ws_ns = workspace_memory_namespace(workspace_root);
    let session_ns = format!("session:{session_id}");
    let namespaces = [ws_ns.as_str(), session_ns.as_str(), "default"];
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for ns in namespaces {
        if out.len() >= limit {
            break;
        }
        if let Ok(items) = store.search_memory(query, Some(ns), limit) {
            for item in items {
                if item.namespace == "code_memory"
                    || item.tags.iter().any(|t| t == "code_memory")
                {
                    continue;
                }
                let belongs_to_other_ws = item
                    .tags
                    .iter()
                    .any(|t| t.starts_with("ws:") && t != &ws_ns);
                if belongs_to_other_ws {
                    continue;
                }
                if seen.insert(item.id.clone()) {
                    out.push(item);
                    if out.len() >= limit {
                        break;
                    }
                }
            }
        }
    }
    out
}

fn build_compaction_summary_request(slice: &[HistoryMessage]) -> StructuredModelRequest {
    const MAX_TRANSCRIPT_CHARS: usize = 24_000;
    let mut lines = Vec::new();
    let mut total_chars = 0usize;
    for msg in slice {
        let entry = match msg {
            HistoryMessage::User { content, .. } => {
                let snippet: String = content.chars().take(1_200).collect();
                format!("USER: {snippet}")
            }
            HistoryMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                let snippet: String = content.chars().take(1_000).collect();
                if tool_calls.is_empty() {
                    format!("ASSISTANT: {snippet}")
                } else {
                    let names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
                    format!("ASSISTANT [tools: {}]: {snippet}", names.join(", "))
                }
            }
            HistoryMessage::Tool {
                tool_name,
                payload,
                is_error,
                ..
            } => {
                let snippet: String = payload.to_model_string().chars().take(600).collect();
                let status = if *is_error { "ERR" } else { "OK" };
                format!("TOOL({tool_name}, {status}): {snippet}")
            }
        };
        if total_chars.saturating_add(entry.len()) > MAX_TRANSCRIPT_CHARS {
            break;
        }
        total_chars = total_chars.saturating_add(entry.len());
        lines.push(entry);
    }
    let transcript = lines.join("\n");
    StructuredModelRequest {
        frozen_system_prefix: vec![FrozenSystemBlock {
            id: "compaction_summarizer".into(),
            text: "You are a concise working-state compactor. Summarize the earlier conversation history into a compact, factual handoff preserving: (1) original user goal & constraints, (2) files read/modified (exact paths), (3) key commands/outputs/errors and any output_id references, and (4) current progress and next steps. Do not invent facts.".into(),
        }],
        prefix_stable_tools: Vec::new(),
        history: vec![HistoryMessage::User {
            content: format!(
                "Compact these {} earlier conversation messages into a dense working-state summary:\n\n{transcript}",
                slice.len()
            ),
            media: Vec::new(),
        }],
        ephemeral_tail_reminders: Vec::new(),
        max_output_tokens: 1_024,
        json_schema: None,
    }
}

fn summarize_history_with_caller(
    caller: &dyn ModelCaller,
    route: &ModelRouteConfig,
    slice: &[HistoryMessage],
) -> Option<String> {
    if slice.is_empty() {
        return None;
    }
    let req = build_compaction_summary_request(slice);
    let resp = caller.call(route, &req, false, &mut |_| {}).ok()?;
    let trimmed = resp.content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn summarize_history_with_llm_client(
    client: &LlmClient,
    route: &ModelRouteConfig,
    slice: &[HistoryMessage],
) -> Option<String> {
    if slice.is_empty() {
        return None;
    }
    let req = build_compaction_summary_request(slice);
    let resp = client.complete(route, &req, false, &mut |_| {}).ok()?;
    let trimmed = resp.content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn extract_compacted_summary(history: &[HistoryMessage]) -> Option<String> {
    let HistoryMessage::User { content, .. } = history.first()? else {
        return None;
    };
    let trimmed = content.trim();
    trimmed
        .strip_prefix("<compacted_history_summary>")
        .and_then(|s| s.strip_suffix("</compacted_history_summary>"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn validate_json_output(raw: &str, schema: &Value) -> Result<(), AgentError> {
    let parsed: Value = serde_json::from_str(raw.trim()).map_err(|e| {
        AgentError::SchemaValidation(format!("final output is not valid JSON: {e}"))
    })?;
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        let obj = parsed.as_object().ok_or_else(|| {
            AgentError::SchemaValidation("expected JSON object matching schema".into())
        })?;
        for key in required.iter().filter_map(|v| v.as_str()) {
            if !obj.contains_key(key) {
                return Err(AgentError::SchemaValidation(format!(
                    "JSON output missing required field '{key}'"
                )));
            }
        }
    }
    Ok(())
}
