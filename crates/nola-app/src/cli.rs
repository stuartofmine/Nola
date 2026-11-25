// SPDX-License-Identifier: Apache-2.0
//! Unified CLI command router supporting headless JSON/text runs, Harbor evals,
//! model config, MCP management, sessions, modules, GoalWorker, RoomRuntime,
//! AutomationScheduler, Software Updater, TUI, WebHost, and UDS daemon.

use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::Utc;
use nola_engine::{
    register_planning_tools, register_subagent_tools, AgentEngine, AgentEvent, AgentRunConfig,
    EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller, ModelCaller, PlanningState,
    SubagentManager,
};
use nola_mcp::McpManager;
use nola_modules::{check_for_update, AutomationScheduler, GoalWorker, ModulesSuite, RoomRuntime};
use nola_store::{Store, Vault};
use nola_tools::{OutputStore, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{HostCapabilities, PermissionProfile};
use serde_json::{json, Value};

use crate::config::{
    apply_config_file, build_frozen_system_blocks, default_db_path, default_home_dir,
    default_vault_path, load_active_config, resolve_model_route, AppliedConfig,
};

#[derive(Debug, Default)]
struct CliOptions {
    project: Option<PathBuf>,
    db_path: Option<PathBuf>,
    json_mode: bool,
    json_schema: Option<String>,
    yes: bool,
    permission_profile: Option<PermissionProfile>,
    network_policy: Option<String>,
    force_tool_set: ForceToolSet,
    session_id: Option<String>,
    resume: bool,
    max_steps: Option<usize>,
    prompt: Option<String>,
    positional: Vec<String>,
}

pub fn run_from_env() -> Result<u8, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    run_with_args(&args)
}

