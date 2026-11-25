// SPDX-License-Identifier: Apache-2.0
//! Unified CLI command router supporting headless JSON/text runs, macOS `nola run`,
//! Harbor evals, config validation/apply, redacted model management & probing,
//! MCP CRUD/OAuth, skills, harness plugins, sessions, GoalWorker, RoomRuntime,
//! AutomationScheduler, Software Updater, TUI, WebHost, and UDS daemon.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use nola_engine::{
    register_planning_tools, register_subagent_tools, AgentEngine, AgentEvent, AgentRunConfig,
    EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller, ModelCaller, PlanningState,
    SubagentManager,
};
use nola_mcp::McpManager;
use nola_modules::local_models_plugins::{
    find_binary_on_path, list_desktop_plugin_pages, list_harness_plugins, KIND_PLUGIN,
};
use nola_modules::skills::{
    discover_skills_detailed, prepare_github_skill_package, set_skill_enabled,
};
use nola_modules::{
    check_for_update, download_update_package, AutomationScheduler, GoalWorker, ModulesSuite,
    RoomRuntime,
};
use nola_store::{Store, Vault, PROTOCOL_VERSION};
use nola_tools::{OutputStore, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    new_id, HostCapabilities, McpServerConfig, McpTransport, ModelCapabilities, ModelProtocol,
    ModelRouteConfig, PermissionProfile, PluginRecord,
};
use serde_json::{json, Value};

use crate::config::{
    apply_config_file, bind_store_vault_path, build_frozen_system_blocks, connection_has_api_key,
    default_db_path, default_home_dir, default_vault_path, discover_config_path,
    load_active_config, resolve_model_route, strip_jsonc, AppliedConfig, ConnectionConfig,
    DefaultModelSelection,
};
use crate::tui::TuiLaunchOptions;

const MAX_SKILL_INSTALL_BYTES: u64 = 256 * 1024;

const EXAMPLE_CONFIG_JSONC: &str = r#"{
  // Nola v2 configuration template (supports JSONC comments & trailing commas)
  "llm": {
    "default_model": {
      "connection": "openai",
      "model": "gpt-4.1",
    },
    "connections": [
      {
        "id": "openai",
        "endpoint": "https://api.openai.com/v1",
        "protocol": "responses",
        // Prefer api_key_env over inline secrets!
        "api_key_env": "OPENAI_API_KEY",
        "models": [
          {
            "id": "gpt-4.1",
            "context_window": 128000,
            "max_output_tokens": 16384,
            "temperature": 0.2,
          },
        ],
      },
    ],
  },
  "network_policy": "allow",
  "max_iterations": 0,
  "instructions": [],
}
"#;

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
    model: Option<String>,
    endpoint: Option<String>,
    host_request_stdin: bool,
    allowed_dirs: Vec<PathBuf>,
    skill_dirs: Vec<PathBuf>,
    no_skills: bool,
    github_skills: Vec<String>,
    positional: Vec<String>,
}

pub fn run_from_env() -> Result<u8, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    run_with_args(&args)
}

fn is_known_subcommand(cmd: &str) -> bool {
    matches!(
        cmd,
        "help"
            | "version"
            | "init"
            | "config"
            | "model"
            | "mcp"
            | "skills"
            | "skill"
            | "memory"
            | "sessions"
            | "session"
            | "plugin"
            | "plugins"
            | "modules"
            | "goal"
            | "room"
            | "scheduler"
            | "update"
            | "run"
            | "web"
            | "server"
            | "tui"
    )
}

fn subcommand_wants_help(rest: &[String]) -> bool {
    let mut skip_next = false;
    for a in rest {
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(
            a.as_str(),
            "--arg"
                | "--command"
                | "--name"
                | "--url"
                | "--id"
                | "--version"
                | "--entry"
                | "--capability"
                | "--to"
                | "--sha256"
                | "--socket"
                | "--parent-pid"
                | "--host"
                | "--port"
                | "--web-root"
        ) {
            skip_next = true;
            continue;
        }
        if a == "--help" || a == "-h" {
            return true;
        }
    }
    false
}

