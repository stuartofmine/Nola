// SPDX-License-Identifier: Apache-2.0
//! Shared immutable delegation profiles; platform tool executors remain separate.
use crate::execution_policy::{ConversationAgentPolicy, ConversationPermissionDeclaration};
use std::collections::BTreeMap;

/// Common parent/child workspace contract, independent of Host transport.
pub const WORKSPACE_GUIDANCE: &str = "Task briefs and results must state scope, constraints, expected evidence, implementation paths, test/scratch paths and the delivered file set. Workspace scope: without an explicit frozen worktree binding, parent and child share the authorized project files. When a worktree is bound, use its admitted execution root and report its paths; its files are separate but Git metadata remains shared. A separate conversation or worker process does not create a private filesystem or OS sandbox. A task brief must distinguish implementation paths, test/scratch paths, and the delivered file set. Follow the existing project test layout; do not assume a source-only delivery directory is a test directory. If no layout exists, keep scratch tests outside the delivery tree. Preserve unrelated files and tests; never automatically delete shared files on task completion. Report changed paths and the exact files/commands actually validated. The parent accepts the complete delivery set, not merely the child's successful checks. This guidance does not grant file access.";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentProfile {
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub model: Option<crate::run_generation_intent::RunGenerationIntent>,
    /// A user-configured external harness, independent of the parent model.
    pub executor: Option<ExternalAgentExecutor>,
    pub tools: Vec<String>,
    pub permission: Vec<ConversationPermissionDeclaration>,
    pub max_turns: Option<usize>,
}

/// External model IDs belong to the selected harness, never the HTTP model catalog.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAgentExecutor {
    pub backend: crate::execution_backend::ExecutionBackendKind,
    pub model: Option<String>,
    pub model_provider: Option<String>,
}
impl ExternalAgentExecutor {
    pub fn validate(&self) -> Result<(), String> {
        if self.backend == crate::execution_backend::ExecutionBackendKind::Nola {
            return Err("Use the model field for a Nola subagent, not an external executor".into());
        }
        if self.backend != crate::execution_backend::ExecutionBackendKind::Codex && self.model_provider.is_some() {
            return Err("Only Codex supports a separate executor model_provider; use a qualified model ID for OpenCode".into());
        }
        if [&self.model, &self.model_provider].into_iter().flatten()
            .any(|s| s.trim().is_empty() || s.len() > 512 || s.chars().any(char::is_control)) {
            return Err("Invalid external subagent model selection".into());
        }
        Ok(())
    }
}

pub fn effective_agent_profiles(
    profiles: &BTreeMap<String, ConversationAgentPolicy>,
) -> BTreeMap<String, AgentProfile> {
    let mut agents = builtins();
    for (name, backend) in [("codex", crate::execution_backend::ExecutionBackendKind::Codex),
        ("claude_code", crate::execution_backend::ExecutionBackendKind::ClaudeCode),
        ("opencode", crate::execution_backend::ExecutionBackendKind::OpenCode)] {
        agents.insert(name.into(), AgentProfile {
            name: name.into(), description: format!("Independent task using the installed {name} harness and its own existing login; desktop Host only"),
            prompt: generic_prompt(), model: None,
            executor: Some(ExternalAgentExecutor { backend, model: None, model_provider: None }),
            tools: Vec::new(), permission: Vec::new(), max_turns: Some(16),
        });
    }
    for (name, item) in profiles {
        let profile = agents.entry(name.clone()).or_insert_with(|| AgentProfile {
            name: name.clone(),
            description: item
                .description
                .clone()
                .unwrap_or_else(|| format!("Custom agent '{name}'")),
            prompt: generic_prompt(),
            model: None,
            executor: None,
            tools: Vec::new(),
            permission: Vec::new(),
            max_turns: None,
        });
        if let Some(description) = &item.description {
            profile.description = description.clone();
        }
        if let Some(prompt) = &item.prompt {
            profile.prompt = prompt.clone();
        }
        if let Some(model) = &item.model {
            profile.executor = None;
            profile.model = Some(model.clone());
        }
        if let Some(executor) = &item.executor {
            profile.model = None;
            profile.executor = Some(executor.clone());
        }
        if !item.tools.is_empty() {
            profile.tools = item.tools.clone();
        }
        profile.permission.extend(item.permission.clone());
        profile.max_turns = item.max_turns.or(profile.max_turns);
    }
    agents
}

