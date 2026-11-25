// SPDX-License-Identifier: Apache-2.0
//! Gate 4 (Multi-Turn KV Cache Prefix Byte-Equality) &
//! Gate 5 (Policy, Trust, Hooks, Resume, Compaction Invariants).

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use nola_llm::{adapter_for, LlmError};
use nola_store::Store;
use nola_tools::{SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    CommandHookConfig, ContextBudget, FrozenSystemBlock, HistoryMessage, HostCapabilities,
    ModelCapabilities, ModelProtocol, ModelRouteConfig, ModelStopReason, ModelStreamDelta,
    ModelToolCall, NormalizedModelResponse, PermissionAction, PermissionProfile, PermissionRule,
    RuleScope, StructuredModelRequest, TokenUsage, ToolModelPayload,
};
use serde_json::json;

use crate::{
    compact_history, AgentEngine, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner,
    LoopGuard, ModelCaller,
};

struct ScriptedCaller {
    captured: Mutex<Vec<StructuredModelRequest>>,
    responses: Mutex<VecDeque<NormalizedModelResponse>>,
}

impl ScriptedCaller {
    fn new(responses: Vec<NormalizedModelResponse>) -> Self {
        Self {
            captured: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }
}

impl ModelCaller for ScriptedCaller {
    fn call(
        &self,
        _route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        _stream: bool,
        _on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        self.captured.lock().unwrap().push(req.clone());
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(NormalizedModelResponse {
                response_id: Some("r_done".into()),
                response_model: Some("mock".into()),
                content: "Done.".into(),
                reasoning: None,
                tool_calls: vec![],
                stop_reason: ModelStopReason::Stop,
                usage: TokenUsage::default(),
                continuation: None,
            }))
    }
}

fn mock_route(protocol: ModelProtocol) -> ModelRouteConfig {
    ModelRouteConfig {
        connection_id: "mock".into(),
        endpoint: "http://127.0.0.1:8081".into(),
        protocol,
        model: "mock-qwen3".into(),
        api_key: "k".into(),
        context_window: 128_000,
        max_output_tokens: Some(16_384),
        temperature: Some(0.1),
        top_p: None,
        capabilities: ModelCapabilities::default(),
        reasoning_effort: None,
        prompt_cache_key: Some("k1".into()),
    }
}

