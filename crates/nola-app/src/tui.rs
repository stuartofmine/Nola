// SPDX-License-Identifier: Apache-2.0
//! Rich interactive terminal UI (`crossterm`) with dimmed reasoning streaming,
//! collapsible tool output toggle (`/verbose`), session-scoped `(tool, target)`
//! permission approvals (`Cautious` default), MCP tool discovery, slash commands,
//! persistent context usage footer, and automatic post-turn memory distillation.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::QueueableCommand;
use nola_engine::{
    compact_session_now, register_planning_tools, register_subagent_tools, AgentEngine, AgentEvent,
    AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller, ModelCaller,
    PlanningState, SubagentManager,
};
use nola_llm::{LlmClient, LlmError, PrivacyRedactor};
use nola_mcp::McpManager;
use nola_modules::local_models_plugins::find_binary_on_path;
use nola_modules::{MemoryWorker, ModulesSuite, RoomRuntime};
use nola_store::{Store, Vault};
use nola_tools::{SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    new_id, now_ms, sha256_hex, GoalRecord, GoalStatus, HostCapabilities, ModelProtocol,
    ModelRouteConfig, ModelStreamDelta, NormalizedModelResponse, PermissionProfile,
    StructuredModelRequest, TokenUsage,
};
use serde_json::{json, Value};

use crate::config::{
    build_frozen_system_blocks, connection_has_api_key, default_vault_path, load_active_config,
    resolve_model_route, AppliedConfig,
};

#[derive(Debug, Clone)]
pub struct TuiLaunchOptions {
    pub db_path: PathBuf,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub permission_profile: Option<PermissionProfile>,
    pub yes: bool,
    pub resume: bool,
    pub session_id: Option<String>,
}

struct TrackingModelCaller {
    inner: HttpModelCaller,
    last_step_usage: Arc<Mutex<Option<TokenUsage>>>,
}

impl ModelCaller for TrackingModelCaller {
    fn call(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        let resp = self.inner.call(route, req, stream, on_delta)?;
        if let Ok(mut guard) = self.last_step_usage.lock() {
            *guard = Some(resp.usage.clone());
        }
        Ok(resp)
    }
}

struct RawModeGuard {
    active: bool,
}

impl RawModeGuard {
    fn enter() -> Option<Self> {
        if io::stdin().is_terminal() && enable_raw_mode().is_ok() {
            Some(Self { active: true })
        } else {
            None
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_raw_mode();
            self.active = false;
        }
    }
}

