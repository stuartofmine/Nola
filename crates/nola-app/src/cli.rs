// SPDX-License-Identifier: Apache-2.0
//! Unified CLI command router supporting headless JSON/text runs, macOS `nola run`,
//! Harbor evals, config validation/apply, redacted model management & probing,
//! MCP CRUD/OAuth, skills, harness plugins, sessions, GoalWorker, RoomRuntime,
//! AutomationScheduler, Software Updater, TUI, WebHost, and UDS daemon.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use nola_engine::{
    register_planning_tools, register_subagent_tools, AgentEngine, AgentEvent, AgentRunConfig,
    ApprovalCallback, EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller, ModelCaller,
    PlanningState, SubagentManager, UserInputCallback,
};
use nola_mcp::McpManager;
use nola_modules::a2ui::register_scoped_tools as register_a2ui_tools;
use nola_modules::code_memory::codebase_architecture;
use nola_modules::local_models_plugins::{
    find_binary_on_path, install_plugin_package, list_desktop_plugin_pages, list_harness_plugins,
    probe_engine_status, probe_hardware, recommend_local_models, reconcile_enabled_plugins,
    run_engine_setup, search_hub_models, ModelDownloadManager, ModelDownloadPlan,
    KIND_LOCAL_MODEL, KIND_PLUGIN,
};
use nola_modules::skills::{
    discover_skills_detailed, prepare_github_skill_package, set_skill_enabled,
};
use nola_modules::updater::verify_signed_feed_bytes;
use nola_modules::{
    check_for_update, download_update_package, list_data_source_usage, AutomationScheduler,
    ExtendedGoalRecord, GoalWorker, ModulesSuite, RoomRuntime,
};
use nola_store::{Store, Vault, PROTOCOL_VERSION};
use nola_tools::{OutputStore, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    new_id, now_ms, sha256_hex, GoalStatus, HostCapabilities, LocalModelRecord, McpServerConfig,
    McpTransport, ModelProtocol, ModelRouteConfig, PermissionProfile, PluginRecord,
};
use serde_json::{json, Value};

use crate::config::{
    apply_config_file, bind_store_vault_path, build_frozen_system_blocks, default_db_path,
    default_home_dir, default_vault_path, discover_config_path, load_active_config,
    parse_applied_config_json, read_config_file_safe, redacted_applied_llm_config,
    resolve_fallback_routes, resolve_model_route, save_applied_config, strip_jsonc, AppliedConfig,
    ConnectionConfig, DefaultModelSelection, ModelEntry,
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
    profile: Option<String>,
    network_policy: Option<String>,
    force_tool_set: ForceToolSet,
    session_id: Option<String>,
    resume: bool,
    continue_last: bool,
    max_steps: Option<usize>,
    prompt: Option<String>,
    model: Option<String>,
    endpoint: Option<String>,
    reasoning_effort: Option<String>,
    no_tools: bool,
    output_file: Option<PathBuf>,
    debug: bool,
    lang: Option<String>,
    host_request_stdin: bool,
    allowed_dirs: Vec<PathBuf>,
    skill_dirs: Vec<PathBuf>,
    no_skills: bool,
    github_skills: Vec<String>,
    double_dash_pos: Option<usize>,
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
            | "model-import-bridge"
            | "model-siwc-bridge"
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
            | "wiki"
            | "run"
            | "web"
            | "server"
            | "tui"
    )
}