pub fn run_with_args(args: &[String]) -> Result<u8, String> {
    let opts = parse_args(args)?;

    // Dispatch metadata and DB-free commands BEFORE opening SQLite (avoid side-effect DB creation).
    if let Some(cmd) = opts.positional.first().map(String::as_str) {
        let rest = &opts.positional[1..];
        if is_known_subcommand(cmd) && subcommand_wants_help(rest) {
            print_help();
            return Ok(0);
        }
        match cmd {
            "help" | "--help" | "-h" => {
                print_help();
                return Ok(0);
            }
            "version" | "--version" | "-V" => {
                let ver = env!("CARGO_PKG_VERSION");
                if opts.json_mode {
                    println!(
                        "{}",
                        json!({
                            "name": "nola",
                            "version": ver,
                            "protocol_version": PROTOCOL_VERSION
                        })
                    );
                } else {
                    println!("nola {ver}");
                }
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
            "config" if rest.first().map(String::as_str) == Some("example") => {
                print!("{EXAMPLE_CONFIG_JSONC}");
                return Ok(0);
            }
            "update" => return handle_update_cmd(rest),
            _ => {}
        }
    }

    let workspace = opts
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let db_path = opts.db_path.clone().unwrap_or_else(default_db_path);
    let store = Store::open(&db_path).map_err(|e| format!("failed to open store: {e}"))?;
    bind_store_vault_path(&store, &default_vault_path(&db_path))?;
    let home = Some(default_home_dir());
    let suite = Arc::new(ModulesSuite::new(store.clone(), workspace.clone(), home.clone()));

    if let Some(cmd) = opts.positional.first().map(String::as_str) {
        match cmd {
            "init" => return handle_init_cmd(&workspace, &opts.positional[1..]),
            "config" => return handle_config_cmd(&store, &workspace, &opts.positional[1..]),
            "model" => return handle_model_cmd(&store, &workspace, &opts.positional[1..]),
            "mcp" => return handle_mcp_cmd(&store, &db_path, &opts.positional[1..]),
            "skills" | "skill" => return handle_skills_cmd(&store, &suite, &opts.positional[1..]),
            "memory" => return handle_memory_cmd(&store, &opts.positional[1..]),
            "sessions" | "session" => {
                return handle_sessions_cmd(&store, &opts.positional[1..], opts.json_mode)
            }
            "plugin" | "plugins" => {
                return handle_plugin_cmd(&store, &suite, &opts.positional[1..])
            }
            "modules" => return handle_modules_cmd(&suite, &opts.positional[1..]),
            "goal" => return handle_goal_cmd(&store, &suite, &opts, &opts.positional[1..]),
            "room" => return handle_room_cmd(&store, &suite, &opts, &opts.positional[1..]),
            "scheduler" => return handle_scheduler_cmd(&store, &suite, &opts, &opts.positional[1..]),
            "run" => {
                return handle_run_subcommand(
                    &store,
                    &suite,
                    &db_path,
                    &workspace,
                    &home,
                    &opts,
                    &opts.positional[1..],
                );
            }
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
            "tui" => {
                return crate::tui::run_tui(
                    store,
                    suite,
                    &workspace,
                    TuiLaunchOptions {
                        db_path: db_path.clone(),
                        model: opts.model.clone(),
                        endpoint: opts.endpoint.clone(),
                        permission_profile: opts.permission_profile,
                        yes: opts.yes,
                        resume: opts.resume,
                        session_id: opts.session_id.clone(),
                    },
                )
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option '{other}'. Run 'nola --help' for usage."));
            }
            _ => {}
        }
    }

    let prompt_text = resolve_prompt(&opts)?;
    let Some(prompt) = prompt_text else {
        if io::stdin().is_terminal() && !opts.json_mode {
            return crate::tui::run_tui(
                store,
                suite,
                &workspace,
                TuiLaunchOptions {
                    db_path: db_path.clone(),
                    model: opts.model.clone(),
                    endpoint: opts.endpoint.clone(),
                    permission_profile: opts.permission_profile,
                    yes: opts.yes,
                    resume: opts.resume,
                    session_id: opts.session_id.clone(),
                },
            );
        }
        return Err("no prompt provided on stdin or via --prompt".into());
    };

    execute_agent_turn(
        &store,
        &suite,
        &db_path,
        &workspace,
        &home,
        &opts,
        &prompt,
        None,
    )
}

struct HostRunOverrides {
    session_id: Option<String>,
    resume: Option<bool>,
    model_id: Option<String>,
    endpoint: Option<String>,
    api_key: Option<String>,
    protocol: Option<String>,
    permission_profile: Option<PermissionProfile>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_output_tokens: Option<usize>,
    max_iterations: Option<usize>,
    context_window_tokens: Option<usize>,
}

fn handle_run_subcommand(
    store: &Store,
    suite: &Arc<ModulesSuite>,
    db_path: &Path,
    workspace: &Path,
    home: &Option<PathBuf>,
    opts: &CliOptions,
    rest: &[String],
) -> Result<u8, String> {
    if opts.host_request_stdin {
        let mut raw = String::new();
        io::stdin()
            .lock()
            .read_line(&mut raw)
            .map_err(|e| format!("failed to read host request from stdin: {e}"))?;
        let req: Value = serde_json::from_str(raw.trim())
            .map_err(|e| format!("invalid host request JSON on stdin: {e}"))?;

        if req
            .get("operation")
            .and_then(|v| v.as_str())
            .is_some_and(|op| op.eq_ignore_ascii_case("cancel"))
        {
            let sid = req
                .get("conversation_id")
                .or_else(|| req.get("session_id"))
                .or_else(|| req.get("conversationId"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .or(opts.session_id.as_deref())
                .unwrap_or("");
            if !sid.is_empty() {
                let _ = store.set_kv(
                    &format!("cancel_requested:{sid}"),
                    &json!({"requested_at_ms": nola_types::now_ms()}),
                );
            }
            println!(
                "{}",
                json!({"ok": true, "cancelled": true, "session_id": sid})
            );
            let _ = io::stdout().flush();
            return Ok(0);
        }

        let prompt = req
            .get("prompt")
            .or_else(|| req.get("message"))
            .or_else(|| req.get("user_message"))
            .or_else(|| req.get("input"))
            .or_else(|| req.get("text"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .or_else(|| opts.prompt.clone())
            .or_else(|| (!rest.is_empty()).then(|| rest.join(" ")))
            .ok_or("host request missing 'prompt' or 'message'")?;
        let session_id = req
            .get("conversation_id")
            .or_else(|| req.get("session_id"))
            .or_else(|| req.get("conversationId"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .or_else(|| opts.session_id.clone());
        let resume = req.get("resume").and_then(|v| v.as_bool()).or_else(|| {
            session_id
                .as_deref()
                .and_then(|id| store.get_session(id).ok().flatten())
                .map(|_| true)
        });
        let model_id = req
            .get("model_id")
            .or_else(|| req.get("model"))
            .or_else(|| req.get("modelId"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string);
        let endpoint = req
            .get("endpoint")
            .or_else(|| req.get("base_url"))
            .or_else(|| req.get("baseURL"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string);
        let api_key = req
            .get("api_key")
            .or_else(|| req.get("apiKey"))
            .or_else(|| req.get("credential"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let protocol = req
            .get("model_protocol")
            .or_else(|| req.get("modelProtocol"))
            .or_else(|| req.get("protocol"))
            .or_else(|| req.get("wire_protocol"))
            .or_else(|| req.get("wireProtocol"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let permission_profile = req
            .get("permission_profile")
            .or_else(|| req.get("permissionProfile"))
            .and_then(|v| v.as_str())
            .and_then(|s| {
                PermissionProfile::parse_strict(s)
                    .ok()
                    .or_else(|| PermissionProfile::parse(s))
            });
        let temperature = req.get("temperature").and_then(|v| v.as_f64());
        let top_p = req
            .get("top_p")
            .or_else(|| req.get("topP"))
            .and_then(|v| v.as_f64());
        let max_output_tokens = req
            .get("max_output_tokens")
            .or_else(|| req.get("maxOutputTokens"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let max_iterations = req
            .get("max_iterations")
            .or_else(|| req.get("maxIterations"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let context_window_tokens = req
            .get("context_window_tokens")
            .or_else(|| req.get("contextWindowTokens"))
            .or_else(|| req.get("context_window"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let overrides = HostRunOverrides {
            session_id,
            resume,
            model_id,
            endpoint,
            api_key,
            protocol,
            permission_profile,
            temperature,
            top_p,
            max_output_tokens,
            max_iterations,
            context_window_tokens,
        };
        return execute_agent_turn(
            store,
            suite,
            db_path,
            workspace,
            home,
            opts,
            &prompt,
            Some(overrides),
        );
    }

    let prompt = if let Some(p) = &opts.prompt {
        p.clone()
    } else if !rest.is_empty() {
        rest.join(" ")
    } else if !io::stdin().is_terminal() {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("failed to read stdin: {e}"))?;
        if buf.trim().is_empty() {
            return Err("no prompt provided for `nola run`".into());
        }
        buf
    } else {
        return Err("no prompt provided for `nola run`".into());
    };

    execute_agent_turn(store, suite, db_path, workspace, home, opts, &prompt, None)
}

#[allow(clippy::too_many_arguments)]
fn execute_agent_turn(
    store: &Store,
    suite: &Arc<ModulesSuite>,
    db_path: &Path,
    workspace: &Path,
    home: &Option<PathBuf>,
    opts: &CliOptions,
    prompt: &str,
    overrides: Option<HostRunOverrides>,
) -> Result<u8, String> {
    let cfg = load_active_config(store);
    let mut route = resolve_route_with_overrides(&cfg, opts, overrides.as_ref())?;

    if let Some(ref model_override) = opts.model {
        route.model = model_override.clone();
    }
    if let Some(ref endpoint_override) = opts.endpoint {
        route.endpoint = endpoint_override.clone();
    }
    if let Some(ref ov) = overrides {
        if let Some(ref m) = ov.model_id {
            route.model = m.clone();
        }
        if let Some(ref ep) = ov.endpoint {
            route.endpoint = ep.clone();
        }
        if let Some(ref key) = ov.api_key {
            route.api_key = key.clone();
        }
        if let Some(ref proto) = ov.protocol {
            if let Some(parsed) = ModelProtocol::parse(proto) {
                route.protocol = parsed;
            }
        }
        if let Some(temp) = ov.temperature {
            route.temperature = Some(temp);
        }
        if let Some(tp) = ov.top_p {
            route.top_p = Some(tp);
        }
        if let Some(max_out) = ov.max_output_tokens {
            route.max_output_tokens = Some(max_out);
        }
        if let Some(cw) = ov.context_window_tokens {
            route.context_window = cw;
        }
    }

    let profile = overrides
        .as_ref()
        .and_then(|o| o.permission_profile)
        .unwrap_or_else(|| resolve_profile(opts));
    let network_allowed = resolve_network_allowed(opts, &cfg);

    let caller: Arc<dyn ModelCaller> =
        Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
    let workspace_trusted = profile == PermissionProfile::AllowAll;
    let policy = EffectivePolicy::load(
        workspace,
        home.as_deref(),
        workspace_trusted,
        Some(profile),
        network_allowed,
    );

    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);

    let cancel = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&cancel));
    let hooks = HookRunner::load(workspace, home.as_deref(), workspace_trusted);

    let subagents = Arc::new(SubagentManager::new(store.clone(), Arc::clone(&caller)));
    subagents.configure_parent(route.clone(), policy.clone());
    subagents.configure_runtime(hooks.clone(), Some(Arc::clone(&cancel)));
    register_subagent_tools(&subagents, &mut registry);

    let planning = Arc::new(PlanningState::default());
    register_planning_tools(store.clone(), &planning, &mut registry);

    if let Ok(servers) = store.list_mcp_servers() {
        let vault = Vault::open(default_vault_path(db_path)).ok().map(Arc::new);
        let mcp = Arc::new(McpManager::new(vault));
        for srv in servers.into_iter().filter(|s| s.enabled) {
            if let Ok(tools) = mcp.discover_tools(&srv) {
                for t in tools {
                    registry.register(t);
                }
            }
        }
    }

    let mut ctx = ToolContext::new(workspace);
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
            let text = if Path::new(raw).is_file() {
                fs::read_to_string(raw).map_err(|e| e.to_string())?
            } else {
                raw.clone()
            };
            Some(serde_json::from_str(&text).map_err(|e| format!("invalid --json-schema: {e}"))?)
        }
        None => None,
    };

    let mut frozen_blocks = build_frozen_system_blocks(workspace, &cfg);
    if !opts.allowed_dirs.is_empty()
        || !opts.skill_dirs.is_empty()
        || opts.no_skills
        || !opts.github_skills.is_empty()
    {
        let mut lines = Vec::new();
        if !opts.allowed_dirs.is_empty() {
            let dirs: Vec<String> = opts
                .allowed_dirs
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            lines.push(format!("Allowed directories: {}", dirs.join(", ")));
        }
        if opts.no_skills {
            lines.push("Skills disabled by --no-skills.".to_string());
        } else {
            if !opts.skill_dirs.is_empty() {
                let dirs: Vec<String> = opts
                    .skill_dirs
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                lines.push(format!("Extra skill directories: {}", dirs.join(", ")));
            }
            if !opts.github_skills.is_empty() {
                lines.push(format!(
                    "Requested GitHub skills: {}",
                    opts.github_skills.join(", ")
                ));
            }
        }
        frozen_blocks.push(nola_types::FrozenSystemBlock {
            id: "host_resource_hints".into(),
            text: lines.join("\n"),
        });
    }

    let max_steps = overrides
        .as_ref()
        .and_then(|o| o.max_iterations)
        .or(opts.max_steps)
        .or(cfg.max_iterations)
        .unwrap_or(0);
    let is_eval = std::env::var("NOLA_BENCHMARK_TOKEN").is_ok();
    let json_mode = opts.json_mode || opts.host_request_stdin;
    let session_id = overrides
        .as_ref()
        .and_then(|o| o.session_id.clone())
        .or_else(|| opts.session_id.clone());
    let resume = overrides
        .as_ref()
        .and_then(|o| o.resume)
        .unwrap_or(opts.resume);

    let run_cfg = AgentRunConfig {
        session_id,
        resume,
        route,
        frozen_system_blocks: frozen_blocks,
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
        .run(&ctx, prompt, run_cfg, |ev| {
            if json_mode {
                if let Ok(line) = serde_json::to_string(&ev) {
                    println!("{line}");
                    let _ = io::stdout().flush();
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

fn resolve_route_with_overrides(
    cfg: &AppliedConfig,
    opts: &CliOptions,
    overrides: Option<&HostRunOverrides>,
) -> Result<ModelRouteConfig, String> {
    if let Ok(route) = resolve_model_route(cfg) {
        return Ok(route);
    }
    let endpoint = overrides
        .and_then(|o| o.endpoint.clone())
        .or_else(|| opts.endpoint.clone());
    let model = overrides
        .and_then(|o| o.model_id.clone())
        .or_else(|| opts.model.clone());
    if let (Some(ep), Some(m)) = (endpoint, model) {
        let protocol = overrides
            .and_then(|o| o.protocol.as_deref())
            .and_then(ModelProtocol::parse)
            .unwrap_or(ModelProtocol::OpenAiChat);
        let api_key = overrides
            .and_then(|o| o.api_key.clone())
            .or_else(|| std::env::var("NOLA_API_KEY").ok())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .unwrap_or_default();
        return Ok(ModelRouteConfig {
            connection_id: "host".into(),
            endpoint: ep,
            protocol,
            model: m,
            api_key,
            context_window: 128_000,
            max_output_tokens: None,
            temperature: Some(0.2),
            top_p: None,
            capabilities: ModelCapabilities::default(),
            reasoning_effort: None,
            prompt_cache_key: Some("nola-v2-host".into()),
        });
    }
    resolve_model_route(cfg)
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
        let active_subcmd = opts.positional.first().map(String::as_str);
        let in_non_agent_subcmd = matches!(
            active_subcmd,
            Some(
                "init"
                    | "config"
                    | "model"
                    | "mcp"
                    | "skills"
                    | "skill"
                    | "memory"
                    | "sessions"
                    | "session"
                    | "plugin"
                    | "plugins"
                    | "modules"
                    | "update"
                    | "web"
                    | "server"
            )
        );
        let prev_takes_subcmd_value = opts.positional.last().is_some_and(|p| {
            matches!(
                p.as_str(),
                "--arg"
                    | "--command"
                    | "--name"
                    | "--url"
                    | "--id"
                    | "--version"
                    | "--entry"
                    | "--capability"
                    | "--to"
                    | "--sha256"
                    | "--socket"
                    | "--parent-pid"
                    | "--host"
                    | "--port"
                    | "--web-root"
            )
        });
        if prev_takes_subcmd_value {
            opts.positional.push(a.to_string());
            i += 1;
            continue;
        }
        if in_non_agent_subcmd {
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
                _ => opts.positional.push(a.to_string()),
            }
            i += 1;
            continue;
        }

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
            "--model" | "-m" => {
                i += 1;
                opts.model = Some(args.get(i).ok_or("--model requires a model id")?.clone());
            }
            "--endpoint" => {
                i += 1;
                opts.endpoint = Some(args.get(i).ok_or("--endpoint requires a URL")?.clone());
            }
            "--host-request-stdin" => opts.host_request_stdin = true,
            "--allowed-dir" | "--allow-dir" => {
                i += 1;
                opts.allowed_dirs.push(PathBuf::from(
                    args.get(i).ok_or("--allowed-dir requires a path")?,
                ));
            }
            "--skill-dir" => {
                i += 1;
                opts.skill_dirs.push(PathBuf::from(
                    args.get(i).ok_or("--skill-dir requires a path")?,
                ));
            }
            "--no-skills" => opts.no_skills = true,
            "--github-skill" => {
                i += 1;
                opts.github_skills
                    .push(args.get(i).ok_or("--github-skill requires an id")?.clone());
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

fn default_config_path() -> PathBuf {
    let home_dir = default_home_dir().join(".nola");
    let jsonc = home_dir.join("config.jsonc");
    if jsonc.exists() {
        return jsonc;
    }
    let json = home_dir.join("config.json");
    if json.exists() {
        return json;
    }
    jsonc
}

fn validate_config_file(path: &Path) -> Result<AppliedConfig, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("failed to stat config '{}': {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "config file '{}' must not be a symlink",
            path.display()
        ));
    }
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("failed to read config '{}': {e}", path.display()))?;
    let clean = strip_jsonc(&raw);
    let cfg: AppliedConfig = serde_json::from_str(&clean)
        .map_err(|e| format!("invalid config JSONC '{}': {e}", path.display()))?;

    let mut conn_ids = BTreeSet::new();
    for conn in &cfg.llm.connections {
        if conn.id.trim().is_empty() {
            return Err("connection 'id' must not be empty".into());
        }
        if !conn_ids.insert(conn.id.clone()) {
            return Err(format!("duplicate connection id '{}'", conn.id));
        }
        let mut model_ids = BTreeSet::new();
        for m in &conn.models {
            if m.id.trim().is_empty() {
                return Err(format!("empty model id in connection '{}'", conn.id));
            }
            if !model_ids.insert(m.id.clone()) {
                return Err(format!(
                    "duplicate model id '{}' in connection '{}'",
                    m.id, conn.id
                ));
            }
        }
    }

    let Some(def) = &cfg.llm.default_model else {
        return Err("missing required 'llm.default_model' selection".into());
    };
    let Some(conn) = cfg.llm.connections.iter().find(|c| c.id == def.connection) else {
        return Err(format!(
            "default_model connection '{}' not found in llm.connections",
            def.connection
        ));
    };
    if def.model.trim().is_empty() {
        return Err("default_model.model must not be empty".into());
    }
    if !conn.models.is_empty() && !conn.models.iter().any(|m| m.id == def.model) {
        return Err(format!(
            "default_model '{}' not found in connection '{}'",
            def.model, conn.id
        ));
    }

    Ok(cfg)
}

fn reject_if_symlink(path: &Path) -> Result<(), String> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(format!("refusing to operate on symlink '{}'", path.display()));
        }
    }
    Ok(())
}

fn resolve_default_config_path(workspace: &Path) -> PathBuf {
    discover_config_path(workspace).unwrap_or_else(default_config_path)
}

fn redacted_connection_json(conn: &ConnectionConfig) -> Value {
    json!({
        "id": conn.id,
        "endpoint": conn.endpoint,
        "protocol": conn.protocol,
        "provider": conn.provider,
        "api_key_env": conn.api_key_env,
        "has_api_key": connection_has_api_key(conn),
        "models": conn.models
    })
}

fn redacted_llm_json(cfg: &AppliedConfig) -> Value {
    let connections: Vec<Value> = cfg
        .llm
        .connections
        .iter()
        .map(redacted_connection_json)
        .collect();
    json!({
        "default_model": cfg.llm.default_model,
        "connections": connections
    })
}

fn handle_init_cmd(workspace: &Path, args: &[String]) -> Result<u8, String> {
    let force = args.iter().any(|a| a == "--force");
    let config_dir = workspace.join(".nola");
    let config_path = config_dir.join("config.jsonc");
    reject_if_symlink(&config_dir)?;
    reject_if_symlink(&config_path)?;
    if config_path.exists() && !force {
        println!(
            "{}",
            json!({
                "ok": true,
                "created": false,
                "path": config_path.display().to_string(),
                "message": "configuration already exists (pass --force to overwrite)"
            })
        );
        return Ok(0);
    }
    fs::create_dir_all(&config_dir)
        .map_err(|e| format!("failed to create '{}': {e}", config_dir.display()))?;
    reject_if_symlink(&config_path)?;
    fs::write(&config_path, EXAMPLE_CONFIG_JSONC)
        .map_err(|e| format!("failed to write '{}': {e}", config_path.display()))?;
    println!(
        "{}",
        json!({
            "ok": true,
            "created": true,
            "path": config_path.display().to_string()
        })
    );
    Ok(0)
}

fn handle_config_cmd(store: &Store, workspace: &Path, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("example") => {
            print!("{EXAMPLE_CONFIG_JSONC}");
            Ok(0)
        }
        Some("check") | None => {
            let path = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| resolve_default_config_path(workspace));
            let cfg = validate_config_file(&path)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "valid": true,
                    "path": path.display().to_string(),
                    "default_model": cfg.llm.default_model,
                    "connections": cfg.llm.connections.len()
                }))
                .unwrap()
            );
            Ok(0)
        }
        Some("apply") => {
            let mut dry_run = false;
            let mut explicit_path: Option<PathBuf> = None;
            for a in &args[1..] {
                if a == "--dry-run" {
                    dry_run = true;
                } else if !a.starts_with('-') && explicit_path.is_none() {
                    explicit_path = Some(PathBuf::from(a));
                }
            }
            let path = explicit_path.unwrap_or_else(|| resolve_default_config_path(workspace));
            let cfg = validate_config_file(&path)?;
            if dry_run {
                let route = resolve_model_route(&cfg)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": true,
                        "dry_run": true,
                        "applied": false,
                        "path": path.display().to_string(),
                        "connection": route.connection_id,
                        "model": route.model,
                        "protocol": route.protocol.as_str(),
                        "endpoint": route.endpoint
                    }))
                    .unwrap()
                );
                return Ok(0);
            }
            let applied = apply_config_file(store, &path)?;
            let route = resolve_model_route(&applied)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "applied": true,
                    "path": path.display().to_string(),
                    "connection": route.connection_id,
                    "model": route.model,
                    "protocol": route.protocol.as_str(),
                    "endpoint": route.endpoint
                }))
                .unwrap()
            );
            Ok(0)
        }
        Some("edit") => {
            let path = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| resolve_default_config_path(workspace));
            reject_if_symlink(&path)?;
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    reject_if_symlink(parent)?;
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                reject_if_symlink(&path)?;
                fs::write(&path, EXAMPLE_CONFIG_JSONC).map_err(|e| e.to_string())?;
            }
            let editor_cmd = std::env::var("VISUAL")
                .or_else(|_| std::env::var("EDITOR"))
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    ["nano", "vi"]
                        .iter()
                        .find(|bin| find_binary_on_path(bin).is_some())
                        .map(|s| s.to_string())
                })
                .ok_or_else(|| {
                    format!(
                        "no editor found (set $VISUAL or $EDITOR, or install nano/vi); config path: {}",
                        path.display()
                    )
                })?;
            let mut parts = editor_cmd.split_whitespace();
            let program = parts
                .next()
                .ok_or_else(|| "empty $VISUAL/$EDITOR command".to_string())?;
            let extra_args: Vec<&str> = parts.collect();
            let status = Command::new(program)
                .args(&extra_args)
                .arg(&path)
                .status()
                .map_err(|e| format!("failed to launch editor '{program}': {e}"))?;
            if !status.success() {
                return Err(format!("editor '{program}' exited with status {status}"));
            }
            Ok(0)
        }
        Some(other) => Err(format!(
            "unknown config subcommand '{other}' (expected check|apply|example|edit)"
        )),
    }
}