pub fn run_tui(
    store: Store,
    suite: Arc<ModulesSuite>,
    workspace: &Path,
    launch: TuiLaunchOptions,
) -> Result<u8, String> {
    let cfg = load_active_config(&store);
    let mut route = resolve_model_route(&cfg)?;
    if let Some(ref model_override) = launch.model {
        apply_tui_model_override(&cfg, &mut route, model_override);
    }
    if let Some(ref ep_override) = launch.endpoint {
        route.endpoint = ep_override.clone();
    }

    let last_step_usage = Arc::new(Mutex::new(None::<TokenUsage>));
    let caller: Arc<dyn ModelCaller> = Arc::new(TrackingModelCaller {
        inner: HttpModelCaller::new(300).map_err(|e| e.to_string())?,
        last_step_usage: Arc::clone(&last_step_usage),
    });
    let llm_client = LlmClient::new(120).ok();

    let network_allowed = !cfg
        .network_policy
        .as_deref()
        .unwrap_or("allow")
        .eq_ignore_ascii_case("deny");

    // Never implicitly trust repository `.nola/settings.json` or `.nola/hooks.json` in TUI!
    let workspace_trusted = false;
    let mut permission_profile = launch
        .permission_profile
        .or_else(|| {
            std::env::var("NOLA_PERMISSION_PROFILE")
                .ok()
                .and_then(|s| PermissionProfile::parse(&s))
        })
        .or_else(|| launch.yes.then_some(PermissionProfile::AllowAll))
        .unwrap_or(PermissionProfile::Cautious);
    let mut base_policy = EffectivePolicy::load(
        workspace,
        suite.user_home.as_deref(),
        workspace_trusted,
        Some(permission_profile),
        network_allowed,
    );

    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);

    // Wire enabled MCP servers into TUI tool registry using the active db's isolated vault.
    if let Ok(servers) = store.list_mcp_servers() {
        let vault = Vault::open(default_vault_path(&launch.db_path))
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
    ctx.network_allowed = network_allowed;
    ctx.permission_profile = permission_profile;
    ctx.sandbox_mode = SandboxMode::from_env();

    let (mut session_id, mut first_turn) = if let Some(sid) = launch.session_id.clone() {
        let exists = store.get_session(&sid).ok().flatten().is_some();
        (sid, !(launch.resume || exists))
    } else if launch.resume {
        if let Ok(list) = store.list_sessions(1) {
            if let Some(latest) = list.into_iter().next() {
                (latest.id, false)
            } else {
                (new_id(), true)
            }
        } else {
            (new_id(), true)
        }
    } else {
        (new_id(), true)
    };

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
    if !first_turn {
        println!("Resuming session {session_id}");
    }
    println!(
        "Type /help for slash commands (/edit /resume /model /allow /compact /config /goal /room /new /quit)\n"
    );
    let _ = stdout.flush();

    let stdin = io::stdin();
    let mut input = String::new();
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

        let prompt_to_run: String;
        if let Some(escaped) = trimmed.strip_prefix("//") {
            prompt_to_run = format!("/{escaped}");
        } else if trimmed.starts_with('/') {
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
                "/edit" => match compose_prompt_in_editor() {
                    Ok(Some(composed)) => {
                        println!("Submitting composed prompt ({} chars)...", composed.chars().count());
                        prompt_to_run = composed;
                    }
                    Ok(None) => {
                        println!("Editor closed with empty prompt; nothing sent.");
                        continue;
                    }
                    Err(e) => {
                        eprintln!("edit error: {e}");
                        continue;
                    }
                },
                "/resume" => {
                    if rest.is_empty() {
                        eprintln!("Usage: /resume <session-id>");
                        continue;
                    }
                    match store.get_session(rest) {
                        Ok(Some(sess)) => {
                            session_id = sess.id;
                            first_turn = false;
                            session_grants.lock().unwrap().clear();
                            println!(
                                "Resumed session {} ({}) (cleared session permission grants)",
                                session_id, sess.title
                            );
                        }
                        Ok(None) => eprintln!("Session '{rest}' not found. Run /sessions to list."),
                        Err(e) => eprintln!("resume error: {e}"),
                    }
                    continue;
                }
                "/new" => {
                    session_id = new_id();
                    first_turn = true;
                    session_grants.lock().unwrap().clear();
                    println!(
                        "Started new session {session_id} (cleared session permission grants)"
                    );
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
                                    network_allowed,
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
                    print_redacted_tui_config(
                        &cfg,
                        &route,
                        permission_profile,
                        workspace,
                        &launch.db_path,
                    );
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
                    eprintln!(
                        "Unknown slash command '{other}'. Type /help for available commands (use // to send a literal leading '/')."
                    );
                    continue;
                }
            }
        } else {
            prompt_to_run = trimmed.to_string();
        }

        cancel.store(false, Ordering::Relaxed);
        if let Ok(mut guard) = last_step_usage.lock() {
            *guard = None;
        }
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
                network_allowed,
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
            max_steps: cfg.max_iterations.unwrap_or(0),
            force_tool_set: ForceToolSet::Auto,
            stream: true,
            json_schema: None,
            cancel: Arc::clone(&cancel),
            auto_recall_memory: true,
            inbox: None,
        };
        first_turn = false;

        match engine.run(&ctx, &prompt_to_run, run_cfg, |ev| match ev {
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
                let safe_args = PrivacyRedactor::redact_text(
                    &serde_json::to_string(&arguments).unwrap_or_default(),
                );
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
                let _ = MemoryWorker::distill_turn(&suite, "tui", &prompt_to_run, &res.final_text);
                let step_usage = last_step_usage.lock().ok().and_then(|g| g.clone());
                println!();
                print_context_usage_footer(&route, step_usage.as_ref().unwrap_or(&res.usage));
                println!();
            }
            Err(e) => eprintln!("\nerror: {e}\n"),
        }
    }
    ctx.pty.close_all();
    Ok(0)
}