pub fn run_with_args(args: &[String]) -> Result<u8, String> {
    let opts = parse_args(args)?;
    let workspace = opts
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let db_path = opts.db_path.clone().unwrap_or_else(default_db_path);
    let store = Store::open(&db_path).map_err(|e| format!("failed to open store: {e}"))?;
    let home = Some(default_home_dir());
    let suite = Arc::new(ModulesSuite::new(store.clone(), workspace.clone(), home.clone()));

    if let Some(cmd) = opts.positional.first().map(String::as_str) {
        match cmd {
            "help" | "--help" | "-h" => {
                print_help();
                return Ok(0);
            }
            "version" | "--version" | "-V" => {
                println!("nola 2.0.0");
                return Ok(0);
            }
            "--list-execution-backends" => {
                println!(
                    "{}",
                    json!([{
                        "id": "nola-v2",
                        "display_name": "Nola v2 Rust Engine",
                        "available": true,
                        "capabilities": {
                            "supports_permission_bridge": true
                        }
                    }])
                );
                return Ok(0);
            }
            "model" => return handle_model_cmd(&store, &opts.positional[1..]),
            "mcp" => return handle_mcp_cmd(&store, &db_path, &opts.positional[1..]),
            "memory" => return handle_memory_cmd(&store, &opts.positional[1..]),
            "sessions" | "session" => {
                return handle_sessions_cmd(&store, &opts.positional[1..], opts.json_mode)
            }
            "plugin" => return handle_plugin_cmd(&opts.positional[1..]),
            "modules" => return handle_modules_cmd(&suite, &opts.positional[1..]),
            "goal" => return handle_goal_cmd(&store, &suite, &opts, &opts.positional[1..]),
            "room" => return handle_room_cmd(&store, &suite, &opts, &opts.positional[1..]),
            "scheduler" => return handle_scheduler_cmd(&store, &suite, &opts, &opts.positional[1..]),
            "update" => return handle_update_cmd(&opts.positional[1..]),
            "web" => {
                return crate::web_server::run_web_server(
                    store,
                    suite,
                    &opts.positional[1..],
                    opts.json_mode,
                )
            }
            "server" => {
                let rest = &opts.positional[1..];
                let wants_http = rest.iter().any(|a| a == "--host" || a == "--port");
                if wants_http {
                    return crate::web_server::run_web_server(store, suite, rest, opts.json_mode);
                }
                return crate::daemon::run_daemon(store, suite, &db_path, rest);
            }
            "tui" => return crate::tui::run_tui(store, suite, &workspace),
            other if other.starts_with("--") => {
                return Err(format!("unknown option '{other}'. Run 'nola --help' for usage."));
            }
            _ => {}
        }
    }

    let prompt_text = resolve_prompt(&opts)?;
    let Some(prompt) = prompt_text else {
        if io::stdin().is_terminal() && !opts.json_mode {
            return crate::tui::run_tui(store, suite, &workspace);
        }
        return Err("no prompt provided on stdin or via --prompt".into());
    };

    let cfg = load_active_config(&store);
    let route = resolve_model_route(&cfg)?;

    let profile = resolve_profile(&opts);
    let net_policy = opts
        .network_policy
        .as_deref()
        .or(cfg.network_policy.as_deref())
        .unwrap_or("allow");
    let network_allowed = !net_policy.eq_ignore_ascii_case("deny");

    let caller: Arc<dyn ModelCaller> =
        Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
    let workspace_trusted = profile == PermissionProfile::AllowAll;
    let policy = EffectivePolicy::load(
        &workspace,
        home.as_deref(),
        workspace_trusted,
        Some(profile),
        network_allowed,
    );

    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);

    let cancel = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&cancel));
    let hooks = HookRunner::load(&workspace, home.as_deref(), workspace_trusted);

    let subagents = Arc::new(SubagentManager::new(store.clone(), Arc::clone(&caller)));
    subagents.configure_parent(route.clone(), policy.clone());
    subagents.configure_runtime(hooks.clone(), Some(Arc::clone(&cancel)));
    register_subagent_tools(&subagents, &mut registry);

    let planning = Arc::new(PlanningState::default());
    register_planning_tools(store.clone(), &planning, &mut registry);

    if let Ok(servers) = store.list_mcp_servers() {
        let vault = Vault::open(default_vault_path(&db_path)).ok().map(Arc::new);
        let mcp = Arc::new(McpManager::new(vault));
        for srv in servers.into_iter().filter(|s| s.enabled) {
            if let Ok(tools) = mcp.discover_tools(&srv) {
                for t in tools {
                    registry.register(t);
                }
            }
        }
    }

    let mut ctx = ToolContext::new(&workspace);
    ctx.output_store = Arc::new(OutputStore::new(PathBuf::from(format!(
        "{}.outputs",
        db_path.display()
    ))));
    ctx.network_allowed = network_allowed;
    ctx.permission_profile = profile;
    ctx.sandbox_mode = SandboxMode::from_env();

    let engine = AgentEngine::new(store.clone(), Arc::new(registry), caller);

    let json_schema: Option<Value> = match &opts.json_schema {
        Some(raw) => {
            let text = if std::path::Path::new(raw).is_file() {
                std::fs::read_to_string(raw).map_err(|e| e.to_string())?
            } else {
                raw.clone()
            };
            Some(serde_json::from_str(&text).map_err(|e| format!("invalid --json-schema: {e}"))?)
        }
        None => None,
    };

    let max_steps = opts.max_steps.or(cfg.max_iterations).unwrap_or(0);
    let is_eval = std::env::var("NOLA_BENCHMARK_TOKEN").is_ok();
    let json_mode = opts.json_mode;
    let run_cfg = AgentRunConfig {
        session_id: opts.session_id.clone(),
        resume: opts.resume,
        route,
        frozen_system_blocks: build_frozen_system_blocks(&workspace, &cfg),
        host_capabilities: HostCapabilities {
            interactive_user_input: false,
            network_allowed,
        },
        policy,
        hooks,
        approval: None,
        max_steps,
        force_tool_set: opts.force_tool_set,
        stream: true,
        json_schema,
        cancel,
        auto_recall_memory: !is_eval,
        inbox: None,
    };

    let res = engine
        .run(&ctx, &prompt, run_cfg, |ev| {
            if json_mode {
                if let Ok(line) = serde_json::to_string(&ev) {
                    println!("{line}");
                }
            } else if let AgentEvent::TextDelta { delta } = ev {
                print!("{delta}");
                let _ = io::stdout().flush();
            }
        })
        .map_err(|e| e.to_string())?;

    ctx.pty.close_all();
    if !json_mode && !res.final_text.ends_with('\n') {
        println!();
    }
    Ok(0)
}