fn handle_model_cmd(store: &Store, workspace: &Path, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("apply-config") => {
            let path = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| resolve_default_config_path(workspace));
            let cfg = apply_config_file(store, &path)?;
            let route = resolve_model_route(&cfg)?;
            println!(
                "{}",
                json!({
                    "applied": true,
                    "path": path.display().to_string(),
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
                serde_json::to_string_pretty(&redacted_llm_json(&cfg))
                    .unwrap_or_else(|_| "{}".into())
            );
            Ok(0)
        }
        Some("connections") => {
            let cfg = load_active_config(store);
            let conns: Vec<Value> = cfg
                .llm
                .connections
                .iter()
                .map(redacted_connection_json)
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"connections": conns}))
                    .unwrap_or_else(|_| "{}".into())
            );
            Ok(0)
        }
        Some("probe") => {
            let cfg = load_active_config(store);
            let target_id = args.get(1).map(String::as_str);
            let conn = match target_id {
                Some(cid) => cfg
                    .llm
                    .connections
                    .iter()
                    .find(|c| c.id == cid)
                    .ok_or_else(|| format!("connection '{cid}' not found"))?,
                None => {
                    let def_id = cfg
                        .llm
                        .default_model
                        .as_ref()
                        .map(|d| d.connection.as_str());
                    def_id
                        .and_then(|cid| cfg.llm.connections.iter().find(|c| c.id == cid))
                        .or_else(|| cfg.llm.connections.first())
                        .ok_or("no model connections configured")?
                }
            };
            let mut probe_cfg = cfg.clone();
            probe_cfg.llm.default_model = Some(DefaultModelSelection {
                connection: conn.id.clone(),
                model: conn
                    .models
                    .first()
                    .map(|m| m.id.clone())
                    .unwrap_or_else(|| "probe".into()),
            });
            let resolved = resolve_model_route(&probe_cfg);
            let (protocol, api_key) = match resolved {
                Ok(route) => (route.protocol, route.api_key),
                Err(_) => {
                    let proto_raw = conn
                        .protocol
                        .as_deref()
                        .or(conn.provider.as_deref())
                        .unwrap_or("chat_completions");
                    (
                        ModelProtocol::parse(proto_raw).unwrap_or(ModelProtocol::OpenAiChat),
                        String::new(),
                    )
                }
            };

            let probe_url = format!("{}/models", conn.endpoint.trim_end_matches('/'));
            let start = Instant::now();
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())?;
            let mut req = client.get(&probe_url);
            if protocol == ModelProtocol::AnthropicMessages {
                req = req.header("anthropic-version", "2023-06-01");
                if !api_key.trim().is_empty() {
                    req = req.header("x-api-key", api_key);
                }
            } else if !api_key.trim().is_empty() {
                req = req.bearer_auth(api_key);
            }
            match req.send() {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let ok = resp.status().is_success();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "ok": ok,
                            "connection_id": conn.id,
                            "protocol": protocol.as_str(),
                            "endpoint": conn.endpoint,
                            "probe_url": probe_url,
                            "status_code": status,
                            "latency_ms": start.elapsed().as_millis() as u64
                        }))
                        .unwrap()
                    );
                    Ok(if ok { 0 } else { 1 })
                }
                Err(e) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "ok": false,
                            "connection_id": conn.id,
                            "protocol": protocol.as_str(),
                            "endpoint": conn.endpoint,
                            "probe_url": probe_url,
                            "latency_ms": start.elapsed().as_millis() as u64,
                            "error": e.to_string()
                        }))
                        .unwrap()
                    );
                    Ok(1)
                }
            }
        }
        Some(other) => Err(format!(
            "unknown model subcommand '{other}' (expected list|connections|probe|apply-config)"
        )),
    }
}