fn subcommand_wants_help(cmd: &str, rest: &[String], double_dash_pos: Option<usize>) -> bool {
    let limit = double_dash_pos
        .map(|pos| pos.saturating_sub(1))
        .unwrap_or(rest.len());
    let scan = &rest[..limit.min(rest.len())];
    if matches!(scan.first().map(String::as_str), Some("--help" | "-h")) {
        return true;
    }
    if cmd == "run" {
        return false;
    }
    let mut skip_next = false;
    for a in scan {
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
                | "--assets"
                | "--transport"
                | "--env"
                | "--source"
                | "--feed"
                | "--target"
                | "--objective"
                | "--criteria"
                | "--note"
                | "--endpoint"
                | "--model"
                | "--model-name"
                | "--protocol"
                | "--provider"
                | "--api-key"
                | "--api-key-env"
                | "--credential-ref"
                | "--path"
                | "--operation"
                | "--op"
                | "--plan-id"
                | "--timeout"
                | "--output"
                | "-o"
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

fn list_execution_backends_json() -> Value {
    let ver = env!("CARGO_PKG_VERSION");
    let mut backends = vec![json!({
        "id": "nola",
        "display_name": "Nola",
        "availability": "available",
        "available": true,
        "version": ver,
        "diagnostic": Value::Null,
        "models": [],
        "default_model_id": Value::Null,
        "capabilities": {
            "supports_permission_bridge": true
        }
    })];
    for (id, display_name, bin) in [
        ("codex", "Codex", "codex"),
        ("claude", "Claude Code", "claude"),
        ("opencode", "OpenCode", "opencode"),
    ] {
        let found = find_binary_on_path(bin);
        let available = found.is_some();
        let diagnostic = if available {
            Value::Null
        } else {
            json!(format!("{bin} binary not found on PATH"))
        };
        backends.push(json!({
            "id": id,
            "display_name": display_name,
            "availability": if available { "available" } else { "unavailable" },
            "available": available,
            "version": Value::Null,
            "diagnostic": diagnostic,
            "models": [],
            "default_model_id": Value::Null,
            "capabilities": {
                "supports_permission_bridge": false
            }
        }));
    }
    Value::Array(backends)
}

pub fn run_with_args(args: &[String]) -> Result<u8, String> {
    let opts = parse_args(args)?;

    // Dispatch metadata and DB-free commands BEFORE opening SQLite (avoid side-effect DB creation).
    if let Some(cmd) = opts.positional.first().map(String::as_str) {
        let rest = &opts.positional[1..];
        if opts.double_dash_pos != Some(0)
            && is_known_subcommand(cmd)
            && subcommand_wants_help(cmd, rest, opts.double_dash_pos)
        {
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
                println!("{}", list_execution_backends_json());
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
            "model-import-bridge" => {
                return handle_model_import_bridge_cmd(&store, &opts.positional[1..])
            }
            "model-siwc-bridge" => {
                return handle_model_siwc_bridge_cmd(&store, &db_path, &opts.positional[1..])
            }
            "mcp" => return handle_mcp_cmd(&store, &db_path, &opts.positional[1..], opts.json_mode),
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
            "wiki" => return handle_wiki_cmd(&store, &workspace, &opts, &opts.positional[1..]),
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
                let wants_http = rest.iter().any(|a| {
                    a == "--host"
                        || a == "--port"
                        || a.starts_with("--host=")
                        || a.starts_with("--port=")
                });
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
            other if other.starts_with("--") && opts.double_dash_pos != Some(0) => {
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
    reasoning_effort: Option<String>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_output_tokens: Option<usize>,
    max_iterations: Option<usize>,
    context_window_tokens: Option<usize>,
}

fn extract_reasoning_effort_from_req(req: &Value) -> Option<String> {
    if let Some(s) = req
        .get("reasoning_effort")
        .or_else(|| req.get("reasoningEffort"))
        .or_else(|| req.get("reasoning_tier"))
        .or_else(|| req.get("reasoningTier"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(s.to_string());
    }
    if let Some(r) = req.get("reasoning") {
        if let Some(s) = r.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
        if let Some(s) = r
            .get("effort")
            .or_else(|| r.get("value"))
            .or_else(|| r.get("level"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    if let Some(sel) = req.get("reasoning_selection").or_else(|| req.get("reasoningSelection")) {
        if let Some(s) = sel
            .get("effort")
            .or_else(|| sel.get("value"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    None
}

fn parse_permission_allow(msg: &Value) -> Option<bool> {
    if let Some(b) = msg
        .get("allow")
        .or_else(|| msg.get("approved"))
        .and_then(|v| v.as_bool())
    {
        return Some(b);
    }
    if let Some(d) = msg.get("decision").and_then(|v| v.as_str()) {
        return Some(matches!(
            d.trim().to_ascii_lowercase().as_str(),
            "allow"
                | "approve"
                | "approved"
                | "allow_once"
                | "allow_session"
                | "allow_conversation"
                | "yes"
        ));
    }
    if let Some(res) = msg.get("resolution") {
        if let Some(b) = res
            .get("allow")
            .or_else(|| res.get("approved"))
            .and_then(|v| v.as_bool())
        {
            return Some(b);
        }
        if let Some(d) = res.get("decision").and_then(|v| v.as_str()) {
            return Some(matches!(
                d.trim().to_ascii_lowercase().as_str(),
                "allow"
                    | "approve"
                    | "approved"
                    | "allow_once"
                    | "allow_session"
                    | "allow_conversation"
                    | "yes"
            ));
        }
        if res.get("type").and_then(|v| v.as_str()) == Some("intervention") {
            return Some(false);
        }
    }
    None
}

fn parse_user_input_answer(msg: &Value, req_id: Option<&str>) -> Option<Option<String>> {
    let cancelled = msg
        .get("cancelled")
        .or_else(|| msg.get("canceled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if cancelled {
        return Some(None);
    }
    if let Some(s) = msg
        .get("answer")
        .or_else(|| msg.get("response"))
        .or_else(|| msg.get("text"))
        .or_else(|| msg.get("value"))
        .and_then(|v| v.as_str())
    {
        return Some(Some(s.to_string()));
    }
    if let Some(answers) = msg.get("answers") {
        if let Some(s) = answers.as_str() {
            return Some(Some(s.to_string()));
        }
        if let Some(map) = answers.as_object() {
            let found = req_id
                .and_then(|id| map.get(id))
                .or_else(|| map.values().next());
            if let Some(v) = found {
                if let Some(s) = v.as_str() {
                    return Some(Some(s.to_string()));
                }
                if let Some(s) = v
                    .get("text")
                    .or_else(|| v.get("answer"))
                    .or_else(|| v.get("value"))
                    .and_then(|x| x.as_str())
                {
                    return Some(Some(s.to_string()));
                }
            }
        }
        if let Some(arr) = answers.as_array() {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .or_else(|| item.get("text").and_then(|v| v.as_str()))
                        .or_else(|| item.get("answer").and_then(|v| v.as_str()))
                        .or_else(|| item.get("value").and_then(|v| v.as_str()))
                        .map(ToString::to_string)
                })
                .collect();
            if !parts.is_empty() {
                return Some(Some(parts.join("\n")));
            }
        }
    }
    None
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
                    &json!({"requested_at_ms": now_ms()}),
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
        let reasoning_effort = extract_reasoning_effort_from_req(&req);
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
            reasoning_effort,
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
    if let Some(ref effort) = opts.reasoning_effort {
        route.reasoning_effort = Some(effort.clone());
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
        if let Some(ref effort) = ov.reasoning_effort {
            route.reasoning_effort = Some(effort.clone());
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
    if opts.no_tools {
        route.capabilities.tool_calls = false;
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

    let mut registry = if opts.no_tools {
        ToolRegistry::default()
    } else {
        let mut r = ToolRegistry::with_builtin_tools();
        suite.register_tools(&mut r);
        r
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&cancel));
    let hooks = HookRunner::load(workspace, home.as_deref(), workspace_trusted);

    let mut session_id = overrides
        .as_ref()
        .and_then(|o| o.session_id.clone())
        .or_else(|| opts.session_id.clone());
    let mut resume = overrides
        .as_ref()
        .and_then(|o| o.resume)
        .unwrap_or(opts.resume || opts.continue_last);
    if session_id.is_none() && opts.continue_last {
        if let Ok(recent) = store.list_sessions(1) {
            if let Some(last) = recent.into_iter().next() {
                session_id = Some(last.id);
                resume = true;
            }
        }
    }
    let effective_session_id = if opts.host_request_stdin {
        Some(session_id.clone().unwrap_or_else(new_id))
    } else {
        session_id
    };

    if opts.debug {
        let fallback_count = resolve_fallback_routes(&cfg).len();
        eprintln!(
            "[nola:debug] connection={} model={} protocol={} endpoint={} reasoning_effort={:?} profile={:?} tools={} fallbacks={} session={:?} resume={}",
            route.connection_id,
            route.model,
            route.protocol.as_str(),
            route.endpoint,
            route.reasoning_effort,
            profile,
            !opts.no_tools,
            fallback_count,
            effective_session_id,
            resume
        );
    }

    let a2ui_session = effective_session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    if !opts.no_tools && (opts.host_request_stdin || opts.json_mode) {
        let host_id = crate::web_api::ensure_host_id(store);
        register_a2ui_tools(suite, &mut registry, &host_id, &a2ui_session);
    }

    if !opts.no_tools {
        let subagents = Arc::new(SubagentManager::new(store.clone(), Arc::clone(&caller)));
        subagents.configure_parent(route.clone(), policy.clone());
        subagents.configure_runtime(hooks.clone(), Some(Arc::clone(&cancel)));
        register_subagent_tools(&subagents, &mut registry);
    }

    let planning = Arc::new(PlanningState::default());
    let stdout_lock = Arc::new(Mutex::new(()));

    let approval_cb: Option<ApprovalCallback> = if opts.host_request_stdin {
        let pending_approvals: Arc<Mutex<BTreeMap<String, mpsc::Sender<bool>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let pending_user_inputs: Arc<Mutex<BTreeMap<String, mpsc::Sender<Option<String>>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let stdin_closed = Arc::new(AtomicBool::new(false));

        // Spawn background stdin reader routing incoming host decision JSON lines
        let pa_reader = Arc::clone(&pending_approvals);
        let pu_reader = Arc::clone(&pending_user_inputs);
        let closed_reader = Arc::clone(&stdin_closed);
        let cancel_reader = Arc::clone(&cancel);
        std::thread::spawn(move || {
            let stdin = io::stdin();
            for line_res in stdin.lock().lines() {
                let Ok(line) = line_res else {
                    break;
                };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                    continue;
                };
                let msg_type = msg
                    .get("type")
                    .or_else(|| msg.get("kind"))
                    .or_else(|| msg.get("operation"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                let req_id = msg
                    .get("request_id")
                    .or_else(|| msg.get("requestId"))
                    .or_else(|| msg.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                match msg_type.as_str() {
                    "permission_response"
                    | "approval_response"
                    | "permission_decision"
                    | "authorization" => {
                        let allow = parse_permission_allow(&msg).unwrap_or(false);
                        let mut guard = pa_reader.lock().unwrap();
                        let sender = req_id
                            .as_deref()
                            .and_then(|id| guard.remove(id))
                            .or_else(|| {
                                if guard.len() == 1 {
                                    guard.keys().next().cloned().and_then(|k| guard.remove(&k))
                                } else {
                                    None
                                }
                            });
                        if let Some(tx) = sender {
                            let _ = tx.send(allow);
                        }
                    }
                    "user_input_response" | "user_question_response" | "user_answer" => {
                        let answer = parse_user_input_answer(&msg, req_id.as_deref()).flatten();
                        let mut guard = pu_reader.lock().unwrap();
                        let sender = req_id
                            .as_deref()
                            .and_then(|id| guard.remove(id))
                            .or_else(|| {
                                if guard.len() == 1 {
                                    guard.keys().next().cloned().and_then(|k| guard.remove(&k))
                                } else {
                                    None
                                }
                            });
                        if let Some(tx) = sender {
                            let _ = tx.send(answer);
                        }
                    }
                    "cancel" => {
                        cancel_reader.store(true, Ordering::SeqCst);
                    }
                    _ => {
                        // Accept macOS legacy / untyped control frames (e.g. `decision` / `resolution` or `answer`)
                        if let Some(allow) = parse_permission_allow(&msg) {
                            let mut guard = pa_reader.lock().unwrap();
                            let sender = req_id
                                .as_deref()
                                .and_then(|id| guard.remove(id))
                                .or_else(|| {
                                    if guard.len() == 1 {
                                        guard.keys().next().cloned().and_then(|k| guard.remove(&k))
                                    } else {
                                        None
                                    }
                                });
                            if let Some(tx) = sender {
                                let _ = tx.send(allow);
                            }
                        } else if let Some(answer) =
                            parse_user_input_answer(&msg, req_id.as_deref())
                        {
                            let mut guard = pu_reader.lock().unwrap();
                            let sender = req_id
                                .as_deref()
                                .and_then(|id| guard.remove(id))
                                .or_else(|| {
                                    if guard.len() == 1 {
                                        guard.keys().next().cloned().and_then(|k| guard.remove(&k))
                                    } else {
                                        None
                                    }
                                });
                            if let Some(tx) = sender {
                                let _ = tx.send(answer);
                            }
                        }
                    }
                }
            }
            closed_reader.store(true, Ordering::SeqCst);
            pa_reader.lock().unwrap().clear();
            pu_reader.lock().unwrap().clear();
        });

        let pa_cb = Arc::clone(&pending_approvals);
        let closed_ap = Arc::clone(&stdin_closed);
        let cancel_ap = Arc::clone(&cancel);
        let out_lock_ap = Arc::clone(&stdout_lock);
        let sid_ap = effective_session_id.clone().unwrap_or_default();
        let approval: ApprovalCallback = Arc::new(move |spec, args, reason| {
            if closed_ap.load(Ordering::SeqCst) || cancel_ap.load(Ordering::SeqCst) {
                return false;
            }
            let req_id = new_id();
            let (tx, rx) = mpsc::channel::<bool>();
            pa_cb.lock().unwrap().insert(req_id.clone(), tx);
            {
                let _g = out_lock_ap.lock().unwrap();
                println!(
                    "{}",
                    json!({
                        "type": "permission_requested",
                        "request_id": req_id,
                        "call_id": req_id,
                        "tool": spec.schema.name,
                        "tool_name": spec.schema.name,
                        "reason": reason,
                        "arguments": args,
                        "session_id": sid_ap
                    })
                );
                let _ = io::stdout().flush();
            }
            let deadline = Instant::now() + Duration::from_secs(300);
            let decision = loop {
                if cancel_ap.load(Ordering::SeqCst) || closed_ap.load(Ordering::SeqCst) {
                    break false;
                }
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(allow) => break allow,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if Instant::now() >= deadline {
                            break false;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break false,
                }
            };
            pa_cb.lock().unwrap().remove(&req_id);
            decision
        });

        let pu_cb = Arc::clone(&pending_user_inputs);
        let closed_ui = Arc::clone(&stdin_closed);
        let cancel_ui = Arc::clone(&cancel);
        let out_lock_ui = Arc::clone(&stdout_lock);
        let sid_ui = effective_session_id.clone().unwrap_or_default();
        let user_input_cb: UserInputCallback = Arc::new(move |question, options| {
            if closed_ui.load(Ordering::SeqCst) || cancel_ui.load(Ordering::SeqCst) {
                return None;
            }
            let req_id = new_id();
            let (tx, rx) = mpsc::channel::<Option<String>>();
            pu_cb.lock().unwrap().insert(req_id.clone(), tx);
            {
                let _g = out_lock_ui.lock().unwrap();
                println!(
                    "{}",
                    json!({
                        "type": "user_question",
                        "request_id": req_id,
                        "question": question,
                        "options": options,
                        "session_id": sid_ui
                    })
                );
                let _ = io::stdout().flush();
            }
            let deadline = Instant::now() + Duration::from_secs(300);
            let answer = loop {
                if cancel_ui.load(Ordering::SeqCst) || closed_ui.load(Ordering::SeqCst) {
                    break None;
                }
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(ans) => break ans,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if Instant::now() >= deadline {
                            break None;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break None,
                }
            };
            pu_cb.lock().unwrap().remove(&req_id);
            answer
        });
        planning.set_user_input_callback(Some(user_input_cb));

        Some(approval)
    } else {
        None
    };

    if !opts.no_tools {
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
    let effective_lang = opts.lang.clone().or_else(|| {
        std::env::var("NOLA_LANG")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    if let Some(ref lang) = effective_lang {
        frozen_blocks.push(nola_types::FrozenSystemBlock {
            id: "host_locale".into(),
            text: format!("Preferred response language/locale: {lang}"),
        });
    }
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

    let run_cfg = AgentRunConfig {
        session_id: effective_session_id,
        resume,
        route,
        frozen_system_blocks: frozen_blocks,
        host_capabilities: HostCapabilities {
            interactive_user_input: opts.host_request_stdin,
            network_allowed,
        },
        policy,
        hooks,
        approval: approval_cb,
        max_steps,
        force_tool_set: opts.force_tool_set,
        stream: true,
        json_schema,
        cancel,
        auto_recall_memory: !is_eval,
        inbox: None,
    };

    let out_lock_ev = Arc::clone(&stdout_lock);
    let mut live_session_id = a2ui_session.clone();
    let res = engine
        .run(&ctx, prompt, run_cfg, |ev| {
            if let AgentEvent::SessionStarted { ref session_id, .. } = ev {
                if !session_id.trim().is_empty() {
                    live_session_id = session_id.clone();
                }
            }
            if json_mode {
                if let Ok(line) = serde_json::to_string(&ev) {
                    let _g = out_lock_ev.lock().unwrap();
                    println!("{line}");
                    let _ = io::stdout().flush();
                }
                if let AgentEvent::ToolFinished {
                    ref name, is_error, ..
                } = ev
                {
                    if !is_error && (name == "render_ui_surface" || name == "delete_ui_surface") {
                        let frames =
                            collect_a2ui_surface_frames(store, &a2ui_session, &live_session_id);
                        if !frames.is_empty() {
                            let _g = out_lock_ev.lock().unwrap();
                            for frame in frames {
                                if let Ok(line) = serde_json::to_string(&frame) {
                                    println!("{line}");
                                }
                            }
                            let _ = io::stdout().flush();
                        }
                    }
                }
            } else if let AgentEvent::TextDelta { delta } = ev {
                print!("{delta}");
                let _ = io::stdout().flush();
            }
        })
        .map_err(|e| e.to_string())?;

    ctx.pty.close_all();
    if let Some(ref out_path) = opts.output_file {
        reject_if_symlink(out_path)?;
        if let Some(parent) = out_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        fs::write(out_path, &res.final_text)
            .map_err(|e| format!("failed to write --output '{}': {e}", out_path.display()))?;
    }
    if !json_mode && !res.final_text.ends_with('\n') {
        println!();
    }
    Ok(0)
}

fn collect_a2ui_surface_frames(
    store: &Store,
    primary_session: &str,
    fallback_session: &str,
) -> Vec<Value> {
    let mut target_session = primary_session;
    let mut surfaces = nola_modules::a2ui::list_conversation_surfaces(store, primary_session);
    if surfaces.is_empty() && !fallback_session.is_empty() && fallback_session != primary_session {
        let alt = nola_modules::a2ui::list_conversation_surfaces(store, fallback_session);
        if !alt.is_empty() {
            target_session = fallback_session;
            surfaces = alt;
        }
    }
    let mut frames = Vec::new();
    for snap in surfaces {
        if let Ok(snap_val) = serde_json::to_value(&snap) {
            frames.push(json!({
                "type": "ui_surface",
                "session_id": target_session,
                "snapshot": snap_val
            }));
        }
    }
    frames
}

fn resolve_route_with_overrides(
    cfg: &AppliedConfig,
    opts: &CliOptions,
    overrides: Option<&HostRunOverrides>,
) -> Result<ModelRouteConfig, String> {
    let ov_endpoint = overrides
        .and_then(|o| o.endpoint.clone())
        .or_else(|| opts.endpoint.clone())
        .filter(|s| !s.trim().is_empty());
    let ov_model = overrides
        .and_then(|o| o.model_id.clone())
        .or_else(|| opts.model.clone())
        .filter(|s| !s.trim().is_empty());
    let ov_api_key = overrides.and_then(|o| o.api_key.clone());
    let ov_protocol = overrides
        .and_then(|o| o.protocol.as_deref())
        .and_then(ModelProtocol::parse);
    let ov_reasoning_effort = overrides
        .and_then(|o| o.reasoning_effort.clone())
        .or_else(|| opts.reasoning_effort.clone())
        .filter(|s| !s.trim().is_empty());

    // Apply explicit overrides BEFORE falling back to cfg.llm.default_model!
    if let (Some(ep), Some(m)) = (ov_endpoint.clone(), ov_model.clone()) {
        let matched_conn = cfg
            .llm
            .connections
            .iter()
            .find(|c| c.endpoint.trim_end_matches('/') == ep.trim_end_matches('/'));
        let matched_entry = matched_conn.and_then(|c| c.models.iter().find(|e| e.id == m));
        let protocol = ov_protocol
            .or_else(|| {
                matched_conn
                    .and_then(|c| c.protocol.as_deref().or(c.provider.as_deref()))
                    .and_then(ModelProtocol::parse)
            })
            .unwrap_or(ModelProtocol::OpenAiChat);
        let api_key = ov_api_key
            .or_else(|| matched_conn.and_then(|c| c.api_key.clone()))
            .or_else(|| {
                matched_conn
                    .and_then(|c| c.api_key_env.as_deref())
                    .and_then(|env_k| std::env::var(env_k).ok())
            })
            .or_else(|| std::env::var("NOLA_API_KEY").ok())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .unwrap_or_default();
        return Ok(ModelRouteConfig {
            connection_id: matched_conn
                .map(|c| c.id.clone())
                .unwrap_or_else(|| "host".into()),
            endpoint: ep,
            protocol,
            model: m,
            api_key,
            context_window: matched_entry.and_then(|e| e.context_window).unwrap_or(128_000),
            max_output_tokens: matched_entry.and_then(|e| e.max_output_tokens),
            temperature: matched_entry.and_then(|e| e.temperature).or(Some(0.2)),
            top_p: matched_entry.and_then(|e| e.top_p),
            capabilities: matched_entry
                .and_then(|e| e.capabilities.clone())
                .unwrap_or_default(),
            reasoning_effort: ov_reasoning_effort
                .or_else(|| matched_entry.and_then(|e| e.reasoning_effort.clone())),
            prompt_cache_key: Some("nola-v2-host".into()),
        });
    }

    // If `--profile <conn_id>` names a configured model connection (and not a permission profile),
    // use that connection as the base selection.
    let base_cfg = if let Some(ref prof_name) = opts.profile {
        if PermissionProfile::parse(prof_name).is_none() {
            if let Some(conn) = cfg
                .llm
                .connections
                .iter()
                .find(|c| c.id == *prof_name)
                .or_else(|| {
                    cfg.connection_extras
                        .iter()
                        .find(|(_, ex)| ex.name.as_deref() == Some(prof_name.as_str()))
                        .and_then(|(cid, _)| cfg.llm.connections.iter().find(|c| c.id == *cid))
                })
            {
                let mut cloned = cfg.clone();
                let chosen_model = ov_model
                    .clone()
                    .or_else(|| conn.models.first().map(|m| m.id.clone()))
                    .unwrap_or_default();
                cloned.llm.default_model = Some(DefaultModelSelection {
                    connection: conn.id.clone(),
                    model: chosen_model,
                });
                Some(cloned)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let effective_cfg = base_cfg.as_ref().unwrap_or(cfg);

    if ov_endpoint.is_some()
        || ov_model.is_some()
        || ov_api_key.is_some()
        || ov_protocol.is_some()
        || ov_reasoning_effort.is_some()
    {
        let mut route = resolve_model_route(effective_cfg)?;
        if let Some(ep) = ov_endpoint {
            route.endpoint = ep;
        }
        if let Some(m) = ov_model {
            route.model = m;
        }
        if let Some(k) = ov_api_key {
            route.api_key = k;
        }
        if let Some(p) = ov_protocol {
            route.protocol = p;
        }
        if let Some(effort) = ov_reasoning_effort {
            route.reasoning_effort = Some(effort);
        }
        return Ok(route);
    }

    resolve_model_route(effective_cfg)
}

fn resolve_profile(opts: &CliOptions) -> PermissionProfile {
    opts.permission_profile
        .or_else(|| {
            opts.profile
                .as_deref()
                .and_then(PermissionProfile::parse)
        })
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
    let mut end_of_options = false;
    while i < args.len() {
        let a = args[i].as_str();
        if end_of_options {
            opts.positional.push(a.to_string());
            i += 1;
            continue;
        }
        let active_subcmd = opts.positional.first().map(String::as_str);
        let in_non_agent_subcmd = matches!(
            active_subcmd,
            Some(
                "init"
                    | "config"
                    | "model"
                    | "model-import-bridge"
                    | "model-siwc-bridge"
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
                    | "wiki"
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
                    | "--assets"
                    | "--transport"
                    | "--env"
                    | "--source"
                    | "--feed"
                    | "--target"
                    | "--objective"
                    | "--criteria"
                    | "--note"
                    | "--endpoint"
                    | "--model"
                    | "--model-name"
                    | "--protocol"
                    | "--provider"
                    | "--api-key"
                    | "--api-key-env"
                    | "--credential-ref"
                    | "--path"
                    | "--operation"
                    | "--op"
                    | "--plan-id"
                    | "--timeout"
            )
        });
        if prev_takes_subcmd_value {
            opts.positional.push(a.to_string());
            i += 1;
            continue;
        }
        if a == "--" {
            end_of_options = true;
            opts.double_dash_pos = Some(opts.positional.len());
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
                "--output" | "-o" => {
                    i += 1;
                    opts.output_file = Some(PathBuf::from(
                        args.get(i).ok_or("--output requires a file path")?,
                    ));
                }
                "--debug" => opts.debug = true,
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
                opts.lang = Some(
                    args.get(i)
                        .ok_or("--lang requires a locale")?
                        .trim()
                        .to_string(),
                );
            }
            "--profile" => {
                i += 1;
                opts.profile = Some(
                    args.get(i)
                        .ok_or("--profile requires a profile name")?
                        .trim()
                        .to_string(),
                );
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
            "--continue" | "-c" => {
                opts.continue_last = true;
                opts.resume = true;
            }
            "--reasoning-effort" => {
                i += 1;
                opts.reasoning_effort = Some(
                    args.get(i)
                        .ok_or("--reasoning-effort requires a level (e.g. none|low|medium|high)")?
                        .trim()
                        .to_string(),
                );
            }
            "--no-tools" => opts.no_tools = true,
            "--output" | "-o" => {
                i += 1;
                opts.output_file = Some(PathBuf::from(
                    args.get(i).ok_or("--output requires a file path")?,
                ));
            }
            "--debug" => opts.debug = true,
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
    let raw = read_config_file_safe(path)?;
    let clean = strip_jsonc(&raw);
    let cfg = parse_applied_config_json(&clean)
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

fn redacted_llm_json(cfg: &AppliedConfig) -> Value {
    redacted_applied_llm_config(cfg)
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
            let llm = redacted_llm_json(&cfg);
            let conns = llm.get("connections").cloned().unwrap_or_else(|| json!([]));
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"connections": conns}))
                    .unwrap_or_else(|_| "{}".into())
            );
            Ok(0)
        }
        Some("discover") => {
            let cfg = load_active_config(store);
            let mut target_id: Option<&str> = None;
            let mut save_discovered = false;
            for a in &args[1..] {
                if a == "--save" || a == "--apply" {
                    save_discovered = true;
                } else if !a.starts_with('-') && target_id.is_none() {
                    target_id = Some(a.as_str());
                }
            }
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
            let (protocol, api_key) = match resolve_model_route(&probe_cfg) {
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
            let base = conn.endpoint.trim_end_matches('/');
            let discover_url = if base.ends_with("/models") {
                base.to_string()
            } else {
                format!("{base}/models")
            };
            let is_loopback = url::Url::parse(&discover_url)
                .ok()
                .and_then(|u| {
                    u.host_str()
                        .map(|h| h.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase())
                })
                .is_some_and(|h| {
                    h == "localhost"
                        || h.parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                });
            let mut client_builder = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none());
            if is_loopback {
                client_builder = client_builder.no_proxy();
            }
            let client = client_builder.build().map_err(|e| e.to_string())?;
            let mut req = client.get(&discover_url);
            if protocol == ModelProtocol::AnthropicMessages {
                req = req.header("anthropic-version", "2023-06-01");
                if !api_key.trim().is_empty() {
                    req = req.header("x-api-key", api_key);
                }
            } else if !api_key.trim().is_empty() {
                req = req.bearer_auth(api_key);
            }
            let resp = req
                .send()
                .map_err(|e| format!("failed to discover models from {discover_url}: {e}"))?;
            let status = resp.status().as_u16();
            if !resp.status().is_success() {
                return Err(format!(
                    "model discovery at {discover_url} returned HTTP {status}"
                ));
            }
            let body: Value = resp
                .json()
                .map_err(|e| format!("invalid model discovery JSON: {e}"))?;
            let raw_items = body
                .get("data")
                .or_else(|| body.get("models"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut discovered: Vec<Value> = Vec::new();
            let mut discovered_ids: Vec<String> = Vec::new();
            for item in raw_items {
                let mid = item
                    .get("id")
                    .or_else(|| item.get("name"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if let Some(id_str) = mid {
                    let clean_id = id_str.strip_prefix("models/").unwrap_or(id_str).to_string();
                    if !discovered_ids.contains(&clean_id) {
                        discovered_ids.push(clean_id.clone());
                        discovered.push(json!({
                            "id": clean_id,
                            "name": item.get("display_name").or_else(|| item.get("displayName")).or_else(|| item.get("name")).and_then(|v| v.as_str()).unwrap_or(&clean_id),
                            "context_window": item.get("context_length").or_else(|| item.get("context_window")).or_else(|| item.get("inputTokenLimit")).and_then(|v| v.as_u64())
                        }));
                    }
                }
            }
            if save_discovered && !discovered_ids.is_empty() {
                let mut updated_cfg = cfg.clone();
                if let Some(c) = updated_cfg
                    .llm
                    .connections
                    .iter_mut()
                    .find(|c| c.id == conn.id)
                {
                    for mid in &discovered_ids {
                        if !c.models.iter().any(|m| m.id == *mid) {
                            c.models.push(ModelEntry {
                                id: mid.clone(),
                                context_window: None,
                                max_output_tokens: None,
                                temperature: None,
                                top_p: None,
                                reasoning_effort: None,
                                capabilities: None,
                            });
                        }
                    }
                }
                save_applied_config(store, &updated_cfg)?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "connection_id": conn.id,
                    "endpoint": conn.endpoint,
                    "discover_url": discover_url,
                    "saved": save_discovered,
                    "count": discovered.len(),
                    "models": discovered
                }))
                .unwrap()
            );
            Ok(0)
        }
        Some("configure") => {
            let mut conn_id: Option<String> = None;
            let mut conn_name: Option<String> = None;
            let mut endpoint: Option<String> = None;
            let mut protocol: Option<String> = None;
            let mut provider: Option<String> = None;
            let mut model_id: Option<String> = None;
            let mut model_name: Option<String> = None;
            let mut api_key: Option<String> = None;
            let mut api_key_env: Option<String> = None;
            let mut credential_ref: Option<String> = None;
            let mut set_default = false;

            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--id" => {
                        i += 1;
                        conn_id = Some(args.get(i).ok_or("--id requires a connection id")?.clone());
                    }
                    "--name" => {
                        i += 1;
                        conn_name = Some(args.get(i).ok_or("--name requires a value")?.clone());
                    }
                    "--endpoint" | "--url" => {
                        i += 1;
                        endpoint = Some(args.get(i).ok_or("--endpoint requires a URL")?.clone());
                    }
                    "--protocol" => {
                        i += 1;
                        protocol = Some(args.get(i).ok_or("--protocol requires a value")?.clone());
                    }
                    "--provider" => {
                        i += 1;
                        provider = Some(args.get(i).ok_or("--provider requires a value")?.clone());
                    }
                    "--model" | "-m" => {
                        i += 1;
                        model_id = Some(args.get(i).ok_or("--model requires a model id")?.clone());
                    }
                    "--model-name" => {
                        i += 1;
                        model_name =
                            Some(args.get(i).ok_or("--model-name requires a value")?.clone());
                    }
                    "--api-key" => {
                        i += 1;
                        api_key = Some(args.get(i).ok_or("--api-key requires a value")?.clone());
                    }
                    "--api-key-env" => {
                        i += 1;
                        api_key_env =
                            Some(args.get(i).ok_or("--api-key-env requires an env var")?.clone());
                    }
                    "--credential-ref" => {
                        i += 1;
                        credential_ref = Some(
                            args.get(i)
                                .ok_or("--credential-ref requires a vault key")?
                                .clone(),
                        );
                    }
                    "--default" => set_default = true,
                    other => return Err(format!("unknown model configure option '{other}'")),
                }
                i += 1;
            }

            if conn_id.is_none() && endpoint.is_none() && !io::stdin().is_terminal() {
                let mut raw = String::new();
                io::stdin()
                    .read_to_string(&mut raw)
                    .map_err(|e| format!("failed to read stdin: {e}"))?;
                if !raw.trim().is_empty() {
                    let v: Value = serde_json::from_str(&strip_jsonc(&raw))
                        .map_err(|e| format!("invalid configure JSON: {e}"))?;
                    conn_id = v.get("id").and_then(|x| x.as_str()).map(ToString::to_string);
                    conn_name = v
                        .get("name")
                        .and_then(|x| x.as_str())
                        .map(ToString::to_string);
                    endpoint = v
                        .get("endpoint")
                        .or_else(|| v.get("base_url"))
                        .and_then(|x| x.as_str())
                        .map(ToString::to_string);
                    protocol = v
                        .get("protocol")
                        .and_then(|x| x.as_str())
                        .map(ToString::to_string);
                    provider = v
                        .get("provider")
                        .and_then(|x| x.as_str())
                        .map(ToString::to_string);
                    model_id = v
                        .get("model")
                        .or_else(|| v.get("model_id"))
                        .and_then(|x| x.as_str())
                        .map(ToString::to_string);
                    model_name = v
                        .get("model_name")
                        .and_then(|x| x.as_str())
                        .map(ToString::to_string);
                    api_key = v
                        .get("api_key")
                        .or_else(|| v.get("apiKey"))
                        .and_then(|x| x.as_str())
                        .map(ToString::to_string);
                    api_key_env = v
                        .get("api_key_env")
                        .and_then(|x| x.as_str())
                        .map(ToString::to_string);
                    credential_ref = v
                        .get("credential_ref")
                        .or_else(|| v.get("credentialRef"))
                        .and_then(|x| x.as_str())
                        .map(ToString::to_string);
                    if v.get("default").and_then(|x| x.as_bool()) == Some(true) {
                        set_default = true;
                    }
                }
            }

            let cid = conn_id.ok_or(
                "usage: nola model configure --id <conn_id> [--endpoint <url>] [--model <id>] [--protocol <proto>] [--name <name>] [--api-key-env <VAR>] [--credential-ref <key>] [--default]",
            )?;
            let mut cfg = load_active_config(store);
            if let Some(existing) = cfg.llm.connections.iter_mut().find(|c| c.id == cid) {
                if let Some(ep) = endpoint {
                    existing.endpoint = ep;
                }
                if protocol.is_some() {
                    existing.protocol = protocol;
                }
                if provider.is_some() {
                    existing.provider = provider;
                }
                if api_key.is_some() {
                    existing.api_key = api_key;
                }
                if api_key_env.is_some() {
                    existing.api_key_env = api_key_env;
                }
                if let Some(ref mid) = model_id {
                    if !existing.models.iter().any(|m| m.id == *mid) {
                        existing.models.push(ModelEntry {
                            id: mid.clone(),
                            context_window: None,
                            max_output_tokens: None,
                            temperature: None,
                            top_p: None,
                            reasoning_effort: None,
                            capabilities: None,
                        });
                    }
                }
            } else {
                let ep = endpoint.ok_or("--endpoint is required when creating a new connection")?;
                let models = model_id
                    .as_ref()
                    .map(|mid| {
                        vec![ModelEntry {
                            id: mid.clone(),
                            context_window: None,
                            max_output_tokens: None,
                            temperature: None,
                            top_p: None,
                            reasoning_effort: None,
                            capabilities: None,
                        }]
                    })
                    .unwrap_or_default();
                cfg.llm.connections.push(ConnectionConfig {
                    id: cid.clone(),
                    endpoint: ep,
                    protocol,
                    provider,
                    api_key,
                    api_key_env,
                    models,
                });
            }
            if conn_name.is_some() || credential_ref.is_some() || model_name.is_some() {
                let ex = cfg.connection_extras.entry(cid.clone()).or_default();
                if let Some(name) = conn_name {
                    ex.name = Some(name);
                }
                if let Some(cref) = credential_ref {
                    ex.credential_ref = Some(cref);
                }
                if let (Some(ref mid), Some(mname)) = (&model_id, model_name) {
                    ex.model_names.insert(mid.clone(), mname);
                }
            }
            if set_default || cfg.llm.default_model.is_none() {
                if let Some(mid) = model_id.or_else(|| {
                    cfg.llm
                        .connections
                        .iter()
                        .find(|c| c.id == cid)
                        .and_then(|c| c.models.first().map(|m| m.id.clone()))
                }) {
                    cfg.llm.default_model = Some(DefaultModelSelection {
                        connection: cid.clone(),
                        model: mid,
                    });
                }
            }
            save_applied_config(store, &cfg)?;
            let reloaded = load_active_config(store);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "configured": cid,
                    "llm": redacted_llm_json(&reloaded)
                }))
                .unwrap()
            );
            Ok(0)
        }
        Some("import") => {
            let mut path_arg: Option<String> = None;
            let mut source = "open_code".to_string();
            let mut dry_run = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--path" => {
                        i += 1;
                        path_arg = Some(args.get(i).ok_or("--path requires a file path")?.clone());
                    }
                    "--source" => {
                        i += 1;
                        source = args.get(i).ok_or("--source requires a value")?.clone();
                    }
                    "--dry-run" => dry_run = true,
                    other if !other.starts_with('-') && path_arg.is_none() => {
                        path_arg = Some(other.to_string());
                    }
                    other => return Err(format!("unknown model import option '{other}'")),
                }
                i += 1;
            }
            let file_path = path_arg.ok_or("usage: nola model import <path> [--source <src>] [--dry-run]")?;
            let extracted = extract_import_candidates_from_path(&file_path, &source)?;
            if dry_run {
                let preview: Vec<Value> = extracted
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.connection_id,
                            "name": c.display_name,
                            "endpoint": c.endpoint,
                            "protocol": c.protocol,
                            "models": c.models,
                            "has_api_key": c.secret.is_some()
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": true,
                        "dry_run": true,
                        "source": source,
                        "path": file_path,
                        "candidates": preview
                    }))
                    .unwrap()
                );
                return Ok(0);
            }
            let imported_ids = commit_import_candidates(store, &extracted, true)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "imported": imported_ids.len(),
                    "connection_ids": imported_ids
                }))
                .unwrap()
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

            let base = conn.endpoint.trim_end_matches('/');
            let probe_url = if base.ends_with("/models") {
                base.to_string()
            } else {
                format!("{base}/models")
            };
            let is_loopback = url::Url::parse(&probe_url)
                .ok()
                .and_then(|u| {
                    u.host_str()
                        .map(|h| h.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase())
                })
                .is_some_and(|h| {
                    h == "localhost"
                        || h.parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                });
            let start = Instant::now();
            let mut client_builder = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none());
            if is_loopback {
                client_builder = client_builder.no_proxy();
            }
            let client = client_builder.build().map_err(|e| e.to_string())?;
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
        Some("acquire") => handle_model_acquire_cmd(store, &args[1..]),
        Some(other) => Err(format!(
            "unknown model subcommand '{other}' (expected list|connections|discover|configure|import|probe|apply-config|acquire)"
        )),
    }
}

fn emit_compact_json_line(value: &Value) {
    if let Ok(line) = serde_json::to_string(value) {
        println!("{line}");
        let _ = io::stdout().flush();
    }
}

fn normalize_engine_status_event(requested_engine: &str, raw_status: &Value) -> Value {
    json!({
        "engine": requested_engine,
        "ready": raw_status.get("ready").and_then(|v| v.as_bool()).unwrap_or(false),
        "canInstall": raw_status.get("canInstall").and_then(|v| v.as_bool()).unwrap_or(false),
        "guideUrl": raw_status.get("guideUrl").cloned().unwrap_or(Value::Null),
        "reason": raw_status
            .get("reason")
            .or_else(|| raw_status.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
    })
}

fn build_hub_download_url(source: &str, repo: &str, rev: &str, filename: &str) -> String {
    if let Ok(mirror) = std::env::var("NOLA_HF_MIRROR") {
        let trimmed = mirror.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return format!("{trimmed}/{repo}/resolve/{rev}/{filename}");
        }
    }
    if source.eq_ignore_ascii_case("modelscope") {
        format!("https://modelscope.cn/models/{repo}/resolve/{rev}/{filename}")
    } else {
        format!("https://huggingface.co/{repo}/resolve/{rev}/{filename}")
    }
}

fn normalize_acquisition_candidate(
    item: &Value,
    engine: &str,
    source: &str,
    device: &str,
) -> Option<Value> {
    let repo = item
        .get("repository")
        .or_else(|| item.get("id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let rev = item
        .get("revision")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("main")
        .to_string();
    let variant = item
        .get("variant")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            let short = repo.rsplit('/').next().unwrap_or(&repo);
            Some(format!("{short}.gguf"))
        });
    let dl_bytes = item
        .get("downloadBytes")
        .and_then(|v| v.as_i64())
        .unwrap_or(4_000_000_000);
    let est_mem = item
        .get("estimatedMemoryBytes")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| ((dl_bytes as f64) * 1.4) as i64);
    let ctx_tokens = item
        .get("contextTokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(32_768);
    let native_known = item
        .get("nativeContextKnown")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| item.get("nativeContextTokens").is_some());
    let native_tokens = item.get("nativeContextTokens").and_then(|v| v.as_i64());
    let limit_reason = item
        .get("contextLimitReason")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            if !native_known {
                Some("metadata_unknown".to_string())
            } else if ctx_tokens >= 131_072 {
                Some("target_met".to_string())
            } else {
                Some("native_limit".to_string())
            }
        });
    let file_name = variant
        .as_deref()
        .unwrap_or("model.gguf")
        .to_string();
    let download_url = item
        .pointer("/plan/url")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| build_hub_download_url(source, &repo, &rev, &file_name));

    let mut plan = item
        .get("plan")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    plan.insert("engine".into(), json!(engine));
    plan.insert("source".into(), json!(source));
    plan.insert("repository".into(), json!(repo));
    plan.insert("revision".into(), json!(rev));
    plan.insert("variant".into(), json!(file_name));
    plan.insert("device".into(), json!(device));
    plan.insert("url".into(), json!(download_url));
    plan.entry("files".to_string())
        .or_insert_with(|| json!([{ "path": file_name, "bytes": dl_bytes }]));

    Some(json!({
        "repository": repo,
        "revision": rev,
        "downloadBytes": dl_bytes,
        "estimatedMemoryBytes": est_mem,
        "contextTokens": ctx_tokens,
        "nativeContextKnown": native_known,
        "nativeContextTokens": native_tokens,
        "contextLimitReason": limit_reason,
        "variant": variant,
        "device": item.get("device").and_then(|v| v.as_str()).unwrap_or(device),
        "plan": Value::Object(plan)
    }))
}

