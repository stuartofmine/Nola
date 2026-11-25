// SPDX-License-Identifier: Apache-2.0
//! Rich interactive terminal UI (`crossterm`) with dimmed reasoning streaming,
//! collapsible tool output toggle (`/verbose`), session-scoped `(tool, target)`
//! permission approvals (`Cautious` default), MCP tool discovery, slash commands,
//! persistent context usage footer, and automatic post-turn memory distillation.

use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::QueueableCommand;
use nola_engine::{
    compact_session_now, load_session_usage, register_planning_tools, register_subagent_tools,
    AgentEngine, AgentEvent, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner,
    HttpModelCaller, ModelCaller, PlanningState, SubagentManager,
};
use nola_llm::{LlmClient, PrivacyRedactor};
use nola_mcp::McpManager;
use nola_modules::{MemoryWorker, ModulesSuite, RoomRuntime};
use nola_store::{Store, Vault};
use nola_tools::{SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    new_id, now_ms, GoalRecord, GoalStatus, HostCapabilities, ModelProtocol, ModelRouteConfig,
    PermissionProfile,
};
use serde_json::{json, Value};

use crate::config::{
    build_frozen_system_blocks, default_db_path, default_vault_path, load_active_config,
    resolve_model_route, AppliedConfig,
};

pub fn run_tui(store: Store, suite: Arc<ModulesSuite>, workspace: &Path) -> Result<u8, String> {
    let cfg = load_active_config(&store);
    let mut route = resolve_model_route(&cfg)?;

    let caller: Arc<dyn ModelCaller> =
        Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
    let llm_client = LlmClient::new(120).ok();

    // Never implicitly trust repository `.nola/settings.json` or `.nola/hooks.json` in TUI!
    let workspace_trusted = false;
    let mut permission_profile = PermissionProfile::Cautious;
    let mut base_policy = EffectivePolicy::load(
        workspace,
        suite.user_home.as_deref(),
        workspace_trusted,
        Some(permission_profile),
        true,
    );

    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);

    // Wire enabled MCP servers into TUI tool registry
    if let Ok(servers) = store.list_mcp_servers() {
        let vault = Vault::open(default_vault_path(&default_db_path()))
            .ok()
            .map(Arc::new);
        let mcp = Arc::new(McpManager::new(vault));
        for srv in servers.into_iter().filter(|s| s.enabled) {
            if let Ok(tools) = mcp.discover_tools(&srv) {
                for t in tools {
                    registry.register(t);
                }
            }
        }
    }

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
    let engine = AgentEngine::new(store.clone(), Arc::clone(&registry), Arc::clone(&caller));

    let mut ctx = ToolContext::new(workspace);
    ctx.interactive_user_input = true;
    ctx.permission_profile = permission_profile;
    ctx.sandbox_mode = SandboxMode::from_env();

    let mut session_id = new_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&cancel));
    let session_grants = Arc::new(Mutex::new(BTreeSet::<(String, String)>::new()));

    let mut stdout = io::stdout();
    let _ = stdout.queue(SetForegroundColor(Color::Cyan));
    println!(
        "Nola v2 Interactive TUI | model={} | permission={} | workspace={}",
        route.model,
        permission_profile.as_canonical_str(),
        workspace.display()
    );
    let _ = stdout.queue(ResetColor);
    println!("Type /help for slash commands (/model /allow /compact /config /goal /room /new /quit)\n");
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

        if trimmed.starts_with('/') {
            let mut parts = trimmed.splitn(2, char::is_whitespace);
            let cmd = parts.next().unwrap_or("");
            let rest = parts.next().unwrap_or("").trim();

            match cmd {
                "/exit" | "/quit" => break,
                "/help" => {
                    print_tui_help();
                    continue;
                }
                "/verbose" => {
                    verbose_tools = !verbose_tools;
                    println!(
                        "Tool detail display: {}",
                        if verbose_tools { "expanded" } else { "compact" }
                    );
                    continue;
                }
                "/new" => {
                    session_id = new_id();
                    first_turn = true;
                    session_grants.lock().unwrap().clear();
                    println!("Started new session {session_id} (cleared session permission grants)");
                    continue;
                }
                "/compact" => {
                    match compact_session_now(&store, llm_client.as_ref(), Some(&route), &session_id)
                    {
                        Ok(val) => {
                            let before = val["before_messages"].as_u64().unwrap_or(0);
                            let after = val["after_messages"].as_u64().unwrap_or(0);
                            let summary = val["summary"].as_str().unwrap_or("");
                            println!("Compacted session ({before} -> {after} messages): {summary}");
                        }
                        Err(e) => eprintln!("compact error: {e}"),
                    }
                    continue;
                }
                "/model" => {
                    handle_tui_model_cmd(&cfg, &mut route, &subagents, &base_policy, rest);
                    continue;
                }
                "/allow" => {
                    if rest.is_empty() {
                        println!(
                            "Active permission profile: {} (use `/allow cautious` or `/allow allow_all`)",
                            permission_profile.as_canonical_str()
                        );
                    } else {
                        match PermissionProfile::parse_strict(rest) {
                            Ok(next) => {
                                permission_profile = next;
                                base_policy = EffectivePolicy::load(
                                    workspace,
                                    suite.user_home.as_deref(),
                                    workspace_trusted,
                                    Some(permission_profile),
                                    true,
                                );
                                ctx.permission_profile = permission_profile;
                                subagents.configure_parent(route.clone(), base_policy.clone());
                                println!(
                                    "Switched permission profile to '{}'",
                                    permission_profile.as_canonical_str()
                                );
                            }
                            Err(e) => eprintln!("{e}"),
                        }
                    }
                    continue;
                }
                "/config" => {
                    print_redacted_tui_config(&cfg, &route, permission_profile, workspace);
                    continue;
                }
                "/goal" | "/goals" => {
                    handle_tui_goal_cmd(&suite, workspace, rest);
                    continue;
                }
                "/room" => {
                    handle_tui_room_cmd(&suite, &caller, &route, &base_policy, rest);
                    continue;
                }
                "/rooms" => {
                    list_tui_rooms(&suite);
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
                    for s in nola_modules::skills::discover_skills_detailed(
                        Some(&store),
                        workspace,
                        suite.user_home.as_deref(),
                    ) {
                        let name = s["name"].as_str().unwrap_or("");
                        let desc = s["description"].as_str().unwrap_or("");
                        let enabled = s["enabled"].as_bool().unwrap_or(true);
                        let state = if enabled { "on" } else { "off" };
                        println!("  [{state}] {name} — {desc}");
                    }
                    continue;
                }
                "/tools" => {
                    for spec in registry.all_specs() {
                        println!(
                            "  [{:?}] {} — {}",
                            spec.tier, spec.schema.name, spec.schema.description
                        );
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
                other => {
                    eprintln!("Unknown slash command '{other}'. Type /help for available commands.");
                    continue;
                }
            }
        }

        cancel.store(false, Ordering::Relaxed);
        let hooks = HookRunner::load(workspace, suite.user_home.as_deref(), workspace_trusted);
        subagents.configure_runtime(hooks.clone(), Some(Arc::clone(&cancel)));
        let grants = Arc::clone(&session_grants);
        let cancel_for_prompt = Arc::clone(&cancel);
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
                let target_key = extract_target_key(&spec.schema.name, args);
                let grant_key = (spec.schema.name.clone(), target_key.clone());
                if grants.lock().unwrap().contains(&grant_key) {
                    return true;
                }
                let summary = format_permission_summary(&spec.schema.name, &target_key, args);
                eprintln!("\n┌─ Permission Approval ─────────────────────────");
                eprintln!("│ Tool:    {}", spec.schema.name);
                eprintln!("│ Target:  {target_key}");
                eprintln!("│ Summary: {summary}");
                eprintln!("│ Reason:  {reason}");
                eprint!("└─ [1] Allow once / [2] Allow for this target in session / [3] Return to input / [n] Deny > ");
                let _ = io::stderr().flush();

                match read_approval_choice() {
                    ApprovalChoice::AllowOnce => true,
                    ApprovalChoice::AllowForTargetInSession => {
                        grants.lock().unwrap().insert(grant_key);
                        true
                    }
                    ApprovalChoice::ReturnToInput => {
                        cancel_for_prompt.store(true, Ordering::SeqCst);
                        false
                    }
                    ApprovalChoice::Deny => false,
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
            AgentEvent::ToolStarted {
                name, arguments, ..
            } if verbose_tools => {
                let safe_args =
                    PrivacyRedactor::redact_text(&serde_json::to_string(&arguments).unwrap_or_default());
                eprintln!("\n[RUN {name}] {safe_args}");
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
                println!();
                print_context_usage_footer(&store, &session_id, &route, &res.usage);
                println!();
            }
            Err(e) => eprintln!("\nerror: {e}\n"),
        }
    }
    ctx.pty.close_all();
    Ok(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalChoice {
    AllowOnce,
    AllowForTargetInSession,
    ReturnToInput,
    Deny,
}

fn read_approval_choice() -> ApprovalChoice {
    if io::stdin().is_terminal() && enable_raw_mode().is_ok() {
        let choice = loop {
            match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('1') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                        break ApprovalChoice::AllowOnce;
                    }
                    KeyCode::Char('2') | KeyCode::Char('a') | KeyCode::Char('A') => {
                        break ApprovalChoice::AllowForTargetInSession;
                    }
                    KeyCode::Char('3') | KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::Esc => {
                        break ApprovalChoice::ReturnToInput;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter => {
                        break ApprovalChoice::Deny;
                    }
                    _ => {}
                },
                Ok(_) => {}
                Err(_) => break ApprovalChoice::Deny,
            }
        };
        let _ = disable_raw_mode();
        eprintln!();
        return choice;
    }

    let mut ans = String::new();
    if io::stdin().read_line(&mut ans).is_err() {
        return ApprovalChoice::Deny;
    }
    match ans.trim().to_ascii_lowercase().as_str() {
        "1" | "y" | "yes" => ApprovalChoice::AllowOnce,
        "2" | "a" | "always" => ApprovalChoice::AllowForTargetInSession,
        "3" | "r" | "return" => ApprovalChoice::ReturnToInput,
        _ => ApprovalChoice::Deny,
    }
}