fn resolve_profile(opts: &CliOptions) -> PermissionProfile {
    opts.permission_profile
        .or_else(|| {
            std::env::var("NOLA_PERMISSION_PROFILE")
                .ok()
                .and_then(|s| PermissionProfile::parse(&s))
        })
        .or_else(|| opts.yes.then_some(PermissionProfile::AllowAll))
        .unwrap_or(PermissionProfile::Cautious)
}

fn resolve_network_allowed(opts: &CliOptions, cfg: &AppliedConfig) -> bool {
    let net_policy = opts
        .network_policy
        .as_deref()
        .or(cfg.network_policy.as_deref())
        .unwrap_or("allow");
    !net_policy.eq_ignore_ascii_case("deny")
}

fn parse_args(args: &[String]) -> Result<CliOptions, String> {
    let mut opts = CliOptions::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--project" | "-C" => {
                i += 1;
                opts.project = Some(PathBuf::from(
                    args.get(i).ok_or("--project requires a directory")?,
                ));
            }
            "--db" | "--database" => {
                i += 1;
                opts.db_path = Some(PathBuf::from(args.get(i).ok_or("--db requires a path")?));
            }
            "--json" => opts.json_mode = true,
            "--output-format" => {
                i += 1;
                let fmt = args.get(i).ok_or("--output-format requires a value")?;
                opts.json_mode = fmt == "json" || fmt == "stream-json";
            }
            "--json-schema" => {
                i += 1;
                opts.json_schema = Some(
                    args.get(i)
                        .ok_or("--json-schema requires JSON or file path")?
                        .clone(),
                );
            }
            "--lang" => {
                i += 1;
            }
            "--yes" | "-y" => opts.yes = true,
            "--permission-profile" => {
                i += 1;
                let raw = args.get(i).ok_or("--permission-profile requires a value")?;
                opts.permission_profile = Some(
                    PermissionProfile::parse(raw)
                        .ok_or_else(|| format!("unknown permission profile '{raw}'"))?,
                );
            }
            "--network-policy" => {
                i += 1;
                opts.network_policy = Some(
                    args.get(i)
                        .ok_or("--network-policy requires allow|ask|deny")?
                        .clone(),
                );
            }
            "--force-tool-set" => {
                i += 1;
                let raw = args
                    .get(i)
                    .ok_or("--force-tool-set requires auto|all|write")?
                    .to_ascii_lowercase();
                opts.force_tool_set = match raw.as_str() {
                    "auto" => ForceToolSet::Auto,
                    "write" => ForceToolSet::Write,
                    "all" => ForceToolSet::All,
                    other => {
                        return Err(format!(
                            "invalid --force-tool-set '{other}' (expected auto|write|all)"
                        ));
                    }
                };
            }
            "--session" => {
                i += 1;
                opts.session_id = Some(args.get(i).ok_or("--session requires an id")?.clone());
            }
            "--resume" => opts.resume = true,
            "--max-steps" | "--max-iterations" => {
                i += 1;
                let n: usize = args
                    .get(i)
                    .ok_or("--max-steps requires an integer")?
                    .parse()
                    .map_err(|_| "--max-steps must be an integer")?;
                opts.max_steps = Some(n);
            }
            "--prompt" | "-p" => {
                i += 1;
                opts.prompt = Some(args.get(i).ok_or("--prompt requires text")?.clone());
            }
            _ => opts.positional.push(a.to_string()),
        }
        i += 1;
    }
    Ok(opts)
}