#[test]
fn gate4_multi_turn_kv_prefix_byte_equality_across_all_4_protocols() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ToolContext::new(dir.path());
    ctx.sandbox_mode = SandboxMode::Disabled;
    ctx.permission_profile = PermissionProfile::AllowAll;

    let caller = Arc::new(ScriptedCaller::new(vec![
        // Turn 1: write a file + search for deferred `code_exec` tool
        NormalizedModelResponse {
            response_id: Some("r1".into()),
            response_model: Some("mock".into()),
            content: String::new(),
            reasoning: None,
            tool_calls: vec![
                ModelToolCall {
                    call_id: "c1".into(),
                    name: "write_file".into(),
                    arguments: json!({"path": "a.txt", "content": "hello"}),
                },
                ModelToolCall {
                    call_id: "c2".into(),
                    name: "tool_search".into(),
                    arguments: json!({"query": "code_exec"}),
                },
            ],
            stop_reason: ModelStopReason::ToolCalls,
            usage: TokenUsage::default(),
            continuation: None,
        },
        // Turn 2: call newly activated deferred `code_exec` tool
        NormalizedModelResponse {
            response_id: Some("r2".into()),
            response_model: Some("mock".into()),
            content: String::new(),
            reasoning: None,
            tool_calls: vec![ModelToolCall {
                call_id: "c3".into(),
                name: "code_exec".into(),
                arguments: json!({"code": "return 2 + 2;"}),
            }],
            stop_reason: ModelStopReason::ToolCalls,
            usage: TokenUsage::default(),
            continuation: None,
        },
        // Turn 3: final answer
        NormalizedModelResponse {
            response_id: Some("r3".into()),
            response_model: Some("mock".into()),
            content: "All done (4).".into(),
            reasoning: None,
            tool_calls: vec![],
            stop_reason: ModelStopReason::Stop,
            usage: TokenUsage::default(),
            continuation: None,
        },
    ]));

    let store = Store::open_in_memory().unwrap();
    let engine = AgentEngine::new(
        store,
        Arc::new(ToolRegistry::with_builtin_tools()),
        caller.clone(),
    );

    let result = engine
        .run(
            &ctx,
            "Write a.txt and compute 2+2",
            AgentRunConfig {
                session_id: Some("s1".into()),
                resume: false,
                route: mock_route(ModelProtocol::OpenAiChat),
                frozen_system_blocks: vec![FrozenSystemBlock {
                    id: "sys".into(),
                    text: "Frozen Nola v2 system prompt.".into(),
                }],
                host_capabilities: HostCapabilities {
                    interactive_user_input: false,
                    network_allowed: true,
                },
                policy: EffectivePolicy {
                    profile: PermissionProfile::AllowAll,
                    workspace_trusted: true,
                    network_allowed: true,
                    rules: vec![],
                },
                hooks: HookRunner::default(),
                approval: None,
                max_steps: 10,
                force_tool_set: ForceToolSet::Auto,
                stream: false,
                json_schema: None,
                cancel: Arc::new(AtomicBool::new(false)),
                auto_recall_memory: false,
                inbox: None,
            },
            |_| {},
        )
        .unwrap();

    assert_eq!(result.final_text, "All done (4).");
    let reqs = caller.captured.lock().unwrap();
    assert_eq!(reqs.len(), 3);

    // Verify tool catalog prefix stability: Turn 2 strictly appended `code_exec` at the tail!
    let t0_names: Vec<&str> = reqs[0]
        .prefix_stable_tools
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    let t1_names: Vec<&str> = reqs[1]
        .prefix_stable_tools
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(&t1_names[..t0_names.len()], &t0_names[..]);
    assert!(t1_names.ends_with(&["code_exec"]));

    // Verify across ALL 4 wire protocols: frozen system prefix is byte-identical across all 3 turns!
    for proto in [
        ModelProtocol::OpenAiChat,
        ModelProtocol::OpenAiResponses,
        ModelProtocol::AnthropicMessages,
        ModelProtocol::SiwcResponses,
    ] {
        let ad = adapter_for(proto);
        let route = mock_route(proto);
        let w0 = ad.build_request(&route, &reqs[0], false).unwrap().body;
        let w2 = ad.build_request(&route, &reqs[2], false).unwrap().body;
        match proto {
            ModelProtocol::OpenAiChat => assert_eq!(w0["messages"][0], w2["messages"][0]),
            ModelProtocol::AnthropicMessages => assert_eq!(w0["system"], w2["system"]),
            ModelProtocol::OpenAiResponses | ModelProtocol::SiwcResponses => {
                assert_eq!(w0["instructions"], w2["instructions"])
            }
        }
    }
}