fn extract_target_key(tool_name: &str, args: &Value) -> String {
    for key in [
        "path",
        "file_path",
        "target_path",
        "destination",
        "url",
        "endpoint",
        "server_id",
        "repo",
    ] {
        if let Some(val) = args
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return PrivacyRedactor::redact_text(val)
                .chars()
                .take(120)
                .collect();
        }
    }
    if let Some(cmd) = args
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let redacted = PrivacyRedactor::redact_text(cmd);
        let head = redacted.lines().next().unwrap_or(&redacted);
        return head.chars().take(120).collect();
    }
    tool_name.to_string()
}

fn format_permission_summary(tool_name: &str, target_key: &str, args: &Value) -> String {
    let raw = match tool_name {
        "bash" | "pty_exec" => format!("Run shell command: {target_key}"),
        "write_file" | "edit_file" | "apply_patch" | "delete_file" | "move_file" => {
            format!("Modify workspace file: {target_key}")
        }
        "web_fetch" | "web_search" => format!("Network request to: {target_key}"),
        _ => {
            let preview: String = serde_json::to_string(args)
                .unwrap_or_default()
                .chars()
                .take(140)
                .collect();
            format!("{tool_name} on {target_key} ({preview})")
        }
    };
    PrivacyRedactor::redact_text(&raw)
}