fn resolve_prompt(opts: &CliOptions) -> Result<Option<String>, String> {
    if let Some(p) = &opts.prompt {
        return Ok(Some(p.clone()));
    }
    if !opts.positional.is_empty() {
        return Ok(Some(opts.positional.join(" ")));
    }
    if !io::stdin().is_terminal() {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("failed to read stdin: {e}"))?;
        if !buf.trim().is_empty() {
            return Ok(Some(buf));
        }
    }
    Ok(None)
}

fn handle_model_cmd(store: &Store, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("apply-config") => {
            let path = args
                .get(1)
                .ok_or("usage: nola model apply-config <config.json>")?;
            let cfg = apply_config_file(store, std::path::Path::new(path))?;
            let route = resolve_model_route(&cfg)?;
            println!(
                "{}",
                json!({
                    "applied": true,
                    "connection": route.connection_id,
                    "model": route.model,
                    "protocol": route.protocol.as_str(),
                    "endpoint": route.endpoint
                })
            );
            Ok(0)
        }
        Some("list") | None => {
            let cfg = load_active_config(store);
            println!(
                "{}",
                serde_json::to_string_pretty(&cfg.llm).unwrap_or_else(|_| "{}".into())
            );
            Ok(0)
        }
        Some(other) => Err(format!("unknown model subcommand '{other}'")),
    }
}

fn handle_mcp_cmd(store: &Store, db_path: &std::path::Path, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let servers = store.list_mcp_servers().map_err(|e| e.to_string())?;
            println!("{}", serde_json::to_string_pretty(&servers).unwrap());
            Ok(0)
        }
        Some("auth") => {
            let server_id = args.get(1).ok_or("usage: nola mcp auth <server_id>")?;
            let vault = Arc::new(
                Vault::open(default_vault_path(db_path)).map_err(|e| e.to_string())?,
            );
            let mgr = McpManager::new(Some(vault));
            let pkce = nola_mcp::McpOAuthManager::generate_pkce().map_err(|e| e.to_string())?;
            let has_token = mgr
                .oauth()
                .and_then(|o| o.load_tokens(server_id).ok().flatten())
                .is_some();
            println!(
                "{}",
                json!({
                    "server_id": server_id,
                    "authenticated": has_token,
                    "pkce_challenge": pkce.challenge,
                    "pkce_method": pkce.method
                })
            );
            Ok(0)
        }
        Some(other) => Err(format!("unknown mcp subcommand '{other}'")),
    }
}

fn handle_memory_cmd(store: &Store, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let items = store.search_memory("", None, 200).map_err(|e| e.to_string())?;
            let records: Vec<Value> = items
                .iter()
                .map(crate::web_api::memory_item_to_record)
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"records": records})).unwrap()
            );
            Ok(0)
        }
        Some("create") | Some("revise") => {
            let is_revise = args.first().map(String::as_str) == Some("revise");
            let explicit_id = if is_revise {
                args.get(1).cloned()
            } else {
                None
            };
            let mut raw = String::new();
            io::stdin()
                .read_to_string(&mut raw)
                .map_err(|e| format!("failed to read memory JSON from stdin: {e}"))?;
            let draft: Value =
                serde_json::from_str(raw.trim()).map_err(|e| format!("invalid memory JSON: {e}"))?;
            let ts = nola_types::now_ms();
            let id = explicit_id
                .or_else(|| draft.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
                .unwrap_or_else(nola_types::new_id);
            let scope = draft.get("scope").and_then(|v| v.as_str()).unwrap_or("global");
            let kind = draft.get("kind").and_then(|v| v.as_str()).unwrap_or("other");
            let content = draft.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let title = draft
                .get("summary")
                .or_else(|| draft.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or(kind)
                .to_string();
            let tags: Vec<String> = draft
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(ToString::to_string)).collect())
                .unwrap_or_default();
            let item = nola_types::MemoryItem {
                id,
                namespace: format!("{scope}:{kind}"),
                title,
                content,
                tags,
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            store.upsert_memory(&item).map_err(|e| e.to_string())?;
            let record = crate::web_api::memory_item_to_record(&item);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"record": record})).unwrap()
            );
            Ok(0)
        }
        Some("delete") => {
            let id = args.get(1).ok_or("usage: nola memory delete <id>")?;
            store.delete_memory(id).map_err(|e| e.to_string())?;
            println!("{}", json!({"ok": true, "deleted": id}));
            Ok(0)
        }
        Some(other) => Err(format!("unknown memory subcommand '{other}'")),
    }
}

