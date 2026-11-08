// SPDX-License-Identifier: Apache-2.0
//! Durable conversation execution preferences. No credentials or live handles.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use crate::reasoning_tiers::ReasoningTier;
use crate::run_generation_intent::RunGenerationIntent;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationExecutionPolicy {
    pub execution_backend: String,
    pub external_execution_session_id: Option<String>,
    pub external_execution_model: Option<String>,
    pub external_execution_model_provider: Option<String>,
    pub max_iterations: usize,
    pub swe_mode: bool,
    pub reasoning_tier: ReasoningTier,
    pub language: Option<String>,
    pub openai_prompt_cache: Option<bool>,
    pub openai_prompt_cache_key: Option<String>,
    pub openai_prompt_cache_key_prefix: Option<String>,
    pub openai_prompt_cache_retention: Option<String>,
    pub openai_prompt_cache_history_breakpoint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_prompt_cache_history_breakpoint_endpoints: Option<Vec<String>>,
    pub instructions: Vec<String>,
    pub agents: BTreeMap<String, ConversationAgentPolicy>,
    pub permissions: Vec<ConversationPermissionDeclaration>,
    pub core_permission_rules: Vec<crate::PermissionRule>,
    pub compaction: ConversationCompactionPolicy,
    pub external_execution: ConversationExternalExecutionBudget,
    pub fallback_model_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationAgentPolicy {
    pub description: Option<String>,
    pub prompt: Option<String>,
    pub model: Option<RunGenerationIntent>,
    pub executor: Option<crate::delegation::ExternalAgentExecutor>,
    pub tools: Vec<String>,
    pub permission: Vec<ConversationPermissionDeclaration>,
    pub max_turns: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationPermissionDeclaration {
    pub permission: String,
    pub pattern: String,
    pub action: ConversationPermissionAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversationPermissionAction { Allow, Ask, Deny }

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationCompactionPolicy {
    pub token_threshold: Option<usize>,
    pub context_window_tokens: Option<usize>,
    pub keep_recent_messages: Option<usize>,
    pub compacted_tool_chars: Option<usize>,
    pub compacted_assistant_chars: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationExternalExecutionBudget {
    pub max_wall_time_seconds: Option<usize>,
    pub max_output_bytes: Option<usize>,
    pub max_cpu_time_seconds: Option<usize>,
    pub max_memory_bytes: Option<usize>,
    pub max_processes: Option<usize>,
    pub max_line_bytes: Option<usize>,
}

impl ConversationExecutionPolicy {
    pub fn validate(&self) -> Result<(), String> {
        let bounded = |value: &str, max| !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control);
        if !bounded(&self.execution_backend, 128)
            || self.max_iterations > 1_000_000
            || self.instructions.len() > 128
            || self.instructions.iter().any(|value| value.len() > 262_144)
            || self.agents.len() > 128 || self.permissions.len() > 512
            || self.core_permission_rules.len() > 512 || self.fallback_model_ids.len() > 32
            || self.fallback_model_ids.iter().any(|id| !bounded(id, 512))
        { return Err("invalid conversation execution policy".into()); }
        if self.compaction.context_window_tokens.is_some_and(|value| !(512..=4_000_000).contains(&value)) {
            return Err("invalid conversation context window".into());
        }
        for rule in &self.permissions {
            if !bounded(&rule.permission, 128) || rule.pattern.len() > 8192 {
                return Err("invalid conversation permission declaration".into());
            }
        }
        for (id, agent) in &self.agents {
            if !bounded(id, 128) || agent.tools.len() > 512 || agent.permission.len() > 512
                || agent.prompt.as_ref().is_some_and(|value| value.len() > 262_144) {
                return Err("invalid conversation agent policy".into());
            }
            if let Some(model) = &agent.model { model.validate()?; }
            if let Some(executor) = &agent.executor {
                executor.validate()?;
                if agent.model.is_some() { return Err("Agent model and external executor are mutually exclusive".into()); }
            }
        }
        Ok(())
    }
}