fn builtins() -> BTreeMap<String, AgentProfile> {
    [
        AgentProfile {
            name: "explore".to_string(),
            description: "Parallel read-only investigation for locating code, behavior, and evidence"
                .to_string(),
            prompt: "You are a read-only exploration sub-agent. Answer the delegated question from repository evidence. Start with high-signal entry points, batch independent reads, and stop exploring once the evidence is sufficient. Never edit files or run mutating commands. Return concise findings first, followed by exact paths or symbols, constraints, and any uncertainty. Do not present guesses as facts."
                .to_string(),
            model: None,
            executor: None,
            tools: vec![
                "read_file", "read_many_files", "list_dir", "find_files", "find_up", "glob", "grep",
                "file_tree", "project_overview", "instruction_context", "context_pack",
                "file_outline", "read_symbol", "workspace_status", "git_diff", "related_files",
                "focused_context", "tool_search", "set_thinking_mode",
            ].into_iter().map(str::to_string).collect(),
            permission: Vec::new(),
            max_turns: Some(12),
        },
        AgentProfile {
            name: "plan".to_string(),
            description: "Evidence-backed implementation planning for complex or high-impact work"
                .to_string(),
            prompt: "You are a read-only planning sub-agent. Build an execution-ready plan from actual repository evidence. Inspect the relevant interfaces, callers, tests, and project instructions before deciding. Do not edit files. Return ordered implementation steps with exact files or symbols, behavioral invariants, risks, permission-requiring operations, and focused validation. Avoid generic advice and do not claim facts you did not inspect."
                .to_string(),
            model: None,
            executor: None,
            tools: vec![
                "read_file", "read_many_files", "list_dir", "find_files", "glob", "grep",
                "project_overview", "instruction_context", "context_pack", "file_outline",
                "read_symbol", "related_files", "impact_analysis", "validation_plan",
                "test_context", "tool_search",
            ].into_iter().map(str::to_string).collect(),
            permission: Vec::new(),
            max_turns: Some(10),
        },
        AgentProfile {
            name: "verify".to_string(),
            description: "Independent review and focused verification after implementation or failure"
                .to_string(),
            prompt: "You are an independent verification sub-agent. Verify the requested behavior against the actual diff, relevant code, and focused checks. Do not make broad edits or silently repair findings. Report findings first in severity order with exact paths, then separate verified behavior, unverified behavior, blocked checks, and residual risk. Never describe a check as passing unless you ran it and observed the result."
                .to_string(),
            model: None,
            executor: None,
            tools: vec![
                "read_file", "read_many_files", "grep", "git_diff", "workspace_status",
                "changed_symbols", "diff_review", "verification_review", "validation_plan",
                "failure_triage", "failure_context", "bash_command_review", "bash", "tool_search",
            ].into_iter().map(str::to_string).collect(),
            permission: Vec::new(),
            max_turns: Some(12),
        },
        AgentProfile {
            name: "workspace".into(),
            description: "File edits with bounded tools; a frozen worktree constrains writes and permits independent file workers. Commands/builds require parent verification.".into(),
            prompt: "Inspect and edit the requested files with file tools. Preserve unrelated data. An admitted worktree is a file scope, not an OS sandbox. Do not claim builds/tests ran; return changed paths and checks actually performed.".into(),
            model:None,executor:None,
            tools:["tool_search","read_file","read_many_files","list_dir","find_files","glob","grep","file_tree","tool_output_read","write_file","edit_file","edit_file_lines","delete_file"].into_iter().map(str::to_string).collect(),
            permission:Vec::new(),max_turns:Some(16),
        },
        AgentProfile {
            name: "general".to_string(),
            description: "Isolated implementation or investigation owned through a concrete result"
                .to_string(),
            prompt: generic_prompt(),
            model: None,
            executor: None,
            tools: Vec::new(),
            permission: Vec::new(),
            max_turns: Some(16),
        },
    ]
    .into_iter()
    .map(|agent| (agent.name.clone(), agent))
    .collect()
}

fn generic_prompt() -> String {
    "You are a focused execution sub-agent. Own the delegated task through a concrete result. Inspect before editing, batch independent reads, preserve unrelated work, and make the smallest coherent change that satisfies the requested behavior. For multi-step work, keep status explicit and recover from tool failures instead of repeating the same action. Ask only when genuinely blocked. Return the outcome, exact changed paths, validation actually performed, and remaining risk; never claim checks you did not run."
        .to_string()
}