fn handle_plugin_cmd(args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            println!("{}", json!({"plugins": []}));
            Ok(0)
        }
        Some(other) => Err(format!("unknown plugin subcommand '{other}'")),
    }
}

fn handle_sessions_cmd(store: &Store, args: &[String], json_mode: bool) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let sessions = store.list_sessions(200).map_err(|e| e.to_string())?;
            if json_mode {
                let summaries: Vec<Value> = sessions
                    .iter()
                    .map(|s| {
                        let (msgs, _) = crate::web_api::reconstruct_v2_messages(store, &s.id);
                        json!({
                            "id": s.id,
                            "title": s.title,
                            "updated_at": crate::web_api::ms_to_rfc3339(s.updated_at_ms),
                            "created_at": crate::web_api::ms_to_rfc3339(s.created_at_ms),
                            "message_count": msgs.len()
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"sessions": summaries})).unwrap()
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&sessions).unwrap());
            }
            Ok(0)
        }
        Some("delete") => {
            let id = args.get(1).ok_or("usage: nola sessions delete <id>")?;
            store.delete_session(id).map_err(|e| e.to_string())?;
            println!("{}", json!({"ok": true, "deleted": id}));
            Ok(0)
        }
        Some("backup") => {
            let dest = args.get(1).ok_or("usage: nola sessions backup <dest.db>")?;
            store
                .backup_to(std::path::Path::new(dest))
                .map_err(|e| e.to_string())?;
            println!("{}", json!({"backup": dest}));
            Ok(0)
        }
        Some(other) => Err(format!("unknown sessions subcommand '{other}'")),
    }
}

fn handle_modules_cmd(suite: &ModulesSuite, args: &[String]) -> Result<u8, String> {
    let kind = args.first().map(String::as_str).unwrap_or("goal");
    if kind == "skills" {
        let skills = nola_modules::skills::discover_skills(
            &suite.workspace_root,
            suite.user_home.as_deref(),
        );
        println!("{}", serde_json::to_string_pretty(&skills).unwrap());
        return Ok(0);
    }
    let items: Vec<Value> = suite.list_records(kind).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&items).unwrap());
    Ok(0)
}

fn handle_goal_cmd(
    store: &Store,
    suite: &Arc<ModulesSuite>,
    opts: &CliOptions,
    args: &[String],
) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("run") => {
            let goal_id = args.get(1).ok_or("usage: nola goal run <goal_id>")?;
            let cfg = load_active_config(store);
            let route = resolve_model_route(&cfg)?;
            let caller: Arc<dyn ModelCaller> =
                Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
            let profile = resolve_profile(opts);
            let network_allowed = resolve_network_allowed(opts, &cfg);
            let policy = EffectivePolicy::load(
                &suite.workspace_root,
                suite.user_home.as_deref(),
                profile == PermissionProfile::AllowAll,
                Some(profile),
                network_allowed,
            );
            let done = GoalWorker::execute_goal(
                suite,
                caller,
                route,
                policy,
                goal_id,
                opts.max_steps.unwrap_or(32),
                Arc::new(AtomicBool::new(false)),
            )?;
            println!("{}", serde_json::to_string_pretty(&done).unwrap());
            Ok(0)
        }
        Some("list") | None => handle_modules_cmd(suite, &["goal".to_string()]),
        Some(other) => Err(format!("unknown goal subcommand '{other}'")),
    }
}