fn handle_mcp_cmd(store: &Store, db_path: &Path, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let servers = store.list_mcp_servers().map_err(|e| e.to_string())?;
            let redacted: Vec<Value> = servers
                .into_iter()
                .map(|s| {
                    let mut val = serde_json::to_value(&s).unwrap_or_else(|_| json!({}));
                    if let Some(obj) = val.as_object_mut() {
                        let red_env: serde_json::Map<String, Value> = s
                            .env
                            .keys()
                            .map(|k| (k.clone(), json!("<redacted>")))
                            .collect();
                        let red_headers: serde_json::Map<String, Value> = s
                            .headers
                            .keys()
                            .map(|k| (k.clone(), json!("<redacted>")))
                            .collect();
                        obj.insert("env".into(), Value::Object(red_env));
                        obj.insert("headers".into(), Value::Object(red_headers));
                    }
                    val
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&redacted).unwrap());
            Ok(0)
        }
        Some("add") => {
            let id = args
                .get(1)
                .ok_or("usage: nola mcp add <id> --transport <stdio|http|sse> [--command <cmd> | --url <url>]")?;
            let mut transport = McpTransport::Stdio;
            let mut name = id.clone();
            let mut command: Option<String> = None;
            let mut cmd_args: Vec<String> = Vec::new();
            let mut url: Option<String> = None;
            let mut enabled = true;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--transport" => {
                        i += 1;
                        let t = args.get(i).ok_or("--transport requires stdio|http|sse")?;
                        transport = match t.trim().to_ascii_lowercase().as_str() {
                            "stdio" => McpTransport::Stdio,
                            "http" | "streamable_http" | "streamable-http" => {
                                McpTransport::StreamableHttp
                            }
                            "sse" | "server_sent_events" | "server-sent-events" => {
                                McpTransport::ServerSentEvents
                            }
                            other => {
                                return Err(format!(
                                    "invalid --transport '{other}' (expected stdio|http|sse)"
                                ));
                            }
                        };
                    }
                    "--name" => {
                        i += 1;
                        name = args.get(i).ok_or("--name requires a value")?.clone();
                    }
                    "--command" => {
                        i += 1;
                        command = Some(args.get(i).ok_or("--command requires a value")?.clone());
                    }
                    "--arg" => {
                        i += 1;
                        cmd_args.push(args.get(i).ok_or("--arg requires a value")?.clone());
                    }
                    "--url" => {
                        i += 1;
                        url = Some(args.get(i).ok_or("--url requires a URL")?.clone());
                    }
                    "--disabled" => enabled = false,
                    other => return Err(format!("unknown mcp add option '{other}'")),
                }
                i += 1;
            }
            if transport == McpTransport::Stdio && command.is_none() {
                return Err("stdio MCP server requires --command <cmd>".into());
            }
            if transport != McpTransport::Stdio && url.is_none() {
                return Err("http/sse MCP server requires --url <url>".into());
            }
            let cfg = McpServerConfig {
                id: id.clone(),
                name,
                transport,
                enabled,
                command,
                args: cmd_args,
                env: BTreeMap::new(),
                url,
                headers: BTreeMap::new(),
                oauth_client_id: None,
                oauth_scopes: Vec::new(),
            };
            store.upsert_mcp_server(&cfg).map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"ok": true, "server": cfg})).unwrap()
            );
            Ok(0)
        }
        Some("remove") | Some("delete") => {
            let id = args.get(1).ok_or("usage: nola mcp remove <id>")?;
            let deleted = store.delete_mcp_server(id).map_err(|e| e.to_string())?;
            println!("{}", json!({"ok": true, "deleted": deleted, "id": id}));
            Ok(0)
        }
        Some("auth") => {
            let server_id = args.get(1).ok_or("usage: nola mcp auth <server_id>")?;
            let vault =
                Arc::new(Vault::open(default_vault_path(db_path)).map_err(|e| e.to_string())?);
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
        Some(other) => Err(format!(
            "unknown mcp subcommand '{other}' (expected list|add|remove|auth)"
        )),
    }
}