fn print_context_usage_footer(
    store: &Store,
    session_id: &str,
    route: &ModelRouteConfig,
    turn_usage: &nola_types::TokenUsage,
) {
    let persisted = load_session_usage(store, session_id);
    let used_tokens = persisted.input_tokens.max(turn_usage.input_tokens);
    let window = route.context_window.max(1) as u64;
    let pct = ((used_tokens as f64 / window as f64) * 100.0).min(100.0);
    let used_k = used_tokens as f64 / 1000.0;
    let win_k = window as f64 / 1000.0;
    let excl_cache = route.protocol == ModelProtocol::AnthropicMessages;
    let cache_bps = turn_usage
        .cache_hit_rate_bps_normalized(excl_cache)
        .or_else(|| persisted.cache_hit_rate_bps_normalized(excl_cache))
        .unwrap_or_else(|| turn_usage.cache_hit_rate_bps());
    let cache_pct = cache_bps as f64 / 100.0;

    let mut out = io::stdout();
    let _ = out.queue(SetAttribute(Attribute::Dim));
    print!(
        "上下文 {pct:.0}% · {used_k:.1}k/{win_k:.0}k · cache hit {cache_pct:.0}%"
    );
    let _ = out.queue(ResetColor);
    let _ = out.flush();
}

fn print_tui_help() {
    println!(
        "Slash commands:\n  \
         /help                              Show this help\n  \
         /compact                           Compact current session history\n  \
         /model [id]                        List configured models or switch active model\n  \
         /allow [cautious|allow_all]        Show or switch strict permission profile\n  \
         /config                            Print redacted active configuration\n  \
         /goal [list|create <title> | <obj>] List or create autonomous goals\n  \
         /room <room-id> [@recipient] <msg> Send message to an AgentRoom\n  \
         /rooms                             List AgentRooms\n  \
         /sessions                          List recent sessions\n  \
         /skills                            List discovered skills\n  \
         /tools                             List registered tools\n  \
         /checkpoint                        Create workspace checkpoint\n  \
         /verbose                           Toggle reasoning & tool argument display\n  \
         /new                               Start a fresh session & clear session grants\n  \
         /quit                              Exit TUI"
    );
}