fn handle_room_cmd(
    store: &Store,
    suite: &Arc<ModulesSuite>,
    opts: &CliOptions,
    args: &[String],
) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("send") => {
            let room_id = args
                .get(1)
                .ok_or("usage: nola room send <room_id> <message>")?;
            let msg = args[2..].join(" ");
            if msg.trim().is_empty() {
                return Err("room message cannot be empty".into());
            }
            let cfg = load_active_config(store);
            let route = resolve_model_route(&cfg)?;
            let caller: Arc<dyn ModelCaller> =
                Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
            let profile = resolve_profile(opts);
            let network_allowed = resolve_network_allowed(opts, &cfg);
            let policy = EffectivePolicy::load(
                &suite.workspace_root,
                suite.user_home.as_deref(),
                profile == PermissionProfile::AllowAll,
                Some(profile),
                network_allowed,
            );
            let replies = RoomRuntime::send_message(suite, caller, route, policy, room_id, &msg)?;
            println!("{}", serde_json::to_string_pretty(&replies).unwrap());
            Ok(0)
        }
        Some("list") | None => handle_modules_cmd(suite, &["agent_room".to_string()]),
        Some(other) => Err(format!("unknown room subcommand '{other}'")),
    }
}

fn handle_scheduler_cmd(
    store: &Store,
    suite: &Arc<ModulesSuite>,
    opts: &CliOptions,
    args: &[String],
) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("tick") => {
            let cfg = load_active_config(store);
            let route = resolve_model_route(&cfg)?;
            let caller: Arc<dyn ModelCaller> =
                Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
            let profile = resolve_profile(opts);
            let network_allowed = resolve_network_allowed(opts, &cfg);
            let policy = EffectivePolicy::load(
                &suite.workspace_root,
                suite.user_home.as_deref(),
                profile == PermissionProfile::AllowAll,
                Some(profile),
                network_allowed,
            );
            let results = AutomationScheduler::run_due(suite, caller, route, policy, Utc::now());
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &results
                        .into_iter()
                        .map(|(id, r)| json!({"id": id, "ok": r.is_ok(), "output": r.ok()}))
                        .collect::<Vec<_>>()
                )
                .unwrap()
            );
            Ok(0)
        }
        Some("list") | None => handle_modules_cmd(suite, &["automation".to_string()]),
        Some(other) => Err(format!("unknown scheduler subcommand '{other}'")),
    }
}

fn handle_update_cmd(args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("check") => {
            let url = args
                .get(1)
                .ok_or("usage: nola update check <https://.../manifest.json>")?;
            let res = check_for_update(url, "2.0.0")?;
            println!("{}", serde_json::to_string_pretty(&res).unwrap());
            Ok(0)
        }
        _ => Err("usage: nola update check <https://.../manifest.json>".into()),
    }
}

fn print_help() {
    println!(
        "Nola v2 (Lean 8-Crate Full-Capability Architecture)\n\n\
         Usage:\n  \
           nola [options] [prompt]\n  \
           nola model apply-config <config.json> | list\n  \
           nola mcp [list|auth <id>]\n  \
           nola sessions [list|delete <id>|backup <dest>]\n  \
           nola goal [list|run <goal_id>]\n  \
           nola room [list|send <room_id> <message>]\n  \
           nola scheduler [list|tick]\n  \
           nola update check <https://manifest.json>\n  \
           nola modules <kind|skills>\n  \
           nola web [--host 127.0.0.1] [--port 5001] [--web-root <dir>] [--json]\n  \
           nola server [--socket <path>] [--parent-pid <pid>]\n  \
           nola tui"
    );
}