fn handle_skills_cmd(store: &Store, suite: &ModulesSuite, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let skills = discover_skills_detailed(
                Some(store),
                &suite.workspace_root,
                suite.user_home.as_deref(),
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"skills": skills})).unwrap()
            );
            Ok(0)
        }
        Some("find") => {
            let query = args
                .get(1)
                .ok_or("usage: nola skills find <query>")?
                .to_ascii_lowercase();
            let skills: Vec<Value> = discover_skills_detailed(
                Some(store),
                &suite.workspace_root,
                suite.user_home.as_deref(),
            )
            .into_iter()
            .filter(|s| {
                let hay = format!(
                    "{} {} {}",
                    s["name"].as_str().unwrap_or(""),
                    s["description"].as_str().unwrap_or(""),
                    s["body"].as_str().unwrap_or("")
                )
                .to_ascii_lowercase();
                hay.contains(&query)
            })
            .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"query": query, "skills": skills})).unwrap()
            );
            Ok(0)
        }
        Some("install") => {
            let spec = args
                .get(1)
                .ok_or("usage: nola skills install <owner/repo | https://github.com/...>")?;
            let skills_dir = suite.workspace_root.join(".nola/skills");
            let pkg = prepare_github_skill_package(spec, &skills_dir)?;
            let raw_url = pkg["raw_url"].as_str().unwrap_or("");
            let dest_dir = PathBuf::from(pkg["base_directory"].as_str().unwrap_or(""));
            let dest_file = PathBuf::from(pkg["dest_file"].as_str().unwrap_or(""));

            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Nola/2.0")
                .build()
                .map_err(|e| e.to_string())?;
            let resp = client
                .get(raw_url)
                .send()
                .map_err(|e| format!("failed downloading skill from {raw_url}: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!(
                    "failed downloading skill from {raw_url}: HTTP {}",
                    resp.status()
                ));
            }
            let mut buf = Vec::new();
            resp.take(MAX_SKILL_INSTALL_BYTES + 1)
                .read_to_end(&mut buf)
                .map_err(|e| format!("failed reading skill payload: {e}"))?;
            if buf.len() as u64 > MAX_SKILL_INSTALL_BYTES {
                return Err(format!(
                    "skill payload exceeds {MAX_SKILL_INSTALL_BYTES} byte limit"
                ));
            }
            reject_if_symlink(&skills_dir)?;
            reject_if_symlink(&dest_dir)?;
            reject_if_symlink(&dest_file)?;
            fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
            reject_if_symlink(&dest_file)?;
            fs::write(&dest_file, &buf).map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "installed": true,
                    "package": pkg
                }))
                .unwrap()
            );
            Ok(0)
        }
        Some("enable") => {
            let name = args.get(1).ok_or("usage: nola skills enable <name>")?;
            set_skill_enabled(store, name, true).map_err(|e| e.to_string())?;
            println!("{}", json!({"ok": true, "name": name, "enabled": true}));
            Ok(0)
        }
        Some("disable") => {
            let name = args.get(1).ok_or("usage: nola skills disable <name>")?;
            set_skill_enabled(store, name, false).map_err(|e| e.to_string())?;
            println!("{}", json!({"ok": true, "name": name, "enabled": false}));
            Ok(0)
        }
        Some(other) => Err(format!(
            "unknown skills subcommand '{other}' (expected list|find|install|enable|disable)"
        )),
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
                .unwrap_or_else(new_id);
            let existing = if is_revise {
                store
                    .search_memory("", None, 500)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|m| m.id == id)
            } else {
                None
            };
            let existing_parts = existing.as_ref().map(|m| {
                m.namespace
                    .split_once(':')
                    .map(|(s, k)| (s.to_string(), k.to_string()))
                    .unwrap_or_else(|| ("global".to_string(), m.namespace.clone()))
            });
            let scope = draft
                .get("scope")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| existing_parts.as_ref().map(|(s, _)| s.clone()))
                .unwrap_or_else(|| "global".to_string());
            let kind = draft
                .get("kind")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| existing_parts.as_ref().map(|(_, k)| k.clone()))
                .unwrap_or_else(|| "other".to_string());
            let content = draft
                .get("content")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| existing.as_ref().map(|m| m.content.clone()))
                .unwrap_or_default();
            let title = draft
                .get("summary")
                .or_else(|| draft.get("title"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| existing.as_ref().map(|m| m.title.clone()))
                .unwrap_or_else(|| kind.clone());
            let tags: Vec<String> = draft
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                })
                .or_else(|| existing.as_ref().map(|m| m.tags.clone()))
                .unwrap_or_default();
            let created_at_ms = existing.as_ref().map(|m| m.created_at_ms).unwrap_or(ts);
            let item = nola_types::MemoryItem {
                id,
                namespace: format!("{scope}:{kind}"),
                title,
                content,
                tags,
                created_at_ms,
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

fn handle_plugin_cmd(store: &Store, suite: &ModulesSuite, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let catalog = list_harness_plugins(Some(store), &suite.workspace_root);
            println!("{}", serde_json::to_string_pretty(&catalog).unwrap());
            Ok(0)
        }
        Some("desktop-pages") => {
            let pages = list_desktop_plugin_pages(&suite.workspace_root);
            if let Some(page_id) = args.get(1) {
                let page = pages
                    .into_iter()
                    .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(page_id.as_str()))
                    .ok_or_else(|| format!("desktop plugin page '{page_id}' not found"))?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "id": page.get("id").cloned().unwrap_or(Value::Null),
                        "title": page.get("title").cloned().unwrap_or(Value::Null),
                        "html": page.get("html").cloned().unwrap_or_else(|| json!(""))
                    }))
                    .unwrap()
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&pages).unwrap());
            }
            Ok(0)
        }
        Some("desktop-library") => {
            let catalog = list_harness_plugins(Some(store), &suite.workspace_root);
            let pages = list_desktop_plugin_pages(&suite.workspace_root);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "plugins": catalog.get("plugins").cloned().unwrap_or_else(|| json!([])),
                    "profiles": catalog.get("profiles").cloned().unwrap_or_else(|| json!([])),
                    "pages": pages
                }))
                .unwrap()
            );
            Ok(0)
        }
        Some("desktop-manage") => {
            let sub_args = &args[1..];
            match sub_args.first().map(String::as_str) {
                Some("enable") | Some("disable") | Some("remove") | Some("install") => {
                    handle_plugin_cmd(store, suite, sub_args)
                }
                Some("list") | None => {
                    let installed: Vec<PluginRecord> =
                        suite.list_records(KIND_PLUGIN).unwrap_or_default();
                    let catalog = list_harness_plugins(Some(store), &suite.workspace_root);
                    let pages = list_desktop_plugin_pages(&suite.workspace_root);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "installed": installed,
                            "plugins": catalog.get("plugins").cloned().unwrap_or_else(|| json!([])),
                            "pages": pages
                        }))
                        .unwrap()
                    );
                    Ok(0)
                }
                Some(other) => Err(format!(
                    "unknown desktop-manage action '{other}' (expected list|install|enable|disable|remove)"
                )),
            }
        }
        Some("pi") | Some("dsh") => {
            let runtime = args[0].as_str();
            let catalog = list_harness_plugins(Some(store), &suite.workspace_root);
            let profiles: Vec<Value> = catalog
                .get("profiles")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter(|p| p.get("runtime").and_then(|r| r.as_str()) == Some(runtime))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"runtime": runtime, "profiles": profiles}))
                    .unwrap()
            );
            Ok(0)
        }
        Some("install") => {
            let name_or_path = args
                .get(1)
                .ok_or("usage: nola plugin install <name_or_entry_path> [--id <id>] [--version <ver>] [--entry <path>]")?;
            let mut id = name_or_path
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                .collect::<String>();
            if id.trim_matches('-').is_empty() {
                id = new_id();
            }
            let mut version = "0.1.0".to_string();
            let mut entry_path = name_or_path.clone();
            let mut capabilities: Vec<String> = Vec::new();
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--id" => {
                        i += 1;
                        id = args.get(i).ok_or("--id requires a value")?.clone();
                    }
                    "--version" => {
                        i += 1;
                        version = args.get(i).ok_or("--version requires a value")?.clone();
                    }
                    "--entry" => {
                        i += 1;
                        entry_path = args.get(i).ok_or("--entry requires a path")?.clone();
                    }
                    "--capability" => {
                        i += 1;
                        capabilities.push(args.get(i).ok_or("--capability requires a value")?.clone());
                    }
                    other => return Err(format!("unknown plugin install option '{other}'")),
                }
                i += 1;
            }
            let rec = PluginRecord {
                id: id.clone(),
                name: name_or_path.clone(),
                version,
                entry_path,
                enabled: true,
                capabilities,
            };
            suite
                .upsert_record(KIND_PLUGIN, &rec.id, &rec)
                .map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"ok": true, "plugin": rec})).unwrap()
            );
            Ok(0)
        }
        Some("enable") | Some("disable") => {
            let enable = args[0] == "enable";
            let id = args.get(1).ok_or("usage: nola plugin enable|disable <id>")?;
            let mut rec = suite
                .get_record::<PluginRecord>(KIND_PLUGIN, id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("plugin '{id}' not found"))?;
            rec.enabled = enable;
            suite
                .upsert_record(KIND_PLUGIN, &rec.id, &rec)
                .map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"ok": true, "plugin": rec})).unwrap()
            );
            Ok(0)
        }
        Some("remove") => {
            let id = args.get(1).ok_or("usage: nola plugin remove <id>")?;
            let deleted = suite
                .delete_record(KIND_PLUGIN, id)
                .map_err(|e| e.to_string())?;
            println!("{}", json!({"ok": true, "deleted": deleted, "id": id}));
            Ok(0)
        }
        Some(other) => Err(format!(
            "unknown plugin subcommand '{other}' (expected list|install|enable|disable|remove|desktop-pages|desktop-library|desktop-manage|pi|dsh)"
        )),
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
            println!("{}", json!({"ok": true, "deleted": true, "id": id}));
            Ok(0)
        }
        Some("backup") => {
            let dest = args.get(1).ok_or("usage: nola sessions backup <dest.db>")?;
            store
                .backup_to(Path::new(dest))
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
            let mut route = resolve_model_route(&cfg)?;
            if let Some(ref m) = opts.model {
                route.model = m.clone();
            }
            if let Some(ref ep) = opts.endpoint {
                route.endpoint = ep.clone();
            }
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
                .ok_or("usage: nola room send <room_id> <message> [--to <participant_id>]")?;
            let mut target_recipient: Option<String> = None;
            let mut msg_parts: Vec<&str> = Vec::new();
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--to" {
                    i += 1;
                    let rec = args
                        .get(i)
                        .ok_or("--to requires a participant id or name")?;
                    target_recipient = Some(rec.clone());
                } else {
                    msg_parts.push(args[i].as_str());
                }
                i += 1;
            }
            let msg = msg_parts.join(" ");
            if msg.trim().is_empty() {
                return Err("room message cannot be empty".into());
            }
            let cfg = load_active_config(store);
            let mut route = resolve_model_route(&cfg)?;
            if let Some(ref m) = opts.model {
                route.model = m.clone();
            }
            if let Some(ref ep) = opts.endpoint {
                route.endpoint = ep.clone();
            }
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
            let replies = RoomRuntime::send_message_to(
                suite,
                caller,
                route,
                policy,
                room_id,
                &msg,
                target_recipient.as_deref(),
            )?;
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
            let mut route = resolve_model_route(&cfg)?;
            if let Some(ref m) = opts.model {
                route.model = m.clone();
            }
            if let Some(ref ep) = opts.endpoint {
                route.endpoint = ep.clone();
            }
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
            let mut any_failed = false;
            let items: Vec<Value> = results
                .into_iter()
                .map(|(id, r)| match r {
                    Ok(out) => json!({"id": id, "ok": true, "output": out, "error": Value::Null}),
                    Err(err) => {
                        any_failed = true;
                        json!({"id": id, "ok": false, "output": Value::Null, "error": err})
                    }
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&items).unwrap());
            Ok(if any_failed { 1 } else { 0 })
        }
        Some("list") | None => handle_modules_cmd(suite, &["automation".to_string()]),
        Some(other) => Err(format!("unknown scheduler subcommand '{other}'")),
    }
}

