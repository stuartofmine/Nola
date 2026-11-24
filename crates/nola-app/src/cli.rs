// SPDX-License-Identifier: Apache-2.0
//! Unified CLI command router supporting headless JSON/text runs, Harbor evals,
//! model config, MCP management, sessions, modules, TUI, WebHost, and UDS daemon.

use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nola_engine::{AgentEngine, AgentEvent, AgentRunConfig, EffectivePolicy, HookRunner, HttpModelCaller};
use nola_mcp::McpManager;
use nola_modules::ModulesSuite;
use nola_store::{Store, Vault};
use nola_tools::{SandboxMode, ToolContext, ToolRegistry};
use nola_types::{HostCapabilities, PermissionProfile};
use serde_json::{json, Value};

use crate::config::{
    apply_config_file, build_frozen_system_blocks, default_db_path, default_home_dir,
    default_vault_path, load_active_config, resolve_model_route,
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
    force_tool_set: String,
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

    // Dispatch explicit subcommands first
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
            "model" => return handle_model_cmd(&store, &opts.positional[1..]),
            "mcp" => return handle_mcp_cmd(&store, &db_path, &opts.positional[1..]),
            "sessions" => return handle_sessions_cmd(&store, &opts.positional[1..]),
            "modules" => return handle_modules_cmd(&suite, &opts.positional[1..]),
            "web" => return crate::web_server::run_web_server(store, suite, &opts.positional[1..]),
            "server" => return crate::daemon::run_daemon(store, suite, &db_path, &opts.positional[1..]),
            "tui" => return crate::tui::run_tui(store, suite, &workspace),
            _ => {}
        }
    }

    // Default: run prompt (stdin or --prompt or positional) or launch interactive TUI
    let prompt_text = resolve_prompt(&opts)?;
    let Some(prompt) = prompt_text else {
        if io::stdin().is_terminal() && !opts.json_mode {
            return crate::tui::run_tui(store, suite, &workspace);
        }
        return Err("no prompt provided on stdin or via --prompt".into());
    };

    let cfg = load_active_config(&store);
    let route = resolve_model_route(&cfg)?;

    let profile = opts
        .permission_profile
        .or_else(|| {
            std::env::var("NOLA_PERMISSION_PROFILE")
                .ok()
                .and_then(|s| PermissionProfile::parse(&s))
        })
        .or_else(|| opts.yes.then_some(PermissionProfile::AllowAll))
        .unwrap_or(PermissionProfile::Cautious);

    let net_policy = opts
        .network_policy
        .as_deref()
        .or(cfg.network_policy.as_deref())
        .unwrap_or("allow");
    let network_allowed = !net_policy.eq_ignore_ascii_case("deny");

    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);

    // Attach enabled MCP servers (best-effort, non-fatal if offline)
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
    ctx.network_allowed = network_allowed;
    ctx.permission_profile = profile;
    ctx.sandbox_mode = SandboxMode::from_env();

    let caller = Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
    let engine = AgentEngine::new(store.clone(), Arc::new(registry), caller);

    let cancel = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&cancel));

    let workspace_trusted = profile == PermissionProfile::AllowAll;
    let policy = EffectivePolicy::load(
        &workspace,
        home.as_deref(),
        workspace_trusted,
        Some(profile),
        network_allowed,
    );
    let hooks = HookRunner::load(&workspace, home.as_deref(), workspace_trusted);

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

    let max_steps = opts
        .max_steps
        .or(cfg.max_iterations)
        .unwrap_or(0); // 0 = unlimited

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
        force_all_tools: opts.force_tool_set == "all",
        stream: true,
        json_schema,
        cancel,
    };

    let res = engine
        .run(&ctx, &prompt, run_cfg, |ev| {
            if json_mode {
                if let Ok(line) = serde_json::to_string(&ev) {
                    println!("{line}");
                }
            } else if let AgentEvent::TextDelta { delta } = ev {
                print!("{delta}");
            }
        })
        .map_err(|e| e.to_string())?;

    if !json_mode && !res.final_text.ends_with('\n') {
        println!();
    }
    Ok(0)
}

fn parse_args(args: &[String]) -> Result<CliOptions, String> {
    let mut opts = CliOptions {
        force_tool_set: "auto".into(),
        ..Default::default()
    };
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
            "--db" => {
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
                i += 1; // accepted for CLI compatibility
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
                opts.force_tool_set = args
                    .get(i)
                    .ok_or("--force-tool-set requires auto|all|write")?
                    .to_ascii_lowercase();
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

fn handle_sessions_cmd(store: &Store, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let sessions = store.list_sessions(50).map_err(|e| e.to_string())?;
            println!("{}", serde_json::to_string_pretty(&sessions).unwrap());
            Ok(0)
        }
        Some("delete") => {
            let id = args.get(1).ok_or("usage: nola sessions delete <id>")?;
            store.delete_session(id).map_err(|e| e.to_string())?;
            println!("{}", json!({"deleted": id}));
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

fn print_help() {
    println!(
        "Nola v2 (Lean 8-Crate Architecture)\n\n\
         Usage:\n  \
           nola [options] [prompt]\n  \
           nola model apply-config <config.json>\n  \
           nola mcp [list|auth <id>]\n  \
           nola sessions [list|delete <id>|backup <dest>]\n  \
           nola modules <goal|sop|agent_room|expert_peer|data_source|automation|messaging_channel|local_model|plugin|skills>\n  \
           nola web [--host 127.0.0.1] [--port 7860]\n  \
           nola server [--socket <path>]\n  \
           nola tui"
    );
}