fn handle_tui_model_cmd(
    cfg: &AppliedConfig,
    route: &mut ModelRouteConfig,
    subagents: &SubagentManager,
    policy: &EffectivePolicy,
    arg: &str,
) {
    if arg.is_empty() {
        println!(
            "Active model: {}:{} ({}, ctx={}k)",
            route.connection_id,
            route.model,
            route.protocol.as_str(),
            route.context_window / 1000
        );
        for conn in &cfg.llm.connections {
            for m in &conn.models {
                let marker = if conn.id == route.connection_id && m.id == route.model {
                    "*"
                } else {
                    " "
                };
                println!("  {marker} {}:{}  ({})", conn.id, m.id, conn.endpoint);
            }
        }
        return;
    }

    for conn in &cfg.llm.connections {
        for m in &conn.models {
            let full_id = format!("{}:{}", conn.id, m.id);
            if m.id == arg || full_id == arg {
                let mut next_cfg = cfg.clone();
                next_cfg.llm.default_model = Some(crate::config::DefaultModelSelection {
                    connection: conn.id.clone(),
                    model: m.id.clone(),
                });
                if let Ok(resolved) = resolve_model_route(&next_cfg) {
                    *route = resolved;
                    subagents.configure_parent(route.clone(), policy.clone());
                    println!("Switched active model to {}:{}", route.connection_id, route.model);
                    return;
                }
            }
        }
    }

    route.model = arg.to_string();
    subagents.configure_parent(route.clone(), policy.clone());
    println!("Switched active model to '{}' on connection '{}'", route.model, route.connection_id);
}

fn print_redacted_tui_config(
    cfg: &AppliedConfig,
    route: &ModelRouteConfig,
    profile: PermissionProfile,
    workspace: &Path,
) {
    let connections: Vec<Value> = cfg
        .llm
        .connections
        .iter()
        .map(|c| {
            let has_key = c.api_key.as_deref().is_some_and(|s| !s.trim().is_empty())
                || c.api_key_env
                    .as_deref()
                    .and_then(|e| std::env::var(e).ok())
                    .is_some_and(|s| !s.trim().is_empty());
            json!({
                "id": c.id,
                "endpoint": c.endpoint,
                "protocol": c.protocol,
                "has_api_key": has_key,
                "models": c.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "workspace": workspace.display().to_string(),
            "db_path": default_db_path().display().to_string(),
            "permission_profile": profile.as_canonical_str(),
            "active_route": {
                "connection_id": route.connection_id,
                "model": route.model,
                "protocol": route.protocol.as_str(),
                "endpoint": route.endpoint,
                "context_window": route.context_window,
                "has_api_key": !route.api_key.is_empty()
            },
            "network_policy": cfg.network_policy,
            "connections": connections
        }))
        .unwrap_or_else(|_| "{}".into())
    );
}