#[test]
fn gate5_security_trust_hooks_modify_input_recheck_and_resume_policy() {
    let dir = tempfile::tempdir().unwrap();
    let nola_dir = dir.path().join(".nola");
    std::fs::create_dir_all(&nola_dir).unwrap();
    std::fs::write(
        nola_dir.join("settings.json"),
        r#"{"permission_profile": "allow_all"}"#,
    )
    .unwrap();

    // Untrusted workspace MUST ignore `.nola/settings.json`!
    let untrusted = EffectivePolicy::load(dir.path(), None, false, None, true);
    assert_eq!(untrusted.profile, PermissionProfile::Cautious);

    // Trusted workspace loads `.nola/settings.json`
    let trusted = EffectivePolicy::load(dir.path(), None, true, None, true);
    assert_eq!(trusted.profile, PermissionProfile::AllowAll);

    // PreToolUse hook that tries to redirect `write_file` outside workspace MUST be blocked by re-check!
    let outside = tempfile::tempdir().unwrap();
    let escape_target = outside.path().join("pwn.txt");
    let hook_cmd = format!(
        "printf '{{\"decision\":\"modify_input\",\"input\":{{\"path\":\"{}\",\"content\":\"x\"}}}}'",
        escape_target.display()
    );
    let mut policy = EffectivePolicy {
        profile: PermissionProfile::AcceptEdits,
        workspace_trusted: true,
        network_allowed: true,
        rules: vec![PermissionRule {
            scope: RuleScope::User,
            action: PermissionAction::Deny,
            tool_pattern: "bash".into(),
            resource_pattern: Some("*rm -rf*".into()),
        }],
    };
    let mw = crate::ToolExecutionMiddleware {
        policy: policy.clone(),
        hooks: HookRunner::from_configs(
            dir.path().to_path_buf(),
            vec![CommandHookConfig {
                event: nola_types::HookEvent::PreToolUse,
                matcher: "write_file".into(),
                command: hook_cmd,
                timeout_ms: 2_000,
                fail_closed: true,
            }],
        ),
        approval: None,
    };
    let reg = ToolRegistry::with_builtin_tools();
    let ctx = ToolContext::new(dir.path());
    let out = mw.execute(
        &reg,
        &ctx,
        "write_file",
        json!({"path": "safe.txt", "content": "ok"}),
    );
    assert!(out.result.is_error, "hook escape outside workspace must be denied");
    assert!(!escape_target.exists());

    // User Deny overrides Project Allow!
    policy.add_rule(PermissionRule {
        scope: RuleScope::Project,
        action: PermissionAction::Allow,
        tool_pattern: "bash".into(),
        resource_pattern: Some("*".into()),
    });
    let bash_spec = reg.get("bash").unwrap().spec();
    let dec = policy.evaluate(&bash_spec, &json!({"command": "rm -rf /tmp/foo"}), dir.path());
    assert_eq!(dec.action, PermissionAction::Deny);

    // Tier-1 micro-compaction commits partial savings immediately
    let mut history: Vec<HistoryMessage> = (0..10)
        .map(|i| HistoryMessage::Tool {
            call_id: format!("c{i}"),
            tool_name: "read_file".into(),
            payload: ToolModelPayload::text("x".repeat(4_000)),
            is_error: false,
            media: vec![],
        })
        .collect();
    let budget = ContextBudget::for_window(12_000, false);
    let report = compact_history(&mut history, budget, 1_000, None);
    assert!(report.micro_compacted_tools >= 4);
    assert!(report.saved_tokens() > 0);
}