fn handle_update_cmd(args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("check") | None => {
            let url = args.get(1).map(String::as_str).unwrap_or("");
            let res = check_for_update(url, env!("CARGO_PKG_VERSION"))?;
            println!("{}", serde_json::to_string_pretty(&res).unwrap());
            Ok(0)
        }
        Some("download") => {
            let url = args
                .get(1)
                .ok_or("usage: nola update download <https://...> <dest_dir> [--sha256 <hex>]")?;
            let dest_dir = args
                .get(2)
                .ok_or("usage: nola update download <https://...> <dest_dir> [--sha256 <hex>]")?;
            let mut sha256: Option<String> = None;
            let mut i = 3;
            while i < args.len() {
                if args[i] == "--sha256" {
                    i += 1;
                    sha256 = Some(args.get(i).ok_or("--sha256 requires a 64-char hex digest")?.clone());
                } else {
                    return Err(format!("unknown update download option '{}'", args[i]));
                }
                i += 1;
            }
            let res = download_update_package(url, sha256.as_deref(), Path::new(dest_dir))?;
            println!("{}", serde_json::to_string_pretty(&res).unwrap());
            Ok(0)
        }
        Some(other) => Err(format!(
            "unknown update subcommand '{other}' (expected check|download)"
        )),
    }
}

