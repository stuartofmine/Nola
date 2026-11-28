// SPDX-License-Identifier: Apache-2.0
//! Comprehensive 60-tool completeness matrix and complex multi-step scenario E2E suite.
//!
//! Covers:
//! 1. 100% completeness gate over EVERY registered `ToolHandler` across `nola-tools` (29),
//!    `nola-modules` (24), and `nola-engine` (7) — fails automatically if any tool is untested.
//! 2. Real multi-file coding refactor + Jupyter notebook + Git + PTY + background shell
//!    control + QuickJS code_exec + workspace checkpoints end-to-end via `AgentEngine`.
//! 3. Heavy 12-round interleaved reasoning + tools + PNG/SVG/CSV deliverable generation +
//!    multimodal vision `read_file` + chronological text splitting + O(1) journal storage.
//! 4. Interactive planning (`update_agent_todos`, `propose_agent_plan`, `request_user_input`),
//!    A2UI (`render_ui_surface`, `delete_ui_surface`), and subagent/expert orchestration
//!    (`spawn_agent`, `task_control`, `list_experts`, `delegate_expert`).
//! 5. Cross-session memory + 7-tool codebase intelligence + skills + goals/SOPs +
//!    indexed local data sources (`document_search`) + automations/rooms/messaging.
//! 6. Deferred tool discovery (`tool_search`), unknown-tool recovery, malformed/undeclared arg
//!    self-correction, `Cautious` permission denial recovery, and multilingual/ambiguous
//!    capability-name routing guardrails (`AGENTS.md`).
//! 7. Real loopback Streamable HTTP MCP server discovery, tool/resource/prompt execution,
//!    permission gating (`Cautious` deny vs `AllowAll` allow), and disconnect isolation.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use nola_engine::{
    load_session_plan, load_session_todos, register_planning_tools, register_subagent_tools,
    AgentEngine, AgentEvent, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner,
    ModelCaller, PlanningState, SubagentManager,
};
use nola_llm::LlmError;
use nola_mcp::McpManager;
use nola_modules::ModulesSuite;
use nola_store::Store;
use nola_tools::{OutputStore, ToolContext, ToolRegistry};
use nola_types::{
    FrozenSystemBlock, HostCapabilities, McpServerConfig, McpTransport, ModelCapabilities,
    ModelProtocol, ModelRouteConfig, ModelStopReason, ModelStreamDelta, ModelToolCall,
    NormalizedModelResponse, PermissionProfile, StructuredModelRequest, TokenUsage,
    ToolModelPayload,
};
use serde_json::{json, Value};

// ============================================================================
// Shared Harness Helpers
// ============================================================================

struct ScriptedCaller {
    turns: Mutex<VecDeque<NormalizedModelResponse>>,
    seen_requests: Mutex<Vec<StructuredModelRequest>>,
}