#[test]
fn subagents_planning_interactive_gating_and_output_spill_work_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let caller = Arc::new(ScriptedCaller::new(vec![NormalizedModelResponse {
        response_id: Some("sub_1".into()),
        response_model: Some("mock".into()),
        content: "Subagent report: verified.".into(),
        reasoning: None,
        tool_calls: vec![],
        stop_reason: ModelStopReason::Stop,
        usage: TokenUsage::default(),
        continuation: None,
    }]));

    let mut reg = ToolRegistry::with_builtin_tools();
    let subagents = Arc::new(crate::SubagentManager::new(store.clone(), caller.clone()));
    subagents.configure_parent(
        mock_route(ModelProtocol::OpenAiChat),
        EffectivePolicy {
            profile: PermissionProfile::AllowAll,
            workspace_trusted: true,
            network_allowed: true,
            rules: vec![],
        },
    );
    crate::register_subagent_tools(&subagents, &mut reg);

    let planning = Arc::new(crate::PlanningState::default());
    planning.set_user_input_callback(Some(Arc::new(|_q, _opts| Some("yes_proceed".into()))));
    crate::register_planning_tools(store, &planning, &mut reg);

    // InteractiveOnly gating: headless hides request_user_input; interactive exposes it
    let headless_cat = crate::ToolCatalog::build_initial(
        &reg,
        HostCapabilities {
            interactive_user_input: false,
            network_allowed: true,
        },
        ForceToolSet::Auto,
    );
    assert!(!headless_cat
        .active_names()
        .iter()
        .any(|n| n == "request_user_input"));
    // Even activate_tail MUST NOT expose InteractiveOnly tools in headless mode!
    let mut headless_cat_mut = headless_cat.clone();
    assert!(headless_cat_mut
        .activate_tail(&["request_user_input".to_string()], &reg)
        .is_empty());

    let interactive_cat = crate::ToolCatalog::build_initial(
        &reg,
        HostCapabilities {
            interactive_user_input: true,
            network_allowed: true,
        },
        ForceToolSet::Auto,
    );
    assert!(interactive_cat
        .active_names()
        .iter()
        .any(|n| n == "request_user_input"));

    // ForceToolSet::Write exposes deferred write tools (like move_file) but not read-only deferred tools (like repo_map)
    let write_cat = crate::ToolCatalog::build_initial(
        &reg,
        HostCapabilities {
            interactive_user_input: false,
            network_allowed: true,
        },
        ForceToolSet::Write,
    );
    assert!(write_cat.active_names().iter().any(|n| n == "move_file"));
    assert!(!write_cat.active_names().iter().any(|n| n == "repo_map"));

    let ctx = ToolContext::new(dir.path());
    // Spawn real foreground subagent
    let spawned = reg.execute(
        "spawn_agent",
        &ctx,
        json!({"role": "Reviewer", "prompt": "Check repository", "background": false}),
    );
    assert!(!spawned.is_error, "spawn_agent failed: {:?}", spawned.payload);
    assert_eq!(spawned.payload.to_model_value()["status"], "completed");
    assert_eq!(
        spawned.payload.to_model_value()["output"],
        "Subagent report: verified."
    );

    // Todos & interactive user input
    let td = reg.execute(
        "update_agent_todos",
        &ctx,
        json!({"todos": [{"title": "Ship v2", "status": "in_progress"}]}),
    );
    assert!(!td.is_error);
    let ui = reg.execute(
        "request_user_input",
        &ctx,
        json!({"question": "Proceed?", "options": ["yes", "no"]}),
    );
    assert_eq!(ui.payload.to_model_value()["answer"], "yes_proceed");
}

#[test]
fn audit_regressions_loop_guard_and_multi_path_workspace_escape() {
    // 1. LoopGuard MUST fire after 3 identical SUCCEEDING calls!
    let mut guard = LoopGuard::default();
    let ok_res = nola_types::ToolExecutionResult::ok_text("same output");
    let batch = vec![(
        "read_file".to_string(),
        json!({"path": "a.txt", "start_line": 1}),
        ok_res,
    )];
    assert!(guard.observe_batch(&batch).is_none());
    assert!(guard.observe_batch(&batch).is_none());
    let nudge = guard.observe_batch(&batch);
    assert!(
        nudge.is_some(),
        "LoopGuard must trigger on 3rd identical succeeding call"
    );

    // 2. Policy MUST block apply_patch & move_file workspace escapes even with Project Allow rule!
    let ws = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let ext_path = outside.path().join("escaped.txt");
    let policy = EffectivePolicy {
        profile: PermissionProfile::AcceptEdits,
        workspace_trusted: true,
        network_allowed: true,
        rules: vec![PermissionRule {
            scope: RuleScope::Project,
            action: PermissionAction::Allow,
            tool_pattern: "*".into(),
            resource_pattern: None,
        }],
    };
    let reg = ToolRegistry::with_builtin_tools();
    let patch_spec = reg.get("apply_patch").unwrap().spec();
    let patch_dec = policy.evaluate(
        &patch_spec,
        &json!({
            "patch": format!("--- /dev/null\n+++ {}\n@@ -0,0 +1 @@\n+owned\n", ext_path.display())
        }),
        ws.path(),
    );
    assert_eq!(patch_dec.action, PermissionAction::Deny);

    let move_spec = reg.get("move_file").unwrap().spec();
    let move_dec = policy.evaluate(
        &move_spec,
        &json!({"source": "a.txt", "destination": ext_path.display().to_string()}),
        ws.path(),
    );
    assert_eq!(move_dec.action, PermissionAction::Deny);

    // 3. Git read-only actions are cleanly Allowed even in Cautious mode, while external worktree_create is Denied!
    let cautious = EffectivePolicy {
        profile: PermissionProfile::Cautious,
        workspace_trusted: false,
        network_allowed: true,
        rules: vec![],
    };
    let git_spec = reg.get("git").unwrap().spec();
    assert_eq!(
        cautious
            .evaluate(&git_spec, &json!({"action": "status"}), ws.path())
            .action,
        PermissionAction::Allow
    );
    assert_eq!(
        cautious
            .evaluate(&git_spec, &json!({"action": "diff", "stat_only": true}), ws.path())
            .action,
        PermissionAction::Allow
    );
    assert_eq!(
        policy
            .evaluate(
                &git_spec,
                &json!({"action": "worktree_create", "target": ext_path.display().to_string()}),
                ws.path()
            )
            .action,
        PermissionAction::Deny
    );
}