fn handle_tui_goal_cmd(suite: &ModulesSuite, workspace: &Path, rest: &str) {
    if rest.is_empty() || rest == "list" {
        match suite.list_records::<GoalRecord>("goal") {
            Ok(list) if !list.is_empty() => {
                for g in list {
                    println!("  {} [{:?}] {} — {}", g.id, g.status, g.title, g.objective);
                }
            }
            Ok(_) => println!("No goals found. Create one with `/goal create <title> | <objective>`."),
            Err(e) => eprintln!("goal list error: {e}"),
        }
        return;
    }

    if let Some(body) = rest.strip_prefix("create").map(str::trim) {
        if body.is_empty() {
            eprintln!("Usage: /goal create <title> | <objective>");
            return;
        }
        let (title, objective) = match body.split_once('|') {
            Some((t, o)) => (t.trim(), o.trim()),
            None => (body, body),
        };
        let ts = now_ms();
        let rec = GoalRecord {
            id: new_id(),
            title: title.to_string(),
            objective: objective.to_string(),
            workspace_root: workspace.display().to_string(),
            status: GoalStatus::Pending,
            progress_notes: Vec::new(),
            created_at_ms: ts,
            updated_at_ms: ts,
        };
        match suite.upsert_record("goal", &rec.id, &rec) {
            Ok(()) => println!("Created goal {} — {}", rec.id, rec.title),
            Err(e) => eprintln!("goal create error: {e}"),
        }
        return;
    }

    eprintln!("Usage: /goal [list|create <title> | <objective>]");
}

fn list_tui_rooms(suite: &ModulesSuite) {
    match suite.list_records::<nola_types::AgentRoom>("agent_room") {
        Ok(list) if !list.is_empty() => {
            for r in list {
                println!("  {} — {}", r.id, r.name);
            }
        }
        Ok(_) => println!("No agent rooms found."),
        Err(e) => eprintln!("room list error: {e}"),
    }
}

fn handle_tui_room_cmd(
    suite: &Arc<ModulesSuite>,
    caller: &Arc<dyn ModelCaller>,
    route: &ModelRouteConfig,
    policy: &EffectivePolicy,
    rest: &str,
) {
    if rest.is_empty() || rest == "list" {
        list_tui_rooms(suite);
        return;
    }
    let mut parts = rest.splitn(2, char::is_whitespace);
    let room_id = parts.next().unwrap_or("").trim();
    let tail = parts.next().unwrap_or("").trim();
    if room_id.is_empty() || tail.is_empty() {
        eprintln!("Usage: /room <room-id> [@recipient] <message>");
        return;
    }
    let (recipient, message) = if let Some(after_at) = tail.strip_prefix('@') {
        let mut rparts = after_at.splitn(2, char::is_whitespace);
        let rec = rparts.next().unwrap_or("").trim();
        let msg = rparts.next().unwrap_or("").trim();
        if rec.is_empty() || msg.is_empty() {
            (None, tail)
        } else {
            (Some(rec), msg)
        }
    } else {
        (None, tail)
    };
    match RoomRuntime::send_message_to(
        suite,
        Arc::clone(caller),
        route.clone(),
        policy.clone(),
        room_id,
        message,
        recipient,
    ) {
        Ok(replies) => {
            for r in replies {
                let name = r["expert_name"].as_str().unwrap_or("Expert");
                let content = r["content"].as_str().unwrap_or("");
                println!("\n[{name}] {content}");
            }
            println!();
        }
        Err(e) => eprintln!("room error: {e}"),
    }
}