fn handle_model_acquire_cmd(store: &Store, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("engine-status") => {
            let engine = args
                .get(1)
                .map(String::as_str)
                .ok_or("usage: nola model acquire engine-status <engine>")?;
            let raw = probe_engine_status(engine);
            let status = normalize_engine_status_event(engine, &raw);
            emit_compact_json_line(&json!({ "engineStatus": status }));
            Ok(0)
        }
        Some("setup-engine") => {
            let engine = args
                .get(1)
                .map(String::as_str)
                .ok_or("usage: nola model acquire setup-engine <engine>")?;
            let setup = run_engine_setup(engine);
            let inner_status = setup.get("status").cloned().unwrap_or_else(|| probe_engine_status(engine));
            let status = normalize_engine_status_event(engine, &inner_status);
            emit_compact_json_line(&json!({
                "engineStatus": status,
                "setup": setup
            }));
            Ok(0)
        }
        Some("hardware") => {
            let hw = probe_hardware();
            emit_compact_json_line(&json!({ "hardware": hw }));
            Ok(0)
        }
        Some("recommend") => {
            let engine = args
                .get(1)
                .map(String::as_str)
                .ok_or("usage: nola model acquire recommend <engine> <source> [query]")?;
            let source = args
                .get(2)
                .map(String::as_str)
                .ok_or("usage: nola model acquire recommend <engine> <source> [query]")?;
            let query = if args.len() > 3 {
                args[3..].join(" ")
            } else {
                String::new()
            };
            let hw = probe_hardware();
            let device = if hw.get("unifiedMemory").and_then(|v| v.as_bool()) == Some(true) {
                "unified"
            } else if hw.get("gpuMemoryBytes").and_then(|v| v.as_u64()).is_some() {
                "gpu"
            } else {
                "cpu"
            };

            let (raw_candidates, rec_source): (Vec<Value>, String) = if query.trim().is_empty() {
                let rec = recommend_local_models(&hw);
                let cands = rec
                    .get("candidates")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let rsrc = rec
                    .get("recommendationSource")
                    .and_then(|v| v.as_str())
                    .unwrap_or("catalog")
                    .to_string();
                (cands, rsrc)
            } else {
                let mirror = if source.eq_ignore_ascii_case("modelscope") {
                    Some("https://hf-mirror.com")
                } else {
                    None
                };
                let found = search_hub_models(&query, engine, mirror)?;
                let is_fallback = found
                    .first()
                    .and_then(|v| v.get("offlineFallback"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                (
                    found,
                    if is_fallback {
                        "catalog".to_string()
                    } else {
                        source.to_string()
                    },
                )
            };

            let candidates: Vec<Value> = raw_candidates
                .iter()
                .filter_map(|c| normalize_acquisition_candidate(c, engine, source, device))
                .collect();
            emit_compact_json_line(&json!({
                "hardware": hw,
                "candidates": candidates,
                "recommendationSource": rec_source,
                "reason": Value::Null
            }));
            Ok(0)
        }
        Some("confirm") => {
            let engine = args
                .get(1)
                .map(String::as_str)
                .ok_or("usage: nola model acquire confirm <engine> <source> <planJSON>")?;
            let source = args
                .get(2)
                .map(String::as_str)
                .ok_or("usage: nola model acquire confirm <engine> <source> <planJSON>")?;
            let plan_raw = args
                .get(3)
                .map(String::as_str)
                .ok_or("usage: nola model acquire confirm <engine> <source> <planJSON>")?;
            let plan_json: Value = serde_json::from_str(plan_raw)
                .map_err(|e| format!("invalid confirm plan JSON: {e}"))?;

            let repo = plan_json
                .get("repository")
                .or_else(|| plan_json.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("local-model")
                .to_string();
            let rev = plan_json
                .get("revision")
                .and_then(|v| v.as_str())
                .unwrap_or("main");
            let first_file = plan_json
                .get("files")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first());
            let variant_file = first_file
                .and_then(|f| f.get("path").and_then(|v| v.as_str()))
                .or_else(|| plan_json.get("variant").and_then(|v| v.as_str()))
                .unwrap_or("model.gguf");
            let url = plan_json
                .get("url")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| build_hub_download_url(source, &repo, rev, variant_file));
            let expected_bytes = plan_json
                .get("expected_bytes")
                .or_else(|| plan_json.get("expectedBytes"))
                .or_else(|| plan_json.get("downloadBytes"))
                .or_else(|| first_file.and_then(|f| f.get("bytes")))
                .and_then(|v| v.as_u64());
            let sha256 = plan_json
                .get("sha256")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);

            let models_root = std::env::var("NOLA_MODELS_ROOT")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| default_home_dir().join(".nola/models"));
            let safe_repo_dir: String = repo
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
                .collect();
            let safe_file: String = Path::new(variant_file)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("model.bin")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
                .collect();
            let dest_path = plan_json
                .get("dest_path")
                .or_else(|| plan_json.get("destPath"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| models_root.join(safe_repo_dir).join(safe_file));

            let dl_plan = ModelDownloadPlan {
                id: Some(new_id()),
                name: repo,
                engine: engine.to_string(),
                url,
                dest_path: dest_path.display().to_string(),
                sha256,
                quantization: plan_json
                    .get("quantization")
                    .or_else(|| plan_json.get("variant"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                expected_bytes,
            };

            let mgr = ModelDownloadManager::global();
            let initial = mgr.start_download(store, dl_plan)?;
            let download_id = initial.download_id.clone();
            emit_compact_json_line(&json!({
                "downloadedBytes": initial.downloaded_bytes,
                "totalBytes": initial.total_bytes,
                "status": initial.status
            }));

            let mut last_downloaded = initial.downloaded_bytes;
            let mut last_status = initial.status;
            loop {
                std::thread::sleep(Duration::from_millis(80));
                let cur = store
                    .get_kv(&format!("local_model_download:{download_id}"))
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| mgr.current_status(store));
                let status = cur
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("downloading")
                    .to_string();
                let dl_bytes = cur
                    .get("downloadedBytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(last_downloaded);
                let total_bytes = cur.get("totalBytes").and_then(|v| v.as_u64());

                if status == "completed" {
                    let model_id = cur
                        .get("modelId")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| download_id.clone());
                    emit_compact_json_line(&json!({
                        "model": { "id": model_id },
                        "downloadedBytes": dl_bytes,
                        "totalBytes": total_bytes.or(Some(dl_bytes)),
                        "status": "completed"
                    }));
                    return Ok(0);
                }
                if status == "failed" || status == "cancelled" {
                    let err_msg = cur
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("model download failed")
                        .to_string();
                    return Err(err_msg);
                }
                if dl_bytes != last_downloaded || status != last_status {
                    emit_compact_json_line(&json!({
                        "downloadedBytes": dl_bytes,
                        "totalBytes": total_bytes,
                        "status": status
                    }));
                    last_downloaded = dl_bytes;
                    last_status = status;
                }
            }
        }
        Some("remove") => {
            let id = args
                .get(1)
                .map(String::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or("usage: nola model acquire remove <id>")?;
            let records: Vec<LocalModelRecord> = store
                .list_module_records(KIND_LOCAL_MODEL)
                .unwrap_or_default();
            let matched = records.into_iter().find(|r| r.id == id || r.name == id);
            if let Some(rec) = matched {
                if let Some(ref p_str) = rec.path {
                    let p = Path::new(p_str);
                    if let Ok(meta) = fs::symlink_metadata(p) {
                        if !meta.file_type().is_symlink() {
                            if meta.is_file() {
                                let _ = fs::remove_file(p);
                            } else if meta.is_dir() {
                                let _ = fs::remove_dir_all(p);
                            }
                        }
                    }
                }
                let _ = store.delete_module_record(KIND_LOCAL_MODEL, &rec.id);
            } else if !id.contains("..") && !id.contains('/') && !id.contains('\\') {
                let models_root = std::env::var("NOLA_MODELS_ROOT")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| default_home_dir().join(".nola/models"));
                let cand = models_root.join(id);
                if let Ok(meta) = fs::symlink_metadata(&cand) {
                    if !meta.file_type().is_symlink() {
                        if meta.is_dir() {
                            let _ = fs::remove_dir_all(&cand);
                        } else if meta.is_file() {
                            let _ = fs::remove_file(&cand);
                        }
                    }
                }
            }
            emit_compact_json_line(&json!({
                "removed": true,
                "id": id
            }));
            Ok(0)
        }
        _ => {
            let mut requested_model: Option<String> = None;
            let mut requested_engine: Option<String> = None;
            let mut i = 0;
            while i < args.len() {
                match args[i].as_str() {
                    "--model" | "-m" => {
                        i += 1;
                        if let Some(v) = args.get(i) {
                            requested_model = Some(v.clone());
                        }
                    }
                    "--engine" => {
                        i += 1;
                        if let Some(v) = args.get(i) {
                            requested_engine = Some(v.clone());
                        }
                    }
                    other if !other.starts_with('-') && requested_model.is_none() => {
                        requested_model = Some(other.to_string());
                    }
                    _ => {}
                }
                i += 1;
            }
            let hardware = probe_hardware();
            let recommendations = recommend_local_models(&hardware);
            let engine_names: Vec<&str> = match requested_engine.as_deref() {
                Some(e) => vec![e],
                None => vec!["ollama", "llama.cpp", "mlx"],
            };
            let engines: Vec<Value> = engine_names
                .iter()
                .map(|e| probe_engine_status(e))
                .collect();
            let any_installed = engines
                .iter()
                .any(|e| e.get("ready").and_then(|v| v.as_bool()) == Some(true));
            let payload = json!({
                "ok": true,
                "status": if any_installed { "ready" } else { "catalog_only" },
                "requested_model": requested_model,
                "requested_engine": requested_engine,
                "hardware": hardware,
                "recommendations": recommendations,
                "engines": engines
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
            );
            Ok(0)
        }
    }
}

#[derive(Clone)]
struct ExtractedImportCandidate {
    candidate_id: String,
    connection_id: String,
    display_name: String,
    source: String,
    endpoint: String,
    protocol: String,
    models: Vec<String>,
    secret: Option<String>,
    credential_status: String,
}

fn extract_import_candidates_from_path(
    path_str: &str,
    src: &str,
) -> Result<Vec<ExtractedImportCandidate>, String> {
    let p = Path::new(path_str);
    reject_if_symlink(p)?;
    let raw = read_config_file_safe(p)?;
    let parsed: Value = serde_json::from_str(&strip_jsonc(&raw)).unwrap_or_else(|_| json!({}));
    let mut out = Vec::new();
    if let Some(providers) = parsed.get("provider").and_then(|v| v.as_object()) {
        for (prov_id, prov_val) in providers {
            let endpoint = prov_val
                .pointer("/options/baseURL")
                .or_else(|| prov_val.get("baseURL"))
                .or_else(|| prov_val.get("endpoint"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let models: Vec<String> = prov_val
                .get("models")
                .and_then(|v| v.as_object())
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            let secret = prov_val
                .pointer("/options/apiKey")
                .or_else(|| prov_val.get("apiKey"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            let display_name = prov_val
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(prov_id)
                .to_string();
            let proto = prov_val
                .get("protocol")
                .and_then(|v| v.as_str())
                .unwrap_or("chat_completions")
                .to_string();
            let cred_status = if secret.is_some() { "present" } else { "missing" }.to_string();
            out.push(ExtractedImportCandidate {
                candidate_id: new_id(),
                connection_id: prov_id.clone(),
                display_name,
                source: src.to_string(),
                endpoint,
                protocol: proto,
                models,
                secret,
                credential_status: cred_status,
            });
        }
    }
    if let Some(conns) = parsed.pointer("/llm/connections").and_then(|v| v.as_array()) {
        for c in conns {
            let cid = c
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("imported")
                .to_string();
            let display_name = c
                .get("name")
                .or_else(|| c.get("display_name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&cid)
                .to_string();
            let endpoint = c
                .get("endpoint")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let proto = c
                .get("protocol")
                .and_then(|v| v.as_str())
                .unwrap_or("chat_completions")
                .to_string();
            let models: Vec<String> = c
                .get("models")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|m| {
                            m.get("id")
                                .and_then(|v| v.as_str())
                                .or_else(|| m.as_str())
                                .map(ToString::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let secret = c
                .get("api_key")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            let cred_status = if secret.is_some() {
                "present"
            } else if c
                .get("api_key_env")
                .and_then(|v| v.as_str())
                .is_some_and(|e| std::env::var(e).ok().is_some_and(|v| !v.trim().is_empty()))
            {
                "environment"
            } else {
                "missing"
            }
            .to_string();
            out.push(ExtractedImportCandidate {
                candidate_id: new_id(),
                connection_id: cid,
                display_name,
                source: src.to_string(),
                endpoint,
                protocol: proto,
                models,
                secret,
                credential_status: cred_status,
            });
        }
    }
    Ok(out)
}

fn commit_import_candidates(
    store: &Store,
    candidates: &[ExtractedImportCandidate],
    store_secrets: bool,
) -> Result<Vec<String>, String> {
    let mut cfg = load_active_config(store);
    let mut imported_ids = Vec::new();
    for cand in candidates {
        let cid = if cand.connection_id.trim().is_empty() {
            new_id()
        } else {
            cand.connection_id.clone()
        };
        let models: Vec<ModelEntry> = cand
            .models
            .iter()
            .map(|s| ModelEntry {
                id: s.clone(),
                context_window: None,
                max_output_tokens: None,
                temperature: None,
                top_p: None,
                reasoning_effort: None,
                capabilities: None,
            })
            .collect();
        let api_key = if store_secrets {
            cand.secret.clone()
        } else {
            None
        };
        if let Some(existing) = cfg.llm.connections.iter_mut().find(|c| c.id == cid) {
            if !cand.endpoint.is_empty() {
                existing.endpoint = cand.endpoint.clone();
            }
            if !cand.protocol.is_empty() {
                existing.protocol = Some(cand.protocol.clone());
            }
            if api_key.is_some() {
                existing.api_key = api_key;
            }
            for m in models {
                if !existing.models.iter().any(|e| e.id == m.id) {
                    existing.models.push(m);
                }
            }
        } else if !cand.endpoint.is_empty() {
            cfg.llm.connections.push(ConnectionConfig {
                id: cid.clone(),
                endpoint: cand.endpoint.clone(),
                protocol: Some(cand.protocol.clone()),
                provider: None,
                api_key,
                api_key_env: None,
                models,
            });
        }
        if !cand.display_name.is_empty() && cand.display_name != cid {
            cfg.connection_extras
                .entry(cid.clone())
                .or_default()
                .name = Some(cand.display_name.clone());
        }
        imported_ids.push(cid);
    }
    if !imported_ids.is_empty() {
        save_applied_config(store, &cfg)?;
    }
    Ok(imported_ids)
}

fn candidate_to_mac_import_json(cand: &ExtractedImportCandidate, existing_rev: Option<i64>) -> Value {
    let models: Vec<Value> = cand
        .models
        .iter()
        .map(|mid| {
            json!({
                "model_id": mid,
                "display_name": mid,
                "protocol": cand.protocol,
                "context_window": Value::Null,
                "output_limit": Value::Null,
                "modalities": ["text"],
                "reasoning_levels": [],
                "summary_supported": Value::Null,
                "execution_capabilities": Value::Null
            })
        })
        .collect();
    let issue = if cand.endpoint.trim().is_empty() {
        Some("Missing base URL")
    } else {
        None
    };
    json!({
        "candidate_id": cand.candidate_id,
        "connection": {
            "id": cand.connection_id,
            "display_name": cand.display_name,
            "base_url": cand.endpoint,
            "protocols": [cand.protocol],
            "account_id": Value::Null,
            "source": cand.source,
            "source_id": cand.connection_id,
            "enabled": true
        },
        "models": models,
        "credential_status": cand.credential_status,
        "issue": issue,
        "existing_revision": existing_rev
    })
}

fn handle_model_import_bridge_cmd(store: &Store, args: &[String]) -> Result<u8, String> {
    let mut cli_op: Option<String> = None;
    let mut cli_source: Option<String> = None;
    let mut cli_path: Option<String> = None;
    let mut cli_plan_id: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--operation" | "--op" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    cli_op = Some(v.clone());
                }
            }
            "--source" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    cli_source = Some(v.clone());
                }
            }
            "--path" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    cli_path = Some(v.clone());
                }
            }
            "--plan-id" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    cli_plan_id = Some(v.clone());
                }
            }
            other if !other.starts_with('-') && cli_op.is_none() => {
                cli_op = Some(other.to_string());
            }
            _ => {}
        }
        i += 1;
    }

    // One-shot CLI mode when explicit CLI operation or terminal stdin
    if cli_op.is_some() || io::stdin().is_terminal() {
        let operation = cli_op.as_deref().unwrap_or("discover").to_ascii_lowercase();
        let src_name = cli_source.unwrap_or_else(|| "open_code".to_string());
        let response = match operation.as_str() {
            "plan" => {
                let candidates = if let Some(ref p) = cli_path {
                    extract_import_candidates_from_path(p, &src_name)?
                } else {
                    Vec::new()
                };
                let plan_id = new_id();
                let cand_json: Vec<Value> = candidates
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.connection_id,
                            "source": c.source,
                            "endpoint": c.endpoint,
                            "protocol": c.protocol,
                            "models": c.models,
                            "has_api_key": c.secret.is_some()
                        })
                    })
                    .collect();
                let plan = json!({
                    "ok": true,
                    "operation": "plan",
                    "plan_id": plan_id,
                    "source": src_name,
                    "path": cli_path,
                    "candidates": cand_json,
                    "expires_at_ms": now_ms().saturating_add(300_000)
                });
                let _ = store.set_kv(&format!("model_import_plan:{plan_id}"), &plan);
                plan
            }
            "commit" => {
                let raw_cands: Vec<Value> = if let Some(ref pid) = cli_plan_id {
                    store
                        .get_kv(&format!("model_import_plan:{pid}"))
                        .ok()
                        .flatten()
                        .and_then(|v| v.get("candidates").and_then(|c| c.as_array()).cloned())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let extracted: Vec<ExtractedImportCandidate> = raw_cands
                    .iter()
                    .map(|c| ExtractedImportCandidate {
                        candidate_id: new_id(),
                        connection_id: c
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        display_name: c
                            .get("name")
                            .or_else(|| c.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("imported")
                            .to_string(),
                        source: src_name.clone(),
                        endpoint: c
                            .get("endpoint")
                            .or_else(|| c.get("base_url"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        protocol: c
                            .get("protocol")
                            .and_then(|v| v.as_str())
                            .unwrap_or("chat_completions")
                            .to_string(),
                        models: c
                            .get("models")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|m| m.as_str().map(ToString::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        secret: None,
                        credential_status: "missing".into(),
                    })
                    .collect();
                let imported_ids = commit_import_candidates(store, &extracted, false)?;
                json!({
                    "ok": true,
                    "operation": "commit",
                    "plan_id": cli_plan_id,
                    "imported": imported_ids.len(),
                    "connection_ids": imported_ids
                })
            }
            _ => {
                let mut candidates = Vec::new();
                let mut checked_paths = Vec::new();
                if let Some(ref p) = cli_path {
                    checked_paths.push(p.clone());
                    if let Ok(found) = extract_import_candidates_from_path(p, &src_name) {
                        for c in found {
                            candidates.push(json!({
                                "id": c.connection_id,
                                "source": c.source,
                                "endpoint": c.endpoint,
                                "protocol": c.protocol,
                                "models": c.models,
                                "has_api_key": c.secret.is_some()
                            }));
                        }
                    }
                } else {
                    let home = default_home_dir();
                    for (sname, p) in [
                        ("open_code", home.join(".config/opencode/opencode.json")),
                        ("nola", home.join(".nola/config.jsonc")),
                    ] {
                        if p.is_file() {
                            let ps = p.display().to_string();
                            checked_paths.push(ps.clone());
                            if let Ok(found) = extract_import_candidates_from_path(&ps, sname) {
                                for c in found {
                                    candidates.push(json!({
                                        "id": c.connection_id,
                                        "source": c.source,
                                        "endpoint": c.endpoint,
                                        "protocol": c.protocol,
                                        "models": c.models,
                                        "has_api_key": c.secret.is_some()
                                    }));
                                }
                            }
                        }
                    }
                }
                json!({
                    "ok": true,
                    "operation": "discover",
                    "source": src_name,
                    "checked_paths": checked_paths,
                    "candidates": candidates
                })
            }
        };
        emit_compact_json_line(&response);
        return Ok(0);
    }

    // Long-lived NDJSON line-by-line bridge loop (zero EOF deadlock for macOS `MacModelImportBridge`)
    let mut reader = BufReader::new(io::stdin());
    let mut plans: BTreeMap<String, (bool, Vec<ExtractedImportCandidate>)> = BTreeMap::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("model-import-bridge stdin read error: {e}"))?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let frame: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                emit_compact_json_line(&json!({
                    "type": "error",
                    "error": format!("invalid JSON frame: {e}")
                }));
                continue;
            }
        };

        let request_id = frame
            .get("request_id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let msg_type = frame
            .get("type")
            .or_else(|| frame.get("operation"))
            .or_else(|| frame.get("action"))
            .and_then(|v| v.as_str())
            .unwrap_or("discover")
            .trim()
            .to_ascii_lowercase();
        let req = frame.get("request").unwrap_or(&frame);

        let reply: Value = match msg_type.as_str() {
            "plan" => {
                let src_name = req
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("open_code")
                    .to_string();
                let include_credentials = req
                    .get("include_credentials")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let path_str = req.get("path").and_then(|v| v.as_str());
                match path_str {
                    Some(p) => match extract_import_candidates_from_path(p, &src_name) {
                        Ok(candidates) => {
                            let plan_id = new_id();
                            let active_cfg = load_active_config(store);
                            let mac_cands: Vec<Value> = candidates
                                .iter()
                                .map(|c| {
                                    let existing_rev = active_cfg
                                        .llm
                                        .connections
                                        .iter()
                                        .any(|ex| ex.id == c.connection_id)
                                        .then_some(1i64);
                                    candidate_to_mac_import_json(c, existing_rev)
                                })
                                .collect();
                            let plan_obj = json!({
                                "plan_id": plan_id,
                                "candidates": mac_cands,
                                "expires_at_ms": now_ms().saturating_add(300_000)
                            });
                            plans.insert(plan_id.clone(), (include_credentials, candidates));
                            let _ = store.set_kv(&format!("model_import_plan:{plan_id}"), &plan_obj);
                            if let Some(ref rid) = request_id {
                                json!({
                                    "type": "plan",
                                    "request_id": rid,
                                    "plan": plan_obj
                                })
                            } else {
                                json!({
                                    "ok": true,
                                    "operation": "plan",
                                    "plan_id": plan_id,
                                    "plan": plan_obj,
                                    "candidates": plan_obj["candidates"]
                                })
                            }
                        }
                        Err(err) => json!({
                            "type": "error",
                            "request_id": request_id,
                            "error": err
                        }),
                    },
                    None => json!({
                        "type": "error",
                        "request_id": request_id,
                        "error": "missing 'path' in plan request"
                    }),
                }
            }
            "commit" => {
                let plan_id = req
                    .get("plan_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let selections = req
                    .get("selections")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                if let Some((may_store_creds, plan_cands)) = plans.remove(&plan_id) {
                    let selected_ids: BTreeSet<String> = selections
                        .iter()
                        .filter_map(|s| {
                            s.get("candidate_id")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string)
                        })
                        .collect();
                    let chosen: Vec<ExtractedImportCandidate> = if selected_ids.is_empty() {
                        plan_cands
                    } else {
                        plan_cands
                            .into_iter()
                            .filter(|c| selected_ids.contains(&c.candidate_id))
                            .collect()
                    };

                    // Perform interactive `credential_request` -> `credential_saved` handshake with macOS when framed & secrets included
                    let mut saved_refs: BTreeMap<String, String> = BTreeMap::new();
                    let mut handshake_failed: Option<String> = None;
                    if may_store_creds {
                        if let Some(ref rid) = request_id {
                            for cand in &chosen {
                                if let Some(ref secret) = cand.secret {
                                    let nonce = new_id();
                                    let account_id = format!("import-account-{}", new_id());
                                    emit_compact_json_line(&json!({
                                        "type": "credential_request",
                                        "request_id": rid,
                                        "nonce": nonce,
                                        "connection_id": cand.connection_id,
                                        "endpoint": cand.endpoint,
                                        "protocols": [cand.protocol],
                                        "account_id": account_id,
                                        "secret": secret
                                    }));
                                    let mut ack_line = String::new();
                                    match reader.read_line(&mut ack_line) {
                                        Ok(0) => {
                                            handshake_failed =
                                                Some("host closed pipe during credential_request".into());
                                            break;
                                        }
                                        Ok(_) => {
                                            if let Ok(ack) =
                                                serde_json::from_str::<Value>(ack_line.trim())
                                            {
                                                if ack.get("type").and_then(|v| v.as_str())
                                                    == Some("credential_saved")
                                                    && ack.get("nonce").and_then(|v| v.as_str())
                                                        == Some(nonce.as_str())
                                                {
                                                    if let Some(r) = ack
                                                        .get("reference")
                                                        .and_then(|v| v.as_str())
                                                    {
                                                        saved_refs.insert(
                                                            cand.connection_id.clone(),
                                                            r.to_string(),
                                                        );
                                                    }
                                                } else {
                                                    handshake_failed =
                                                        Some("unexpected credential_saved reply".into());
                                                    break;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            handshake_failed = Some(e.to_string());
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(err) = handshake_failed {
                        json!({
                            "type": "error",
                            "request_id": request_id,
                            "error": err
                        })
                    } else {
                        match commit_import_candidates(store, &chosen, may_store_creds) {
                            Ok(_) => {
                                if !saved_refs.is_empty() {
                                    let mut cfg = load_active_config(store);
                                    for (cid, cref) in saved_refs {
                                        cfg.connection_extras
                                            .entry(cid)
                                            .or_default()
                                            .credential_ref = Some(cref);
                                    }
                                    let _ = save_applied_config(store, &cfg);
                                }
                                let results: Vec<Value> = chosen
                                    .iter()
                                    .map(|c| {
                                        json!({
                                            "candidate_id": c.candidate_id,
                                            "connection_id": c.connection_id,
                                            "status": "imported",
                                            "error": Value::Null
                                        })
                                    })
                                    .collect();
                                if let Some(ref rid) = request_id {
                                    json!({
                                        "type": "commit",
                                        "request_id": rid,
                                        "results": results
                                    })
                                } else {
                                    json!({
                                        "ok": true,
                                        "operation": "commit",
                                        "plan_id": plan_id,
                                        "results": results
                                    })
                                }
                            }
                            Err(err) => json!({
                                "type": "error",
                                "request_id": request_id,
                                "error": err
                            }),
                        }
                    }
                } else {
                    json!({
                        "type": "error",
                        "request_id": request_id,
                        "error": format!("unknown or expired import plan '{plan_id}'")
                    })
                }
            }
            "probe_prepare" => {
                let conn_id = req
                    .get("connection_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let expected_rev = req
                    .get("expected_revision")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1);
                let model_id = req.get("model_id").and_then(|v| v.as_str()).unwrap_or("");
                let proto = req
                    .get("protocol")
                    .and_then(|v| v.as_str())
                    .unwrap_or("chat_completions");
                let cfg = load_active_config(store);
                let meta = resolve_probe_connection_metadata(store, &cfg, conn_id, &req);
                let binding_input = format!(
                    "{conn_id}:{expected_rev}:{model_id}:{proto}:{}:{}",
                    meta.account_id.as_deref().unwrap_or(""),
                    meta.credential_secret
                );
                let confirmation = json!({
                    "request_id": new_id(),
                    "connection_id": conn_id,
                    "expected_revision": expected_rev,
                    "account_id": meta.account_id,
                    "account_revision": meta.account_revision,
                    "credential_binding": sha256_hex(binding_input.as_bytes()),
                    "model_id": model_id,
                    "protocol": proto,
                    "max_requests": 1,
                    "acknowledge_possible_charges": false
                });
                json!({
                    "type": "probe_prepare",
                    "request_id": request_id,
                    "confirmation": confirmation
                })
            }
            "probe" => {
                let ack = req
                    .get("acknowledge_possible_charges")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !ack {
                    json!({
                        "type": "error",
                        "request_id": request_id,
                        "error": "probe requires acknowledge_possible_charges: true"
                    })
                } else {
                    let conn_id = req
                        .get("connection_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let expected_rev = req
                        .get("expected_revision")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(1);
                    let model_id = req.get("model_id").and_then(|v| v.as_str()).unwrap_or("");
                    let proto = req
                        .get("protocol")
                        .and_then(|v| v.as_str())
                        .unwrap_or("chat_completions");
                    let cred_binding = req
                        .get("credential_binding")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let cfg = load_active_config(store);
                    let meta = resolve_probe_connection_metadata(store, &cfg, conn_id, &req);
                    let snapshot = json!({
                        "revision": expected_rev + 1,
                        "document": {
                            "format_version": 1,
                            "connection": {
                                "id": conn_id,
                                "display_name": meta.display_name,
                                "base_url": meta.base_url,
                                "protocols": [proto],
                                "account_id": meta.account_id,
                                "source": "nola",
                                "source_id": conn_id,
                                "enabled": true
                            },
                            "catalog": {
                                "observed_at_ms": now_ms(),
                                "source": "probe",
                                "models": [{
                                    "model_id": model_id,
                                    "display_name": model_id,
                                    "protocol": proto,
                                    "context_window": 128000,
                                    "output_limit": 16384,
                                    "modalities": ["text"],
                                    "reasoning_levels": [],
                                    "summary_supported": Value::Null,
                                    "execution_capabilities": Value::Null
                                }]
                            },
                            "capability_assessments": [{
                                "model_id": model_id,
                                "protocol": proto,
                                "evidence": {
                                    "text_generation": true,
                                    "account_revision": meta.account_revision,
                                    "credential_binding": cred_binding,
                                    "observed_at_ms": now_ms()
                                },
                                "user_override": Value::Null
                            }]
                        }
                    });
                    json!({
                        "type": "probe",
                        "request_id": request_id,
                        "snapshot": snapshot
                    })
                }
            }
            _ => {
                let src_name = req
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("open_code");
                let mut candidates = Vec::new();
                if let Some(p) = req.get("path").and_then(|v| v.as_str()) {
                    if let Ok(found) = extract_import_candidates_from_path(p, src_name) {
                        for c in found {
                            candidates.push(candidate_to_mac_import_json(&c, None));
                        }
                    }
                }
                json!({
                    "ok": true,
                    "type": "discover",
                    "request_id": request_id,
                    "candidates": candidates
                })
            }
        };
        emit_compact_json_line(&reply);
    }
    Ok(0)
}

#[derive(Clone)]
struct SiwcFlowState {
    owner_id: String,
    flow_id: String,
    account_id: String,
    expected_account_revision: Option<i64>,
    display_name: String,
    phase: String,
    authorization_url: Option<String>,
    account: Option<Value>,
}

impl SiwcFlowState {
    fn to_json(&self) -> Value {
        json!({
            "owner_id": self.owner_id,
            "flow_id": self.flow_id,
            "account_id": self.account_id,
            "expected_account_revision": self.expected_account_revision,
            "phase": self.phase,
            "authorization_url": self.authorization_url,
            "error": Value::Null,
            "private_session_may_be_retained": false,
            "account": self.account,
            "inference_performed": false
        })
    }
}

struct ProbeConnectionMetadata {
    base_url: String,
    display_name: String,
    account_id: Option<String>,
    account_revision: Option<i64>,
    credential_secret: String,
}

fn resolve_probe_connection_metadata(
    store: &Store,
    cfg: &AppliedConfig,
    conn_id: &str,
    req: &Value,
) -> ProbeConnectionMetadata {
    let cfg_conn = cfg.llm.connections.iter().find(|c| c.id == conn_id);
    let store_conn = store
        .get_module_record::<Value>("model_connection", conn_id)
        .ok()
        .flatten()
        .or_else(|| {
            store
                .get_kv("model_connections")
                .ok()
                .flatten()
                .and_then(|val| match val {
                    Value::Array(arr) => arr.into_iter().find(|item| {
                        item.get("id")
                            .or_else(|| item.get("connection_id"))
                            .or_else(|| item.pointer("/document/connection/id"))
                            .and_then(|v| v.as_str())
                            == Some(conn_id)
                    }),
                    Value::Object(mut map) => map.remove(conn_id),
                    _ => None,
                })
        });

    let base_url = cfg_conn
        .map(|c| c.endpoint.trim())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            store_conn.as_ref().and_then(|rec| {
                rec.get("base_url")
                    .or_else(|| rec.get("endpoint"))
                    .or_else(|| rec.get("baseURL"))
                    .or_else(|| rec.pointer("/document/connection/base_url"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
            })
        })
        .unwrap_or_default();

    let display_name = cfg
        .connection_extras
        .get(conn_id)
        .and_then(|e| e.name.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            store_conn.as_ref().and_then(|rec| {
                rec.get("display_name")
                    .or_else(|| rec.get("name"))
                    .or_else(|| rec.pointer("/document/connection/display_name"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
            })
        })
        .unwrap_or_else(|| conn_id.to_string());

    let account_id = req
        .get("account_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            store_conn.as_ref().and_then(|rec| {
                rec.get("account_id")
                    .or_else(|| rec.pointer("/document/connection/account_id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
            })
        })
        .or_else(|| {
            conn_id
                .strip_prefix("siwc-")
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
        });

    let account_record = account_id.as_deref().and_then(|aid| {
        store
            .get_module_record::<Value>("model_account", aid)
            .ok()
            .flatten()
            .or_else(|| {
                store
                    .get_kv(&format!("siwc_account:{aid}"))
                    .ok()
                    .flatten()
                    .filter(|v| v.is_object())
            })
            .or_else(|| {
                store
                    .get_kv("siwc_account")
                    .ok()
                    .flatten()
                    .filter(|v| {
                        v.get("account_id")
                            .or_else(|| v.get("id"))
                            .and_then(|s| s.as_str())
                            == Some(aid)
                    })
            })
    });

    let account_revision = req
        .get("account_revision")
        .and_then(|v| v.as_i64())
        .or_else(|| {
            account_record
                .as_ref()
                .and_then(|acc| {
                    acc.get("revision")
                        .or_else(|| acc.get("account_revision"))
                        .and_then(|v| v.as_i64())
                })
        })
        .or_else(|| {
            store_conn
                .as_ref()
                .and_then(|rec| rec.get("account_revision").and_then(|v| v.as_i64()))
        })
        .or_else(|| account_record.as_ref().map(|_| 1));

    let credential_secret = cfg_conn
        .and_then(|c| c.api_key.as_deref())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            cfg.connection_extras
                .get(conn_id)
                .and_then(|e| e.credential_ref.as_deref())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            account_record.as_ref().and_then(|acc| {
                acc.get("credential_ref")
                    .or_else(|| acc.pointer("/account/credential_ref"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
            })
        })
        .or_else(|| {
            store_conn.as_ref().and_then(|rec| {
                rec.get("credential_ref")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
            })
        })
        .unwrap_or_default();

    ProbeConnectionMetadata {
        base_url,
        display_name,
        account_id,
        account_revision,
        credential_secret,
    }
}

fn siwc_account_snapshot(
    account_id: &str,
    display_name: &str,
    revision: i64,
    connected: bool,
) -> Value {
    json!({
        "revision": revision,
        "account": {
            "id": account_id,
            "display_name": if display_name.trim().is_empty() { "ChatGPT" } else { display_name },
            "source": "nola_chatgpt",
            "credential_ref": if connected { json!(format!("siwc:{account_id}")) } else { Value::Null },
            "state": if connected { "ready" } else { "disconnected" }
        }
    })
}

fn siwc_default_connection_snapshot(account_id: &str, revision: i64) -> Value {
    let conn_id = format!("siwc-{account_id}");
    json!({
        "revision": revision,
        "document": {
            "format_version": 1,
            "connection": {
                "id": conn_id,
                "display_name": "ChatGPT (Sign in with ChatGPT)",
                "base_url": "https://chatgpt.com/backend-api/codex",
                "protocols": ["siwc_responses"],
                "account_id": account_id,
                "source": "nola_chatgpt",
                "source_id": account_id,
                "enabled": true
            },
            "catalog": {
                "observed_at_ms": now_ms(),
                "source": "nola_chatgpt",
                "models": [
                    {
                        "model_id": "gpt-5.3-codex",
                        "display_name": "GPT-5.3 Codex",
                        "protocol": "siwc_responses",
                        "context_window": 272000,
                        "output_limit": 32768,
                        "modalities": ["text", "image"],
                        "reasoning_levels": ["low", "medium", "high"],
                        "summary_supported": true,
                        "execution_capabilities": Value::Null
                    },
                    {
                        "model_id": "gpt-5.2-codex",
                        "display_name": "GPT-5.2 Codex",
                        "protocol": "siwc_responses",
                        "context_window": 272000,
                        "output_limit": 32768,
                        "modalities": ["text", "image"],
                        "reasoning_levels": ["low", "medium", "high"],
                        "summary_supported": true,
                        "execution_capabilities": Value::Null
                    }
                ]
            },
            "capability_assessments": []
        }
    })
}

fn handle_model_siwc_bridge_cmd(
    store: &Store,
    db_path: &Path,
    args: &[String],
) -> Result<u8, String> {
    let vault = Vault::open(default_vault_path(db_path)).map_err(|e| e.to_string())?;
    let cli_op = args
        .first()
        .map(String::as_str)
        .filter(|s| !s.starts_with('-'));

    let run_unframed_op = |op: &str, req: Option<&Value>| -> Result<Value, String> {
        match op {
            "save" | "bind" | "connect" => {
                let account_id = req
                    .and_then(|v| {
                        v.get("account_id")
                            .or_else(|| v.get("id"))
                            .and_then(|s| s.as_str())
                    })
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                let email = req
                    .and_then(|v| {
                        v.get("email")
                            .or_else(|| v.get("label"))
                            .or_else(|| v.get("display_name"))
                            .and_then(|s| s.as_str())
                    })
                    .unwrap_or("")
                    .to_string();
                if let Some(tok) = req
                    .and_then(|v| {
                        v.get("access_token")
                            .or_else(|| v.get("token"))
                            .and_then(|s| s.as_str())
                    })
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    vault
                        .set_secret("siwc_access_token", tok)
                        .map_err(|e| e.to_string())?;
                    let _ = vault.set_secret(&format!("siwc_access_token:{account_id}"), tok);
                }
                let has_token = vault
                    .get_secret("siwc_access_token")
                    .ok()
                    .flatten()
                    .is_some_and(|s| !s.trim().is_empty());
                let meta = json!({
                    "account_id": account_id,
                    "email": email,
                    "display_name": email,
                    "revision": 1,
                    "session_revision": 1,
                    "authenticated": has_token,
                    "updated_at_ms": now_ms()
                });
                let _ = store.set_kv("siwc_account", &meta);
                let _ = store.set_kv(&format!("siwc_account:{account_id}"), &meta);
                Ok(json!({
                    "ok": true,
                    "status": if has_token { "connected" } else { "disconnected" },
                    "authenticated": has_token,
                    "account": meta
                }))
            }
            "forget" | "logout" | "clear" | "disconnect" | "sign_out" => {
                let _ = vault.delete_secret("siwc_access_token");
                if let Some(aid) = req
                    .and_then(|v| v.get("account_id").and_then(|s| s.as_str()))
                    .filter(|s| !s.trim().is_empty())
                {
                    let _ = vault.delete_secret(&format!("siwc_access_token:{aid}"));
                    let _ = store.set_kv(&format!("siwc_account:{aid}"), &Value::Null);
                }
                let _ = store.set_kv("siwc_account", &Value::Null);
                Ok(json!({
                    "ok": true,
                    "status": "disconnected",
                    "authenticated": false,
                    "cleared": true
                }))
            }
            _ => {
                let has_token = vault
                    .get_secret("siwc_access_token")
                    .ok()
                    .flatten()
                    .is_some_and(|s| !s.trim().is_empty());
                let account = store
                    .get_kv("siwc_account")
                    .ok()
                    .flatten()
                    .filter(|v| !v.is_null());
                let accounts: Vec<Value> = account.iter().cloned().collect();
                Ok(json!({
                    "ok": true,
                    "status": if has_token { "connected" } else { "disconnected" },
                    "authenticated": has_token,
                    "account": account,
                    "accounts": accounts
                }))
            }
        }
    };

    if let Some(op) = cli_op {
        let resp = run_unframed_op(&op.to_ascii_lowercase(), None)?;
        emit_compact_json_line(&resp);
        return Ok(0);
    }
    if io::stdin().is_terminal() {
        let resp = run_unframed_op("status", None)?;
        emit_compact_json_line(&resp);
        return Ok(0);
    }

    // Long-lived NDJSON line-by-line loop matching `MacSiwcAccountBridge.swift`
    let owner_id = new_id();
    let mut current_flow: Option<SiwcFlowState> = None;
    let mut reader = BufReader::new(io::stdin());
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("model-siwc-bridge stdin read error: {e}"))?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let frame: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => {
                emit_compact_json_line(&json!({
                    "type": "error",
                    "error": "native_siwc_invalid_frame"
                }));
                continue;
            }
        };

        let request_id = frame
            .get("request_id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let raw_type = frame
            .get("type")
            .or_else(|| frame.get("operation"))
            .or_else(|| frame.get("action"))
            .and_then(|v| v.as_str())
            .unwrap_or("status")
            .trim()
            .to_ascii_lowercase();
        let req = frame.get("request").unwrap_or(&frame);

        // Unframed single-line CLI JSON request
        if request_id.is_none()
            && matches!(
                raw_type.as_str(),
                "save" | "bind" | "connect" | "forget" | "logout" | "clear" | "disconnect" | "status"
            )
        {
            match run_unframed_op(&raw_type, Some(req)) {
                Ok(resp) => emit_compact_json_line(&resp),
                Err(e) => emit_compact_json_line(&json!({"ok": false, "error": e})),
            }
            continue;
        }

        // Normalize `update` wrapper (`{"type":"update","request":{"action":"status"|"cancel"|"finish",...}}`)
        let op = if raw_type == "update" {
            req.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("status")
                .trim()
                .to_ascii_lowercase()
        } else {
            raw_type
        };

        let reply: Value = match op.as_str() {
            "hello" => json!({
                "type": "hello",
                "request_id": request_id,
                "protocol": 1,
                "owner_id": owner_id
            }),
            "start" => {
                let account_id = req
                    .get("account_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                let expected_rev = req
                    .get("expected_account_revision")
                    .and_then(|v| v.as_i64());
                let display_name = req
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ChatGPT")
                    .to_string();
                let pkce = nola_mcp::McpOAuthManager::generate_pkce().ok();
                let state = new_id();
                let challenge = pkce.map(|p| p.challenge).unwrap_or_else(new_id);
                let auth_url = format!(
                    "https://auth.openai.com/oauth/authorize?response_type=code&client_id=app_EMoamEEZ73f0CkXaXp7hrann&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback&scope=openid+profile+email+offline_access&code_challenge={challenge}&code_challenge_method=S256&state={state}"
                );
                let flow = SiwcFlowState {
                    owner_id: owner_id.clone(),
                    flow_id: new_id(),
                    account_id,
                    expected_account_revision: expected_rev,
                    display_name,
                    phase: "awaiting_authorization".into(),
                    authorization_url: Some(auth_url),
                    account: None,
                };
                let flow_val = flow.to_json();
                current_flow = Some(flow);
                json!({
                    "type": "flow",
                    "request_id": request_id,
                    "flow": flow_val
                })
            }
            "status" => {
                if let Some(ref flow) = current_flow {
                    json!({
                        "type": "flow",
                        "request_id": request_id,
                        "flow": flow.to_json()
                    })
                } else {
                    json!({
                        "type": "error",
                        "request_id": request_id,
                        "error": "native_siwc_private_session_changed"
                    })
                }
            }
            "cancel" => {
                if let Some(ref mut flow) = current_flow {
                    flow.phase = "cancelled".into();
                    flow.authorization_url = None;
                    let flow_val = flow.to_json();
                    json!({
                        "type": "flow",
                        "request_id": request_id,
                        "flow": flow_val
                    })
                } else {
                    json!({
                        "type": "error",
                        "request_id": request_id,
                        "error": "native_siwc_private_session_changed"
                    })
                }
            }
            "finish" => {
                if let Some(ref mut flow) = current_flow {
                    let next_rev = flow.expected_account_revision.unwrap_or(0) + 1;
                    let acct = siwc_account_snapshot(
                        &flow.account_id,
                        &flow.display_name,
                        next_rev,
                        true,
                    );
                    let meta = json!({
                        "account_id": flow.account_id,
                        "display_name": flow.display_name,
                        "revision": next_rev,
                        "session_revision": 1,
                        "authenticated": true,
                        "updated_at_ms": now_ms()
                    });
                    let _ = store.set_kv("siwc_account", &meta);
                    let _ = store.set_kv(&format!("siwc_account:{}", flow.account_id), &meta);
                    flow.phase = "committed".into();
                    flow.authorization_url = None;
                    flow.account = Some(acct);
                    let flow_val = flow.to_json();
                    json!({
                        "type": "flow",
                        "request_id": request_id,
                        "flow": flow_val
                    })
                } else {
                    json!({
                        "type": "error",
                        "request_id": request_id,
                        "error": "native_siwc_private_session_changed"
                    })
                }
            }
            "account_status" | "models" | "discovermodels" | "refresh" | "sign_out"
            | "disconnect" => {
                let account_id = req
                    .get("account_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("siwc-default")
                    .to_string();
                let expected_rev = req
                    .get("expected_account_revision")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1);
                let stored_meta = store
                    .get_kv(&format!("siwc_account:{account_id}"))
                    .ok()
                    .flatten()
                    .filter(|v| !v.is_null())
                    .or_else(|| {
                        store
                            .get_kv("siwc_account")
                            .ok()
                            .flatten()
                            .filter(|v| !v.is_null())
                    });
                let stored_rev = stored_meta
                    .as_ref()
                    .and_then(|m| m.get("revision").and_then(|v| v.as_i64()))
                    .unwrap_or(expected_rev)
                    .max(expected_rev);
                let mut session_rev = stored_meta
                    .as_ref()
                    .and_then(|m| m.get("session_revision").and_then(|v| v.as_i64()))
                    .unwrap_or(1);
                let display_name = stored_meta
                    .as_ref()
                    .and_then(|m| {
                        m.get("display_name")
                            .or_else(|| m.get("email"))
                            .and_then(|v| v.as_str())
                    })
                    .unwrap_or("ChatGPT")
                    .to_string();
                let has_token = vault
                    .get_secret(&format!("siwc_access_token:{account_id}"))
                    .ok()
                    .flatten()
                    .is_some_and(|s| !s.trim().is_empty())
                    || vault
                        .get_secret("siwc_access_token")
                        .ok()
                        .flatten()
                        .is_some_and(|s| !s.trim().is_empty())
                    || stored_meta
                        .as_ref()
                        .and_then(|m| m.get("authenticated").and_then(|v| v.as_bool()))
                        .unwrap_or(false);

                match op.as_str() {
                    "sign_out" | "disconnect" => {
                        let _ = vault.delete_secret("siwc_access_token");
                        let _ = vault.delete_secret(&format!("siwc_access_token:{account_id}"));
                        let next_rev = stored_rev + 1;
                        let meta = json!({
                            "account_id": account_id,
                            "display_name": display_name,
                            "revision": next_rev,
                            "session_revision": session_rev + 1,
                            "authenticated": false,
                            "updated_at_ms": now_ms()
                        });
                        let _ = store.set_kv(&format!("siwc_account:{account_id}"), &meta);
                        let _ = store.set_kv("siwc_account", &Value::Null);
                        json!({
                            "type": "account",
                            "request_id": request_id,
                            "result": {
                                "owner_id": owner_id,
                                "account_id": account_id,
                                "account": siwc_account_snapshot(&account_id, &display_name, next_rev, false),
                                "session": {
                                    "account_id": account_id,
                                    "session_revision": session_rev + 1,
                                    "signed_in": false,
                                    "plan_usage_authorized": false,
                                    "reauthorization_required": false,
                                    "revocation_unconfirmed": false
                                },
                                "connection": Value::Null,
                                "inference_performed": false
                            }
                        })
                    }
                    "refresh" => {
                        session_rev += 1;
                        let meta = json!({
                            "account_id": account_id,
                            "display_name": display_name,
                            "revision": stored_rev,
                            "session_revision": session_rev,
                            "authenticated": has_token,
                            "updated_at_ms": now_ms()
                        });
                        let _ = store.set_kv(&format!("siwc_account:{account_id}"), &meta);
                        json!({
                            "type": "account",
                            "request_id": request_id,
                            "result": {
                                "owner_id": owner_id,
                                "account_id": account_id,
                                "account": siwc_account_snapshot(&account_id, &display_name, stored_rev, has_token),
                                "session": {
                                    "account_id": account_id,
                                    "session_revision": session_rev,
                                    "signed_in": has_token,
                                    "plan_usage_authorized": has_token,
                                    "reauthorization_required": !has_token,
                                    "revocation_unconfirmed": false
                                },
                                "connection": Value::Null,
                                "inference_performed": false
                            }
                        })
                    }
                    "models" | "discovermodels" => {
                        let conn_snap = siwc_default_connection_snapshot(&account_id, stored_rev);
                        let _ = store.set_kv(&format!("siwc_connection:{account_id}"), &conn_snap);
                        json!({
                            "type": "account",
                            "request_id": request_id,
                            "result": {
                                "owner_id": owner_id,
                                "account_id": account_id,
                                "account": siwc_account_snapshot(&account_id, &display_name, stored_rev, has_token),
                                "session": {
                                    "account_id": account_id,
                                    "session_revision": session_rev,
                                    "signed_in": has_token,
                                    "plan_usage_authorized": has_token,
                                    "reauthorization_required": !has_token,
                                    "revocation_unconfirmed": false
                                },
                                "connection": conn_snap,
                                "inference_performed": false
                            }
                        })
                    }
                    _ => {
                        // account_status
                        json!({
                            "type": "account",
                            "request_id": request_id,
                            "result": {
                                "owner_id": owner_id,
                                "account_id": account_id,
                                "account": siwc_account_snapshot(&account_id, &display_name, stored_rev, has_token),
                                "session": {
                                    "account_id": account_id,
                                    "session_revision": session_rev,
                                    "signed_in": has_token,
                                    "plan_usage_authorized": has_token,
                                    "reauthorization_required": !has_token,
                                    "revocation_unconfirmed": false
                                },
                                "connection": Value::Null,
                                "inference_performed": false
                            }
                        })
                    }
                }
            }
            _ => json!({
                "type": "error",
                "request_id": request_id,
                "error": format!("unsupported_siwc_action:{op}")
            }),
        };
        emit_compact_json_line(&reply);
    }
    Ok(0)
}

fn handle_mcp_cmd(
    store: &Store,
    db_path: &Path,
    args: &[String],
    json_mode: bool,
) -> Result<u8, String> {
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
            let mut env: BTreeMap<String, String> = BTreeMap::new();
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
                    "--env" => {
                        i += 1;
                        let raw = args.get(i).ok_or("--env requires KEY=VALUE")?;
                        let (k, v) = raw
                            .split_once('=')
                            .filter(|(k, _)| !k.trim().is_empty())
                            .ok_or_else(|| format!("invalid --env '{raw}' (expected KEY=VALUE)"))?;
                        env.insert(k.trim().to_string(), v.to_string());
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
                env,
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
            let server_id = args
                .get(1)
                .ok_or("usage: nola mcp auth <server_id> [--status] [--timeout <secs>]")?;
            let mut status_only = false;
            let mut custom_timeout: Option<Duration> = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--status" => status_only = true,
                    "--timeout" => {
                        i += 1;
                        let secs: u64 = args
                            .get(i)
                            .ok_or("--timeout requires seconds")?
                            .parse()
                            .map_err(|_| "--timeout must be an integer number of seconds")?;
                        custom_timeout = Some(Duration::from_secs(secs.max(1)));
                    }
                    other => return Err(format!("unknown mcp auth option '{other}'")),
                }
                i += 1;
            }
            let vault =
                Arc::new(Vault::open(default_vault_path(db_path)).map_err(|e| e.to_string())?);
            if status_only {
                let mgr = McpManager::new(Some(Arc::clone(&vault)));
                let token_info = mgr.oauth().and_then(|o| o.load_tokens(server_id).ok().flatten());
                println!(
                    "{}",
                    json!({
                        "server_id": server_id,
                        "authenticated": token_info.is_some(),
                        "token_endpoint": token_info.as_ref().map(|t| t.token_endpoint.as_str()),
                        "expires_at_ms": token_info.and_then(|t| t.expires_at_ms)
                    })
                );
                return Ok(0);
            }
            let servers = store.list_mcp_servers().map_err(|e| e.to_string())?;
            let server = servers
                .into_iter()
                .find(|s| s.id == *server_id)
                .ok_or_else(|| format!("MCP server '{server_id}' not found"))?;
            let server_url = server
                .url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    format!(
                        "MCP server '{server_id}' has no HTTP/SSE URL configured for OAuth"
                    )
                })?;

            let on_auth_url = |auth_url: &str| {
                if json_mode {
                    emit_compact_json_line(&json!({
                        "event": "authorization_url",
                        "server_id": server_id,
                        "authorization_url": auth_url
                    }));
                } else {
                    eprintln!(
                        "Open this URL in your browser to authorize MCP server '{server_id}':\n{auth_url}"
                    );
                    let _ = io::stderr().flush();
                }
            };
            let tokens = if server.oauth_client_id.is_none()
                && server.oauth_scopes.is_empty()
                && custom_timeout.is_none()
            {
                nola_mcp::run_interactive_oauth_flow(server_id, server_url, vault, on_auth_url)
                    .map_err(|e| e.to_string())?
            } else {
                nola_mcp::run_interactive_oauth_flow_with_options(
                    server_id,
                    server_url,
                    vault,
                    server.oauth_client_id.as_deref(),
                    &server.oauth_scopes,
                    custom_timeout.unwrap_or(nola_mcp::DEFAULT_OAUTH_CALLBACK_TIMEOUT),
                    on_auth_url,
                )
                .map_err(|e| e.to_string())?
            };
            println!(
                "{}",
                json!({
                    "ok": true,
                    "server_id": server_id,
                    "authenticated": true,
                    "token_endpoint": tokens.token_endpoint,
                    "client_id": tokens.client_id,
                    "expires_at_ms": tokens.expires_at_ms
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
            let is_action = |s: &str| {
                matches!(
                    s,
                    "list" | "install" | "add" | "enable" | "disable" | "remove" | "uninstall"
                        | "reconcile" | "update"
                )
            };
            let is_runtime_form = sub_args
                .first()
                .map(|s| !is_action(s.as_str()))
                .unwrap_or(false)
                && sub_args.len() >= 3;

            let (runtime, profile, action, source): (String, String, String, Option<String>) =
                if is_runtime_form {
                    (
                        sub_args[0].clone(),
                        sub_args[1].clone(),
                        sub_args[2].to_ascii_lowercase(),
                        sub_args.get(3).cloned(),
                    )
                } else {
                    let act = sub_args
                        .first()
                        .map(|s| s.to_ascii_lowercase())
                        .unwrap_or_else(|| "list".to_string());
                    (
                        "dsh".to_string(),
                        "web".to_string(),
                        act,
                        sub_args.get(1).cloned(),
                    )
                };

            let find_matching_plugin = |target: &str| -> Option<PluginRecord> {
                let target_trimmed = target.trim();
                suite
                    .list_records::<PluginRecord>(KIND_PLUGIN)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|r| {
                        r.id == target_trimmed
                            || r.name == target_trimmed
                            || format!("{}@{}", r.name, r.version) == target_trimmed
                    })
            };

            match action.as_str() {
                "install" | "add" => {
                    let src = source.as_deref().ok_or(
                        "usage: nola plugin desktop-manage [<runtime> <profile>] install <source>",
                    )?;
                    if !is_runtime_form && sub_args.len() > 2 {
                        return handle_plugin_cmd(store, suite, sub_args);
                    }
                    let _rec = install_plugin_package(
                        store,
                        &suite.workspace_root,
                        &runtime,
                        &profile,
                        src,
                    )?;
                }
                "enable" | "disable" => {
                    let target = source.as_deref().ok_or(
                        "usage: nola plugin desktop-manage [<runtime> <profile>] enable|disable <id>",
                    )?;
                    let mut rec = find_matching_plugin(target)
                        .ok_or_else(|| format!("plugin '{target}' not found"))?;
                    rec.enabled = action == "enable";
                    suite
                        .upsert_record(KIND_PLUGIN, &rec.id, &rec)
                        .map_err(|e| e.to_string())?;
                }
                "remove" | "uninstall" => {
                    let target = source.as_deref().ok_or(
                        "usage: nola plugin desktop-manage [<runtime> <profile>] remove <id>",
                    )?;
                    if let Some(rec) = find_matching_plugin(target) {
                        let _ = suite.delete_record(KIND_PLUGIN, &rec.id);
                        let staged_dir = suite.workspace_root.join(".nola/plugins").join(&rec.id);
                        if staged_dir.is_dir() && !fs::symlink_metadata(&staged_dir).is_ok_and(|m| m.file_type().is_symlink()) {
                            let _ = fs::remove_dir_all(&staged_dir);
                        }
                    } else {
                        let _ = suite.delete_record(KIND_PLUGIN, target);
                    }
                }
                "list" | "reconcile" | "update" => {}
                other => {
                    return Err(format!(
                        "unknown desktop-manage action '{other}' (expected list|install|enable|disable|remove|reconcile|update)"
                    ));
                }
            }

            let reconciled = reconcile_enabled_plugins(store, &suite.workspace_root);
            let catalog = list_harness_plugins(Some(store), &suite.workspace_root);
            let pages = list_desktop_plugin_pages(&suite.workspace_root);
            let installed: Vec<PluginRecord> = suite.list_records(KIND_PLUGIN).unwrap_or_default();

            let matched_profile = catalog
                .get("profiles")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    arr.iter().find(|p| {
                        p.get("runtime").and_then(|r| r.as_str()) == Some(runtime.as_str())
                            && p.get("profile").and_then(|pr| pr.as_str()) == Some(profile.as_str())
                    })
                });
            let packages: Vec<String> = matched_profile
                .and_then(|p| p.get("packages").and_then(|v| v.as_array()))
                .map(|arr| {
                    arr.iter()
                        .filter_map(|pkg| {
                            pkg.get("source")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let available = matched_profile
                .and_then(|p| p.get("available").and_then(|v| v.as_bool()))
                .unwrap_or(false)
                || !packages.is_empty();

            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "runtime": runtime,
                    "profile": profile,
                    "action": action,
                    "available": available,
                    "packages": packages,
                    "installed": installed,
                    "plugins": catalog.get("plugins").cloned().unwrap_or_else(|| json!([])),
                    "pages": pages,
                    "reconciled": reconciled
                }))
                .unwrap()
            );
            Ok(0)
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
        Some("show") => {
            let id = args.get(1).ok_or("usage: nola sessions show <id>")?;
            let session = store
                .get_session(id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("session '{id}' not found"))?;
            let events = store
                .read_event_range(id, 0, 5000)
                .map_err(|e| e.to_string())?;
            let (messages, _) = crate::web_api::reconstruct_v2_messages(store, id);
            let usage = crate::web_api::session_usage_json(store, id);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "session": session,
                    "message_count": messages.len(),
                    "event_count": events.len(),
                    "messages": messages,
                    "events": events,
                    "usage": usage
                }))
                .unwrap()
            );
            Ok(0)
        }
        Some("part") => {
            let id = args.get(1).ok_or("usage: nola sessions part <id> <seq>")?;
            let seq_raw = args.get(2).ok_or("usage: nola sessions part <id> <seq>")?;
            let seq: i64 = seq_raw
                .parse()
                .map_err(|_| format!("invalid event sequence '{seq_raw}' (expected integer)"))?;
            let events = store
                .read_event_range(id, seq, 1)
                .map_err(|e| e.to_string())?;
            let part = events
                .into_iter()
                .find(|ev| ev.seq == seq)
                .ok_or_else(|| format!("session '{id}' has no event at seq {seq}"))?;
            println!("{}", serde_json::to_string_pretty(&part).unwrap());
            Ok(0)
        }
        Some("sources") => {
            let id = args.get(1).ok_or("usage: nola sessions sources <id>")?;
            let _session = store
                .get_session(id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("session '{id}' not found"))?;
            let knowledge = crate::web_api::load_conversation_knowledge(store, id);
            let usage = list_data_source_usage(store);
            let events = store.read_event_range(id, 0, 2000).unwrap_or_default();
            let referenced_tools: BTreeSet<String> = events
                .iter()
                .filter_map(|ev| {
                    ev.payload
                        .get("name")
                        .or_else(|| ev.payload.get("tool_name"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "session_id": id,
                    "knowledge": knowledge,
                    "data_source_usage": usage,
                    "referenced_tools": referenced_tools
                }))
                .unwrap()
            );
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
        Some(other) => Err(format!(
            "unknown sessions subcommand '{other}' (expected list|show|part|sources|delete|backup)"
        )),
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
        Some("create") => {
            let mut id: Option<String> = None;
            let mut title_parts: Vec<String> = Vec::new();
            let mut objective: Option<String> = None;
            let mut criteria: Vec<String> = Vec::new();
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--id" => {
                        i += 1;
                        id = Some(args.get(i).ok_or("--id requires a value")?.clone());
                    }
                    "--objective" => {
                        i += 1;
                        objective = Some(args.get(i).ok_or("--objective requires text")?.clone());
                    }
                    "--criteria" => {
                        i += 1;
                        criteria.push(args.get(i).ok_or("--criteria requires text")?.clone());
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown goal create option '{other}'"));
                    }
                    other => title_parts.push(other.to_string()),
                }
                i += 1;
            }
            let title = title_parts.join(" ").trim().to_string();
            if title.is_empty() {
                return Err(
                    "usage: nola goal create <title> [--objective <text>] [--id <id>] [--criteria <text>]"
                        .into(),
                );
            }
            let ts = now_ms();
            let rec = ExtendedGoalRecord {
                id: id.unwrap_or_else(new_id),
                title: title.clone(),
                objective: objective.unwrap_or(title),
                workspace_root: suite.workspace_root.display().to_string(),
                status: GoalStatus::Pending,
                progress_notes: Vec::new(),
                created_at_ms: ts,
                updated_at_ms: ts,
                lease_until_ms: None,
                retry_count: 0,
                next_retry_at_ms: None,
                needs_user_question: None,
                acceptance_criteria: criteria,
                accepted: false,
            };
            suite
                .upsert_record("goal", &rec.id, &rec)
                .map_err(|e| e.to_string())?;
            println!("{}", serde_json::to_string_pretty(&rec).unwrap());
            Ok(0)
        }
        Some("pause") => {
            let goal_id = args.get(1).ok_or("usage: nola goal pause <goal_id> [--note <text>]")?;
            let mut note: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--note" {
                    i += 1;
                    note = Some(args.get(i).ok_or("--note requires text")?.clone());
                }
                i += 1;
            }
            let mut rec = suite
                .get_record::<ExtendedGoalRecord>("goal", goal_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("goal '{goal_id}' not found"))?;
            rec.status = GoalStatus::Paused;
            rec.lease_until_ms = None;
            if let Some(n) = note {
                rec.progress_notes.push(format!("[paused] {n}"));
            }
            rec.updated_at_ms = now_ms();
            suite
                .upsert_record("goal", &rec.id, &rec)
                .map_err(|e| e.to_string())?;
            println!("{}", serde_json::to_string_pretty(&rec).unwrap());
            Ok(0)
        }
        Some("cancel") => {
            let goal_id = args.get(1).ok_or("usage: nola goal cancel <goal_id> [--note <text>]")?;
            let mut note: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--note" {
                    i += 1;
                    note = Some(args.get(i).ok_or("--note requires text")?.clone());
                }
                i += 1;
            }
            let mut rec = suite
                .get_record::<ExtendedGoalRecord>("goal", goal_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("goal '{goal_id}' not found"))?;
            rec.status = GoalStatus::Failed;
            rec.lease_until_ms = None;
            rec.next_retry_at_ms = None;
            rec.needs_user_question = None;
            rec.progress_notes.push(match note {
                Some(n) => format!("[cancelled] {n}"),
                None => "[cancelled] Cancelled by user".to_string(),
            });
            rec.updated_at_ms = now_ms();
            suite
                .upsert_record("goal", &rec.id, &rec)
                .map_err(|e| e.to_string())?;
            println!("{}", serde_json::to_string_pretty(&rec).unwrap());
            Ok(0)
        }
        Some("status") => {
            let goal_id = args.get(1).ok_or("usage: nola goal status <goal_id>")?;
            let rec = suite
                .get_record::<ExtendedGoalRecord>("goal", goal_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("goal '{goal_id}' not found"))?;
            println!("{}", serde_json::to_string_pretty(&rec).unwrap());
            Ok(0)
        }
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
        Some(other) => Err(format!(
            "unknown goal subcommand '{other}' (expected list|create|status|run|pause|cancel)"
        )),
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
        Some("verify-feed") => {
            let mut source_path: Option<PathBuf> = None;
            let mut feed_path: Option<PathBuf> = None;
            let mut target_filter: Option<String> = None;
            let mut positional: Vec<String> = Vec::new();
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--source" => {
                        i += 1;
                        source_path = Some(PathBuf::from(
                            args.get(i).ok_or("--source requires a file path")?,
                        ));
                    }
                    "--feed" => {
                        i += 1;
                        feed_path = Some(PathBuf::from(
                            args.get(i).ok_or("--feed requires a file path")?,
                        ));
                    }
                    "--target" => {
                        i += 1;
                        target_filter =
                            Some(args.get(i).ok_or("--target requires a target name")?.clone());
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown update verify-feed option '{other}'"));
                    }
                    other => positional.push(other.to_string()),
                }
                i += 1;
            }
            let src = source_path
                .or_else(|| positional.first().map(PathBuf::from))
                .ok_or(
                    "usage: nola update verify-feed --source <update-source.json> --feed <feed.json> [--target <target>]",
                )?;
            let feed = feed_path
                .or_else(|| positional.get(1).map(PathBuf::from))
                .ok_or(
                    "usage: nola update verify-feed --source <update-source.json> --feed <feed.json> [--target <target>]",
                )?;
            if target_filter.is_none() {
                target_filter = positional.get(2).cloned();
            }
            let source_bytes = fs::read(&src)
                .map_err(|e| format!("failed to read source '{}': {e}", src.display()))?;
            let feed_bytes = fs::read(&feed)
                .map_err(|e| format!("failed to read feed '{}': {e}", feed.display()))?;
            let verified = verify_signed_feed_bytes(&source_bytes, &feed_bytes)?;
            if let Some(ref target) = target_filter {
                let found = verified
                    .get("targets")
                    .and_then(|v| v.as_array())
                    .is_some_and(|arr| {
                        arr.iter()
                            .any(|t| t.get("target").and_then(|s| s.as_str()) == Some(target.as_str()))
                    });
                if !found {
                    return Err(format!(
                        "signed feed verified, but does not contain target '{target}'"
                    ));
                }
            }
            println!("{}", serde_json::to_string_pretty(&verified).unwrap());
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
                    sha256 = Some(
                        args.get(i)
                            .ok_or("--sha256 requires a 64-char hex digest")?
                            .clone(),
                    );
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
            "unknown update subcommand '{other}' (expected check|verify-feed|download)"
        )),
    }
}