fn apply_tui_model_override(cfg: &AppliedConfig, route: &mut ModelRouteConfig, arg: &str) {
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
                    return;
                }
            }
        }
    }
    route.model = arg.to_string();
}

fn compose_prompt_in_editor() -> Result<Option<String>, String> {
    let editor_cmd = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            ["vi", "nano"]
                .iter()
                .find(|bin| find_binary_on_path(bin).is_some())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| "no editor found: set $VISUAL or $EDITOR (or install vi/nano)".to_string())?;

    let mut parts = editor_cmd.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| "empty $VISUAL/$EDITOR command".to_string())?;
    let extra_args: Vec<&str> = parts.collect();

    let tmp_path = std::env::temp_dir().join(format!("nola-prompt-{}.txt", new_id()));
    if fs::symlink_metadata(&tmp_path).is_ok() {
        return Err(format!(
            "refusing to overwrite existing temp file '{}'",
            tmp_path.display()
        ));
    }
    fs::write(&tmp_path, "").map_err(|e| format!("failed to create temp prompt file: {e}"))?;

    let status = Command::new(program)
        .args(&extra_args)
        .arg(&tmp_path)
        .status()
        .map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            format!("failed to launch editor '{program}': {e}")
        })?;

    let content = fs::read_to_string(&tmp_path).unwrap_or_default();
    let _ = fs::remove_file(&tmp_path);

    if !status.success() {
        return Err(format!("editor '{program}' exited with status {status}"));
    }

    let trimmed = content.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalChoice {
    AllowOnce,
    AllowForTargetInSession,
    ReturnToInput,
    Deny,
}

