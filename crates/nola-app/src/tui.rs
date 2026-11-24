// SPDX-License-Identifier: Apache-2.0
//! Clean interactive terminal REPL/TUI using `crossterm`.

use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossterm::style::{Color, ResetColor, SetForegroundColor};
use crossterm::QueueableCommand;
use nola_engine::{
    AgentEngine, AgentEvent, AgentRunConfig, EffectivePolicy, HookRunner, HttpModelCaller,
};
use nola_modules::ModulesSuite;
use nola_store::Store;
use nola_tools::{SandboxMode, ToolContext, ToolRegistry};
use nola_types::{new_id, HostCapabilities, PermissionProfile};

use crate::config::{build_frozen_system_blocks, load_active_config, resolve_model_route};

pub fn run_tui(store: Store, suite: Arc<ModulesSuite>, workspace: &Path) -> Result<u8, String> {
    let cfg = load_active_config(&store);
    let route = resolve_model_route(&cfg)?;
    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);

    let caller = Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
    let engine = AgentEngine::new(store.clone(), Arc::new(registry), caller);

    let mut ctx = ToolContext::new(workspace);
    ctx.permission_profile = PermissionProfile::AcceptEdits;
    ctx.sandbox_mode = SandboxMode::from_env();

    let mut session_id = new_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&cancel));

    let mut stdout = io::stdout();
    let _ = stdout.queue(SetForegroundColor(Color::Cyan));
    println!(
        "Nola v2 Interactive TUI | model={} | workspace={}",
        route.model,
        workspace.display()
    );
    let _ = stdout.queue(ResetColor);
    println!("Commands: /sessions, /skills, /new, /exit\n");
    let _ = stdout.flush();

    let stdin = io::stdin();
    let mut input = String::new();
    let mut first_turn = true;
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
                let skills = nola_modules::skills::discover_skills(
                    workspace,
                    suite.user_home.as_deref(),
                );
                for s in skills {
                    println!("  {} — {}", s.name, s.description);
                }
                continue;
            }
            _ => {}
        }

        cancel.store(false, Ordering::Relaxed);
        let policy = EffectivePolicy::load(
            workspace,
            suite.user_home.as_deref(),
            true,
            Some(PermissionProfile::AcceptEdits),
            true,
        );
        let hooks = HookRunner::load(workspace, suite.user_home.as_deref(), true);
        let run_cfg = AgentRunConfig {
            session_id: Some(session_id.clone()),
            resume: !first_turn,
            route: route.clone(),
            frozen_system_blocks: build_frozen_system_blocks(workspace, &cfg),
            host_capabilities: HostCapabilities {
                interactive_user_input: true,
                network_allowed: true,
            },
            policy,
            hooks,
            approval: Some(Arc::new(|spec, _args, reason| {
                eprint!("[approve {} ({reason})? y/N] ", spec.schema.name);
                let mut ans = String::new();
                io::stdin().read_line(&mut ans).is_ok()
                    && matches!(ans.trim().to_ascii_lowercase().as_str(), "y" | "yes")
            })),
            max_steps: 0,
            force_all_tools: false,
            stream: true,
            json_schema: None,
            cancel: Arc::clone(&cancel),
        };
        first_turn = false;

        match engine.run(&ctx, trimmed, run_cfg, |ev| match ev {
            AgentEvent::TextDelta { delta } => {
                print!("{delta}");
                let _ = io::stdout().flush();
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
            Ok(_) => println!("\n"),
            Err(e) => eprintln!("\nerror: {e}\n"),
        }
    }
    Ok(0)
}