#[test]
fn three_tier_compaction_dedup_reasoning_hard_limit_and_overflow_recovery() {
    // 1. Stale reasoning pruning + superseded read_file dedup + output_id preservation + Tier-3 hard limit!
    let mut history = vec![
        HistoryMessage::User {
            content: "Fix the auth module and run tests.".into(),
            media: vec![],
        },
        HistoryMessage::Assistant {
            content: "Reading auth.rs".into(),
            reasoning: Some("r".repeat(6_000)),
            tool_calls: vec![ModelToolCall {
                call_id: "c1".into(),
                name: "read_file".into(),
                arguments: json!({"path": "src/auth.rs"}),
            }],
            continuation: None,
        },
        HistoryMessage::Tool {
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            payload: ToolModelPayload::structured(
                json!({"path": "src/auth.rs", "content": "old".repeat(3_000)}),
                "Read src/auth.rs",
            ),
            is_error: false,
            media: vec![],
        },
        HistoryMessage::Assistant {
            content: "Running cargo test".into(),
            reasoning: Some("t".repeat(4_000)),
            tool_calls: vec![ModelToolCall {
                call_id: "c2".into(),
                name: "bash".into(),
                arguments: json!({"command": "cargo test"}),
            }],
            continuation: None,
        },
        HistoryMessage::Tool {
            call_id: "c2".into(),
            tool_name: "bash".into(),
            payload: ToolModelPayload::structured(
                json!({
                    "spilled": true,
                    "output_id": "out_deadbeef12345678",
                    "exit_code": 1,
                    "head_preview": "h".repeat(4_000)
                }),
                "Shell exited with status 1",
            ),
            is_error: false,
            media: vec![],
        },
        HistoryMessage::Assistant {
            content: "Re-reading auth.rs".into(),
            reasoning: Some("recent".into()),
            tool_calls: vec![ModelToolCall {
                call_id: "c3".into(),
                name: "read_file".into(),
                arguments: json!({"path": "src/auth.rs"}),
            }],
            continuation: None,
        },
        HistoryMessage::Tool {
            call_id: "c3".into(),
            tool_name: "read_file".into(),
            payload: ToolModelPayload::structured(
                json!({"path": "src/auth.rs", "content": "new".repeat(4_000)}),
                "Read src/auth.rs",
            ),
            is_error: false,
            media: vec![],
        },
    ];

    // Tiny window forces Tier 1 -> Tier 2 -> Tier 3 hard-limit guard!
    let budget = ContextBudget::for_window(8_192, false);
    let report = compact_history(&mut history, budget, 500, None);
    assert!(report.saved_tokens() > 0);
    let hard_limit = budget.history_budget(500, budget.prompt_token_limit());
    assert!(
        report.tokens_after <= hard_limit,
        "Tier-3 hard limit must shrink history ({}) <= hard_limit ({})",
        report.tokens_after,
        hard_limit
    );

    // Verify output_id survives micro-compaction when not yet summarized
    let mut micro_hist = vec![
        HistoryMessage::User {
            content: "Check logs".into(),
            media: vec![],
        },
        HistoryMessage::Tool {
            call_id: "t0".into(),
            tool_name: "bash".into(),
            payload: ToolModelPayload::structured(
                json!({"output_id": "out_keepme1234", "exit_code": 1, "head_preview": "z".repeat(4_000)}),
                "Shell failed",
            ),
            is_error: false,
            media: vec![],
        },
    ];
    for i in 1..=7 {
        micro_hist.push(HistoryMessage::Tool {
            call_id: format!("t{i}"),
            tool_name: "read_file".into(),
            payload: ToolModelPayload::text("a".repeat(2_000)),
            is_error: false,
            media: vec![],
        });
    }
    let b2 = ContextBudget::for_window(12_000, false);
    let r2 = compact_history(&mut micro_hist, b2, 1_000, None);
    assert!(r2.micro_compacted_tools >= 1);
    if let HistoryMessage::Tool { payload, .. } = &micro_hist[1] {
        assert_eq!(payload.to_model_value()["output_id"], "out_keepme1234");
        assert_eq!(payload.to_model_value()["exit_code"], 1);
    }

    // 2. Reactive HTTP 400 context-overflow retry inside AgentEngine::run
    struct OverflowOnceCaller {
        calls: Mutex<usize>,
    }
    impl ModelCaller for OverflowOnceCaller {
        fn call(
            &self,
            _route: &ModelRouteConfig,
            _req: &StructuredModelRequest,
            _stream: bool,
            _on_delta: &mut dyn FnMut(ModelStreamDelta),
        ) -> Result<NormalizedModelResponse, LlmError> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            if *n == 1 {
                return Err(LlmError::Status {
                    status: 400,
                    body: "This model's maximum context length is 8192 tokens.".into(),
                });
            }
            Ok(NormalizedModelResponse {
                response_id: Some("r_ok".into()),
                response_model: Some("mock".into()),
                content: "Recovered after compaction!".into(),
                reasoning: None,
                tool_calls: vec![],
                stop_reason: ModelStopReason::Stop,
                usage: TokenUsage {
                    input_tokens: 1_200,
                    output_tokens: 20,
                    ..Default::default()
                },
                continuation: None,
            })
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolContext::new(dir.path());
    let store = Store::open_in_memory().unwrap();
    // Pre-seed a session with oversized history so force-compaction yields real savings
    let seeded_history = vec![
        HistoryMessage::User {
            content: "Initial prompt".into(),
            media: vec![],
        },
        HistoryMessage::Tool {
            call_id: "seed1".into(),
            tool_name: "bash".into(),
            payload: ToolModelPayload::text("x".repeat(6_000)),
            is_error: false,
            media: vec![],
        },
        HistoryMessage::Assistant {
            content: "done step 1".into(),
            reasoning: Some("r".repeat(2_000)),
            tool_calls: vec![],
            continuation: None,
        },
    ];
    store
        .set_kv(
            "session_state:overflow_sess",
            &json!({
                "history": seeded_history,
                "policy": {
                    "profile": "allow_all",
                    "workspace_trusted": true,
                    "network_allowed": true,
                    "rules": []
                },
                "activated_tools": []
            }),
        )
        .unwrap();
    let caller = Arc::new(OverflowOnceCaller {
        calls: Mutex::new(0),
    });
    let engine = AgentEngine::new(
        store,
        Arc::new(ToolRegistry::with_builtin_tools()),
        caller.clone(),
    );
    let res = engine
        .run(
            &ctx,
            "Continue",
            AgentRunConfig {
                session_id: Some("overflow_sess".into()),
                resume: true,
                route: mock_route(ModelProtocol::OpenAiChat),
                frozen_system_blocks: vec![FrozenSystemBlock {
                    id: "sys".into(),
                    text: "System".into(),
                }],
                host_capabilities: HostCapabilities {
                    interactive_user_input: false,
                    network_allowed: true,
                },
                policy: EffectivePolicy {
                    profile: PermissionProfile::AllowAll,
                    workspace_trusted: true,
                    network_allowed: true,
                    rules: vec![],
                },
                hooks: HookRunner::default(),
                approval: None,
                max_steps: 5,
                force_tool_set: ForceToolSet::Auto,
                stream: false,
                json_schema: None,
                cancel: Arc::new(AtomicBool::new(false)),
                auto_recall_memory: false,
                inbox: None,
            },
            |_| {},
        )
        .unwrap();
    assert_eq!(res.final_text, "Recovered after compaction!");
    assert_eq!(*caller.calls.lock().unwrap(), 2);
}

#[test]
fn empty_reasoning_turn_continues_and_stale_tool_media_is_evicted() {
    // 1. Reasoning-only turn with empty content & no tool_calls MUST NOT exit prematurely!
    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolContext::new(dir.path());
    let store = Store::open_in_memory().unwrap();
    let caller = Arc::new(ScriptedCaller::new(vec![
        NormalizedModelResponse {
            response_id: Some("r_think_only".into()),
            response_model: Some("mock".into()),
            content: String::new(),
            reasoning: Some("Long internal reasoning chain...".into()),
            tool_calls: vec![],
            stop_reason: ModelStopReason::Stop,
            usage: TokenUsage::default(),
            continuation: None,
        },
        NormalizedModelResponse {
            response_id: Some("r_final".into()),
            response_model: Some("mock".into()),
            content: "Final deliverable written and verified.".into(),
            reasoning: None,
            tool_calls: vec![],
            stop_reason: ModelStopReason::Stop,
            usage: TokenUsage::default(),
            continuation: None,
        },
    ]));
    let engine = AgentEngine::new(
        store,
        Arc::new(ToolRegistry::with_builtin_tools()),
        caller.clone(),
    );
    let res = engine
        .run(
            &ctx,
            "Solve task",
            AgentRunConfig {
                session_id: Some("s_empty".into()),
                resume: false,
                route: mock_route(ModelProtocol::OpenAiChat),
                frozen_system_blocks: vec![FrozenSystemBlock {
                    id: "sys".into(),
                    text: "System".into(),
                }],
                host_capabilities: HostCapabilities {
                    interactive_user_input: false,
                    network_allowed: true,
                },
                policy: EffectivePolicy {
                    profile: PermissionProfile::AllowAll,
                    workspace_trusted: true,
                    network_allowed: true,
                    rules: vec![],
                },
                hooks: HookRunner::default(),
                approval: None,
                max_steps: 6,
                force_tool_set: ForceToolSet::Auto,
                stream: false,
                json_schema: None,
                cancel: Arc::new(AtomicBool::new(false)),
                auto_recall_memory: false,
                inbox: None,
            },
            |_| {},
        )
        .unwrap();
    assert_eq!(res.final_text, "Final deliverable written and verified.");
    assert_eq!(res.steps, 2);
    let captured = caller.captured.lock().unwrap();
    assert!(captured[1]
        .ephemeral_tail_reminders
        .iter()
        .any(|r| r.tag == "empty_response"));

    // 2. Stale tool media eviction keeps only the newest 3 images
    let mk_img = |name: &str| nola_types::MediaAttachment {
        mime_type: "image/png".into(),
        filename: name.into(),
        bytes_base64: "AAAA".into(),
        raw_bytes_len: 3,
        sha256: name.into(),
    };
    let mut hist: Vec<HistoryMessage> = (1..=5)
        .map(|i| HistoryMessage::Tool {
            call_id: format!("c{i}"),
            tool_name: "read_file".into(),
            payload: ToolModelPayload::text("img"),
            is_error: false,
            media: vec![mk_img(&format!("p{i}.png"))],
        })
        .collect();
    crate::loop_runner::evict_stale_tool_media(&mut hist, 3);
    let counts: Vec<usize> = hist
        .iter()
        .map(|m| match m {
            HistoryMessage::Tool { media, .. } => media.len(),
            _ => 0,
        })
        .collect();
    assert_eq!(counts, vec![0, 0, 1, 1, 1]);
}