fn read_approval_choice() -> ApprovalChoice {
    if let Some(_raw_guard) = RawModeGuard::enter() {
        let choice = loop {
            match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                    if k.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(
                            k.code,
                            KeyCode::Char('c')
                                | KeyCode::Char('C')
                                | KeyCode::Char('d')
                                | KeyCode::Char('D')
                        )
                    {
                        break ApprovalChoice::ReturnToInput;
                    }
                    if k.modifiers.is_empty() || k.modifiers == KeyModifiers::SHIFT {
                        match k.code {
                            KeyCode::Char('1') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                                break ApprovalChoice::AllowOnce;
                            }
                            KeyCode::Char('2') | KeyCode::Char('a') | KeyCode::Char('A') => {
                                break ApprovalChoice::AllowForTargetInSession;
                            }
                            KeyCode::Char('3')
                            | KeyCode::Char('r')
                            | KeyCode::Char('R')
                            | KeyCode::Esc => {
                                break ApprovalChoice::ReturnToInput;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter => {
                                break ApprovalChoice::Deny;
                            }
                            _ => {}
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => break ApprovalChoice::Deny,
            }
        };
        drop(_raw_guard);
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
    if matches!(tool_name, "bash" | "pty_exec") {
        if let Some(cmd) = args
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let normalized: String = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
            let hash = sha256_hex(normalized.as_bytes());
            let short_hash = &hash[..16];
            let redacted = PrivacyRedactor::redact_text(&normalized);
            let preview: String = redacted.chars().take(80).collect();
            return format!("{preview} [sha256:{short_hash}]");
        }
    }
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
        let normalized: String = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
        let hash = sha256_hex(normalized.as_bytes());
        let short_hash = &hash[..16];
        let redacted = PrivacyRedactor::redact_text(&normalized);
        let preview: String = redacted.chars().take(80).collect();
        return format!("{preview} [sha256:{short_hash}]");
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

fn print_context_usage_footer(route: &ModelRouteConfig, latest_usage: &TokenUsage) {
    let excl_cache = route.protocol == ModelProtocol::AnthropicMessages;
    let used_tokens = if excl_cache {
        latest_usage
            .input_tokens
            .saturating_add(latest_usage.cache_read_tokens)
            .saturating_add(latest_usage.cache_write_tokens)
    } else {
        latest_usage.input_tokens
    };
    let window = route.context_window.max(1) as u64;
    let pct = ((used_tokens as f64 / window as f64) * 100.0).min(100.0);
    let used_k = used_tokens as f64 / 1000.0;
    let win_k = window as f64 / 1000.0;
    let cache_bps = latest_usage
        .cache_hit_rate_bps_normalized(excl_cache)
        .unwrap_or_else(|| latest_usage.cache_hit_rate_bps());
    let cache_pct = cache_bps as f64 / 100.0;

    let mut out = io::stdout();
    let _ = out.queue(SetAttribute(Attribute::Dim));
    print!("Context {pct:.0}% · {used_k:.1}k/{win_k:.0}k · cache hit {cache_pct:.0}%");
    let _ = out.queue(ResetColor);
    let _ = out.flush();
}

fn print_tui_help() {
    println!(
        "Slash commands:\n  \
         /help                              Show this help\n  \
         /edit                              Compose a multi-line prompt in $VISUAL/$EDITOR\n  \
         /resume <session-id>               Resume an existing session\n  \
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
         /quit                              Exit TUI\n  \
         //<text>                           Send a literal message starting with '/'"
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
    println!(
        "Switched active model to '{}' on connection '{}'",
        route.model, route.connection_id
    );
}

fn print_redacted_tui_config(
    cfg: &AppliedConfig,
    route: &ModelRouteConfig,
    profile: PermissionProfile,
    workspace: &Path,
    db_path: &Path,
) {
    let connections: Vec<Value> = cfg
        .llm
        .connections
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "endpoint": c.endpoint,
                "protocol": c.protocol,
                "has_api_key": connection_has_api_key(c),
                "models": c.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "workspace": workspace.display().to_string(),
            "db_path": db_path.display().to_string(),
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
            "max_iterations": cfg.max_iterations,
            "connections": connections
        }))
        .unwrap_or_else(|_| "{}".into())
    );
}

fn handle_tui_goal_cmd(suite: &ModulesSuite, workspace: &Path, rest: &str) {
    let mut parts = rest.splitn(2, char::is_whitespace);
    let sub = parts.next().unwrap_or("").trim();
    let body = parts.next().unwrap_or("").trim();

    if sub.is_empty() || sub == "list" {
        match suite.list_records::<GoalRecord>("goal") {
            Ok(list) if !list.is_empty() => {
                for g in list {
                    println!("  {} [{:?}] {} — {}", g.id, g.status, g.title, g.objective);
                }
            }
            Ok(_) => {
                println!("No goals found. Create one with `/goal create <title> | <objective>`.")
            }
            Err(e) => eprintln!("goal list error: {e}"),
        }
        return;
    }

    if sub == "create" {
        if body.is_empty() {
            eprintln!("Usage: /goal create <title> | <objective>");
            return;
        }
        let (title, objective) = match body.split_once('|') {
            Some((t, o)) => (t.trim(), o.trim()),
            None => (body, body),
        };
        if title.is_empty() || objective.is_empty() {
            eprintln!("Goal title and objective must not be empty. Usage: /goal create <title> | <objective>");
            return;
        }
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
            eprintln!("Usage: /room <room-id> [@recipient] <message>");
            return;
        }
        (Some(rec), msg)
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