fn print_help() {
    println!(
        "Nola v2 (Lean 8-Crate Full-Capability Architecture)\n\n\
         Usage:\n  \
           nola [options] [prompt]\n  \
           nola run [--json] [--host-request-stdin] [--session <id>] [--project <dir>] [-m <model>] [--endpoint <url>] [--allow-dir <dir>] [--no-skills] [--github-skill <id>]\n  \
           nola init [--force]\n  \
           nola config [check [path] | apply [path] [--dry-run] | example | edit [path]]\n  \
           nola model [list | connections | probe [connection_id] | apply-config [path]]\n  \
           nola mcp [list | add <id> --transport <stdio|http|sse> ... | remove <id> | auth <id>]\n  \
           nola skills [list | find <query> | install <owner/repo|url> | enable <name> | disable <name>]\n  \
           nola memory [list | create | revise <id> | delete <id>]\n  \
           nola plugin [list | install <spec> | enable <id> | disable <id> | remove <id> | desktop-pages [id] | desktop-library | desktop-manage | pi | dsh]\n  \
           nola sessions [list | delete <id> | backup <dest>]\n  \
           nola goal [list | run <goal_id>]\n  \
           nola room [list | send <room_id> <message> [--to <participant_id>]]\n  \
           nola scheduler [list | tick]\n  \
           nola update [check [manifest_url] | download <url> <dest_dir> [--sha256 <hex>]]\n  \
           nola modules <kind|skills>\n  \
           nola version [--json]\n  \
           nola web [--host 127.0.0.1] [--port 5001] [--web-root <dir>] [--json]\n  \
           nola server [--socket <path>] [--parent-pid <pid>]\n  \
           nola tui [--db <path>] [-m <model>] [--permission-profile <profile>] [-y] [--resume]"
    );
}