fn handle_wiki_cmd(
    store: &Store,
    default_workspace: &Path,
    opts: &CliOptions,
    args: &[String],
) -> Result<u8, String> {
    let target_ws = args
        .first()
        .filter(|s| !s.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_else(|| default_workspace.to_path_buf());
    let arch = codebase_architecture(store, &target_ws, 50)?;
    let rendered = if opts.json_mode {
        serde_json::to_string_pretty(&arch).unwrap_or_else(|_| "{}".into())
    } else {
        let mut md = String::new();
        md.push_str(&format!(
            "# Codebase Architecture Wiki\n\n- **Workspace**: `{}`\n- **Indexed Files**: {}\n- **Symbols**: {}\n- **Edges**: {}\n\n## Modules\n\n| Module | Files | Symbols | Languages |\n| --- | ---: | ---: | --- |\n",
            arch["workspace_root"].as_str().unwrap_or(""),
            arch["total_files"].as_u64().unwrap_or(0),
            arch["total_symbols"].as_u64().unwrap_or(0),
            arch["total_edges"].as_u64().unwrap_or(0),
        ));
        if let Some(mods) = arch.get("modules").and_then(|v| v.as_array()) {
            for m in mods {
                let langs = m
                    .get("languages")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                md.push_str(&format!(
                    "| `{}` | {} | {} | {} |\n",
                    m["module"].as_str().unwrap_or("."),
                    m["file_count"].as_u64().unwrap_or(0),
                    m["symbol_count"].as_u64().unwrap_or(0),
                    langs
                ));
            }
        }
        if let Some(deps) = arch
            .get("cross_file_dependencies")
            .and_then(|v| v.as_array())
        {
            if !deps.is_empty() {
                md.push_str("\n## Cross-File Dependencies\n\n");
                for dep in deps.iter().take(25) {
                    md.push_str(&format!(
                        "- `{}` -> `{}` ({} refs)\n",
                        dep["from_file"].as_str().unwrap_or(""),
                        dep["to_file"].as_str().unwrap_or(""),
                        dep["reference_count"].as_u64().unwrap_or(0),
                    ));
                }
            }
        }
        md
    };
    if let Some(ref out_path) = opts.output_file {
        reject_if_symlink(out_path)?;
        if let Some(parent) = out_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        fs::write(out_path, &rendered)
            .map_err(|e| format!("failed to write wiki output '{}': {e}", out_path.display()))?;
    }
    println!("{rendered}");
    Ok(0)
}

fn print_help() {
    println!(
        "Nola v2 (Lean 8-Crate Full-Capability Architecture)\n\n\
         Usage:\n  \
           nola [options] [prompt]\n  \
           nola run [--json] [--host-request-stdin] [--session <id>] [--continue] [--project <dir>] [-m <model>] [--endpoint <url>] [--reasoning-effort <lvl>] [--profile <name>] [--no-tools] [--output <file>] [--lang <locale>] [--debug] [--allow-dir <dir>] [--no-skills] [--github-skill <id>]\n  \
           nola init [--force]\n  \
           nola config [check [path] | apply [path] [--dry-run] | example | edit [path]]\n  \
           nola model [list | connections | discover [conn] [--save] | configure ... | import <path> | probe [conn] | apply-config [path] | acquire <engine-status|setup-engine|hardware|recommend|confirm|remove>]\n  \
           nola mcp [list | add <id> --transport <stdio|http|sse> ... | remove <id> | auth <id> [--status]]\n  \
           nola skills [list | find <query> | install <owner/repo|url> | enable <name> | disable <name>]\n  \
           nola memory [list | create | revise <id> | delete <id>]\n  \
           nola plugin [list | install <spec> | enable <id> | disable <id> | remove <id> | desktop-pages [id] | desktop-library | desktop-manage [<rt> <prof>] <action> [src] | pi | dsh]\n  \
           nola sessions [list | show <id> | part <id> <seq> | sources <id> | delete <id> | backup <dest>]\n  \
           nola goal [list | create <title> | status <id> | run <id> | pause <id> | cancel <id>]\n  \
           nola room [list | send <room_id> <message> [--to <participant_id>]]\n  \
           nola scheduler [list | tick]\n  \
           nola update [check [manifest_url] | verify-feed --source <path> --feed <path> | download <url> <dest_dir> --sha256 <hex>]\n  \
           nola wiki [workspace] [--json] [--output <file>]\n  \
           nola modules <kind|skills>\n  \
           nola version [--json]\n  \
           nola web [--host 127.0.0.1] [--port 5001] [--web-root <dir>] [--json]\n  \
           nola server [--socket <path>] [--parent-pid <pid>]\n  \
           nola tui [--db <path>] [-m <model>] [--permission-profile <profile>] [-y] [--resume]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_route_with_overrides_prioritizes_explicit_overrides_and_reasoning_effort() {
        let mut cfg = AppliedConfig::default();
        cfg.llm.connections.push(ConnectionConfig {
            id: "default-conn".into(),
            endpoint: "https://api.openai.com/v1".into(),
            protocol: Some("responses".into()),
            provider: None,
            api_key: Some("sk-default".into()),
            api_key_env: None,
            models: vec![ModelEntry {
                id: "gpt-4.1".into(),
                context_window: None,
                max_output_tokens: None,
                temperature: None,
                top_p: None,
                reasoning_effort: None,
                capabilities: None,
            }],
        });
        cfg.llm.default_model = Some(DefaultModelSelection {
            connection: "default-conn".into(),
            model: "gpt-4.1".into(),
        });

        let opts = CliOptions::default();
        let overrides = HostRunOverrides {
            session_id: None,
            resume: None,
            model_id: Some("local-qwen".into()),
            endpoint: Some("http://127.0.0.1:8080/v1".into()),
            api_key: Some("sk-override".into()),
            protocol: Some("openai_chat".into()),
            permission_profile: None,
            reasoning_effort: Some("high".into()),
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            max_iterations: None,
            context_window_tokens: None,
        };
        let route = resolve_route_with_overrides(&cfg, &opts, Some(&overrides)).unwrap();
        assert_eq!(route.model, "local-qwen");
        assert_eq!(route.endpoint, "http://127.0.0.1:8080/v1");
        assert_eq!(route.api_key, "sk-override");
        assert_eq!(route.protocol, ModelProtocol::OpenAiChat);
        assert_eq!(route.reasoning_effort.as_deref(), Some("high"));

        // Partial override still applies on top of default model
        let partial = HostRunOverrides {
            session_id: None,
            resume: None,
            model_id: Some("gpt-5".into()),
            endpoint: None,
            api_key: None,
            protocol: Some("gemini".into()),
            permission_profile: None,
            reasoning_effort: Some("medium".into()),
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            max_iterations: None,
            context_window_tokens: None,
        };
        let route2 = resolve_route_with_overrides(&cfg, &opts, Some(&partial)).unwrap();
        assert_eq!(route2.model, "gpt-5");
        assert_eq!(route2.endpoint, "https://api.openai.com/v1");
        assert_eq!(route2.protocol, ModelProtocol::Gemini);
        assert_eq!(route2.reasoning_effort.as_deref(), Some("medium"));

        // Without overrides, falls back to cfg.llm.default_model
        let route3 = resolve_route_with_overrides(&cfg, &opts, None).unwrap();
        assert_eq!(route3.model, "gpt-4.1");
        assert_eq!(route3.endpoint, "https://api.openai.com/v1");
    }

    #[test]
    fn execution_backends_includes_nola_and_external_entries() {
        let val = list_execution_backends_json();
        let arr = val.as_array().unwrap();
        assert_eq!(arr[0]["id"], "nola");
        assert_eq!(arr[0]["availability"], "available");
        assert_eq!(arr[0]["capabilities"]["supports_permission_bridge"], true);
        let ids: Vec<&str> = arr.iter().filter_map(|b| b["id"].as_str()).collect();
        assert!(ids.contains(&"codex"));
        assert!(ids.contains(&"claude"));
        assert!(ids.contains(&"opencode"));
    }

    #[test]
    fn parse_args_supports_extended_cli_flags() {
        let args: Vec<String> = [
            "--profile",
            "allow_all",
            "--continue",
            "--reasoning-effort",
            "high",
            "--no-tools",
            "--output",
            "/tmp/out.txt",
            "--debug",
            "--lang",
            "zh-CN",
            "hello",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let opts = parse_args(&args).unwrap();
        assert_eq!(opts.profile.as_deref(), Some("allow_all"));
        assert!(opts.continue_last);
        assert!(opts.resume);
        assert_eq!(opts.reasoning_effort.as_deref(), Some("high"));
        assert!(opts.no_tools);
        assert_eq!(opts.output_file, Some(PathBuf::from("/tmp/out.txt")));
        assert!(opts.debug);
        assert_eq!(opts.lang.as_deref(), Some("zh-CN"));
        assert_eq!(resolve_profile(&opts), PermissionProfile::AllowAll);
    }

    #[test]
    fn host_stdin_parsers_handle_v2_and_macos_legacy_shapes() {
        assert_eq!(
            parse_permission_allow(&json!({
                "request_id": "r1",
                "resolution": { "type": "authorization", "decision": "allow_conversation" }
            })),
            Some(true)
        );
        assert_eq!(
            parse_permission_allow(&json!({ "decision": "deny" })),
            Some(false)
        );
        assert_eq!(
            parse_user_input_answer(&json!({ "answer": "yes please" }), Some("q1")),
            Some(Some("yes please".to_string()))
        );
        assert_eq!(
            parse_user_input_answer(&json!({ "cancelled": true }), Some("q1")),
            Some(None)
        );
        assert_eq!(
            extract_reasoning_effort_from_req(&json!({ "reasoning": { "effort": "high" } })),
            Some("high".to_string())
        );
        assert_eq!(
            extract_reasoning_effort_from_req(&json!({ "reasoning_selection": { "kind": "effort", "value": "low" } })),
            Some("low".to_string())
        );
    }

    #[test]
    fn desktop_manage_supports_macos_runtime_profile_action_order_and_shorthand() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));

        let plugin_src = dir.path().join("demo-plugin");
        fs::create_dir_all(&plugin_src).unwrap();
        fs::write(
            plugin_src.join("nola-plugin.json"),
            r#"{"id":"demo-plugin","name":"Demo Plugin","version":"1.0.0","runtime":"dsh","profile":"web"}"#,
        )
        .unwrap();

        // macOS 4-arg order: desktop-manage <runtime> <profile> <action> <source>
        let rc = handle_plugin_cmd(
            &store,
            &suite,
            &[
                "desktop-manage".into(),
                "dsh".into(),
                "web".into(),
                "install".into(),
                plugin_src.display().to_string(),
            ],
        )
        .unwrap();
        assert_eq!(rc, 0);
        let rec = suite
            .get_record::<PluginRecord>(KIND_PLUGIN, "demo-plugin")
            .unwrap()
            .expect("plugin should be installed");
        assert!(rec.enabled);

        // Disable via macOS order
        let rc = handle_plugin_cmd(
            &store,
            &suite,
            &[
                "desktop-manage".into(),
                "dsh".into(),
                "web".into(),
                "disable".into(),
                "demo-plugin".into(),
            ],
        )
        .unwrap();
        assert_eq!(rc, 0);
        let rec2 = suite
            .get_record::<PluginRecord>(KIND_PLUGIN, "demo-plugin")
            .unwrap()
            .unwrap();
        assert!(!rec2.enabled);

        // Enable via CLI shorthand
        let rc = handle_plugin_cmd(
            &store,
            &suite,
            &[
                "desktop-manage".into(),
                "enable".into(),
                "demo-plugin".into(),
            ],
        )
        .unwrap();
        assert_eq!(rc, 0);
        assert!(
            suite
                .get_record::<PluginRecord>(KIND_PLUGIN, "demo-plugin")
                .unwrap()
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn goal_lifecycle_and_sessions_and_wiki_subcommands_work() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("lib.rs"), "pub fn greet() -> &'static str { \"hi\" }\n").unwrap();
        let store = Store::open_in_memory().unwrap();
        bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));
        let opts = CliOptions::default();

        // Goal create -> status -> pause -> cancel
        handle_goal_cmd(
            &store,
            &suite,
            &opts,
            &[
                "create".into(),
                "Ship".into(),
                "v2".into(),
                "--id".into(),
                "g-1".into(),
                "--objective".into(),
                "Finish CLI".into(),
            ],
        )
        .unwrap();
        handle_goal_cmd(&store, &suite, &opts, &["status".into(), "g-1".into()]).unwrap();
        handle_goal_cmd(
            &store,
            &suite,
            &opts,
            &["pause".into(), "g-1".into(), "--note".into(), "hold".into()],
        )
        .unwrap();
        let paused = suite
            .get_record::<ExtendedGoalRecord>("goal", "g-1")
            .unwrap()
            .unwrap();
        assert_eq!(paused.status, GoalStatus::Paused);
        handle_goal_cmd(&store, &suite, &opts, &["cancel".into(), "g-1".into()]).unwrap();
        let cancelled = suite
            .get_record::<ExtendedGoalRecord>("goal", "g-1")
            .unwrap()
            .unwrap();
        assert_eq!(cancelled.status, GoalStatus::Failed);

        // Sessions show / part / sources
        let ts = now_ms();
        store
            .upsert_session(&nola_types::Session {
                id: "s-1".into(),
                title: "Demo".into(),
                workspace_root: dir.path().display().to_string(),
                model_connection: None,
                model_id: None,
                created_at_ms: ts,
                updated_at_ms: ts,
            })
            .unwrap();
        let ev = store
            .append_event("s-1", "user_message", json!({"content": "hello"}), ts)
            .unwrap();
        handle_sessions_cmd(&store, &["show".into(), "s-1".into()], true).unwrap();
        handle_sessions_cmd(
            &store,
            &["part".into(), "s-1".into(), ev.seq.to_string()],
            true,
        )
        .unwrap();
        handle_sessions_cmd(&store, &["sources".into(), "s-1".into()], true).unwrap();

        // Wiki generation
        let out_md = dir.path().join("WIKI.md");
        let mut wiki_opts = CliOptions::default();
        wiki_opts.output_file = Some(out_md.clone());
        handle_wiki_cmd(&store, dir.path(), &wiki_opts, &[]).unwrap();
        let wiki_text = fs::read_to_string(&out_md).unwrap();
        assert!(wiki_text.contains("Codebase Architecture Wiki"));

        // Model acquire hardware / engine-status / recommend
        handle_model_acquire_cmd(&store, &["hardware".into()]).unwrap();
        handle_model_acquire_cmd(&store, &["engine-status".into(), "llama.cpp".into()]).unwrap();
        handle_model_acquire_cmd(
            &store,
            &["recommend".into(), "llama.cpp".into(), "huggingface".into()],
        )
        .unwrap();
    }

    #[test]
    fn siwc_snapshot_probe_metadata_and_a2ui_surface_frames_match_mac_contracts() {
        let store = Store::open_in_memory().unwrap();

        // 1. siwc_account_snapshot emits state="ready" and credential_ref="siwc:<id>"
        let snap = siwc_account_snapshot("acct-42", "Work", 3, true);
        assert_eq!(snap["account"]["state"], "ready");
        assert_eq!(snap["account"]["credential_ref"], "siwc:acct-42");
        let disconnected = siwc_account_snapshot("acct-42", "Work", 4, false);
        assert_eq!(disconnected["account"]["state"], "disconnected");
        assert!(disconnected["account"]["credential_ref"].is_null());

        // 2. probe_prepare / probe metadata resolves connection & account from module_records + kv
        store
            .put_module_record(
                "model_connection",
                "conn-store-1",
                &json!({
                    "id": "conn-store-1",
                    "display_name": "Stored OpenAI",
                    "base_url": "https://api.stored.example/v1",
                    "account_id": "acct-99"
                }),
            )
            .unwrap();
        store
            .put_module_record(
                "model_account",
                "acct-99",
                &json!({
                    "id": "acct-99",
                    "revision": 7,
                    "credential_ref": "siwc:acct-99"
                }),
            )
            .unwrap();
        let cfg = load_active_config(&store);
        let meta = resolve_probe_connection_metadata(&store, &cfg, "conn-store-1", &json!({}));
        assert_eq!(meta.base_url, "https://api.stored.example/v1");
        assert_eq!(meta.display_name, "Stored OpenAI");
        assert_eq!(meta.account_id.as_deref(), Some("acct-99"));
        assert_eq!(meta.account_revision, Some(7));

        // 3. collect_a2ui_surface_frames emits ui_surface frames for rendered surfaces
        let owner = nola_modules::a2ui::UiSurfaceOwner {
            authority_id: "local-host".into(),
            conversation_id: "sess-a2ui-1".into(),
            participant_id: "assistant".into(),
            context_id: "conversation".into(),
            originating_run_id: "run-a2ui-1".into(),
            message_id: "msg-a2ui-1".into(),
            part_id: "part-a2ui-1".into(),
        };
        let (root_id, nodes) = nola_modules::a2ui::build_nodes_from_spec(&json!({
            "components": [
                {"id": "t1", "component": "Text", "text": "Hello A2UI"}
            ]
        }))
        .unwrap();
        nola_modules::a2ui::upsert_ui_surface(
            &store,
            owner,
            "card-1",
            "Summary Card",
            root_id,
            nodes,
            json!({}),
        )
        .unwrap();
        let frames = collect_a2ui_surface_frames(&store, "sess-a2ui-1", "sess-a2ui-1");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["type"], "ui_surface");
        assert_eq!(frames[0]["session_id"], "sess-a2ui-1");
        assert_eq!(frames[0]["snapshot"]["identity"]["surface_id"], "card-1");
    }
}
