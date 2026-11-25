// SPDX-License-Identifier: Apache-2.0
//! Rich interactive terminal UI (`crossterm`) with dimmed reasoning streaming,
//! collapsible tool output toggle (`/verbose`), interactive permission approval cards
//! (allow once / always allow / deny), `request_user_input` wiring, checkpoints,
//! and automatic post-turn memory distillation.

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::QueueableCommand;
use nola_engine::{
    register_planning_tools, register_subagent_tools, AgentEngine, AgentEvent, AgentRunConfig,
    EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller, ModelCaller, PlanningState,
    SubagentManager,
};
use nola_modules::{MemoryWorker, ModulesSuite};
use nola_store::Store;
use nola_tools::{SandboxMode, ToolContext, ToolRegistry};
use nola_types::{new_id, HostCapabilities, PermissionProfile};

use crate::config::{build_frozen_system_blocks, load_active_config, resolve_model_route};

pub fn run_tui(store: Store, suite: Arc<ModulesSuite>, workspace: &Path) -> Result<u8, String> {
    let cfg = load_active_config(&store);
    let route = resolve_model_route(&cfg)?;

    let caller: Arc<dyn ModelCaller> =
        Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
    // Never implicitly trust repository `.nola/settings.json` or `.nola/hooks.json` in TUI!
    let workspace_trusted = false;
    let base_policy = EffectivePolicy::load(
        workspace,
        suite.user_home.as_deref(),
        workspace_trusted,
        Some(PermissionProfile::AcceptEdits),
        true,
    );

    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);

    let subagents = Arc::new(SubagentManager::new(store.clone(), Arc::clone(&caller)));
    subagents.configure_parent(route.clone(), base_policy.clone());
    register_subagent_tools(&subagents, &mut registry);

    let planning = Arc::new(PlanningState::default());
    planning.set_user_input_callback(Some(Arc::new(|question, options| {
        eprintln!("\n[Question] {question}");
        for (i, opt) in options.iter().enumerate() {
            eprintln!("  {}. {opt}", i + 1);
        }
        eprint!("Answer> ");
        let _ = io::stderr().flush();
        let mut ans = String::new();
        io::stdin().read_line(&mut ans).ok()?;
        let t = ans.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })));
    register_planning_tools(store.clone(), &planning, &mut registry);

    let registry = Arc::new(registry);
    let engine = AgentEngine::new(store.clone(), Arc::clone(&registry), caller);

    let mut ctx = ToolContext::new(workspace);
    ctx.interactive_user_input = true;
    ctx.permission_profile = PermissionProfile::AcceptEdits;
    ctx.sandbox_mode = SandboxMode::from_env();

    let mut session_id = new_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&cancel));
    let always_allowed = Arc::new(Mutex::new(BTreeSet::<String>::new()));

    let mut stdout = io::stdout();
    let _ = stdout.queue(SetForegroundColor(Color::Cyan));
    println!(
        "Nola v2 Interactive TUI | model={} | workspace={}",
        route.model,
        workspace.display()
    );
    let _ = stdout.queue(ResetColor);
    println!("Commands: /sessions /skills /tools /goals /rooms /checkpoint /verbose /new /exit\n");
    let _ = stdout.flush();

    let stdin = io::stdin();
    let mut input = String::new();
    let mut first_turn = true;
    let mut verbose_tools = false;

    loop {
        print!("nola> ");
        let _ = io::stdout().flush();
        input.clear();
        let n = stdin.read_line(&mut input).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed {
            "/exit" | "/quit" => break,
            "/verbose" => {
                verbose_tools = !verbose_tools;
                println!("Tool detail display: {}", if verbose_tools { "expanded" } else { "compact" });
                continue;
            }
            "/new" => {
                session_id = new_id();
                first_turn = true;
                println!("Started new session {session_id}");
                continue;
            }
            "/sessions" => {
                if let Ok(list) = store.list_sessions(10) {
                    for s in list {
                        println!("  {}  {}", s.id, s.title);
                    }
                }
                continue;
            }
            "/skills" => {
                for s in nola_modules::skills::discover_skills(workspace, suite.user_home.as_deref()) {
                    println!("  {} — {}", s.name, s.description);
                }
                continue;
            }
            "/tools" => {
                for spec in registry.all_specs() {
                    println!("  [{:?}] {} — {}", spec.tier, spec.schema.name, spec.schema.description);
                }
                continue;
            }
            "/goals" => {
                if let Ok(list) = suite.list_records::<nola_types::GoalRecord>("goal") {
                    for g in list {
                        println!("  {} [{:?}] {}", g.id, g.status, g.title);
                    }
                }
                continue;
            }
            "/rooms" => {
                if let Ok(list) = suite.list_records::<nola_types::AgentRoom>("agent_room") {
                    for r in list {
                        println!("  {} — {}", r.id, r.name);
                    }
                }
                continue;
            }
            "/checkpoint" => {
                match ctx.checkpoints.create(workspace, "tui") {
                    Ok((id, files)) => println!("Created checkpoint {id} ({files} files)"),
                    Err(e) => eprintln!("checkpoint error: {e}"),
                }
                continue;
            }
            _ => {}
        }

        cancel.store(false, Ordering::Relaxed);
        let hooks = HookRunner::load(workspace, suite.user_home.as_deref(), workspace_trusted);
        subagents.configure_runtime(hooks.clone(), Some(Arc::clone(&cancel)));
        let allow_set = Arc::clone(&always_allowed);
        let run_cfg = AgentRunConfig {
            session_id: Some(session_id.clone()),
            resume: !first_turn,
            route: route.clone(),
            frozen_system_blocks: build_frozen_system_blocks(workspace, &cfg),
            host_capabilities: HostCapabilities {
                interactive_user_input: true,
                network_allowed: true,
            },
            policy: base_policy.clone(),
            hooks,
            approval: Some(Arc::new(move |spec, args, reason| {
                if allow_set.lock().unwrap().contains(&spec.schema.name) {
                    return true;
                }
                eprintln!("\n┌─ Permission Approval ─────────────────────────");
                eprintln!("│ Tool:   {}", spec.schema.name);
                eprintln!("│ Reason: {reason}");
                eprintln!("│ Args:   {}", serde_json::to_string(args).unwrap_or_default());
                eprint!("└─ [y] Allow once / [a] Always allow / [N] Deny > ");
                let _ = io::stderr().flush();
                let mut ans = String::new();
                if io::stdin().read_line(&mut ans).is_err() {
                    return false;
                }
                match ans.trim().to_ascii_lowercase().as_str() {
                    "y" | "yes" => true,
                    "a" | "always" => {
                        allow_set.lock().unwrap().insert(spec.schema.name.clone());
                        true
                    }
                    _ => false,
                }
            })),
            max_steps: 0,
            force_tool_set: ForceToolSet::Auto,
            stream: true,
            json_schema: None,
            cancel: Arc::clone(&cancel),
            auto_recall_memory: true,
            inbox: None,
        };
        first_turn = false;

        match engine.run(&ctx, trimmed, run_cfg, |ev| match ev {
            AgentEvent::ReasoningDelta { delta } if verbose_tools => {
                let mut out = io::stdout();
                let _ = out.queue(SetAttribute(Attribute::Dim));
                print!("{delta}");
                let _ = out.queue(ResetColor);
                let _ = out.flush();
            }
            AgentEvent::TextDelta { delta } => {
                print!("{delta}");
                let _ = io::stdout().flush();
            }
            AgentEvent::ToolStarted { name, arguments, .. } if verbose_tools => {
                eprintln!("\n[RUN {name}] {}", serde_json::to_string(&arguments).unwrap_or_default());
            }
            AgentEvent::ToolFinished {
                name,
                is_error,
                summary,
                ..
            } => {
                let tag = if is_error { "ERR" } else { "OK" };
                eprintln!("\n[{tag} {name}] {summary}");
            }
            _ => {}
        }) {
            Ok(res) => {
                let _ = MemoryWorker::distill_turn(&suite, "tui", trimmed, &res.final_text);
                println!("\n");
            }
            Err(e) => eprintln!("\nerror: {e}\n"),
        }
    }
    ctx.pty.close_all();
    Ok(0)
}