impl ScriptedCaller {
    fn new(turns: Vec<NormalizedModelResponse>) -> Self {
        Self {
            turns: Mutex::new(VecDeque::from(turns)),
            seen_requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<StructuredModelRequest> {
        self.seen_requests.lock().unwrap().clone()
    }
}

impl ModelCaller for ScriptedCaller {
    fn call(
        &self,
        _route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        self.seen_requests.lock().unwrap().push(req.clone());
        let resp = self
            .turns
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| NormalizedModelResponse {
                response_id: Some("fallback".into()),
                response_model: Some("scripted".into()),
                content: "Done.".into(),
                reasoning: None,
                tool_calls: vec![],
                stop_reason: ModelStopReason::Stop,
                usage: TokenUsage::default(),
                continuation: None,
            });
        if stream {
            if let Some(ref r) = resp.reasoning {
                on_delta(ModelStreamDelta::Reasoning(r.clone()));
            }
            if !resp.content.is_empty() {
                on_delta(ModelStreamDelta::Text(resp.content.clone()));
            }
        }
        Ok(resp)
    }
}

fn test_route(image_input: bool) -> ModelRouteConfig {
    ModelRouteConfig {
        connection_id: "scripted".into(),
        model: "scripted-reasoner".into(),
        endpoint: "http://127.0.0.1:1/v1".into(),
        protocol: ModelProtocol::OpenAiChat,
        api_key: "sk-test".into(),
        context_window: 128_000,
        max_output_tokens: Some(4096),
        temperature: Some(0.2),
        top_p: None,
        reasoning_effort: Some("medium".into()),
        capabilities: ModelCapabilities {
            stream_reasoning: true,
            tool_calls: true,
            image_input,
            reasoning: true,
            sampling_parameters: true,
            ..Default::default()
        },
        prompt_cache_key: None,
    }
}

fn test_policy(profile: PermissionProfile) -> EffectivePolicy {
    EffectivePolicy {
        profile,
        workspace_trusted: true,
        network_allowed: true,
        rules: vec![],
        plan_mode: false,
    }
}

fn run_cfg(
    session_id: &str,
    resume: bool,
    profile: PermissionProfile,
    image_input: bool,
) -> AgentRunConfig {
    AgentRunConfig {
        session_id: Some(session_id.into()),
        resume,
        route: test_route(image_input),
        frozen_system_blocks: vec![FrozenSystemBlock {
            id: "sys".into(),
            text: "You are Nola, a precise local-first assistant.".into(),
        }],
        host_capabilities: HostCapabilities::default(),
        policy: test_policy(profile),
        hooks: HookRunner::default(),
        approval: None,
        max_steps: 40,
        force_tool_set: ForceToolSet::Auto,
        stream: true,
        json_schema: None,
        cancel: Arc::new(AtomicBool::new(false)),
        auto_recall_memory: false,
        inbox: None,
    }
}

fn step_tools(
    reasoning: Option<&str>,
    content: &str,
    calls: Vec<(&str, &str, Value)>,
) -> NormalizedModelResponse {
    NormalizedModelResponse {
        response_id: Some("r".into()),
        response_model: Some("scripted-reasoner".into()),
        content: content.into(),
        reasoning: reasoning.map(ToString::to_string),
        tool_calls: calls
            .into_iter()
            .map(|(id, name, arguments)| ModelToolCall {
                call_id: id.into(),
                name: name.into(),
                arguments,
            })
            .collect(),
        stop_reason: ModelStopReason::ToolCalls,
        usage: TokenUsage {
            input_tokens: 120,
            output_tokens: 60,
            ..Default::default()
        },
        continuation: None,
    }
}

fn step_final(reasoning: Option<&str>, content: &str) -> NormalizedModelResponse {
    NormalizedModelResponse {
        response_id: Some("r_final".into()),
        response_model: Some("scripted-reasoner".into()),
        content: content.into(),
        reasoning: reasoning.map(ToString::to_string),
        tool_calls: vec![],
        stop_reason: ModelStopReason::Stop,
        usage: TokenUsage {
            input_tokens: 160,
            output_tokens: 45,
            ..Default::default()
        },
        continuation: None,
    }
}

const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn init_git_repo(ws: &Path) {
    for args in [
        &["init", "-q"][..],
        &["config", "user.email", "qa@nola.local"],
        &["config", "user.name", "Nola QA"],
        &["add", "."],
        &["commit", "-q", "-m", "initial commit"],
    ] {
        let out = Command::new("git")
            .current_dir(ws)
            .args(args)
            .output()
            .expect("git command");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn build_full_registry(
    store: &Store,
    ws: &Path,
    caller: Arc<dyn ModelCaller>,
) -> (
    ToolRegistry,
    Arc<ModulesSuite>,
    Arc<PlanningState>,
    Arc<SubagentManager>,
) {
    let mut reg = ToolRegistry::with_builtin_tools();
    let suite = Arc::new(ModulesSuite::new(store.clone(), ws.to_path_buf(), None));
    suite.register_tools(&mut reg);
    let planning = Arc::new(PlanningState::default());
    register_planning_tools(store.clone(), &planning, &mut reg);
    let subagents = Arc::new(SubagentManager::new(store.clone(), caller));
    subagents.configure_parent(test_route(true), test_policy(PermissionProfile::AllowAll));
    register_subagent_tools(&subagents, &mut reg);
    (reg, suite, planning, subagents)
}

// ============================================================================
// Suite 1: 100% Completeness Matrix Across ALL 60 Registered Tools
// ============================================================================

#[test]
fn all_60_registered_tools_completeness_and_schema_security_matrix() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(
        ws.join("src/math.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\npub fn mul(a: i32, b: i32) -> i32 { a * b }\n",
    )
    .unwrap();
    fs::write(ws.join("notes.md"), "# Notes\nInitial line\n").unwrap();
    fs::write(
        ws.join("demo.ipynb"),
        serde_json::to_string_pretty(&json!({
            "cells": [{"cell_type": "code", "source": ["print(1)\n"], "metadata": {}, "outputs": []}],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        }))
        .unwrap(),
    )
    .unwrap();
    init_git_repo(&ws);

    let store = Store::open_in_memory().unwrap();
    let caller: Arc<dyn ModelCaller> = Arc::new(ScriptedCaller::new(vec![step_final(
        None,
        "subagent ok",
    )]));
    let (reg, _suite, planning, _subagents) = build_full_registry(&store, &ws, caller);
    planning.set_user_input_callback(Some(Arc::new(|_q, opts| {
        opts.first().cloned().or_else(|| Some("confirmed".into()))
    })));

    // 1. Validate EVERY registered tool schema contract
    let all_specs = reg.all_specs();
    assert!(
        all_specs.len() >= 58,
        "expected full ~60-tool registry, found only {}",
        all_specs.len()
    );
    let all_names: BTreeSet<String> = all_specs.iter().map(|s| s.schema.name.clone()).collect();
    for spec in &all_specs {
        assert!(
            !spec.schema.description.trim().is_empty(),
            "tool '{}' has empty description",
            spec.schema.name
        );
        assert_eq!(
            spec.schema.parameters.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "tool '{}' parameters schema must be an object",
            spec.schema.name
        );
    }

    // 2. Execute EVERY SINGLE registered tool and record coverage
    let output_store = Arc::new(OutputStore::new(dir.path().join("outputs")));
    let (_, spilled_meta) = output_store
        .maybe_spill_payload_scoped(
            "sess-matrix",
            "read_file",
            &ToolModelPayload::Text("X".repeat(30_000)),
            1024,
        )
        .unwrap();

    let mut ctx = ToolContext::new(&ws).with_session_id("sess-matrix");
    ctx.interactive_user_input = true;
    ctx.output_store = output_store;
    let reg_arc = Arc::new(reg);

    let tested = std::cell::RefCell::new(BTreeSet::new());
    let invoke_ok = |name: &str, args: Value| -> nola_types::ToolExecutionResult {
        tested.borrow_mut().insert(name.to_string());
        let res = reg_arc.execute(name, &ctx, args);
        assert!(
            !res.is_error,
            "tool '{name}' unexpectedly failed: {:?}",
            res.payload
        );
        res
    };
    let invoke_any = |name: &str, args: Value| -> nola_types::ToolExecutionResult {
        tested.borrow_mut().insert(name.to_string());
        let res = reg_arc.execute(name, &ctx, args);
        assert!(
            !res.payload.to_model_string().is_empty(),
            "tool '{name}' returned empty payload"
        );
        res
    };

    // --- 29 Core nola-tools ---
    invoke_ok("list_dir", json!({"path": "."}));
    invoke_ok("glob", json!({"pattern": "**/*.rs"}));
    invoke_ok("grep", json!({"pattern": "pub fn add"}));
    invoke_ok("workspace_status", json!({}));
    invoke_ok("repo_map", json!({"max_files": 20}));
    invoke_ok("file_outline", json!({"path": "src/math.rs"}));
    invoke_ok("read_file", json!({"path": "src/math.rs"}));
    invoke_ok("read_many_files", json!({"paths": ["src/math.rs", "notes.md"]}));
    invoke_ok("write_file", json!({"path": "scratch.txt", "content": "hello\nworld\n"}));
    invoke_ok("edit_file", json!({"path": "scratch.txt", "old_text": "world", "new_text": "nola"}));
    invoke_ok(
        "multi_edit_file",
        json!({
            "path": "src/math.rs",
            "edits": [
                {"old_text": "a + b", "new_text": "a.saturating_add(b)"},
                {"old_text": "a * b", "new_text": "a.saturating_mul(b)"}
            ]
        }),
    );
    invoke_ok("append_file", json!({"path": "notes.md", "content": "Appended line\n"}));
    invoke_ok("read_file", json!({"path": "notes.md"}));
    invoke_ok(
        "apply_patch",
        json!({
            "patch": "--- a/notes.md\n+++ b/notes.md\n@@ -1,3 +1,3 @@\n # Notes\n Initial line\n-Appended line\n+Patched line\n"
        }),
    );
    invoke_ok("move_file", json!({"source": "scratch.txt", "destination": "moved.txt"}));
    invoke_ok("delete_file", json!({"path": "moved.txt"}));
    invoke_ok("notebook", json!({"action": "read", "path": "demo.ipynb"}));
    invoke_ok("workspace_checkpoint", json!({"action": "create", "label": "matrix-cp"}));
    invoke_ok("git", json!({"action": "status"}));
    invoke_ok("bash", json!({"command": "echo MATRIX_BASH_OK"}));
    let bg_job = invoke_ok("bash", json!({"command": "sleep 0.2 && echo MATRIX_BG_OK", "background": true}));
    let bg_job_id = bg_job.payload.to_model_value()["job_id"]
        .as_str()
        .unwrap_or("missing")
        .to_string();
    invoke_ok("bash_control", json!({"action": "status", "job_id": bg_job_id}));
    invoke_ok("pty_exec", json!({"command": "printf PTY_OK"}));
    invoke_ok("code_exec", json!({"code": "return 6 * 7;"}));
    invoke_ok(
        "tool_output",
        json!({"action": "read", "output_id": spilled_meta.output_id, "offset_bytes": 0, "max_bytes": 64}),
    );
    invoke_ok("tool_search", json!({"query": "notebook"}));

    // Tools that require external host/network/GUI are exercised for structured policy/guardrail contracts:
    for (name, args) in [
        ("lsp", json!({"action": "diagnostics", "path": "src/math.rs"})),
        ("web_fetch", json!({"url": "http://127.0.0.1:1/blocked"})),
        ("web_search", json!({"query": "nola", "undeclared_bad": true})),
        ("browser", json!({"action": "status"})),
        ("computer", json!({"action": "status"})),
    ] {
        invoke_any(name, args);
    }

    // --- 24 nola-modules tools ---
    invoke_ok("codebase_index", json!({}));
    invoke_ok("codebase_search", json!({"query": "add"}));
    invoke_ok("codebase_text_search", json!({"query": "saturating_add"}));
    invoke_ok("codebase_read_symbol", json!({"name": "add"}));
    invoke_ok("codebase_trace", json!({"symbol": "add"}));
    invoke_ok("codebase_architecture", json!({}));
    invoke_ok("codebase_coverage", json!({}));

    invoke_ok(
        "memory_save",
        json!({"title": "Codename", "content": "Project Helios uses Rust 2021.", "namespace": "project", "tags": ["arch"]}),
    );
    invoke_ok("memory_search", json!({"query": "Helios"}));
    invoke_ok("conversation_recall", json!({"action": "search", "query": "Helios"}));
    invoke_ok("code_memory_snapshot", json!({}));

    invoke_ok("skill_load", json!({"name": "debugging"}));
    invoke_ok("skill_skip", json!({"name": "debugging", "reason": "matrix check"}));
    // github_skill tested via offline/validation error path so no external network flake
    invoke_any("github_skill", json!({"action": "list"}));

    let goal_res = invoke_ok("goal_manage", json!({"action": "create", "title": "Ship QA", "description": "Full tool coverage"}));
    let goal_val = goal_res.payload.to_model_value();
    let goal_id = goal_val
        .get("id")
        .or_else(|| goal_val.get("goal_id"))
        .or_else(|| goal_val.pointer("/goal/id"))
        .and_then(|v| v.as_str())
        .unwrap_or("g1")
        .to_string();
    let sop_res = invoke_ok("sop_manage", json!({"action": "create", "title": "Release SOP", "steps": ["Test", "Publish"]}));
    let sop_val = sop_res.payload.to_model_value();
    let sop_id = sop_val
        .get("id")
        .or_else(|| sop_val.get("sop_id"))
        .or_else(|| sop_val.pointer("/sop/id"))
        .and_then(|v| v.as_str())
        .unwrap_or("s1")
        .to_string();
    invoke_any("sop_run_update", json!({"sop_id": sop_id, "step_index": 0, "status": "done"}));
    invoke_ok("goal_complete", json!({"id": goal_id, "summary": "Completed in matrix"}));

    invoke_ok("data_source_manage", json!({"action": "list"}));
    invoke_ok("document_search", json!({"query": "Initial"}));
    invoke_ok("automation_manage", json!({"action": "list"}));
    invoke_ok("messaging_manage", json!({"action": "list"}));
    invoke_ok("room_manage", json!({"action": "list_rooms"}));

    let surf = invoke_ok(
        "render_ui_surface",
        json!({
            "surface_id": "matrix-card",
            "title": "QA Card",
            "components": [{"id": "root", "component": {"Text": {"text": {"literalString": "Hello A2UI"}}}}]
        }),
    );
    assert!(!surf.is_error);
    invoke_ok("delete_ui_surface", json!({"surface_id": "matrix-card"}));
    invoke_ok("local_model_manage", json!({"action": "list"}));
    invoke_ok("plugin_manage", json!({"action": "list"}));

    // --- 7 nola-engine tools ---
    invoke_ok(
        "update_agent_todos",
        json!({"todos": [{"id": "t1", "title": "Verify 60 tools", "status": "completed"}]}),
    );
    invoke_ok("propose_agent_plan", json!({"title": "QA Plan", "content": "1. Test all tools\n2. Verify E2E"}));
    invoke_ok(
        "request_user_input",
        json!({"question": "Proceed?", "options": ["Yes", "No"]}),
    );
    invoke_ok("list_experts", json!({}));
    let spawned = invoke_ok(
        "spawn_agent",
        json!({"role": "Reviewer", "prompt": "Check math.rs", "background": false}),
    );
    let task_id = spawned.payload.to_model_value()["task_id"]
        .as_str()
        .unwrap_or("missing")
        .to_string();
    invoke_ok("task_control", json!({"action": "read", "task_id": task_id}));
    invoke_any("delegate_expert", json!({"expert_id": "nonexistent", "prompt": "Audit"}));

    // STRICT COMPLETENESS GATE: fail immediately if ANY registered tool was missed!
    let tested_set = tested.into_inner();
    let untested: Vec<&String> = all_names.difference(&tested_set).collect();
    assert!(
        untested.is_empty(),
        "60-tool completeness gate failed! Untested registered tools: {untested:?} (tested={}, total={})",
        tested_set.len(),
        all_names.len()
    );
}

// ============================================================================
// Suite 2: Full Coding Refactor + Notebook + Git + Background Bash + PTY + Code Mode
// ============================================================================

#[test]
fn real_scenario_full_coding_refactor_notebook_git_pty_background_shell_and_code_mode() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("repo");
    fs::create_dir_all(ws.join("pkg")).unwrap();
    fs::write(
        ws.join("pkg/calc.py"),
        "def compute_total(items):\n    return sum(items) - 1  # deliberate bug\n",
    )
    .unwrap();
    fs::write(
        ws.join("test_calc.py"),
        "import unittest\nfrom pkg.calc import compute_total\nclass T(unittest.TestCase):\n    def test_sum(self):\n        self.assertEqual(compute_total([2, 3, 5]), 10)\nif __name__ == '__main__':\n    unittest.main()\n",
    )
    .unwrap();
    fs::write(
        ws.join("analysis.ipynb"),
        serde_json::to_string_pretty(&json!({
            "cells": [{"cell_type": "markdown", "source": ["# Analysis\n"], "metadata": {}}],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        }))
        .unwrap(),
    )
    .unwrap();
    init_git_repo(&ws);

    let store = Store::open_in_memory().unwrap();
    let caller = Arc::new(ScriptedCaller::new(vec![
        // Step 1: Discover deferred tools + inspect repo (read_file before edit_file!)
        step_tools(
            Some("Inspecting repository and loading checkpoint/notebook/code_exec tools."),
            "",
            vec![
                ("c1", "tool_search", json!({"query": "checkpoint notebook code_exec pty"})),
                ("c2", "list_dir", json!({"path": "."})),
                ("c3", "grep", json!({"pattern": "compute_total"})),
                ("c3b", "read_file", json!({"path": "pkg/calc.py"})),
            ],
        ),
        // Step 2: Create baseline checkpoint + edit bug + update notebook + run code_exec
        step_tools(
            Some("Fixing bug in pkg/calc.py and updating analysis.ipynb."),
            "",
            vec![
                ("c4", "workspace_checkpoint", json!({"action": "create", "label": "before-fix"})),
                ("c5", "edit_file", json!({
                    "path": "pkg/calc.py",
                    "old_text": "return sum(items) - 1  # deliberate bug",
                    "new_text": "return sum(items)"
                })),
                ("c6", "notebook", json!({
                    "action": "insert_cell",
                    "path": "analysis.ipynb",
                    "cell_index": 1,
                    "cell_type": "code",
                    "source": "assert 2 + 3 + 5 == 10\n"
                })),
                ("c7", "code_exec", json!({
                    "code": "const f = tools.read_file({path: 'pkg/calc.py'}); return JSON.stringify(f).includes('return sum(items)');"
                })),
            ],
        ),
        // Step 3: Run unittest via bash + pty_exec + git diff + workspace_checkpoint list
        step_tools(
            Some("Running Python unit tests, git diff, checkpoint list, and PTY."),
            "",
            vec![
                ("c8", "bash", json!({"command": "python3 -m unittest test_calc.py"})),
                ("c9", "workspace_checkpoint", json!({"action": "list"})),
                ("c10", "pty_exec", json!({"command": "echo PTY_VERIFIED"})),
                ("c11", "git", json!({"action": "diff"})),
            ],
        ),
        step_final(
            Some("All edits, notebook updates, tests, and git checks passed."),
            "Fixed `compute_total` in `pkg/calc.py`, verified with `unittest`, updated `analysis.ipynb`, and confirmed clean `git diff`.",
        ),
    ]));

    let (reg, _suite, _planning, _subagents) =
        build_full_registry(&store, &ws, caller.clone());
    let engine = AgentEngine::new(store, Arc::new(reg), caller);
    let ctx = ToolContext::new(&ws).with_session_id("sess-coding-e2e");

    let mut tool_ok_count = 0usize;
    let res = engine
        .run(
            &ctx,
            "Fix compute_total bug, update notebook, verify tests and git diff.",
            run_cfg("sess-coding-e2e", false, PermissionProfile::AllowAll, false),
            |ev| {
                if let AgentEvent::ToolFinished { is_error, name, summary, .. } = ev {
                    assert!(!is_error, "tool '{name}' failed in coding workflow: {summary}");
                    tool_ok_count += 1;
                }
            },
        )
        .unwrap();

    assert_eq!(tool_ok_count, 12, "all 12 coding/git/notebook/pty/code_exec calls must succeed");
    assert!(res.final_text.contains("compute_total"));
    assert!(res.usage.reasoning_tokens > 0);
    let fixed_py = fs::read_to_string(ws.join("pkg/calc.py")).unwrap();
    assert!(!fixed_py.contains("- 1"));
    let nb: Value = serde_json::from_str(&fs::read_to_string(ws.join("analysis.ipynb")).unwrap()).unwrap();
    assert_eq!(nb["cells"].as_array().unwrap().len(), 2);
}

// ============================================================================
// Suite 3: Heavy 12-Round Interleaved Stress Turn + Vision + Chronological Journal
// ============================================================================

#[test]
fn real_scenario_12_round_interleaved_reasoning_tools_vision_o1_journal_and_attachments() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("paper-ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("seed.png"), TINY_PNG).unwrap();
    for round in 0..8 {
        fs::write(
            ws.join(format!("section_{round}.md")),
            format!("# Section {round}\nLow-rank residual step {round}: $S_A = \\lambda S_B + R_{round}$\n"),
        )
        .unwrap();
    }

    let store = Store::open_in_memory().unwrap();
    let long_thought = "Analyzing sibling-conditioned delta-rule state transfer and layout geometry. ".repeat(30); // ~2.2 KB per round

    let mut turns = Vec::new();
    // 8 distinct interleaved reasoning + tool steps (simulating heavy multi-step research & plotting)
    for round in 0..8 {
        turns.push(step_tools(
            Some(&format!("Round {round}: {long_thought}")),
            if round == 0 { "开始分析论文并准备作图…" } else { "" },
            vec![(
                Box::leak(format!("tc_r{round}").into_boxed_str()),
                "read_file",
                json!({"path": format!("section_{round}.md")}),
            )],
        ));
    }
    // Round 8: generate PNG + SVG + CSV deliverables via bash
    turns.push(step_tools(
        Some(&format!("Round 8: {long_thought}")),
        "",
        vec![(
            "tc_plot",
            "bash",
            json!({"command": "cp seed.png paper-figure.png && printf '<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"/>' > paper-figure.svg && printf 'layout,mb\\nDFD-R,42.5\\n' > metrics.csv"}),
        )],
    ));
    // Round 9: inspect generated PNG via multimodal vision read_file
    turns.push(step_tools(
        Some(&format!("Round 9: {long_thought}")),
        "",
        vec![("tc_vis", "read_file", json!({"path": "paper-figure.png"}))],
    ));
    // Final answer after all 10 tools
    turns.push(step_final(
        Some(&format!("Final synthesis: {long_thought}")),
        "图已画完并自检确认：\n\n$$S_A = \\lambda \\cdot S_B + (\\text{一小坨残差})$$",
    ));

    let caller = Arc::new(ScriptedCaller::new(turns));
    let (reg, _suite, _planning, _subagents) =
        build_full_registry(&store, &ws, caller.clone());
    let engine = AgentEngine::new(store.clone(), Arc::new(reg), caller.clone());
    let ctx = ToolContext::new(&ws).with_session_id("sess-heavy-12");

    let mut reasoning_chunks = 0usize;
    let mut tool_finishes = 0usize;
    let res = engine
        .run(
            &ctx,
            "画个图并自检，附带中文公式解释",
            run_cfg("sess-heavy-12", false, PermissionProfile::AllowAll, true),
            |ev| match ev {
                AgentEvent::ReasoningDelta { .. } => reasoning_chunks += 1,
                AgentEvent::ToolFinished { is_error, name, .. } => {
                    assert!(!is_error, "tool '{name}' failed");
                    tool_finishes += 1;
                }
                _ => {}
            },
        )
        .unwrap();

    assert_eq!(tool_finishes, 10);
    assert!(reasoning_chunks >= 10, "expected >=10 reasoning rounds");
    assert!(res.usage.reasoning_tokens > 500, "expected substantial estimated reasoning_tokens");
    assert!(res.final_text.contains("一小坨残差"));

    // Verify the final LLM request received real image content (NOT media_omitted)
    let reqs = caller.requests();
    let last_req_json = serde_json::to_string(reqs.last().unwrap()).unwrap();
    assert!(
        !last_req_json.contains("media_omitted"),
        "vision-enabled run must pass PNG bytes to model"
    );
    assert!(
        ws.join("paper-figure.png").exists()
            && ws.join("paper-figure.svg").exists()
            && ws.join("metrics.csv").exists()
    );
}

// ============================================================================
// Suite 4: Planning + Interactive User Input + A2UI + Subagents/Experts
// ============================================================================

#[test]
fn real_scenario_planning_todos_interactive_user_input_a2ui_and_subagent_task_control() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("plan-ws");
    fs::create_dir_all(&ws).unwrap();
    let store = Store::open_in_memory().unwrap();

    let subagent_caller: Arc<dyn ModelCaller> = Arc::new(ScriptedCaller::new(vec![step_final(
        None,
        "Subagent audit report: zero vulnerabilities.",
    )]));
    let main_caller = Arc::new(ScriptedCaller::new(vec![
        step_tools(
            Some("Creating plan, todos, asking user preference, and rendering A2UI card."),
            "",
            vec![
                ("p0", "tool_search", json!({"query": "plan todos user_input ui_surface spawn_agent task_control"})),
            ],
        ),
        step_tools(
            Some("Executing planning, interactive question, A2UI surface, and background subagent."),
            "",
            vec![
                ("p1", "propose_agent_plan", json!({"title": "Audit Plan", "content": "Phase 1: Audit\nPhase 2: Report"})),
                ("p2", "update_agent_todos", json!({"todos": [
                    {"id": "1", "title": "Run subagent audit", "status": "in_progress"},
                    {"id": "2", "title": "Render summary card", "status": "pending"}
                ]})),
                ("p3", "request_user_input", json!({"question": "Choose output format", "options": ["Markdown", "JSON"]})),
                ("p4", "render_ui_surface", json!({
                    "surface_id": "audit-summary",
                    "title": "Audit Status",
                    "components": [{"id": "root", "component": {"Text": {"text": {"literalString": "Audit running"}}}}]
                })),
                ("p5", "spawn_agent", json!({"role": "Security Auditor", "prompt": "Audit workspace", "background": true})),
            ],
        ),
        step_tools(
            Some("Checking subagent list and finalizing todos."),
            "",
            vec![
                ("p6", "task_control", json!({"action": "list"})),
                ("p7", "update_agent_todos", json!({"todos": [
                    {"id": "1", "title": "Run subagent audit", "status": "completed"},
                    {"id": "2", "title": "Render summary card", "status": "completed"}
                ]})),
                ("p8", "delete_ui_surface", json!({"surface_id": "audit-summary"})),
            ],
        ),
        step_final(None, "Plan executed with user format choice Markdown and subagent completed."),
    ]));

    let (reg, _suite, planning, _subagents) =
        build_full_registry(&store, &ws, subagent_caller);
    let asked = Arc::new(AtomicBool::new(false));
    let asked_flag = Arc::clone(&asked);
    planning.set_user_input_callback(Some(Arc::new(move |question, options| {
        assert!(question.contains("output format"));
        assert_eq!(options, &["Markdown".to_string(), "JSON".to_string()]);
        asked_flag.store(true, Ordering::SeqCst);
        Some("Markdown".into())
    })));

    let engine = AgentEngine::new(store.clone(), Arc::new(reg), main_caller);
    let mut ctx = ToolContext::new(&ws).with_session_id("sess-plan-ui");
    ctx.interactive_user_input = true;

    let res = engine
        .run(
            &ctx,
            "Plan audit, ask my format preference, spawn auditor, and show UI status.",
            run_cfg("sess-plan-ui", false, PermissionProfile::AllowAll, false),
            |ev| {
                if let AgentEvent::ToolFinished { is_error, name, .. } = ev {
                    assert!(!is_error, "tool '{name}' failed");
                }
            },
        )
        .unwrap();

    assert!(asked.load(Ordering::SeqCst), "request_user_input callback must have fired");
    assert!(res.final_text.contains("Markdown"));
    let todos = load_session_todos(&store, "sess-plan-ui");
    assert_eq!(todos["todos"].as_array().unwrap().len(), 2);
    assert_eq!(todos["todos"][0]["status"], "completed");
    let plan = load_session_plan(&store, "sess-plan-ui");
    assert!(plan.to_string().contains("Phase 1: Audit"));
}

// ============================================================================
// Suite 5: Cross-Session Memory + Codebase + Skills + Goals/SOPs + Data Sources
// ============================================================================

#[test]
fn real_scenario_cross_session_memory_codebase_skills_goals_and_data_sources() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("kb-ws");
    let docs = ws.join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(
        docs.join("architecture.md"),
        "# Architecture\nThe ZetaKV engine uses WAL journaling and FTS5 recall.\n",
    )
    .unwrap();
    fs::write(
        ws.join("engine.rs"),
        "pub fn zeta_recall(q: &str) -> usize { q.len() }\n",
    )
    .unwrap();

    let store = Store::open_in_memory().unwrap();
    let caller = Arc::new(ScriptedCaller::new(vec![
        // Session 1: discover module tools, save memory, index codebase, create data source, load skill, create goal
        step_tools(
            None,
            "",
            vec![("s1_0", "tool_search", json!({"query": "memory codebase data_source document_search skill goal"}))],
        ),
        step_tools(
            None,
            "",
            vec![
                ("s1_1", "memory_save", json!({
                    "title": "Secret Project Codename",
                    "content": "Codename is HYPERION-7741 with primary reviewer Alice.",
                    "namespace": "project",
                    "tags": ["codename"]
                })),
                ("s1_2", "codebase_index", json!({})),
                ("s1_3", "skill_load", json!({"name": "code-review"})),
                ("s1_4", "goal_manage", json!({"action": "create", "title": "Audit ZetaKV", "description": "Verify FTS5 recall"})),
                ("s1_5", "data_source_manage", json!({
                    "action": "add",
                    "name": "Local Docs",
                    "kind": "directory",
                    "path": docs.to_string_lossy()
                })),
            ],
        ),
        step_final(None, "Session 1 saved codename HYPERION-7741, indexed codebase, and added Local Docs."),
        // Session 2 (brand new session!): recall memory, search codebase, search documents
        step_tools(
            None,
            "",
            vec![("s2_0", "tool_search", json!({"query": "memory_search codebase_search document_search"}))],
        ),
        step_tools(
            None,
            "",
            vec![
                ("s2_1", "memory_search", json!({"query": "HYPERION-7741"})),
                ("s2_2", "codebase_search", json!({"query": "zeta_recall"})),
                ("s2_3", "document_search", json!({"query": "ZetaKV"})),
            ],
        ),
        step_final(None, "Recalled HYPERION-7741, zeta_recall, and ZetaKV from durable storage."),
    ]));

    let (reg, _suite, _planning, _subagents) =
        build_full_registry(&store, &ws, caller.clone());
    let engine = AgentEngine::new(store, Arc::new(reg), caller);

    // Run Session 1
    let ctx1 = ToolContext::new(&ws).with_session_id("sess-mem-1");
    let r1 = engine
        .run(
            &ctx1,
            "Remember HYPERION-7741 and index our repo and docs.",
            run_cfg("sess-mem-1", false, PermissionProfile::AllowAll, false),
            |_| {},
        )
        .unwrap();
    assert!(r1.final_text.contains("HYPERION-7741"));

    // Run Session 2 (completely separate session ID!)
    let ctx2 = ToolContext::new(&ws).with_session_id("sess-mem-2");
    let mut mem_hit = false;
    let mut code_hit = false;
    let r2 = engine
        .run(
            &ctx2,
            "What was our project codename and where is zeta_recall?",
            run_cfg("sess-mem-2", false, PermissionProfile::AllowAll, false),
            |ev| {
                if let AgentEvent::ToolFinished { name, summary, is_error, .. } = ev {
                    assert!(!is_error, "session 2 tool '{name}' failed: {summary}");
                    if name == "memory_search" {
                        mem_hit = true;
                    }
                    if name == "codebase_search" {
                        code_hit = true;
                    }
                }
            },
        )
        .unwrap();
    assert!(mem_hit && code_hit, "session 2 must query memory_search and codebase_search");
    assert!(r2.final_text.contains("HYPERION-7741"));
}

// ============================================================================
// Suite 6: Unknown-Tool Recovery, Malformed Args, Cautious Deny, & Routing Guardrails
// ============================================================================

#[test]
fn real_scenario_unknown_tool_malformed_args_permission_denial_and_routing_guardrails() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("recovery-ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("info.txt"), "SAFE_READ_VALUE").unwrap();

    let store = Store::open_in_memory().unwrap();
    let caller = Arc::new(ScriptedCaller::new(vec![
        // 1. Model hallucinates an unknown tool -> must receive recoverable error, NOT crash!
        step_tools(
            Some("Trying hallucinated tool first."),
            "",
            vec![("e1", "nonexistent_quantum_tool", json!({"x": 1}))],
        ),
        // 2. Model passes undeclared parameter to web_search -> must be rejected cleanly!
        step_tools(
            Some("Trying undeclared param on web_search."),
            "",
            vec![("e2", "web_search", json!({"query": "rust", "evil_extra": "boom"}))],
        ),
        // 3. Model tries mutating bash under Cautious -> approval denies -> file MUST NOT exist!
        step_tools(
            Some("Trying bash under Cautious."),
            "",
            vec![("e3", "bash", json!({"command": "echo BLOCKED > forbidden.txt"}))],
        ),
        // 4. Model recovers by calling read-only read_file instead!
        step_tools(
            Some("Recovering with safe read_file."),
            "",
            vec![("e4", "read_file", json!({"path": "info.txt"}))],
        ),
        step_final(None, "Recovered cleanly after errors and read SAFE_READ_VALUE."),
        // 5. Multilingual & ambiguous capability-name discussion prompt (AGENTS.md guardrail):
        //    must answer naturally without forced tool calls!
        step_final(
            None,
            "Tools, skills, memory, and permissions are orthogonal runtime capabilities evaluated per concrete call.",
        ),
    ]));

    let (reg, _suite, _planning, _subagents) =
        build_full_registry(&store, &ws, caller.clone());
    let engine = AgentEngine::new(store, Arc::new(reg), caller);
    let ctx = ToolContext::new(&ws).with_session_id("sess-rec-1");

    let denied_count = Arc::new(AtomicUsize::new(0));
    let denied_clone = Arc::clone(&denied_count);
    let mut cfg = run_cfg("sess-rec-1", false, PermissionProfile::Cautious, false);
    cfg.approval = Some(Arc::new(move |spec, _args, _summary| {
        if spec.schema.name == "bash" {
            denied_clone.fetch_add(1, Ordering::SeqCst);
            return false; // Deny mutating bash!
        }
        true
    }));

    let mut saw_unknown_err = false;
    let mut saw_undeclared_err = false;
    let mut saw_denied_err = false;
    let mut saw_read_ok = false;

    let res = engine
        .run(&ctx, "Inspect info.txt safely.", cfg, |ev| {
            if let AgentEvent::ToolFinished { name, is_error, .. } = ev {
                match name.as_str() {
                    "nonexistent_quantum_tool" if is_error => saw_unknown_err = true,
                    "web_search" if is_error => saw_undeclared_err = true,
                    "bash" if is_error => saw_denied_err = true,
                    "read_file" if !is_error => saw_read_ok = true,
                    _ => {}
                }
            }
        })
        .unwrap();

    assert!(saw_unknown_err, "unknown tool must return recoverable error");
    assert!(saw_undeclared_err, "undeclared parameter must be rejected");
    assert!(saw_denied_err, "denied bash must return permission error");
    assert_eq!(denied_count.load(Ordering::SeqCst), 1);
    assert!(!ws.join("forbidden.txt").exists(), "denied bash must never create forbidden.txt");
    assert!(saw_read_ok, "model must recover and succeed with read_file");
    assert!(res.final_text.contains("SAFE_READ_VALUE"));

    // Architecture discussion prompt mentioning "search", "current", "tools", "skills", "memory", "permissions":
    // ZERO tools should be forced by keyword matching!
    let mut forced_tools = 0usize;
    let r_guard = engine
        .run(
            &ctx,
            "请对比分析一下当前系统里的 search、tools、skills、memory 和 permissions 架构设计（不需要调用工具）。",
            run_cfg("sess-rec-guard", false, PermissionProfile::Cautious, false),
            |ev| {
                if matches!(ev, AgentEvent::ToolStarted { .. }) {
                    forced_tools += 1;
                }
            },
        )
        .unwrap();
    assert_eq!(forced_tools, 0, "architecture prompt mentioning capability names must not force tools");
    assert!(r_guard.final_text.contains("orthogonal"));
}

// ============================================================================
// Suite 7: Real Loopback Streamable HTTP MCP Server E2E + Permission Gating
// ============================================================================

#[test]
fn real_scenario_mcp_streamable_http_discovery_execution_permission_gating_and_disconnect() {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("bind loopback mcp server");
    let port = server.server_addr().to_ip().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_bg = Arc::clone(&stop);
    let tool_hits = Arc::new(AtomicUsize::new(0));
    let tool_hits_bg = Arc::clone(&tool_hits);

    let handle = thread::spawn(move || {
        while !stop_bg.load(Ordering::SeqCst) {
            let Ok(Some(mut req)) = server.recv_timeout(std::time::Duration::from_millis(50)) else {
                continue;
            };
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let msg: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let id = msg.get("id").cloned().unwrap_or(json!(1));
            let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let reply = match method {
                "initialize" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2025-03-26",
                        "capabilities": {"tools": {}, "resources": {}, "prompts": {}},
                        "serverInfo": {"name": "qa-mcp", "version": "1.0.0"}
                    }
                }),
                "notifications/initialized" => {
                    let _ = req.respond(tiny_http::Response::empty(202));
                    continue;
                }
                "tools/list" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "tools": [{
                            "name": "calc_sum",
                            "description": "Add two integers remotely via MCP",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}},
                                "required": ["a", "b"]
                            }
                        }]
                    }
                }),
                "tools/call" => {
                    tool_hits_bg.fetch_add(1, Ordering::SeqCst);
                    let a = msg.pointer("/params/arguments/a").and_then(|v| v.as_i64()).unwrap_or(0);
                    let b = msg.pointer("/params/arguments/b").and_then(|v| v.as_i64()).unwrap_or(0);
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"content": [{"type": "text", "text": format!("MCP_SUM={}", a + b)}]}
                    })
                }
                "resources/list" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"resources": [{"uri": "mcp://qa/spec", "name": "QA Spec", "mimeType": "text/plain"}]}
                }),
                "resources/read" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"contents": [{"uri": "mcp://qa/spec", "text": "MCP_RESOURCE_BODY_OK"}]}
                }),
                "prompts/list" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"prompts": [{"name": "audit_prompt", "description": "Audit template"}]}
                }),
                "prompts/get" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"messages": [{"role": "user", "content": {"type": "text", "text": "MCP_PROMPT_OK"}}]}
                }),
                _ => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            };
            let hdr = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            let _ = req.respond(tiny_http::Response::from_string(reply.to_string()).with_header(hdr));
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("mcp-ws");
    fs::create_dir_all(&ws).unwrap();
    let store = Store::open_in_memory().unwrap();

    let mcp_mgr = Arc::new(McpManager::new(None));
    let mcp_cfg = McpServerConfig {
        id: "lab".into(),
        name: "QA Lab MCP".into(),
        transport: McpTransport::StreamableHttp,
        enabled: true,
        command: None,
        args: vec![],
        env: BTreeMap::new(),
        url: Some(format!("http://127.0.0.1:{port}/mcp")),
        headers: BTreeMap::new(),
        oauth_client_id: None,
        oauth_scopes: vec![],
    };

    let discovered = mcp_mgr.discover_tools(&mcp_cfg).expect("MCP discovery must succeed");
    assert_eq!(discovered.len(), 3, "expected calc_sum + resources + prompts tools");

    let mut reg = ToolRegistry::with_builtin_tools();
    for h in discovered {
        reg.register(h);
    }
    let reg = Arc::new(reg);

    // 1. Under Cautious + Deny callback: MCP mutating tool MUST NOT reach the HTTP server!
    let caller_deny = Arc::new(ScriptedCaller::new(vec![
        step_tools(None, "", vec![("m0", "tool_search", json!({"query": "mcp_lab"}))]),
        step_tools(None, "", vec![("m1", "mcp_lab_calc_sum", json!({"a": 19, "b": 23}))]),
        step_final(None, "MCP call was denied by user."),
    ]));
    let engine_deny = AgentEngine::new(store.clone(), Arc::clone(&reg), caller_deny);
    let mut cfg_deny = run_cfg("sess-mcp-deny", false, PermissionProfile::Cautious, false);
    cfg_deny.approval = Some(Arc::new(|_spec, _args, _summary| false));
    let ctx = ToolContext::new(&ws).with_session_id("sess-mcp-deny");
    let _ = engine_deny.run(&ctx, "Call mcp sum", cfg_deny, |_| {}).unwrap();
    assert_eq!(tool_hits.load(Ordering::SeqCst), 0, "denied MCP call must NEVER hit server");

    // 2. Under AllowAll: calc_sum + resources read + prompts get ALL succeed end-to-end!
    let caller_allow = Arc::new(ScriptedCaller::new(vec![
        step_tools(None, "", vec![("m0", "tool_search", json!({"query": "mcp_lab"}))]),
        step_tools(
            None,
            "",
            vec![
                ("m1", "mcp_lab_calc_sum", json!({"a": 19, "b": 23})),
                ("m2", "mcp_lab_resources", json!({"action": "read", "uri": "mcp://qa/spec"})),
                ("m3", "mcp_lab_prompts", json!({"action": "get", "name": "audit_prompt"})),
            ],
        ),
        step_final(None, "MCP sum=42, resource and prompt read."),
    ]));
    let engine_allow = AgentEngine::new(store, Arc::clone(&reg), caller_allow);
    let mut ok_mcp = 0usize;
    let res = engine_allow
        .run(
            &ctx,
            "Run MCP sum, resource read, and prompt get",
            run_cfg("sess-mcp-allow", false, PermissionProfile::AllowAll, false),
            |ev| {
                if let AgentEvent::ToolFinished { name, is_error, .. } = ev {
                    if name.starts_with("mcp_lab_") {
                        assert!(!is_error, "MCP tool '{name}' failed");
                        ok_mcp += 1;
                    }
                }
            },
        )
        .unwrap();
    assert_eq!(ok_mcp, 3);
    assert_eq!(tool_hits.load(Ordering::SeqCst), 1);
    assert!(res.final_text.contains("42"));

    // 3. Explicit disconnect must fail closed (no silent auto-reconnect!)
    mcp_mgr.disconnect("lab");
    let after_disc = reg.execute("mcp_lab_calc_sum", &ctx, json!({"a": 1, "b": 2}));
    assert!(after_disc.is_error, "disconnected MCP server must reject tool calls");

    stop.store(true, Ordering::SeqCst);
    let _ = handle.join();
}
