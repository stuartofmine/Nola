// SPDX-License-Identifier: Apache-2.0
//! Core Agent Loop Runner:
//! - Immutable `frozen_system_prefix` built ONCE on Turn 0
//! - Prefix-stable tool catalog with tail-only deferred tool activation
//! - Parallel read-only tool batches (up to 12) + sequential mutating tools
//! - Automatic `nola-store` session + event + `--resume` policy persistence
//! - Clean `OutputLimit` continuation + cancellation + `--json-schema` validation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nola_llm::{LlmClient, LlmError};
use nola_store::{EventPage, Store, StoreError, DEFAULT_MAX_EVENTS_PER_SCOPE};
use nola_tools::{ToolContext, ToolRegistry};
use nola_types::{
    estimate_request_tokens, new_id, now_ms, ContextBudget, FrozenSystemBlock, HistoryMessage,
    HostCapabilities, ModelRouteConfig, ModelStopReason, ModelStreamDelta, NormalizedModelResponse,
    ResourceEffect, RuntimeReminder, Session, StructuredModelRequest, TokenUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::catalog::ToolCatalog;
use crate::compaction::compact_history;
use crate::hooks::HookRunner;
use crate::loop_guard::LoopGuard;
use crate::middleware::{ApprovalCallback, ToolExecutionMiddleware};
use crate::policy::EffectivePolicy;

const MAX_PARALLEL_READ_TOOLS: usize = 12;

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
    pub force_all_tools: bool,
    pub stream: bool,
    pub json_schema: Option<Value>,
    pub cancel: Arc<AtomicBool>,
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
        mut on_event: impl FnMut(AgentEvent),
    ) -> Result<AgentRunResult, AgentError> {
        let session_id = cfg.session_id.clone().unwrap_or_else(new_id);
        let now = now_ms();

        // Restore history + EffectivePolicy + activated tools when resuming a session!
        let state_key = format!("session_state:{session_id}");
        let mut active_policy = cfg.policy.clone();
        let mut catalog = ToolCatalog::build_initial(
            &self.registry,
            cfg.host_capabilities,
            cfg.force_all_tools,
        );
        let mut history: Vec<HistoryMessage> = if cfg.resume {
            if let Some(val) = self.store.get_kv(&state_key)? {
                if let Ok(saved) = serde_json::from_value::<PersistedSessionState>(val) {
                    active_policy = saved.policy;
                    catalog.restore_activated(&saved.activated_tools, &self.registry);
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
            workspace_root: ctx.workspace_root.display().to_string(),
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

        // Append new user message
        history.push(HistoryMessage::User {
            content: user_prompt.to_string(),
            media: Vec::new(),
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
        let mut total_usage = TokenUsage::default();
        let mut all_effects: Vec<ResourceEffect> = Vec::new();
        let mut final_text = String::new();
        let mut steps = 0usize;
        let mut cancelled = false;

        loop {
            if cfg.cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            if cfg.max_steps > 0 && steps >= cfg.max_steps {
                break;
            }
            steps += 1;

            // Prefix-stable tools & fixed overhead estimation
            let tools = catalog.compile_preserving_prefix_order(&self.registry);
            let probe_req = StructuredModelRequest {
                frozen_system_prefix: cfg.frozen_system_blocks.clone(),
                prefix_stable_tools: tools.clone(),
                history: Vec::new(),
                ephemeral_tail_reminders: pending_reminders.clone(),
                max_output_tokens: budget.reserved_output_tokens,
                json_schema: cfg.json_schema.clone(),
            };
            let fixed_overhead = estimate_request_tokens(&probe_req);

            // Two-tier compaction (Tier-1 micro-compaction commits partial savings first!)
            let comp = compact_history(&mut history, budget, fixed_overhead, None);
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

            let request = StructuredModelRequest {
                frozen_system_prefix: cfg.frozen_system_blocks.clone(),
                prefix_stable_tools: tools,
                history: history.clone(),
                ephemeral_tail_reminders: std::mem::take(&mut pending_reminders),
                max_output_tokens: budget.reserved_output_tokens,
                json_schema: cfg.json_schema.clone(),
            };

            let resp = self.caller.call(&cfg.route, &request, cfg.stream, &mut |delta| {
                match delta {
                    ModelStreamDelta::Text(t) => on_event(AgentEvent::TextDelta { delta: t }),
                    ModelStreamDelta::Reasoning(r) => {
                        on_event(AgentEvent::ReasoningDelta { delta: r })
                    }
                    ModelStreamDelta::ToolCallStart { .. }
                    | ModelStreamDelta::ToolCallArgsDelta { .. } => {}
                }
            })?;

            total_usage.input_tokens = total_usage.input_tokens.saturating_add(resp.usage.input_tokens);
            total_usage.output_tokens = total_usage.output_tokens.saturating_add(resp.usage.output_tokens);
            total_usage.cache_read_tokens = total_usage
                .cache_read_tokens
                .saturating_add(resp.usage.cache_read_tokens);
            total_usage.cache_write_tokens = total_usage
                .cache_write_tokens
                .saturating_add(resp.usage.cache_write_tokens);

            // Record assistant turn verbatim (including provider continuation!)
            history.push(HistoryMessage::Assistant {
                content: resp.content.clone(),
                reasoning: resp.reasoning.clone(),
                tool_calls: resp.tool_calls.clone(),
                continuation: resp.continuation.clone(),
            });

            // Handle OutputLimit cleanly (truncated tool calls were already atomically discarded!)
            if resp.stop_reason == ModelStopReason::OutputLimit {
                pending_reminders.push(RuntimeReminder {
                    tag: "output_limit".into(),
                    text: "Your previous response reached max_output_tokens and was truncated. Any partial tool call was safely discarded. Continue cleanly using smaller, focused edits.".into(),
                });
                self.persist_state(&state_key, &history, &active_policy, catalog.active_names())?;
                continue;
            }

            // Final assistant response (no tool calls)
            if resp.tool_calls.is_empty() {
                if let Some(schema) = &cfg.json_schema {
                    validate_json_output(&resp.content, schema)?;
                }
                final_text = resp.content;
                self.persist_state(&state_key, &history, &active_policy, catalog.active_names())?;
                break;
            }

            // Execute tool batch: read-only batches run in parallel (up to 12); mutating sequentially!
            let all_read_only = resp.tool_calls.len() > 1
                && resp.tool_calls.iter().all(|tc| {
                    self.registry
                        .get(&tc.name)
                        .map(|h| h.spec().read_only)
                        .unwrap_or(false)
                });

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

            let outcomes: Vec<crate::middleware::MiddlewareOutcome> = if all_read_only {
                let mut collected = Vec::with_capacity(resp.tool_calls.len());
                for chunk in resp.tool_calls.chunks(MAX_PARALLEL_READ_TOOLS) {
                    let mw = &middleware;
                    let chunk_out: Vec<_> = std::thread::scope(|s| {
                        let handles: Vec<_> = chunk
                            .iter()
                            .map(|tc| {
                                let name = tc.name.as_str();
                                let args = tc.arguments.clone();
                                s.spawn(move || {
                                    mw.execute(&self.registry, ctx, name, args)
                                })
                            })
                            .collect();
                        handles
                            .into_iter()
                            .map(|h| h.join().unwrap())
                            .collect()
                    });
                    collected.extend(chunk_out);
                }
                collected
            } else {
                resp.tool_calls
                    .iter()
                    .map(|tc| {
                        middleware.execute(&self.registry, ctx, &tc.name, tc.arguments.clone())
                    })
                    .collect()
            };

            let mut guard_batch = Vec::with_capacity(resp.tool_calls.len());
            for (tc, out) in resp.tool_calls.iter().zip(outcomes.into_iter()) {
                // Auto-activate deferred tools returned by `tool_search` strictly at tail!
                if tc.name == "tool_search" && !out.result.is_error {
                    let val = out.result.payload.to_model_value();
                    if let Some(arr) = val.get("tools").and_then(|v| v.as_array()) {
                        let names: Vec<String> = arr
                            .iter()
                            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
                            .map(ToString::to_string)
                            .collect();
                        catalog.activate_tail(&names, &self.registry);
                    }
                }
                // Inject any skill body returned by `skill_load` as an ephemeral tail reminder
                // (NEVER mutating Message 0 frozen_system_prefix!).
                if tc.name == "skill_load" && !out.result.is_error {
                    let val = out.result.payload.to_model_value();
                    if let Some(body) = val.get("body").and_then(|v| v.as_str()) {
                        let skill_name = val
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("skill");
                        pending_reminders.push(RuntimeReminder {
                            tag: format!("skill:{skill_name}"),
                            text: body.to_string(),
                        });
                    }
                }

                pending_reminders.extend(out.reminders);
                all_effects.extend(out.result.effects.clone());
                self.emit_and_persist(
                    &session_id,
                    AgentEvent::ToolFinished {
                        call_id: tc.call_id.clone(),
                        name: tc.name.clone(),
                        is_error: out.result.is_error,
                        summary: out.result.payload.ui_summary().to_string(),
                        effects: out.result.effects.clone(),
                    },
                    &mut on_event,
                );
                history.push(HistoryMessage::Tool {
                    call_id: tc.call_id.clone(),
                    tool_name: tc.name.clone(),
                    payload: out.result.payload.clone(),
                    is_error: out.result.is_error,
                    media: out.result.media.clone(),
                });
                guard_batch.push((tc.name.clone(), tc.arguments.clone(), out.result));
            }

            if let Some(reminder) = loop_guard.observe_batch(&guard_batch) {
                pending_reminders.push(reminder);
            }

            self.persist_state(&state_key, &history, &active_policy, catalog.active_names())?;
        }

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
    ) -> Result<(), StoreError> {
        let val = serde_json::to_value(PersistedSessionState {
            history: history.to_vec(),
            policy: policy.clone(),
            activated_tools: activated_tools.to_vec(),
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
