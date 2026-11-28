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
    apply_config_file, bind_store_vault_path, default_db_path, default_home_dir, default_vault_path,
    discover_config_path, frozen_system_blocks_for_run, load_active_config,
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
            | "ask"
            | "chat"
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
        ("claude_code", "Claude Code", "claude"),
        ("opencode", "OpenCode", "opencode"),
    ] {
        let found = find_binary_on_path(bin);
        let available = found.is_some();
        let diagnostic = if available {
            Value::Null
        } else {
            json!(format!("{bin} binary not found on PATH"))
        };
        let (models, default_model_id) = if available && id == "codex" {
            (
                json!([
                    {
                        "id": "gpt-5.4",
                        "display_name": "GPT-5.4",
                        "is_default": true,
                        "supports_reasoning": true
                    },
                    {
                        "id": "gpt-5.3-codex",
                        "display_name": "GPT-5.3 Codex",
                        "is_default": false,
                        "supports_reasoning": true
                    }
                ]),
                json!("gpt-5.4"),
            )
        } else {
            (json!([]), Value::Null)
        };
        backends.push(json!({
            "id": id,
            "display_name": display_name,
            "availability": if available { "available" } else { "unavailable" },
            "available": available,
            "version": Value::Null,
            "diagnostic": diagnostic,
            "models": models,
            "default_model_id": default_model_id,
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
    let is_server_cmd = opts.positional.first().map(String::as_str) == Some("server");
    let archive_incompatible_dir: Option<PathBuf> = if is_server_cmd {
        let rest = &opts.positional[1..];
        let mut found_dir = None;
        let mut idx = 0;
        while idx < rest.len() {
            if rest[idx] == "--archive-incompatible-store" {
                if let Some(dir) = rest.get(idx + 1) {
                    found_dir = Some(PathBuf::from(dir));
                }
                idx += 2;
            } else if let Some(val) = rest[idx].strip_prefix("--archive-incompatible-store=") {
                found_dir = Some(PathBuf::from(val));
                idx += 1;
            } else {
                idx += 1;
            }
        }
        found_dir
    } else {
        None
    };
    let store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(first_err) => {
            let err_text = first_err.to_string();
            let is_incompatible = err_text.contains("unsupported database schema")
                || err_text.contains("user_version")
                || err_text.contains("incompatible")
                || err_text.contains("file is not a database");
            if is_server_cmd && is_incompatible {
                if let Some(archive_dir) = archive_incompatible_dir {
                    std::fs::create_dir_all(&archive_dir).map_err(|e| {
                        format!(
                            "failed to create archive directory '{}': {e}",
                            archive_dir.display()
                        )
                    })?;
                    let base_name = db_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("nola.db");
                    for suffix in ["", "-wal", "-shm"] {
                        let src = if suffix.is_empty() {
                            db_path.clone()
                        } else {
                            PathBuf::from(format!("{}{}", db_path.display(), suffix))
                        };
                        if src.exists() {
                            let dst = archive_dir.join(format!("{base_name}{suffix}"));
                            std::fs::rename(&src, &dst)
                                .or_else(|_| {
                                    std::fs::copy(&src, &dst)
                                        .and_then(|_| std::fs::remove_file(&src))
                                })
                                .map_err(|e| {
                                    format!(
                                        "failed to archive incompatible store '{}' to '{}': {e}",
                                        src.display(),
                                        dst.display()
                                    )
                                })?;
                        }
                    }
                    Store::open(&db_path)
                        .map_err(|e| format!("failed to open fresh store after archive: {e}"))?
                } else {
                    let failure = json!({
                        "type": "nola.service.startup_failure",
                        "protocol_version": 1,
                        "code": "incompatible_canonical_store",
                        "detail": err_text
                    });
                    eprintln!("{failure}");
                    return Ok(1);
                }
            } else {
                return Err(format!("failed to open store: {first_err}"));
            }
        }
    };
    bind_store_vault_path(&store, &default_vault_path(&db_path))?;
    let home = Some(default_home_dir());
    let suite = Arc::new(ModulesSuite::new(store.clone(), workspace.clone(), home.clone()));

    if let Some(cmd) = opts.positional.first().map(String::as_str) {
        match cmd {
            "init" => return handle_init_cmd(&workspace, &opts.positional[1..]),
            "config" => return handle_config_cmd(&store, &workspace, &opts.positional[1..]),
            "model" => return handle_model_cmd(&store, &workspace, &opts.positional[1..]),
            "model-import-bridge" => {
                return handle_model_import_bridge_cmd(&store, &suite, &opts.positional[1..])
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
                return handle_plugin_cmd(&store, &suite, &db_path, &opts.positional[1..])
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

#[derive(Default)]
struct HostRunOverrides {
    session_id: Option<String>,
    submission_id: Option<String>,
    user_message_id: Option<String>,
    assistant_message_id: Option<String>,
    resume: Option<bool>,
    connection_id: Option<String>,
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
    image_input: Option<bool>,
    reasoning_supported: Option<bool>,
    tool_calls_supported: Option<bool>,
    sampling_parameters: Option<bool>,
    stream_reasoning: Option<bool>,
    selected_experts: Vec<Value>,
    expert_mentions: Vec<Value>,
    context_components: Vec<Value>,
    attachments: Vec<Value>,
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
            .or_else(|| req.get("type"))
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
            if sid.is_empty() {
                return Err("cancel requires a session_id".into());
            }
            let key = match req.get("expected_run_id").and_then(Value::as_str) {
                Some(run) => format!("cancel_requested_run:{sid}:{run}"),
                None => format!("cancel_requested:{sid}"),
            };
            store
                .set_kv(
                    &key,
                    &json!({"cancel_requested": true, "requested_at_ms": now_ms()}),
                )
                .map_err(|e| e.to_string())?;
            println!(
                "{}",
                json!({"ok": true, "cancel_requested": true, "cancelled": false, "session_id": sid})
            );
            let _ = io::stdout().flush();
            return Ok(0);
        }

        if req
            .get("operation")
            .or_else(|| req.get("type"))
            .and_then(|v| v.as_str())
            .is_some_and(|op| op.eq_ignore_ascii_case("goal_control"))
        {
            let receipt = handle_host_goal_control_request(store, suite, &req)?;
            println!("{receipt}");
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

        let submission_id = req
            .get("submission_id")
            .or_else(|| req.get("run_id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string);
        let user_message_id = req
            .get("user_message_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string);
        let assistant_message_id = req
            .get("assistant_message_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string);
        let connection_id = req
            .get("connection_id")
            .or_else(|| req.get("connectionId"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .or_else(|| {
                req.get("model_profile_id")
                    .and_then(|v| v.as_str())
                    .and_then(|pid| pid.strip_prefix("conn:"))
                    .and_then(|rest| rest.split(':').next())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
            });
        let selected_experts = req
            .get("selected_experts")
            .or_else(|| req.get("selectedExperts"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let expert_mentions = req
            .get("expert_mentions")
            .or_else(|| req.get("expertMentions"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let context_components = req
            .get("context_components")
            .or_else(|| req.get("contextComponents"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let attachments = req
            .get("attachments")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mc = req
            .get("model_capabilities")
            .or_else(|| req.get("modelCapabilities"));
        let image_input = mc
            .and_then(|c| {
                c.get("vision_input")
                    .or_else(|| c.get("image_input"))
                    .or_else(|| c.get("imageInput"))
                    .and_then(|v| v.as_bool())
            })
            .or_else(|| req.get("vision_input").and_then(|v| v.as_bool()));
        let reasoning_supported = mc
            .and_then(|c| c.get("reasoning").and_then(|v| v.as_bool()));
        let tool_calls_supported = mc.and_then(|c| {
            c.get("native_tool_calls")
                .or_else(|| c.get("tool_calls"))
                .or_else(|| c.get("toolCalls"))
                .and_then(|v| v.as_bool())
        });
        let sampling_parameters = mc.and_then(|c| {
            c.get("sampling_parameters")
                .or_else(|| c.get("samplingParameters"))
                .and_then(|v| v.as_bool())
        });
        let stream_reasoning = mc.and_then(|c| {
            c.get("stream_reasoning")
                .or_else(|| c.get("streamReasoning"))
                .and_then(|v| v.as_bool())
        });
        let overrides = HostRunOverrides {
            session_id,
            submission_id,
            user_message_id,
            assistant_message_id,
            resume,
            connection_id,
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
            image_input,
            reasoning_supported,
            tool_calls_supported,
            sampling_parameters,
            stream_reasoning,
            selected_experts,
            expert_mentions,
            context_components,
            attachments,
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
        if let Some(img) = ov.image_input {
            route.capabilities.image_input = img;
        }
        if let Some(rs) = ov.reasoning_supported {
            route.capabilities.reasoning = rs;
        }
        if let Some(tc) = ov.tool_calls_supported {
            route.capabilities.tool_calls = tc;
        }
        if let Some(sp) = ov.sampling_parameters {
            route.capabilities.sampling_parameters = sp;
            if !sp {
                route.temperature = None;
                route.top_p = None;
            }
        }
        if let Some(sr) = ov.stream_reasoning {
            route.capabilities.stream_reasoning = sr;
        }
    }
    // Enrich missing capabilities (e.g. image_input / reasoning) from discovered model_connection catalog
    if !route.capabilities.image_input {
        if let Ok(conns) = store.list_module_records::<Value>("model_connection") {
            let ep_norm = route.endpoint.trim_end_matches('/');
            'conn_scan: for c in &conns {
                let c_ep = c
                    .get("base_url")
                    .or_else(|| c.get("endpoint"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim_end_matches('/');
                if !ep_norm.is_empty() && c_ep != ep_norm {
                    continue;
                }
                if let Some(models) = c.pointer("/catalog/models").and_then(|v| v.as_array()) {
                    for m in models {
                        let mid = m
                            .get("model_id")
                            .or_else(|| m.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if mid == route.model {
                            let explicit_img = overrides.as_ref().and_then(|o| o.image_input);
                            if explicit_img.is_none() {
                                let cat_img = m
                                    .pointer("/execution_capabilities/imageInput")
                                    .or_else(|| m.pointer("/execution_capabilities/image_input"))
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                                    || m.get("modalities")
                                        .and_then(|v| v.as_array())
                                        .is_some_and(|arr| {
                                            arr.iter().any(|x| x.as_str() == Some("image"))
                                        });
                                if cat_img {
                                    route.capabilities.image_input = true;
                                }
                            }
                            break 'conn_scan;
                        }
                    }
                }
            }
        }
    }
    if nola_llm::normalized_wire_reasoning_effort(route.reasoning_effort.as_deref()).is_some() {
        route.capabilities.reasoning = true;
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
    let mut policy = EffectivePolicy::load(
        workspace,
        home.as_deref(),
        workspace_trusted,
        Some(profile),
        network_allowed,
    );
    if overrides.as_ref().is_some_and(|ov| {
        ov.context_components.iter().any(|c| {
            c.get("kind")
                .and_then(|v| v.as_str())
                .is_some_and(|k| k.eq_ignore_ascii_case("plan_mode"))
                || c.get("id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|id| id.contains("plan-mode") || id.contains("plan_mode"))
        })
    }) {
        policy = policy.with_plan_mode(true);
    }

    let mut registry = if opts.no_tools {
        ToolRegistry::default()
    } else {
        let mut r = ToolRegistry::with_builtin_tools();
        suite.register_tools(&mut r);
        r
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&cancel));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&cancel));
    let hooks = HookRunner::load(workspace, home.as_deref(), workspace_trusted);

    let mut session_id = overrides
        .as_ref()
        .and_then(|o| o.session_id.clone())
        .or_else(|| opts.session_id.clone());
    let mut resume = overrides
        .as_ref()
        .and_then(|o| o.resume)
        .unwrap_or(opts.resume || opts.continue_last || opts.session_id.is_some());
    if session_id.is_none() && opts.continue_last {
        if let Ok(recent) = store.list_sessions(1) {
            if let Some(last) = recent.into_iter().next() {
                session_id = Some(last.id);
                resume = true;
            }
        }
    }
    let effective_session_id = session_id.unwrap_or_else(new_id);

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

    if !opts.no_tools && (opts.host_request_stdin || opts.json_mode) {
        let host_id = crate::web_api::ensure_host_id(store);
        register_a2ui_tools(suite, &mut registry, &host_id, &effective_session_id);
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
        let store_reader = store.clone();
        let suite_reader = Arc::clone(suite);
        let stdout_lock_reader = Arc::clone(&stdout_lock);
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
                    "goal_control" => {
                        let action = msg
                            .get("action")
                            .or_else(|| msg.pointer("/payload/action"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_ascii_lowercase();
                        if matches!(action.as_str(), "pause" | "cancel") {
                            cancel_reader.store(true, Ordering::SeqCst);
                        }
                        let resp = match handle_host_goal_control_request(
                            &store_reader,
                            &suite_reader,
                            &msg,
                        ) {
                            Ok(mut v) => {
                                if let Some(obj) = v.as_object_mut() {
                                    obj.insert("type".into(), json!("goal_control_result"));
                                    if let Some(ref rid) = req_id {
                                        obj.insert("request_id".into(), json!(rid));
                                    }
                                }
                                v
                            }
                            Err(err) => json!({
                                "type": "goal_control_result",
                                "ok": false,
                                "request_id": req_id,
                                "error": err
                            }),
                        };
                        let _g = stdout_lock_reader.lock().unwrap();
                        emit_compact_json_line(&resp);
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
        let sid_ap = effective_session_id.clone();
        let ws_ap = workspace.to_path_buf();
        let approval: ApprovalCallback = Arc::new(move |spec, args, reason| {
            if closed_ap.load(Ordering::SeqCst) || cancel_ap.load(Ordering::SeqCst) {
                return false;
            }
            let req_id = new_id();
            let resource = nola_engine::policy::primary_resource_pattern(
                &spec.schema.name,
                args,
                &ws_ap,
            )
            .unwrap_or_else(|| spec.schema.name.clone());
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
                        "resource": resource,
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
        let sid_ui = effective_session_id.clone();
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

    let mut frozen_blocks = frozen_system_blocks_for_run(workspace, &cfg, !opts.no_tools);
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
    // Inject saved ExpertPeer directory and resolve any @expert mentions or selected_experts
    // so Nola immediately recognizes user-created experts without probing SQLite or files.
    append_expert_and_host_context_blocks(store, prompt, overrides.as_ref(), &mut frozen_blocks);
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

    let route_endpoint = route.endpoint.clone();
    let run_cfg = AgentRunConfig {
        session_id: Some(effective_session_id.clone()),
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
        cancel: Arc::clone(&cancel),
        auto_recall_memory: !is_eval,
        inbox: None,
    };

    let out_lock_ev = Arc::clone(&stdout_lock);
    let mut live_session_id = effective_session_id.clone();
    let canonical_mac_host = opts.host_request_stdin
        && overrides
            .as_ref()
            .is_some_and(|o| o.submission_id.is_some() && o.assistant_message_id.is_some());

    if canonical_mac_host {
        let ov = overrides.as_ref().unwrap();
        let conv_id = effective_session_id.clone();
        let run_id = ov.submission_id.clone().unwrap();
        let assistant_msg_id = ov.assistant_message_id.clone().unwrap();
        let source_host_id = crate::web_api::ensure_host_id(store);
        let exact_key = format!("exact_run_invocation:{conv_id}:{run_id}");
        let host_invocation_snapshot = json!({
            "primary": {
                "model_id": run_cfg.route.model,
                "endpoint": run_cfg.route.endpoint,
                "protocol": format!("{:?}", run_cfg.route.protocol),
                "image_input": run_cfg.route.capabilities.image_input,
                "reasoning": run_cfg.route.capabilities.reasoning,
                "tool_calls": run_cfg.route.capabilities.tool_calls
            },
            "generation": {
                "reasoning_effort": run_cfg.route.reasoning_effort,
                "temperature": run_cfg.route.temperature,
                "max_output_tokens": run_cfg.route.max_output_tokens
            },
            "permission_mode": match run_cfg.policy.profile {
                PermissionProfile::AllowAll => "allow_all",
                PermissionProfile::Cautious => "cautious",
            },
            "plan_mode": run_cfg.policy.plan_mode
        });
        let core_session_id = match store.get_kv(&exact_key).ok().flatten() {
            Some(mut inv) => {
                let csid = inv
                    .get("core_session_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| conv_id.clone());
                let needs_host_inv = inv
                    .get("host_invocation")
                    .and_then(|v| v.as_object())
                    .is_none_or(|o| o.is_empty());
                if inv.get("checkpoint_sha256").is_none_or(|v| v.is_null()) || needs_host_inv {
                    if let Some(obj) = inv.as_object_mut() {
                        obj.insert("core_session_id".into(), json!(csid));
                        if needs_host_inv {
                            obj.insert("host_invocation".into(), host_invocation_snapshot.clone());
                        }
                        if obj.get("checkpoint_sha256").is_none_or(|v| v.is_null()) {
                            obj.insert("checkpoint_revision".into(), json!(1i64));
                            obj.insert("checkpoint_event_cursor".into(), json!(0i64));
                            obj.insert(
                                "checkpoint_sha256".into(),
                                json!(nola_types::sha256_hex(
                                    format!("checkpoint:{conv_id}:{run_id}").as_bytes()
                                )),
                            );
                        }
                    }
                    let _ = store.set_kv(&exact_key, &inv);
                }
                csid
            }
            None => {
                let digest = nola_types::sha256_hex(
                    format!("{conv_id}:{run_id}:{assistant_msg_id}").as_bytes(),
                );
                let ckpt_sha = nola_types::sha256_hex(
                    format!("checkpoint:{conv_id}:{run_id}").as_bytes(),
                );
                let inv = json!({
                    "conversation_id": conv_id,
                    "run_id": run_id,
                    "assistant_message_id": assistant_msg_id,
                    "invocation_config_digest": digest,
                    "status": "running",
                    "host_invocation": host_invocation_snapshot,
                    "core_session_id": conv_id,
                    "checkpoint_revision": 1i64,
                    "checkpoint_event_cursor": 0i64,
                    "checkpoint_sha256": ckpt_sha,
                    "created_at_ms": now_ms()
                });
                let _ = store.set_kv(&exact_key, &inv);
                conv_id.clone()
            }
        };
        let trace_id = format!("trace-{run_id}");
        let trace_seq = std::sync::atomic::AtomicU64::new(0);
        let emit_line = |val: &Value| {
            if let Ok(line) = serde_json::to_string(val) {
                let _g = out_lock_ev.lock().unwrap();
                println!("{line}");
                let _ = io::stdout().flush();
            }
        };
        // Immediately notify MacLocalAgentRuntime that this canonical Run has been admitted
        // so mid-stream cancellation (Stop button) and session recovery are live from token 0.
        emit_line(&json!({
            "type": "submission_admitted",
            "conversation_id": conv_id,
            "run_id": run_id,
            "assistant_message_id": assistant_msg_id
        }));
        let emit_trace = |kind: &str,
                          status: &str,
                          span_id: Option<&str>,
                          payload: Value|
         -> u64 {
            let seq = trace_seq.fetch_add(1, Ordering::SeqCst) + 1;
            let frame = json!({
                "type": kind,
                "canonical_run_id": run_id,
                "trace": {
                    "schema_version": 1,
                    "trace_id": trace_id,
                    "run_id": run_id,
                    "conversation_id": conv_id,
                    "message_id": assistant_msg_id,
                    "turn_id": run_id,
                    "event_id": format!("{run_id}:ev:{seq}"),
                    "sequence": seq,
                    "emitted_at": crate::web_api::ms_to_rfc3339(now_ms()),
                    "kind": kind,
                    "span_id": span_id,
                    "parent_span_id": Value::Null,
                    "status": status,
                    "visibility": "public",
                    "payload": payload
                }
            });
            emit_line(&frame);
            seq
        };
        let mut acc_text = String::new();
        let mut acc_reasoning = String::new();
        let mut recorded_tools: Vec<Value> = Vec::new();
        let mut run_state = json!({
            "conversation_id": conv_id,
            "run_id": run_id,
            "assistant_message_id": assistant_msg_id,
            "session_id": core_session_id,
            "created_at_ms": now_ms(),
            "journal_identity": crate::daemon::journal_identity_value(
                store,
                &conv_id,
                &run_id,
                &assistant_msg_id,
            ),
            "journal_seq": 0u64,
            "sequence": 1u64,
            "status": "running"
        });
        if !prompt.trim().is_empty() {
            let user_msg_id = ov
                .user_message_id
                .clone()
                .unwrap_or_else(|| format!("{run_id}:user"));
            let _ = store.append_event(
                &conv_id,
                "chat_user",
                json!({
                    "id": user_msg_id,
                    "message_id": user_msg_id,
                    "content": prompt
                }),
                now_ms(),
            );
        }
        let effective_prompt = format_turn_prompt_with_attachments(prompt, overrides.as_ref());
        let mut next_part_ordinal: u64 = 0;
        let mut alloc_ordinal = || {
            let v = next_part_ordinal;
            next_part_ordinal += 1;
            v
        };
        let mut reasoning_seq: usize = 0;
        let mut text_seq: usize = 0;
        let mut active_reasoning_part: Option<(String, u64, String)> = None;
        let mut active_text_part: Option<(String, u64, String)> = None;
        let mut last_published_text_part_id: Option<String> = None;
        let mut last_reasoning_publish =
            std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut last_text_publish =
            std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut tool_part_meta: std::collections::HashMap<String, (u64, String, Value, String)> =
            std::collections::HashMap::new();
        let mut bash_ws_snapshots: std::collections::HashMap<String, std::collections::HashMap<PathBuf, (u64, std::time::SystemTime)>> =
            std::collections::HashMap::new();
        let mut generated_deliverables: Vec<(PathBuf, String)> = Vec::new();
        let mut published_any_reasoning_part = false;
        let mut published_any_text_part = false;

        let on_event = |ev: AgentEvent| match ev {
            AgentEvent::SessionStarted { ref session_id, .. } => {
                if !session_id.trim().is_empty() {
                    live_session_id = session_id.clone();
                }
                emit_trace("run.started", "running", None, json!({}));
            }
            AgentEvent::TextDelta { ref delta } => {
                if let Some((r_pid, r_ord, r_text)) = active_reasoning_part.take() {
                    let cursor = crate::daemon::publish_journal_part(
                        store,
                        &mut run_state,
                        &r_pid,
                        r_ord,
                        "reasoning",
                        json!({
                            "state": "completed",
                            "title": {"kind": "inline", "value": "Thinking"}
                        }),
                        &[("reasoning_summary", r_text)],
                    );
                    emit_line(&json!({
                        "type": "assistant_journal_committed_v1",
                        "session_id": conv_id,
                        "source_host_id": source_host_id,
                        "through": cursor
                    }));
                }
                acc_text.push_str(delta);
                if !delta.is_empty() {
                    let is_first = active_text_part.is_none();
                    let (t_pid, t_ord, t_text) = active_text_part.get_or_insert_with(|| {
                        let pid = if text_seq == 0 {
                            format!("{assistant_msg_id}:text")
                        } else {
                            format!("{assistant_msg_id}:text:{text_seq}")
                        };
                        text_seq += 1;
                        (pid, alloc_ordinal(), String::new())
                    });
                    t_text.push_str(delta);
                    published_any_text_part = true;
                    last_published_text_part_id = Some(t_pid.clone());
                    if is_first
                        || last_text_publish.elapsed() >= std::time::Duration::from_millis(120)
                    {
                        last_text_publish = std::time::Instant::now();
                        let cursor = crate::daemon::publish_journal_part(
                            store,
                            &mut run_state,
                            t_pid,
                            *t_ord,
                            "text",
                            json!({
                                "visibility": "public",
                                "is_final": false
                            }),
                            &[("markdown", t_text.clone())],
                        );
                        emit_line(&json!({
                            "type": "assistant_journal_committed_v1",
                            "session_id": conv_id,
                            "source_host_id": source_host_id,
                            "through": cursor
                        }));
                    }
                    emit_line(&json!({
                        "type": "text_delta",
                        "delta": delta,
                        "session_id": conv_id
                    }));
                }
            }
            AgentEvent::ReasoningDelta { ref delta } => {
                // Seal any preceding step's text part BEFORE opening a new reasoning round so
                // chronological ordering stays exact (text -> reasoning -> tool -> final text).
                if let Some((t_pid, t_ord, t_text)) = active_text_part.take() {
                    let cursor = crate::daemon::publish_journal_part(
                        store,
                        &mut run_state,
                        &t_pid,
                        t_ord,
                        "text",
                        json!({
                            "visibility": "public",
                            "is_final": true
                        }),
                        &[("markdown", t_text)],
                    );
                    last_published_text_part_id = Some(t_pid);
                    emit_line(&json!({
                        "type": "assistant_journal_committed_v1",
                        "session_id": conv_id,
                        "source_host_id": source_host_id,
                        "through": cursor
                    }));
                }
                acc_reasoning.push_str(delta);
                if !delta.is_empty() {
                    let is_first = active_reasoning_part.is_none();
                    let (r_pid, r_ord, r_text) = active_reasoning_part.get_or_insert_with(|| {
                        let pid = format!("{assistant_msg_id}:reasoning:{reasoning_seq}");
                        reasoning_seq += 1;
                        (pid, alloc_ordinal(), String::new())
                    });
                    r_text.push_str(delta);
                    published_any_reasoning_part = true;
                    if is_first
                        || last_reasoning_publish.elapsed()
                            >= std::time::Duration::from_millis(160)
                    {
                        last_reasoning_publish = std::time::Instant::now();
                        let cursor = crate::daemon::publish_journal_part(
                            store,
                            &mut run_state,
                            r_pid,
                            *r_ord,
                            "reasoning",
                            json!({
                                "state": "running",
                                "title": {"kind": "inline", "value": "Thinking"}
                            }),
                            &[("reasoning_summary", r_text.clone())],
                        );
                        emit_line(&json!({
                            "type": "assistant_journal_committed_v1",
                            "session_id": conv_id,
                            "source_host_id": source_host_id,
                            "through": cursor
                        }));
                    }
                    emit_line(&json!({
                        "type": "reasoning_delta",
                        "delta": delta,
                        "session_id": conv_id
                    }));
                }
            }
            AgentEvent::ToolStarted {
                ref call_id,
                ref name,
                ref arguments,
                ..
            } => {
                // Seal any active pre-tool text part at its true pre-tool ordinal so post-tool
                // final answers NEVER overwrite position 1 above the tool calls!
                if let Some((t_pid, t_ord, t_text)) = active_text_part.take() {
                    let cursor = crate::daemon::publish_journal_part(
                        store,
                        &mut run_state,
                        &t_pid,
                        t_ord,
                        "text",
                        json!({
                            "visibility": "public",
                            "is_final": true
                        }),
                        &[("markdown", t_text)],
                    );
                    last_published_text_part_id = Some(t_pid);
                    emit_line(&json!({
                        "type": "assistant_journal_committed_v1",
                        "session_id": conv_id,
                        "source_host_id": source_host_id,
                        "through": cursor
                    }));
                }
                if let Some((r_pid, r_ord, r_text)) = active_reasoning_part.take() {
                    let cursor = crate::daemon::publish_journal_part(
                        store,
                        &mut run_state,
                        &r_pid,
                        r_ord,
                        "reasoning",
                        json!({
                            "state": "completed",
                            "title": {"kind": "inline", "value": "Thinking"}
                        }),
                        &[("reasoning_summary", r_text)],
                    );
                    emit_line(&json!({
                        "type": "assistant_journal_committed_v1",
                        "session_id": conv_id,
                        "source_host_id": source_host_id,
                        "through": cursor
                    }));
                }
                if name == "bash" {
                    bash_ws_snapshots.insert(call_id.clone(), snapshot_workspace_files(workspace));
                }
                let ord = alloc_ordinal();
                let display_title = format_tool_display_title(name, arguments);
                tool_part_meta.insert(
                    call_id.clone(),
                    (ord, name.clone(), arguments.clone(), display_title.clone()),
                );
                let input_body = json!({"protocol_version": 1, "value": arguments}).to_string();
                let artifacts_body = json!({"protocol_version": 1, "artifacts": []}).to_string();
                let cursor = crate::daemon::publish_journal_part(
                    store,
                    &mut run_state,
                    &format!("tool:{call_id}"),
                    ord,
                    "tool",
                    json!({
                        "tool_call_id": call_id,
                        "capability_id": name,
                        "state": "running",
                        "execution_source": {"kind": "model"},
                        "title": {"kind": "inline", "value": display_title},
                        "origin": {"kind": "builtin"},
                        "artifacts": "tool_artifacts",
                        "affected_resources": {"kind": "inline", "value": []}
                    }),
                    &[("tool_input", input_body), ("tool_artifacts", artifacts_body)],
                );
                emit_line(&json!({
                    "type": "assistant_journal_committed_v1",
                    "session_id": conv_id,
                    "source_host_id": source_host_id,
                    "through": cursor
                }));
                recorded_tools.push(json!({
                    "id": call_id,
                    "call_id": call_id,
                    "name": name,
                    "arguments": arguments,
                    "is_error": false,
                    "summary": "",
                    "started_at_ms": now_ms()
                }));
                emit_trace(
                    "tool.started",
                    "running",
                    Some(call_id),
                    json!({
                        "call_id": call_id,
                        "tool_name": name
                    }),
                );
            }
            AgentEvent::ToolFinished {
                ref call_id,
                ref name,
                is_error,
                ref summary,
                ref effects,
                ..
            } => {
                let (ord, stored_name, stored_args, display_title) = tool_part_meta
                    .get(call_id)
                    .cloned()
                    .unwrap_or_else(|| (alloc_ordinal(), name.clone(), json!({}), name.clone()));
                let input_body =
                    json!({"protocol_version": 1, "value": stored_args}).to_string();
                let output_body = json!({
                    "protocol_version": 1,
                    "value": {"summary": summary, "is_error": is_error}
                })
                .to_string();
                let mut affected_resources: Vec<Value> = Vec::new();
                let mut artifacts_vec: Vec<Value> = Vec::new();
                let mut seen_artifact_paths: std::collections::HashSet<PathBuf> =
                    std::collections::HashSet::new();
                for eff in effects {
                    let op = format!("{:?}", eff.operation).to_ascii_lowercase();
                    let raw_path = eff.uri.strip_prefix("file://").unwrap_or(&eff.uri);
                    let p = PathBuf::from(raw_path);
                    let file_uri = if eff.uri.starts_with("file://") {
                        eff.uri.clone()
                    } else {
                        format!("file://{}", p.display())
                    };
                    affected_resources.push(json!({
                        "resource_type": "file",
                        "operation": op,
                        "uri": file_uri
                    }));
                    if !is_error && p.is_file() && seen_artifact_paths.insert(p.clone()) {
                        let fname = p
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("file")
                            .to_string();
                        let mime = infer_artifact_mime(&p);
                        artifacts_vec.push(json!({
                            "uri": file_uri,
                            "name": fname,
                            "mime_type": mime,
                            "operation": op
                        }));
                        if is_deliverable_artifact_path(&p) {
                            record_deliverable(&mut generated_deliverables, p, op);
                        }
                    }
                }
                if let Some(before_snap) = bash_ws_snapshots.remove(call_id) {
                    if !is_error {
                        for (changed_path, op) in diff_workspace_files(workspace, &before_snap) {
                            if seen_artifact_paths.insert(changed_path.clone()) {
                                let file_uri = format!("file://{}", changed_path.display());
                                let fname = changed_path
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("file")
                                    .to_string();
                                let mime = infer_artifact_mime(&changed_path);
                                affected_resources.push(json!({
                                    "resource_type": "file",
                                    "operation": op,
                                    "uri": file_uri
                                }));
                                artifacts_vec.push(json!({
                                    "uri": file_uri,
                                    "name": fname,
                                    "mime_type": mime,
                                    "operation": op
                                }));
                                if is_deliverable_artifact_path(&changed_path) {
                                    record_deliverable(&mut generated_deliverables, changed_path, op);
                                }
                            }
                        }
                    }
                }
                let artifacts_body =
                    json!({"protocol_version": 1, "artifacts": artifacts_vec}).to_string();
                let mut tool_data = json!({
                    "tool_call_id": call_id,
                    "capability_id": stored_name,
                    "state": if is_error { "failed" } else { "completed" },
                    "execution_source": {"kind": "model"},
                    "title": {"kind": "inline", "value": display_title},
                    "origin": {"kind": "builtin"},
                    "artifacts": "tool_artifacts",
                    "affected_resources": {"kind": "inline", "value": affected_resources}
                });
                if !summary.is_empty() {
                    tool_data["summary"] = json!({"kind": "inline", "value": summary});
                }
                let cursor = crate::daemon::publish_journal_part(
                    store,
                    &mut run_state,
                    &format!("tool:{call_id}"),
                    ord,
                    "tool",
                    tool_data,
                    &[
                        ("tool_input", input_body),
                        ("tool_output", output_body),
                        ("tool_output_text", summary.clone()),
                        ("tool_artifacts", artifacts_body),
                    ],
                );
                emit_line(&json!({
                    "type": "assistant_journal_committed_v1",
                    "session_id": conv_id,
                    "source_host_id": source_host_id,
                    "through": cursor
                }));
                if let Some(entry) = recorded_tools
                    .iter_mut()
                    .rev()
                    .find(|t| t.get("call_id").and_then(|v| v.as_str()) == Some(call_id.as_str()))
                {
                    entry["is_error"] = json!(is_error);
                    entry["summary"] = json!(summary);
                    entry["finished_at_ms"] = json!(now_ms());
                } else {
                    recorded_tools.push(json!({
                        "id": call_id,
                        "call_id": call_id,
                        "name": name,
                        "arguments": {},
                        "is_error": is_error,
                        "summary": summary,
                        "finished_at_ms": now_ms()
                    }));
                }
                emit_trace(
                    "tool.completed",
                    if is_error { "failed" } else { "completed" },
                    Some(call_id),
                    json!({
                        "call_id": call_id,
                        "tool_name": name,
                        "summary": summary
                    }),
                );
            }
            AgentEvent::Compacted {
                tokens_before,
                tokens_after,
                ..
            } => {
                emit_trace(
                    "context.compacted",
                    "completed",
                    None,
                    json!({
                        "summary": format!("Context compacted ({tokens_before} -> {tokens_after} tokens)")
                    }),
                );
            }
            _ => {}
        };
        let run_res = engine.run_identified(
            &ctx,
            &effective_prompt,
            Vec::new(),
            run_cfg,
            Some(&run_id),
            on_event,
        );
        ctx.pty.close_all();
        match run_res {
            Ok(res) => {
                let was_cancelled = res.cancelled || cancel.load(Ordering::Relaxed);
                let terminal_status = if was_cancelled { "cancelled" } else { "completed" };
                let final_text = if !res.final_text.is_empty() {
                    res.final_text.clone()
                } else {
                    acc_text
                };
                if let Some((r_pid, r_ord, r_text)) = active_reasoning_part.take() {
                    let _ = crate::daemon::publish_journal_part(
                        store,
                        &mut run_state,
                        &r_pid,
                        r_ord,
                        "reasoning",
                        json!({
                            "state": terminal_status,
                            "title": {"kind": "inline", "value": "Thinking"}
                        }),
                        &[("reasoning_summary", r_text)],
                    );
                } else if !published_any_reasoning_part && !acc_reasoning.is_empty() {
                    let r_part_id = format!("{assistant_msg_id}:reasoning");
                    let _ = crate::daemon::publish_journal_part(
                        store,
                        &mut run_state,
                        &r_part_id,
                        0,
                        "reasoning",
                        json!({
                            "state": terminal_status,
                            "title": {"kind": "inline", "value": "Thinking"}
                        }),
                        &[("reasoning_summary", acc_reasoning.clone())],
                    );
                }
                let (text_part_id, cursor) = if let Some((t_pid, t_ord, t_text)) =
                    active_text_part.take()
                {
                    let seg_text = if text_seq == 1 && final_text.len() > t_text.len() {
                        final_text.clone()
                    } else {
                        t_text
                    };
                    let c = crate::daemon::publish_journal_part(
                        store,
                        &mut run_state,
                        &t_pid,
                        t_ord,
                        "text",
                        json!({
                            "visibility": "public",
                            "is_final": true
                        }),
                        &[("markdown", seg_text)],
                    );
                    (t_pid, c)
                } else if !published_any_text_part {
                    let t_pid = format!("{assistant_msg_id}:text");
                    let t_ord = alloc_ordinal();
                    let c = crate::daemon::publish_journal_part(
                        store,
                        &mut run_state,
                        &t_pid,
                        t_ord,
                        "text",
                        json!({
                            "visibility": "public",
                            "is_final": true
                        }),
                        &[("markdown", final_text.clone())],
                    );
                    (t_pid, c)
                } else {
                    let t_pid = last_published_text_part_id
                        .clone()
                        .unwrap_or_else(|| format!("{assistant_msg_id}:text"));
                    let c = store
                        .get_kv(&format!("journal_head:{conv_id}:{run_id}"))
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| json!({"identity": run_state["journal_identity"], "sequence": run_state["journal_seq"].to_string()}));
                    (t_pid, c)
                };
                let mut latest_cursor = cursor;
                // Surface newly generated deliverable files (e.g. PNG/SVG charts, PDFs, Office docs)
                // as canonical attachment parts so macOS renders inline thumbnails + QuickLook + Artifacts!
                if !was_cancelled {
                    for (idx, (path, op)) in generated_deliverables.iter().enumerate() {
                        if !path.is_file() {
                            continue;
                        }
                        let fname = path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("artifact")
                            .to_string();
                        let file_uri = format!("file://{}", path.display());
                        let mime = infer_artifact_mime(path);
                        let att_id = format!("{assistant_msg_id}:artifact:{idx}");
                        latest_cursor = crate::daemon::publish_journal_part(
                            store,
                            &mut run_state,
                            &att_id,
                            alloc_ordinal(),
                            "attachment",
                            json!({
                                "attachment_id": att_id,
                                "name": {"kind": "inline", "value": fname},
                                "mime_type": {"kind": "inline", "value": mime},
                                "uri": {"kind": "inline", "value": file_uri},
                                "operation": {"kind": "inline", "value": op}
                            }),
                            &[],
                        );
                    }
                }
                if !was_cancelled || !final_text.is_empty() || !acc_reasoning.is_empty() {
                    let _ = store.append_event(
                        &conv_id,
                        "chat_assistant",
                        json!({
                            "id": assistant_msg_id,
                            "message_id": assistant_msg_id,
                            "content": final_text,
                            "reasoning": acc_reasoning,
                            "toolCalls": recorded_tools
                        }),
                        now_ms(),
                    );
                }
                emit_line(&json!({
                    "type": "assistant_journal_committed_v1",
                    "session_id": conv_id,
                    "source_host_id": source_host_id,
                    "through": latest_cursor
                }));
                let seq = emit_trace(
                    if was_cancelled { "run.cancelled" } else { "run.completed" },
                    terminal_status,
                    None,
                    json!({}),
                );
                run_state["status"] = json!(terminal_status);
                run_state["sequence"] = json!(seq);
                let seal =
                    crate::daemon::build_and_store_terminal_seal(store, &run_state, &text_part_id);
                emit_line(&json!({
                    "type": "assistant_run_terminal_committed_v1",
                    "session_id": conv_id,
                    "source_host_id": source_host_id,
                    "run_id": run_id,
                    "seal_digest": seal["seal_digest"]
                }));
                if was_cancelled {
                    emit_line(&json!({
                        "type": "turn_cancelled",
                        "protocol_version": 1,
                        "session_id": core_session_id,
                        "turn_id": run_id,
                        "canonical_run_id": run_id,
                        "cursor": seq,
                        "revision": seq,
                        "reason": "cancelled_by_user"
                    }));
                    return Ok(0);
                }
                emit_line(&json!({
                    "type": "turn_completed",
                    "protocol_version": 1,
                    "session_id": core_session_id,
                    "turn_id": run_id,
                    "canonical_run_id": run_id,
                    "cursor": seq,
                    "revision": seq
                }));
                if let Some(ref out_path) = opts.output_file {
                    reject_if_symlink(out_path)?;
                    if let Some(parent) = out_path.parent() {
                        if !parent.as_os_str().is_empty() {
                            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                        }
                    }
                    fs::write(out_path, &final_text)
                        .map_err(|e| format!("failed to write --output '{}': {e}", out_path.display()))?;
                }
                return Ok(0);
            }
            Err(e) => {
                let raw_err = e.to_string();
                let err_msg = format_user_facing_run_error(&raw_err, &route_endpoint);
                let was_cancelled = cancel.load(Ordering::Relaxed);
                let status = if was_cancelled { "cancelled" } else { "failed" };
                if let Some((r_pid, r_ord, r_text)) = active_reasoning_part.take() {
                    let _ = crate::daemon::publish_journal_part(
                        store,
                        &mut run_state,
                        &r_pid,
                        r_ord,
                        "reasoning",
                        json!({
                            "state": status,
                            "title": {"kind": "inline", "value": "Thinking"}
                        }),
                        &[("reasoning_summary", r_text)],
                    );
                }
                let _ = store.append_event(
                    &conv_id,
                    "run_error",
                    json!({"run_id": run_id, "assistant_message_id": assistant_msg_id, "status": status, "error": err_msg}),
                    now_ms(),
                );
                let seq = trace_seq.fetch_add(1, Ordering::SeqCst) + 1;
                run_state["status"] = json!(status);
                run_state["sequence"] = json!(seq);
                run_state["error"] = json!({"code": "run_failed", "message": err_msg});
                let text_part_id = format!("{assistant_msg_id}:text");
                let seal =
                    crate::daemon::build_and_store_terminal_seal(store, &run_state, &text_part_id);
                emit_line(&json!({
                    "type": "assistant_run_terminal_committed_v1",
                    "session_id": conv_id,
                    "source_host_id": source_host_id,
                    "run_id": run_id,
                    "seal_digest": seal["seal_digest"]
                }));
                emit_line(&json!({
                    "type": if was_cancelled { "turn_cancelled" } else { "turn_failed" },
                    "protocol_version": 1,
                    "session_id": core_session_id,
                    "turn_id": run_id,
                    "canonical_run_id": run_id,
                    "cursor": seq,
                    "revision": seq,
                    "reason": err_msg
                }));
                return Err(err_msg);
            }
        }
    }

    store.append_event(
        &effective_session_id, "chat_user", json!({"content": prompt}), nola_store::DEFAULT_MAX_EVENTS_PER_SCOPE,
    ).map_err(|err| err.to_string())?;
    let effective_prompt = format_turn_prompt_with_attachments(prompt, overrides.as_ref());
    let res = engine
        .run(&ctx, &effective_prompt, run_cfg, |ev| {
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
                            collect_a2ui_surface_frames(store, &effective_session_id, &live_session_id);
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

fn append_expert_and_host_context_blocks(
    store: &Store,
    prompt: &str,
    overrides: Option<&HostRunOverrides>,
    frozen_blocks: &mut Vec<nola_types::FrozenSystemBlock>,
) {
    let raw_experts: Vec<Value> = store
        .list_module_records::<Value>("expert_peer")
        .unwrap_or_default()
        .into_iter()
        .filter(|v| {
            v.get("availability")
                .and_then(|a| a.as_str())
                .is_none_or(|a| !a.eq_ignore_ascii_case("disabled"))
        })
        .collect();

    if !raw_experts.is_empty() {
        let mut lines = vec![
            "Saved Nola experts:".to_string(),
        ];
        for rec in raw_experts.iter().take(32) {
            let name = rec.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let id = rec.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let role = rec
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("specialist");
            let desc = rec
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if !name.is_empty() {
                if desc.is_empty() {
                    lines.push(format!("- {name} (id={id}, role={role})"));
                } else {
                    lines.push(format!("- {name} (id={id}, role={role}): {desc}"));
                }
            }
        }
        frozen_blocks.push(nola_types::FrozenSystemBlock {
            id: "saved_experts_directory".into(),
            text: lines.join("\n"),
        });

        // Resolve explicitly selected experts (from macOS composer) AND @mentions in prompt text.
        let mut target_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut target_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Some(ov) = overrides {
            for sel in &ov.selected_experts {
                if let Some(id) = sel
                    .get("agent_id")
                    .or_else(|| sel.get("agentId"))
                    .or_else(|| sel.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    target_ids.insert(id.to_string());
                }
                if let Some(n) = sel
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    target_names.insert(n.to_lowercase());
                }
            }
            for m in &ov.expert_mentions {
                if let Some(id) = m
                    .get("agent_id")
                    .or_else(|| m.get("agentId"))
                    .or_else(|| m.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    target_ids.insert(id.to_string());
                }
                if let Some(n) = m
                    .get("name")
                    .or_else(|| m.get("display_name"))
                    .or_else(|| m.get("label"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().trim_start_matches('@'))
                    .filter(|s| !s.is_empty())
                {
                    target_names.insert(n.to_lowercase());
                }
            }
        }
        for mention in nola_modules::parse_expert_mentions(prompt) {
            let clean = mention.trim().trim_start_matches('@');
            if !clean.is_empty() {
                target_names.insert(clean.to_lowercase());
            }
        }

        let matched: Vec<&Value> = raw_experts
            .iter()
            .filter(|rec| {
                let id = rec.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = rec.get("name").and_then(|v| v.as_str()).unwrap_or("");
                (!id.is_empty() && target_ids.contains(id))
                    || (!name.is_empty() && target_names.contains(&name.to_lowercase()))
            })
            .collect();

        if !matched.is_empty() {
            let mut active_lines = vec![
                "The user addressed the following saved experts. Use their supplied persona instructions. If specialized work needs delegation and delegate_expert is available, you may call it:".to_string(),
            ];
            for rec in matched {
                let name = rec.get("name").and_then(|v| v.as_str()).unwrap_or("Expert");
                let id = rec.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let role = rec
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("specialist");
                let desc = rec
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("");
                let sys = rec
                    .get("system_prompt")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("");
                let mut entry = format!("### Expert: {name} (id={id}, role={role})");
                if !desc.is_empty() {
                    entry.push_str(&format!("\nResponsibilities: {desc}"));
                }
                if !sys.is_empty() && sys != desc {
                    entry.push_str(&format!("\nPersona instructions: {sys}"));
                }
                active_lines.push(entry);
            }
            frozen_blocks.push(nola_types::FrozenSystemBlock {
                id: "active_expert_mentions".into(),
                text: active_lines.join("\n"),
            });
        }
    }

    let Some(ov) = overrides else {
        return;
    };

    if !ov.context_components.is_empty() {
        let mut ctx_parts: Vec<String> = Vec::new();
        for comp in &ov.context_components {
            let kind = comp
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("context");
            let id = comp.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "main_expert_selection" || id == "nola.main-expert-selection" {
                continue;
            }
            let title = comp
                .pointer("/source/title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(kind);
            let content_text = match comp.get("content") {
                Some(Value::String(s)) => s.trim().to_string(),
                Some(other) if !other.is_null() => other.to_string(),
                _ => String::new(),
            };
            if !content_text.is_empty() {
                let clipped: String = content_text.chars().take(4000).collect();
                ctx_parts.push(format!("[{title}]\n{clipped}"));
            }
        }
        if !ctx_parts.is_empty() {
            frozen_blocks.push(nola_types::FrozenSystemBlock {
                id: "host_context_components".into(),
                text: ctx_parts.join("\n\n"),
            });
        }
    }
}

fn format_tool_display_title(name: &str, args: &Value) -> String {
    fn clip(s: &str, max_chars: usize) -> String {
        let trimmed = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.chars().count() <= max_chars {
            trimmed
        } else {
            let head: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
            format!("{head}…")
        }
    }
    fn basename(raw: &str) -> String {
        Path::new(raw)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| clip(raw, 40))
    }
    match name {
        "read_file" => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| {
                if let Some(pg) = args.get("page").and_then(|v| v.as_u64()) {
                    format!("Read {} (p.{pg})", basename(p))
                } else {
                    format!("Read {}", basename(p))
                }
            })
            .unwrap_or_else(|| name.to_string()),
        "write_file" => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| format!("Write {}", basename(p)))
            .unwrap_or_else(|| name.to_string()),
        "edit_file" | "multi_edit_file" => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| format!("Edit {}", basename(p)))
            .unwrap_or_else(|| name.to_string()),
        "delete_file" => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| format!("Delete {}", basename(p)))
            .unwrap_or_else(|| name.to_string()),
        "bash" => args
            .get("command")
            .and_then(|v| v.as_str())
            .map(|c| format!("Run {}", clip(c, 52)))
            .unwrap_or_else(|| name.to_string()),
        "web_search" => args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| format!("Search \"{}\"", clip(q, 48)))
            .unwrap_or_else(|| name.to_string()),
        "web_fetch" => args
            .get("url")
            .and_then(|v| v.as_str())
            .map(|u| {
                let target = url::Url::parse(u)
                    .ok()
                    .and_then(|parsed| {
                        parsed.host_str().map(|h| {
                            let path = parsed.path();
                            if path.len() > 1 {
                                format!("{h}{}", clip(path, 28))
                            } else {
                                h.to_string()
                            }
                        })
                    })
                    .unwrap_or_else(|| clip(u, 48));
                format!("Fetch {target}")
            })
            .unwrap_or_else(|| name.to_string()),
        "tool_search" => args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| format!("Discover tools: {}", clip(q, 40)))
            .unwrap_or_else(|| name.to_string()),
        "grep" => args
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|p| format!("Grep \"{}\"", clip(p, 44)))
            .unwrap_or_else(|| name.to_string()),
        "glob" => args
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(|p| format!("Glob {}", clip(p, 44)))
            .unwrap_or_else(|| name.to_string()),
        "list_dir" => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| format!("List {}", basename(p)))
            .unwrap_or_else(|| "List workspace".to_string()),
        _ => name.to_string(),
    }
}

fn snapshot_workspace_files(
    workspace: &Path,
) -> std::collections::HashMap<PathBuf, (u64, std::time::SystemTime)> {
    let mut map = std::collections::HashMap::new();
    let Ok(entries) = fs::read_dir(workspace) else {
        return map;
    };
    for entry in entries.flatten().take(256) {
        let p = entry.path();
        if p.is_file() {
            if let Ok(meta) = entry.metadata() {
                let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                map.insert(p, (meta.len(), mtime));
            }
        }
    }
    map
}

fn diff_workspace_files(
    workspace: &Path,
    before: &std::collections::HashMap<PathBuf, (u64, std::time::SystemTime)>,
) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(workspace) else {
        return out;
    };
    for entry in entries.flatten().take(256) {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        let cur = (meta.len(), mtime);
        match before.get(&p) {
            None => out.push((p, "created".to_string())),
            Some(prev) if *prev != cur => out.push((p, "modified".to_string())),
            _ => {}
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn infer_artifact_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("md") => "text/markdown",
        Some("json") => "application/json",
        Some("py") => "text/x-python",
        _ => "application/octet-stream",
    }
}

fn is_deliverable_artifact_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    if name.starts_with('.') {
        return false;
    }
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "svg" | "pdf" | "docx" | "xlsx" | "pptx" | "csv" | "html")
    )
}

fn record_deliverable(list: &mut Vec<(PathBuf, String)>, path: PathBuf, op: String) {
    if let Some(existing) = list.iter_mut().find(|(p, _)| *p == path) {
        existing.1 = op;
    } else if list.len() < 12 {
        list.push((path, op));
    }
}

fn format_turn_prompt_with_attachments(prompt: &str, overrides: Option<&HostRunOverrides>) -> String {
    let Some(ov) = overrides else {
        return prompt.to_string();
    };
    if ov.attachments.is_empty() {
        return prompt.to_string();
    }
    let mut lines = Vec::new();
    for att in &ov.attachments {
        let name = att.get("name").and_then(|v| v.as_str()).unwrap_or("file");
        let path = att.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let mime = att
            .get("mime_type")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream");
        if !path.is_empty() {
            lines.push(format!("- {name} ({mime}): {path}"));
        }
    }
    if lines.is_empty() {
        return prompt.to_string();
    }
    let trimmed = prompt.trim_end();
    if trimmed.is_empty() {
        format!(
            "[Attached files — inspect directly with read_file]\n{}",
            lines.join("\n")
        )
    } else {
        format!(
            "{trimmed}\n\n[Attached files — inspect directly with read_file]\n{}",
            lines.join("\n")
        )
    }
}

fn format_user_facing_run_error(raw: &str, endpoint: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let is_local_route_block = lower.contains("no route to host")
        || lower.contains("os error 65")
        || lower.contains("ehostunreach")
        || lower.contains("local network prohibited");
    let ep_lower = endpoint.to_ascii_lowercase();
    let is_lan_endpoint = ep_lower.contains("://192.168.")
        || ep_lower.contains("://10.")
        || ep_lower.contains("://172.")
        || ep_lower.contains(".local");
    if is_local_route_block && is_lan_endpoint {
        return format!(
            "无法连接到局域网模型服务（{endpoint}）：macOS 阻止了本地网络访问（No route to host / os error 65）。请在「系统设置 → 隐私与安全性 → 本地网络」中允许 Nola，或确认局域网服务已启动后重试。\n\n原始错误：{raw}"
        );
    }
    raw.to_string()
}

fn resolve_route_with_overrides(
    cfg: &AppliedConfig,
    opts: &CliOptions,
    overrides: Option<&HostRunOverrides>,
) -> Result<ModelRouteConfig, String> {
    let ov_connection_id = overrides
        .and_then(|o| o.connection_id.clone())
        .filter(|s| !s.trim().is_empty());
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
        let matched_conn = ov_connection_id
            .as_deref()
            .and_then(|cid| cfg.llm.connections.iter().find(|c| c.id == cid))
            .or_else(|| {
                cfg.llm
                    .connections
                    .iter()
                    .find(|c| c.endpoint.trim_end_matches('/') == ep.trim_end_matches('/'))
            });
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
        let effective_reasoning_effort = ov_reasoning_effort
            .or_else(|| matched_entry.and_then(|e| e.reasoning_effort.clone()));
        let mut capabilities = matched_entry
            .and_then(|e| e.capabilities.clone())
            .unwrap_or_default();
        if nola_llm::normalized_wire_reasoning_effort(effective_reasoning_effort.as_deref())
            .is_some()
        {
            capabilities.reasoning = true;
        }
        return Ok(ModelRouteConfig {
            connection_id: ov_connection_id
                .or_else(|| matched_conn.map(|c| c.id.clone()))
                .unwrap_or_else(|| "host".into()),
            endpoint: ep,
            protocol,
            model: m,
            api_key,
            context_window: matched_entry.and_then(|e| e.context_window).unwrap_or(128_000),
            max_output_tokens: matched_entry.and_then(|e| e.max_output_tokens),
            temperature: matched_entry.and_then(|e| e.temperature).or(Some(0.2)),
            top_p: matched_entry.and_then(|e| e.top_p),
            capabilities,
            reasoning_effort: effective_reasoning_effort,
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
            if nola_llm::normalized_wire_reasoning_effort(Some(&effort)).is_some() {
                route.capabilities.reasoning = true;
            }
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
                    | "--archive-incompatible-store"
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
                    | "--title"
                    | "--content"
                    | "--summary"
                    | "--kind"
                    | "--category"
                    | "--scope"
                    | "--tags"
                    | "--tag"
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
                let raw = args
                    .get(i)
                    .ok_or("--profile requires a profile name")?
                    .trim()
                    .to_string();
                if raw.eq_ignore_ascii_case("accept_edits")
                    || raw.eq_ignore_ascii_case("accept-edits")
                {
                    return Err(PermissionProfile::parse_strict(&raw).unwrap_err());
                }
                opts.profile = Some(raw);
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
                let _ = upsert_model_connection_module_record(
                    store,
                    &updated_cfg,
                    &conn.id,
                    "discover",
                );
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
            let _ = upsert_model_connection_module_record(store, &cfg, &cid, "manual");
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
    let is_mlx = engine.eq_ignore_ascii_case("mlx");
    let variant = item
        .get("variant")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            if is_mlx {
                None
            } else {
                let short = repo.rsplit('/').next().unwrap_or(&repo);
                Some(format!("{short}.gguf"))
            }
        });
    let dl_bytes = item
        .get("downloadBytes")
        .and_then(|v| v.as_i64())
        .unwrap_or(4_000_000_000);
    let est_mem = item
        .get("estimatedMemoryBytes")
        .and_then(|v| v.as_i64())
        .unwrap_or(((dl_bytes as f64) * 1.4) as i64);
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
        .unwrap_or(if is_mlx { "config.json" } else { "model.gguf" })
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
    if let Some(ref v) = variant {
        plan.insert("variant".into(), json!(v));
    }
    plan.insert("device".into(), json!(device));
    plan.insert("url".into(), json!(download_url));
    if !is_mlx {
        plan.entry("files".to_string())
            .or_insert_with(|| json!([{ "path": file_name, "bytes": dl_bytes }]));
    }

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

fn default_local_acquire_engine() -> &'static str {
    if std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64" {
        "mlx"
    } else {
        "llama.cpp"
    }
}

fn handle_model_acquire_cmd(store: &Store, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("engine-status") => {
            if let Some(engine) = args
                .get(1)
                .map(String::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                let raw = probe_engine_status(engine);
                let status = normalize_engine_status_event(engine, &raw);
                emit_compact_json_line(&json!({ "engineStatus": status }));
                return Ok(0);
            }
            let default_engine = default_local_acquire_engine();
            let engines: &[&str] = if default_engine == "mlx" {
                &["mlx", "llama.cpp", "ollama"]
            } else {
                &["llama.cpp", "ollama", "vllm"]
            };
            let statuses: Vec<Value> = engines
                .iter()
                .map(|eng| {
                    let raw = probe_engine_status(eng);
                    normalize_engine_status_event(eng, &raw)
                })
                .collect();
            let primary = statuses.first().cloned().unwrap_or_else(|| {
                let raw = probe_engine_status(default_engine);
                normalize_engine_status_event(default_engine, &raw)
            });
            emit_compact_json_line(&json!({
                "engineStatus": primary,
                "engines": statuses
            }));
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
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(default_local_acquire_engine());
            let source = args
                .get(2)
                .map(String::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("huggingface");
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
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("main");
            let models_root = resolve_local_models_root();
            let safe_repo_dir: String = repo
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let snapshot_dir = models_root.join(&safe_repo_dir);
            let ctx_tokens = plan_json
                .get("contextTokens")
                .or_else(|| plan_json.get("contextWindowTokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(32_768);
            let device = plan_json
                .get("device")
                .and_then(|v| v.as_str())
                .unwrap_or("unified")
                .to_string();
            let quant = plan_json
                .get("quantization")
                .or_else(|| plan_json.get("variant"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);

            if engine.eq_ignore_ascii_case("mlx") {
                let model_id = new_id();
                let (artifacts, total_bytes) = download_mlx_repo_snapshot(
                    source,
                    &repo,
                    rev,
                    &plan_json,
                    &snapshot_dir,
                )?;
                write_local_snapshot_manifest(&snapshot_dir, &artifacts, total_bytes)?;
                let rec = LocalModelRecord {
                    id: model_id.clone(),
                    name: repo.clone(),
                    engine: "mlx".to_string(),
                    path: Some(snapshot_dir.display().to_string()),
                    size_bytes: total_bytes,
                    quantization: quant.clone(),
                    installed: true,
                    updated_at_ms: now_ms(),
                };
                let _ = store.put_module_record(KIND_LOCAL_MODEL, &rec.id, &rec);
                upsert_mac_local_model_catalog_entry(
                    &models_root,
                    &MacLocalCatalogRecordInput {
                        id: &model_id,
                        name: &repo,
                        repository: &repo,
                        runtime: "mlx",
                        format: "mlx",
                        quantization: quant.as_deref(),
                        context_window_tokens: ctx_tokens,
                        recommended_device: &device,
                        snapshot_path: &snapshot_dir,
                        artifact_path: None,
                        artifact_bytes: total_bytes as i64,
                    },
                )?;
                emit_compact_json_line(&json!({
                    "model": { "id": model_id },
                    "downloadedBytes": total_bytes,
                    "totalBytes": total_bytes,
                    "status": "completed"
                }));
                return Ok(0);
            }

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

            let safe_file: String = Path::new(variant_file)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("model.gguf")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
                .collect();
            let dest_path = plan_json
                .get("dest_path")
                .or_else(|| plan_json.get("destPath"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| snapshot_dir.join(&safe_file));

            let dl_plan = ModelDownloadPlan {
                id: Some(new_id()),
                name: repo.clone(),
                engine: engine.to_string(),
                url,
                dest_path: dest_path.display().to_string(),
                sha256,
                quantization: quant.clone(),
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
                    let actual_bytes = fs::metadata(&dest_path)
                        .map(|m| m.len())
                        .unwrap_or(dl_bytes)
                        .max(1);
                    let eff_snapshot_dir = dest_path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| snapshot_dir.clone());
                    let rel_name = dest_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&safe_file)
                        .to_string();
                    let _ = write_local_snapshot_manifest(
                        &eff_snapshot_dir,
                        &[(rel_name, actual_bytes)],
                        actual_bytes,
                    );
                    let _ = upsert_mac_local_model_catalog_entry(
                        &models_root,
                        &MacLocalCatalogRecordInput {
                            id: &model_id,
                            name: &repo,
                            repository: &repo,
                            runtime: if engine.eq_ignore_ascii_case("mlx") {
                                "mlx"
                            } else {
                                "llama.cpp"
                            },
                            format: if engine.eq_ignore_ascii_case("mlx") {
                                "mlx"
                            } else {
                                "gguf"
                            },
                            quantization: quant.as_deref(),
                            context_window_tokens: ctx_tokens,
                            recommended_device: &device,
                            snapshot_path: &eff_snapshot_dir,
                            artifact_path: Some(&dest_path),
                            artifact_bytes: actual_bytes as i64,
                        },
                    );
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
            let models_root = resolve_local_models_root();
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
                let _ = remove_mac_local_model_catalog_entry(&models_root, &rec.id, Some(&rec.name));
            } else {
                let _ = remove_mac_local_model_catalog_entry(&models_root, id, None);
                if !id.contains("..") && !id.contains('/') && !id.contains('\\') {
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
pub(crate) struct ExtractedImportCandidate {
    pub(crate) candidate_id: String,
    pub(crate) connection_id: String,
    pub(crate) display_name: String,
    pub(crate) source: String,
    pub(crate) endpoint: String,
    pub(crate) protocol: String,
    pub(crate) models: Vec<String>,
    pub(crate) secret: Option<String>,
    pub(crate) credential_status: String,
    pub(crate) issue: Option<String>,
}

fn parse_simple_toml(raw: &str) -> BTreeMap<String, BTreeMap<String, Value>> {
    let mut sections: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut current_section = String::new();
    for raw_line in raw.lines() {
        let mut in_double = false;
        let mut in_single = false;
        let mut escaped = false;
        let mut cut = raw_line.len();
        for (idx, ch) in raw_line.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' if in_double => escaped = true,
                '"' if !in_single => in_double = !in_double,
                '\'' if !in_double => in_single = !in_single,
                '#' if !in_double && !in_single => {
                    cut = idx;
                    break;
                }
                _ => {}
            }
        }
        let line = raw_line[..cut].trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let inner = line.trim_start_matches('[').trim_end_matches(']').trim();
            let normalized = inner
                .split('.')
                .map(|seg| seg.trim().trim_matches('"').trim_matches('\''))
                .collect::<Vec<_>>()
                .join(".");
            current_section = normalized;
            sections.entry(current_section.clone()).or_default();
            continue;
        }
        if let Some((k_raw, v_raw)) = line.split_once('=') {
            let key = k_raw.trim().trim_matches('"').trim_matches('\'').to_string();
            let v_str = v_raw.trim();
            let val = if (v_str.starts_with('"') && v_str.ends_with('"') && v_str.len() >= 2)
                || (v_str.starts_with('\'') && v_str.ends_with('\'') && v_str.len() >= 2)
            {
                if v_str.starts_with('"') {
                    serde_json::from_str::<String>(v_str)
                        .map(Value::String)
                        .unwrap_or_else(|_| Value::String(v_str[1..v_str.len() - 1].to_string()))
                } else {
                    Value::String(v_str[1..v_str.len() - 1].to_string())
                }
            } else if v_str == "true" {
                Value::Bool(true)
            } else if v_str == "false" {
                Value::Bool(false)
            } else if v_str.starts_with('[') && v_str.ends_with(']') {
                serde_json::from_str::<Value>(v_str).unwrap_or_else(|_| {
                    let inner = &v_str[1..v_str.len() - 1];
                    let items: Vec<Value> = inner
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').trim_matches('\''))
                        .filter(|s| !s.is_empty())
                        .map(|s| Value::String(s.to_string()))
                        .collect();
                    Value::Array(items)
                })
            } else if let Ok(n) = v_str.parse::<i64>() {
                json!(n)
            } else if let Ok(f) = v_str.parse::<f64>() {
                json!(f)
            } else {
                Value::String(v_str.to_string())
            };
            sections
                .entry(current_section.clone())
                .or_default()
                .insert(key, val);
        }
    }
    sections
}

fn resolve_inline_or_env_secret(raw: Option<&str>, allow_environment: bool) -> Option<String> {
    let val = raw?.trim();
    if val.is_empty() {
        return None;
    }
    if let Some(rest) = val.strip_prefix("{env:").and_then(|s| s.strip_suffix('}')) {
        if allow_environment {
            return std::env::var(rest.trim())
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
        }
        return None;
    }
    if let Some(var_name) = val.strip_prefix('$') {
        let name = var_name.trim_matches(|c| c == '{' || c == '}');
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            if allow_environment {
                return std::env::var(name)
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty());
            }
            return None;
        }
    }
    Some(val.to_string())
}

fn lookup_credential_in_file(cred_val: &Value, keys: &[&str]) -> Option<String> {
    let obj = cred_val.as_object()?;
    for key in keys {
        if let Some(v) = obj.get(*key) {
            if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                return Some(s.to_string());
            }
            if let Some(sub) = v.as_object() {
                for field in ["key", "api_key", "apiKey", "token", "access_token"] {
                    if let Some(s) = sub
                        .get(field)
                        .and_then(|x| x.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    None
}

fn finalize_credential_state(
    secret: Option<String>,
    include_credentials: bool,
) -> (Option<String>, String) {
    if !include_credentials {
        return (None, "not_requested".to_string());
    }
    if secret.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        (secret, "available".to_string())
    } else {
        (None, "missing".to_string())
    }
}

fn extract_import_candidates_from_path(
    path_str: &str,
    src: &str,
) -> Result<Vec<ExtractedImportCandidate>, String> {
    extract_import_candidates_with_options(path_str, src, None, true, true)
}

pub(crate) fn extract_import_candidates_with_options(
    path_str: &str,
    src: &str,
    credential_path: Option<&str>,
    include_credentials: bool,
    allow_environment: bool,
) -> Result<Vec<ExtractedImportCandidate>, String> {
    let p = Path::new(path_str);
    reject_if_symlink(p)?;
    let raw = read_config_file_safe(p)?;
    let cred_doc: Option<Value> = if include_credentials {
        if let Some(cp) = credential_path.filter(|s| !s.trim().is_empty()) {
            let cp_path = Path::new(cp);
            reject_if_symlink(cp_path)?;
            let cred_raw = read_config_file_safe(cp_path)?;
            if let Ok(v) = serde_json::from_str::<Value>(&strip_jsonc(&cred_raw)) {
                Some(v)
            } else {
                let toml_map = parse_simple_toml(&cred_raw);
                let mut flat = serde_json::Map::new();
                for (sec, kvs) in toml_map {
                    if sec.is_empty() {
                        for (k, v) in kvs {
                            flat.insert(k, v);
                        }
                    } else {
                        flat.insert(sec, Value::Object(kvs.into_iter().collect()));
                    }
                }
                Some(Value::Object(flat))
            }
        } else {
            None
        }
    } else {
        None
    };

    let src_lower = src.trim().to_ascii_lowercase();
    let mut out = Vec::new();
    let parsed_json = serde_json::from_str::<Value>(&strip_jsonc(&raw)).ok();

    if let Some(ref parsed) = parsed_json {
        if let Some(providers) = parsed.get("provider").and_then(|v| v.as_object()) {
            for (prov_id, prov_val) in providers {
                let endpoint = prov_val
                    .pointer("/options/baseURL")
                    .or_else(|| prov_val.pointer("/options/base_url"))
                    .or_else(|| prov_val.get("baseURL"))
                    .or_else(|| prov_val.get("base_url"))
                    .or_else(|| prov_val.get("endpoint"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let models: Vec<String> = prov_val
                    .get("models")
                    .map(|mv| {
                        if let Some(obj) = mv.as_object() {
                            obj.keys().cloned().collect()
                        } else if let Some(arr) = mv.as_array() {
                            arr.iter()
                                .filter_map(|m| {
                                    m.as_str()
                                        .or_else(|| m.get("id").and_then(|v| v.as_str()))
                                        .map(ToString::to_string)
                                })
                                .collect()
                        } else {
                            Vec::new()
                        }
                    })
                    .unwrap_or_default();
                let raw_secret = prov_val
                    .pointer("/options/apiKey")
                    .or_else(|| prov_val.pointer("/options/api_key"))
                    .or_else(|| prov_val.get("apiKey"))
                    .or_else(|| prov_val.get("api_key"))
                    .and_then(|v| v.as_str());
                let resolved_secret = resolve_inline_or_env_secret(raw_secret, allow_environment)
                    .or_else(|| {
                        cred_doc
                            .as_ref()
                            .and_then(|cd| lookup_credential_in_file(cd, &[prov_id.as_str()]))
                    });
                let (secret, cred_status) =
                    finalize_credential_state(resolved_secret, include_credentials);
                let display_name = prov_val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(prov_id)
                    .to_string();
                let inferred_proto = prov_val
                    .get("protocol")
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        let npm = prov_val.get("npm").and_then(|v| v.as_str()).unwrap_or("");
                        if npm.contains("anthropic") {
                            Some("anthropic")
                        } else if npm.contains("openai") && !npm.contains("compatible") {
                            Some("responses")
                        } else {
                            None
                        }
                    })
                    .unwrap_or("chat_completions");
                let proto = crate::daemon::normalize_wire_protocol(inferred_proto);
                let issue = if endpoint.is_empty() {
                    Some("Missing base URL".to_string())
                } else {
                    None
                };
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
                    issue,
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
                    .or_else(|| c.get("base_url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let proto = crate::daemon::normalize_wire_protocol(
                    c.get("protocol")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            c.get("protocols")
                                .and_then(|v| v.as_array())
                                .and_then(|a| a.first())
                                .and_then(|v| v.as_str())
                        })
                        .unwrap_or("chat_completions"),
                );
                let models: Vec<String> = c
                    .get("models")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| {
                                m.get("id")
                                    .or_else(|| m.get("model_id"))
                                    .and_then(|v| v.as_str())
                                    .or_else(|| m.as_str())
                                    .map(ToString::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let env_secret = if allow_environment {
                    c.get("api_key_env")
                        .and_then(|v| v.as_str())
                        .and_then(|e| std::env::var(e).ok())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                } else {
                    None
                };
                let resolved_secret = resolve_inline_or_env_secret(
                    c.get("api_key").and_then(|v| v.as_str()),
                    allow_environment,
                )
                .or(env_secret)
                .or_else(|| {
                    cred_doc
                        .as_ref()
                        .and_then(|cd| lookup_credential_in_file(cd, &[cid.as_str()]))
                });
                let (secret, cred_status) =
                    finalize_credential_state(resolved_secret, include_credentials);
                let issue = if endpoint.is_empty() {
                    Some("Missing base URL".to_string())
                } else {
                    None
                };
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
                    issue,
                });
            }
        }
    }

    if out.is_empty() && parsed_json.is_none() {
        let toml = parse_simple_toml(&raw);
        let top = toml.get("");
        let top_model = top
            .and_then(|t| t.get("model"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let top_provider = top
            .and_then(|t| t.get("model_provider").or_else(|| t.get("default_provider")))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        let mut models_by_provider: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (sec, kvs) in &toml {
            if let Some(mkey) = sec.strip_prefix("models.") {
                let prov = kvs
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let mid = kvs
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(mkey)
                    .to_string();
                if !prov.is_empty() && !mid.is_empty() {
                    let entry = models_by_provider.entry(prov).or_default();
                    if !entry.contains(&mid) {
                        entry.push(mid);
                    }
                }
            }
        }

        let has_codex_subscription_only = cred_doc.as_ref().is_some_and(|cd| {
            let has_api_key = cd
                .get("OPENAI_API_KEY")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty());
            let has_sub = cd.get("tokens").is_some_and(|v| v.is_object())
                || cd.get("auth_mode").and_then(|v| v.as_str()) == Some("chatgpt");
            !has_api_key && has_sub
        });

        for (sec, kvs) in &toml {
            let prov_id = if let Some(id) = sec.strip_prefix("model_providers.") {
                id
            } else if let Some(id) = sec.strip_prefix("providers.") {
                id
            } else {
                continue;
            };
            if prov_id.contains('.') || prov_id.is_empty() {
                continue;
            }
            let display_name = kvs
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(prov_id)
                .to_string();
            let endpoint = kvs
                .get("base_url")
                .or_else(|| kvs.get("baseURL"))
                .or_else(|| kvs.get("endpoint"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            let wire_raw = kvs
                .get("wire_api")
                .or_else(|| kvs.get("protocol"))
                .or_else(|| kvs.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or(if src_lower == "codex" {
                    "responses"
                } else {
                    "chat_completions"
                });
            let proto = if wire_raw.to_ascii_lowercase().contains("anthropic") {
                "anthropic".to_string()
            } else if wire_raw.to_ascii_lowercase().contains("responses") {
                "responses".to_string()
            } else {
                "chat_completions".to_string()
            };
            let env_key = kvs
                .get("env_key")
                .or_else(|| kvs.get("api_key_env"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let env_secret = if allow_environment {
                env_key
                    .and_then(|ek| std::env::var(ek).ok())
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            } else {
                None
            };
            let cred_keys: Vec<&str> = match env_key {
                Some(ek) => vec![ek, prov_id, "OPENAI_API_KEY"],
                None => vec![prov_id, "OPENAI_API_KEY"],
            };
            let resolved_secret = resolve_inline_or_env_secret(
                kvs.get("api_key")
                    .or_else(|| kvs.get("apiKey"))
                    .and_then(|v| v.as_str()),
                allow_environment,
            )
            .or(env_secret)
            .or_else(|| {
                cred_doc
                    .as_ref()
                    .and_then(|cd| lookup_credential_in_file(cd, &cred_keys))
            });
            let (secret, cred_status) =
                finalize_credential_state(resolved_secret, include_credentials);
            let mut models = models_by_provider.remove(prov_id).unwrap_or_default();
            if let Some(m) = kvs
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if !models.iter().any(|x| x == m) {
                    models.push(m.to_string());
                }
            }
            if top_provider.as_deref() == Some(prov_id) {
                if let Some(ref tm) = top_model {
                    if !models.iter().any(|x| x == tm) {
                        models.push(tm.clone());
                    }
                }
            }
            let issue = if endpoint.is_empty() {
                Some("Missing base URL".to_string())
            } else if include_credentials && secret.is_none() && has_codex_subscription_only {
                Some("subscription_credential_reuse_not_supported".to_string())
            } else {
                None
            };
            out.push(ExtractedImportCandidate {
                candidate_id: new_id(),
                connection_id: format!("{src_lower}-{prov_id}"),
                display_name,
                source: src.to_string(),
                endpoint,
                protocol: proto,
                models,
                secret,
                credential_status: cred_status,
                issue,
            });
        }

        if out.is_empty() && src_lower == "codex" && (top_model.is_some() || top_provider.is_some())
        {
            let prov_name = top_provider.unwrap_or_else(|| "openai".to_string());
            let env_secret = if allow_environment {
                std::env::var("OPENAI_API_KEY")
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            } else {
                None
            };
            let resolved_secret = env_secret.or_else(|| {
                cred_doc
                    .as_ref()
                    .and_then(|cd| lookup_credential_in_file(cd, &["OPENAI_API_KEY", &prov_name]))
            });
            let (secret, cred_status) =
                finalize_credential_state(resolved_secret, include_credentials);
            let issue = if include_credentials && secret.is_none() && has_codex_subscription_only {
                Some("subscription_credential_reuse_not_supported".to_string())
            } else {
                None
            };
            let models = top_model.into_iter().collect();
            out.push(ExtractedImportCandidate {
                candidate_id: new_id(),
                connection_id: format!("codex-{prov_name}"),
                display_name: if prov_name == "openai" {
                    "OpenAI (Codex)".to_string()
                } else {
                    prov_name
                },
                source: src.to_string(),
                endpoint: "https://api.openai.com/v1".to_string(),
                protocol: "responses".to_string(),
                models,
                secret,
                credential_status: cred_status,
                issue,
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
    let mut imported_sources: BTreeMap<String, String> = BTreeMap::new();
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
            if !cand.endpoint.trim().is_empty() {
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
        } else if !cand.endpoint.trim().is_empty() {
            cfg.llm.connections.push(ConnectionConfig {
                id: cid.clone(),
                endpoint: cand.endpoint.clone(),
                protocol: Some(cand.protocol.clone()),
                provider: None,
                api_key,
                api_key_env: None,
                models,
            });
        } else {
            // Skip candidates with empty endpoint when no existing connection matches.
            continue;
        }
        if !cand.display_name.is_empty() && cand.display_name != cid {
            cfg.connection_extras
                .entry(cid.clone())
                .or_default()
                .name = Some(cand.display_name.clone());
        }
        imported_sources.insert(cid.clone(), cand.source.clone());
        imported_ids.push(cid);
    }
    if !imported_ids.is_empty() {
        save_applied_config(store, &cfg)?;
        for cid in &imported_ids {
            let src = imported_sources
                .get(cid)
                .map(String::as_str)
                .unwrap_or("import");
            let _ = upsert_model_connection_module_record(store, &cfg, cid, src);
        }
    }
    Ok(imported_ids)
}

fn candidate_to_mac_import_json(
    cand: &ExtractedImportCandidate,
    existing_rev: Option<i64>,
    existing_account_id: Option<&str>,
) -> Value {
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
    let issue = cand.issue.clone().or_else(|| {
        if cand.endpoint.trim().is_empty() {
            Some("Missing base URL".to_string())
        } else {
            None
        }
    });
    json!({
        "candidate_id": cand.candidate_id,
        "connection": {
            "id": cand.connection_id,
            "display_name": cand.display_name,
            "base_url": cand.endpoint,
            "protocols": [cand.protocol],
            "account_id": existing_account_id.map(|s| json!(s)).unwrap_or(Value::Null),
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

fn handle_model_import_bridge_cmd(
    store: &Store,
    suite: &ModulesSuite,
    args: &[String],
) -> Result<u8, String> {
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
                        issue: None,
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
                let allow_environment = req
                    .get("allow_environment")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let credential_path = req
                    .get("credential_path")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let path_str = req.get("path").and_then(|v| v.as_str());
                match path_str {
                    Some(p) => match extract_import_candidates_with_options(
                        p,
                        &src_name,
                        credential_path,
                        include_credentials,
                        allow_environment,
                    ) {
                        Ok(candidates) => {
                            let plan_id = new_id();
                            let active_cfg = load_active_config(store);
                            let mac_cands: Vec<Value> = candidates
                                .iter()
                                .map(|c| {
                                    let existing_rec = suite
                                        .get_record::<Value>("model_connection", &c.connection_id)
                                        .ok()
                                        .flatten();
                                    let existing_rev = existing_rec
                                        .as_ref()
                                        .and_then(|r| r.get("revision").and_then(|v| v.as_i64()))
                                        .or_else(|| {
                                            active_cfg
                                                .llm
                                                .connections
                                                .iter()
                                                .any(|ex| ex.id == c.connection_id)
                                                .then_some(1i64)
                                        });
                                    let existing_acct_id = existing_rec
                                        .as_ref()
                                        .and_then(|r| r.get("account_id").and_then(|v| v.as_str()))
                                        .filter(|s| !s.trim().is_empty());
                                    candidate_to_mac_import_json(c, existing_rev, existing_acct_id)
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
                    let mut selection_opts: BTreeMap<String, (Option<String>, bool, bool)> =
                        BTreeMap::new();
                    for s in &selections {
                        if let Some(cid) = s.get("candidate_id").and_then(|v| v.as_str()) {
                            let disp = s
                                .get("display_name")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .filter(|x| !x.is_empty())
                                .map(ToString::to_string);
                            let upd = s
                                .get("update_existing")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let enable = s
                                .get("enable_models")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true);
                            selection_opts.insert(cid.to_string(), (disp, upd, enable));
                        }
                    }
                    let chosen: Vec<ExtractedImportCandidate> = if selection_opts.is_empty() {
                        plan_cands
                    } else {
                        plan_cands
                            .into_iter()
                            .filter(|c| selection_opts.contains_key(&c.candidate_id))
                            .map(|mut c| {
                                if let Some((Some(disp), _, _)) =
                                    selection_opts.get(&c.candidate_id)
                                {
                                    c.display_name = disp.clone();
                                }
                                c
                            })
                            .collect()
                    };

                    // Filter out existing connections where update_existing was not approved
                    let mut approved_to_commit: Vec<ExtractedImportCandidate> = Vec::new();
                    let mut skipped_results: Vec<Value> = Vec::new();
                    for cand in chosen {
                        let existing_rec = suite
                            .get_record::<Value>("model_connection", &cand.connection_id)
                            .ok()
                            .flatten();
                        let update_ok = selection_opts
                            .get(&cand.candidate_id)
                            .map(|(_, upd, _)| *upd)
                            .unwrap_or(true);
                        if existing_rec.is_some() && !update_ok {
                            skipped_results.push(json!({
                                "candidate_id": cand.candidate_id,
                                "connection_id": cand.connection_id,
                                "status": "skipped",
                                "error": "Existing connection requires update confirmation"
                            }));
                        } else {
                            approved_to_commit.push(cand);
                        }
                    }

                    // Perform interactive `credential_request` -> `credential_saved` handshake with macOS when framed & secrets included
                    let mut saved_accounts: BTreeMap<String, (String, String)> = BTreeMap::new();
                    let mut handshake_failed: Option<String> = None;
                    if may_store_creds {
                        if let Some(ref rid) = request_id {
                            for cand in &approved_to_commit {
                                if let Some(ref secret) = cand.secret {
                                    let nonce = new_id();
                                    let existing_acct = suite
                                        .get_record::<Value>("model_connection", &cand.connection_id)
                                        .ok()
                                        .flatten()
                                        .and_then(|r| {
                                            r.get("account_id")
                                                .and_then(|v| v.as_str())
                                                .filter(|s| !s.trim().is_empty())
                                                .map(ToString::to_string)
                                        });
                                    let account_id = existing_acct
                                        .unwrap_or_else(|| format!("import-account-{}", new_id()));
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
                                                        saved_accounts.insert(
                                                            cand.connection_id.clone(),
                                                            (account_id, r.to_string()),
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
                        // When framed (macOS Keychain holds secrets), never write raw secrets into SQLite KV config.
                        let write_plaintext_secret = request_id.is_none() && may_store_creds;
                        match commit_import_candidates(store, &approved_to_commit, write_plaintext_secret) {
                            Ok(_) => {
                                let ts = now_ms();
                                if !saved_accounts.is_empty() {
                                    let mut cfg = load_active_config(store);
                                    for (cid, (_, cref)) in &saved_accounts {
                                        cfg.connection_extras
                                            .entry(cid.clone())
                                            .or_default()
                                            .credential_ref = Some(cref.clone());
                                    }
                                    let _ = save_applied_config(store, &cfg);
                                }
                                let mut results = skipped_results;
                                for cand in &approved_to_commit {
                                    let existing_conn = suite
                                        .get_record::<Value>("model_connection", &cand.connection_id)
                                        .ok()
                                        .flatten();
                                    let mut account_id_for_conn = existing_conn
                                        .as_ref()
                                        .and_then(|r| r.get("account_id").and_then(|v| v.as_str()))
                                        .filter(|s| !s.trim().is_empty())
                                        .map(ToString::to_string);
                                    if let Some((acct_id, cref)) =
                                        saved_accounts.get(&cand.connection_id)
                                    {
                                        let prev_acct_rev = suite
                                            .get_record::<Value>("model_account", acct_id)
                                            .ok()
                                            .flatten()
                                            .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                                            .unwrap_or(0);
                                        let acct_rec = json!({
                                            "id": acct_id,
                                            "account_id": acct_id,
                                            "display_name": cand.display_name,
                                            "source": cand.source,
                                            "credential_ref": cref,
                                            "state": "ready",
                                            "revision": prev_acct_rev + 1,
                                            "updated_at_ms": ts
                                        });
                                        if let Err(e) =
                                            suite.upsert_record("model_account", acct_id, &acct_rec)
                                        {
                                            results.push(json!({
                                                "candidate_id": cand.candidate_id,
                                                "connection_id": cand.connection_id,
                                                "status": "failed",
                                                "error": e.to_string()
                                            }));
                                            continue;
                                        }
                                        account_id_for_conn = Some(acct_id.clone());
                                    }

                                    let prev_rev = existing_conn
                                        .as_ref()
                                        .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                                        .unwrap_or(0);
                                    let catalog_models: Vec<Value> = cand
                                        .models
                                        .iter()
                                        .filter_map(|mid| {
                                            crate::daemon::normalize_catalog_model_entry(
                                                &Value::String(mid.clone()),
                                                &cand.protocol,
                                            )
                                        })
                                        .collect();
                                    let catalog_val = if !catalog_models.is_empty() {
                                        json!({
                                            "observed_at_ms": ts,
                                            "source": cand.source,
                                            "models": catalog_models
                                        })
                                    } else {
                                        existing_conn
                                            .as_ref()
                                            .and_then(|r| r.get("catalog").cloned())
                                            .unwrap_or(Value::Null)
                                    };
                                    let conn_rec = json!({
                                        "id": cand.connection_id,
                                        "connection_id": cand.connection_id,
                                        "display_name": cand.display_name,
                                        "base_url": cand.endpoint,
                                        "protocols": [cand.protocol],
                                        "account_id": account_id_for_conn,
                                        "source": cand.source,
                                        "source_id": cand.connection_id,
                                        "enabled": true,
                                        "revision": prev_rev + 1,
                                        "updated_at_ms": ts,
                                        "catalog": catalog_val
                                    });
                                    if let Err(e) = suite.upsert_record(
                                        "model_connection",
                                        &cand.connection_id,
                                        &conn_rec,
                                    ) {
                                        results.push(json!({
                                            "candidate_id": cand.candidate_id,
                                            "connection_id": cand.connection_id,
                                            "status": "failed",
                                            "error": e.to_string()
                                        }));
                                        continue;
                                    }

                                    let enable_models = selection_opts
                                        .get(&cand.candidate_id)
                                        .map(|(_, _, en)| *en)
                                        .unwrap_or(true);
                                    if enable_models && !cand.models.is_empty() {
                                        let enable_keys: Vec<String> = cand
                                            .models
                                            .iter()
                                            .map(|mid| format!("{}:{mid}", cand.protocol))
                                            .collect();
                                        let _ = crate::daemon::sync_connection_model_profiles(
                                            store,
                                            suite,
                                            &conn_rec,
                                            &enable_keys,
                                            None,
                                            ts,
                                        );
                                    }

                                    results.push(json!({
                                        "candidate_id": cand.candidate_id,
                                        "connection_id": cand.connection_id,
                                        "status": "imported",
                                        "error": Value::Null
                                    }));
                                }
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
                let meta = resolve_probe_connection_metadata(store, &cfg, conn_id, req);
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
                    let existing_rec = suite
                        .get_record::<Value>("model_connection", conn_id)
                        .ok()
                        .flatten();
                    let meta = resolve_probe_connection_metadata(store, &cfg, conn_id, req);

                    // Obtain API key: prefer credential_read_request handshake when account_id is present & framed
                    let mut api_key = cfg
                        .llm
                        .connections
                        .iter()
                        .find(|c| c.id == conn_id)
                        .and_then(|c| c.api_key.as_deref())
                        .filter(|s| !s.trim().is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let mut cred_error: Option<String> = None;
                    if api_key.is_empty() {
                        if let (Some(ref acct_id), Some(ref rid)) = (&meta.account_id, &request_id) {
                            let nonce = new_id();
                            emit_compact_json_line(&json!({
                                "type": "credential_read_request",
                                "request_id": rid,
                                "nonce": nonce,
                                "connection_id": conn_id,
                                "endpoint": meta.base_url,
                                "protocol": proto,
                                "account_id": acct_id
                            }));
                            let mut cred_line = String::new();
                            match reader.read_line(&mut cred_line) {
                                Ok(0) => {
                                    cred_error =
                                        Some("stdin closed awaiting credential_read".into());
                                }
                                Ok(_) => {
                                    if let Ok(cred_frame) =
                                        serde_json::from_str::<Value>(cred_line.trim())
                                    {
                                        if cred_frame.get("type").and_then(|v| v.as_str())
                                            == Some("credential_read")
                                            && cred_frame.get("nonce").and_then(|v| v.as_str())
                                                == Some(nonce.as_str())
                                        {
                                            api_key = cred_frame
                                                .get("secret")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                        } else {
                                            cred_error = Some(
                                                "invalid credential_read handshake frame".into(),
                                            );
                                        }
                                    } else {
                                        cred_error =
                                            Some("malformed credential_read JSON frame".into());
                                    }
                                }
                                Err(e) => {
                                    cred_error = Some(format!("credential_read IO error: {e}"));
                                }
                            }
                        }
                    }

                    if let Some(err_msg) = cred_error {
                        json!({
                            "type": "error",
                            "request_id": request_id,
                            "error": err_msg
                        })
                    } else if meta.base_url.trim().is_empty() {
                        json!({
                            "type": "error",
                            "request_id": request_id,
                            "error": "missing base_url for connection probe"
                        })
                    } else {
                        match perform_text_generation_probe(
                            conn_id,
                            &meta.base_url,
                            proto,
                            model_id,
                            &api_key,
                        ) {
                            Ok(()) => {
                                let ts = now_ms();
                                let existing_rev = existing_rec
                                    .as_ref()
                                    .and_then(|r| r.get("revision").and_then(|v| v.as_i64()))
                                    .unwrap_or(0);
                                let next_rev = existing_rev.max(expected_rev) + 1;

                                // Preserve ALL existing connection fields & catalog models
                                let mut rec = existing_rec.unwrap_or_else(|| {
                                    json!({
                                        "id": conn_id,
                                        "display_name": meta.display_name,
                                        "base_url": meta.base_url,
                                        "protocols": [proto],
                                        "account_id": meta.account_id,
                                        "source": "nola",
                                        "source_id": conn_id,
                                        "enabled": true
                                    })
                                });
                                let mut models_list: Vec<Value> = rec
                                    .pointer("/catalog/models")
                                    .and_then(|v| v.as_array())
                                    .cloned()
                                    .unwrap_or_default();
                                let has_model = models_list.iter().any(|m| {
                                    m.get("model_id").and_then(|v| v.as_str()) == Some(model_id)
                                        && m.get("protocol")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or(proto)
                                            == proto
                                });
                                if !has_model && !model_id.is_empty() {
                                    models_list.push(json!({
                                        "model_id": model_id,
                                        "display_name": model_id,
                                        "protocol": proto,
                                        "context_window": 128000,
                                        "output_limit": 16384,
                                        "modalities": ["text"],
                                        "reasoning_levels": [],
                                        "summary_supported": Value::Null,
                                        "execution_capabilities": Value::Null
                                    }));
                                }
                                let catalog_source = rec
                                    .pointer("/catalog/source")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("probe")
                                    .to_string();
                                let catalog_val = json!({
                                    "observed_at_ms": ts,
                                    "source": catalog_source,
                                    "models": models_list
                                });

                                let mut assessments: Vec<Value> = rec
                                    .get("capability_assessments")
                                    .and_then(|v| v.as_array())
                                    .cloned()
                                    .unwrap_or_default();
                                let mut existing_override = Value::Null;
                                if let Some(pos) = assessments.iter().position(|a| {
                                    a.get("model_id").and_then(|v| v.as_str()) == Some(model_id)
                                        && a.get("protocol").and_then(|v| v.as_str()) == Some(proto)
                                }) {
                                    existing_override = assessments[pos]
                                        .get("user_override")
                                        .cloned()
                                        .unwrap_or(Value::Null);
                                    assessments.remove(pos);
                                }
                                assessments.push(json!({
                                    "model_id": model_id,
                                    "protocol": proto,
                                    "evidence": {
                                        "text_generation": true,
                                        "account_revision": meta.account_revision,
                                        "credential_binding": cred_binding,
                                        "observed_at_ms": ts
                                    },
                                    "user_override": existing_override
                                }));

                                let existing_protocols: Vec<Value> = rec
                                    .get("protocols")
                                    .and_then(|v| v.as_array())
                                    .filter(|a| !a.is_empty())
                                    .cloned()
                                    .unwrap_or_else(|| vec![json!(proto)]);
                                let conn_source = rec
                                    .get("source")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("nola")
                                    .to_string();
                                let conn_source_id = rec
                                    .get("source_id")
                                    .cloned()
                                    .unwrap_or_else(|| json!(conn_id));
                                let conn_enabled = rec
                                    .get("enabled")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(true);
                                if let Some(obj) = rec.as_object_mut() {
                                    obj.insert("id".into(), json!(conn_id));
                                    obj.insert("display_name".into(), json!(meta.display_name));
                                    obj.insert("base_url".into(), json!(meta.base_url));
                                    obj.insert("protocols".into(), Value::Array(existing_protocols.clone()));
                                    obj.insert("account_id".into(), json!(meta.account_id));
                                    obj.insert("source".into(), json!(conn_source));
                                    obj.insert("source_id".into(), conn_source_id.clone());
                                    obj.insert("enabled".into(), json!(conn_enabled));
                                    obj.insert("revision".into(), json!(next_rev));
                                    obj.insert("updated_at_ms".into(), json!(ts));
                                    obj.insert("catalog".into(), catalog_val.clone());
                                    obj.insert("capability_assessments".into(), json!(assessments));
                                }
                                let _ = suite.upsert_record("model_connection", conn_id, &rec);

                                let snapshot = json!({
                                    "revision": next_rev,
                                    "document": {
                                        "format_version": 1,
                                        "connection": {
                                            "id": conn_id,
                                            "display_name": meta.display_name,
                                            "base_url": meta.base_url,
                                            "protocols": existing_protocols,
                                            "account_id": meta.account_id,
                                            "source": conn_source,
                                            "source_id": conn_source_id,
                                            "enabled": conn_enabled
                                        },
                                        "catalog": catalog_val,
                                        "capability_assessments": assessments
                                    }
                                });
                                json!({
                                    "type": "probe",
                                    "request_id": request_id,
                                    "snapshot": snapshot
                                })
                            }
                            Err(err) => json!({
                                "type": "error",
                                "request_id": request_id,
                                "error": err
                            }),
                        }
                    }
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
                            candidates.push(candidate_to_mac_import_json(&c, None, None));
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

type SiwcCallbackResult = Arc<Mutex<Option<Result<String, String>>>>;

#[derive(Clone)]
struct SiwcFlowState {
    owner_id: String,
    flow_id: String,
    account_id: String,
    expected_account_revision: Option<i64>,
    display_name: String,
    phase: String,
    authorization_url: Option<String>,
    error: Option<String>,
    account: Option<Value>,
    pkce_verifier: String,
    redirect_uri: String,
    callback_result: SiwcCallbackResult,
    callback_shutdown: Arc<AtomicBool>,
}

impl SiwcFlowState {
    fn sync_callback(&mut self) {
        if self.phase != "awaiting_authorization" {
            return;
        }
        let received = self
            .callback_result
            .lock()
            .ok()
            .and_then(|g| g.clone());
        if let Some(res) = received {
            match res {
                Ok(_code) => {
                    self.phase = "authorized".into();
                    self.authorization_url = None;
                    self.error = None;
                }
                Err(code) => {
                    self.phase = "failed".into();
                    self.authorization_url = None;
                    self.error = Some(code);
                }
            }
        }
    }

    fn shutdown_listener(&self) {
        self.callback_shutdown.store(true, Ordering::SeqCst);
    }

    fn to_json(&self) -> Value {
        json!({
            "owner_id": self.owner_id,
            "flow_id": self.flow_id,
            "account_id": self.account_id,
            "expected_account_revision": self.expected_account_revision,
            "phase": self.phase,
            "authorization_url": self.authorization_url,
            "error": self.error,
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
                if let Some(prev) = current_flow.take() {
                    prev.shutdown_listener();
                }
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
                let (verifier, challenge) = match pkce {
                    Some(p) => (p.verifier, p.challenge),
                    None => {
                        let v = new_id();
                        (v.clone(), v)
                    }
                };
                let (port, callback_result, callback_shutdown) =
                    spawn_siwc_callback_listener(state.clone());
                let redirect_uri = format!("http://localhost:{port}/auth/callback");
                let encoded_redirect: String =
                    url::form_urlencoded::byte_serialize(redirect_uri.as_bytes()).collect();
                let auth_url = format!(
                    "https://auth.openai.com/oauth/authorize?response_type=code&client_id=app_EMoamEEZ73f0CkXaXp7hrann&redirect_uri={encoded_redirect}&scope=openid+profile+email+offline_access&code_challenge={challenge}&code_challenge_method=S256&state={state}"
                );
                let flow = SiwcFlowState {
                    owner_id: owner_id.clone(),
                    flow_id: new_id(),
                    account_id,
                    expected_account_revision: expected_rev,
                    display_name,
                    phase: "awaiting_authorization".into(),
                    authorization_url: Some(auth_url),
                    error: None,
                    account: None,
                    pkce_verifier: verifier,
                    redirect_uri,
                    callback_result,
                    callback_shutdown,
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
                if let Some(ref mut flow) = current_flow {
                    flow.sync_callback();
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
                    flow.shutdown_listener();
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
                    flow.sync_callback();
                    flow.shutdown_listener();
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
                        "redirect_uri": flow.redirect_uri,
                        "has_pkce_verifier": !flow.pkce_verifier.is_empty(),
                        "updated_at_ms": now_ms()
                    });
                    let _ = store.set_kv("siwc_account", &meta);
                    let _ = store.set_kv(&format!("siwc_account:{}", flow.account_id), &meta);
                    upsert_siwc_model_account_record(
                        store,
                        &flow.account_id,
                        &flow.display_name,
                        next_rev,
                        true,
                    );
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
                        upsert_siwc_model_account_record(
                            store,
                            &account_id,
                            &display_name,
                            next_rev,
                            false,
                        );
                        let conn_id = format!("siwc-{account_id}");
                        if let Ok(Some(mut existing_conn)) =
                            store.get_module_record::<Value>("model_connection", &conn_id)
                        {
                            let c_rev = existing_conn
                                .get("revision")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(1)
                                + 1;
                            if let Some(obj) = existing_conn.as_object_mut() {
                                obj.insert("enabled".into(), json!(false));
                                obj.insert("revision".into(), json!(c_rev));
                                obj.insert("updated_at_ms".into(), json!(now_ms()));
                            }
                            let _ = store.put_module_record(
                                "model_connection",
                                &conn_id,
                                &existing_conn,
                            );
                        }
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
                        upsert_siwc_model_account_record(
                            store,
                            &account_id,
                            &display_name,
                            stored_rev,
                            has_token,
                        );
                        upsert_siwc_model_connection_record(store, &conn_snap);
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
            let mut redacted = cfg.clone();
            for val in redacted.env.values_mut() {
                if !val.is_empty() {
                    *val = "[REDACTED]".to_string();
                }
            }
            for val in redacted.headers.values_mut() {
                if !val.is_empty() {
                    *val = "[REDACTED]".to_string();
                }
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"ok": true, "server": redacted})).unwrap()
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

fn parse_memory_cli_flags(
    is_revise: bool,
    tail: &[String],
) -> Result<(Option<String>, Option<Value>), String> {
    let mut explicit_id: Option<String> = None;
    let mut map = serde_json::Map::new();
    let mut tags: Vec<String> = Vec::new();
    let mut has_flags = false;
    let mut has_tags = false;

    let mut i = 0;
    while i < tail.len() {
        let arg = tail[i].as_str();
        let (flag, inline_val) = if let Some((f, v)) = arg.split_once('=') {
            (f, Some(v.to_string()))
        } else {
            (arg, None)
        };
        let take_value = |idx: &mut usize, name: &str| -> Result<String, String> {
            if let Some(v) = inline_val.clone() {
                return Ok(v);
            }
            *idx += 1;
            tail.get(*idx)
                .cloned()
                .ok_or_else(|| format!("{name} requires a value"))
        };
        match flag {
            "--id" => {
                let v = take_value(&mut i, "--id")?;
                explicit_id = Some(v.clone());
                map.insert("id".into(), json!(v));
                has_flags = true;
            }
            "--title" => {
                let v = take_value(&mut i, "--title")?;
                map.insert("title".into(), json!(v));
                has_flags = true;
            }
            "--summary" => {
                let v = take_value(&mut i, "--summary")?;
                map.insert("summary".into(), json!(v));
                has_flags = true;
            }
            "--content" => {
                let v = take_value(&mut i, "--content")?;
                map.insert("content".into(), json!(v));
                has_flags = true;
            }
            "--kind" | "--category" => {
                let v = take_value(&mut i, flag)?;
                map.insert("kind".into(), json!(v));
                has_flags = true;
            }
            "--scope" => {
                let v = take_value(&mut i, "--scope")?;
                map.insert("scope".into(), json!(v));
                has_flags = true;
            }
            "--tags" => {
                let v = take_value(&mut i, "--tags")?;
                for part in v.split(',') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        tags.push(trimmed.to_string());
                    }
                }
                has_tags = true;
                has_flags = true;
            }
            "--tag" => {
                let v = take_value(&mut i, "--tag")?;
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    tags.push(trimmed.to_string());
                }
                has_tags = true;
                has_flags = true;
            }
            "--confidence" => {
                let v = take_value(&mut i, "--confidence")?;
                let n: f64 = v
                    .parse()
                    .map_err(|_| "--confidence must be a valid number".to_string())?;
                map.insert("confidence".into(), json!(n));
                has_flags = true;
            }
            "--importance" => {
                let v = take_value(&mut i, "--importance")?;
                let n: f64 = v
                    .parse()
                    .map_err(|_| "--importance must be a valid number".to_string())?;
                map.insert("importance".into(), json!(n));
                has_flags = true;
            }
            "--sensitivity" => {
                let v = take_value(&mut i, "--sensitivity")?;
                map.insert("sensitivity".into(), json!(v));
                has_flags = true;
            }
            "--expires-at" => {
                let v = take_value(&mut i, "--expires-at")?;
                map.insert("expires_at".into(), json!(v));
                has_flags = true;
            }
            "--source" => {
                let v = take_value(&mut i, "--source")?;
                map.insert("source".into(), json!(v));
                has_flags = true;
            }
            "--provenance-kind" => {
                let v = take_value(&mut i, "--provenance-kind")?;
                map.insert("provenance_kind".into(), json!(v));
                has_flags = true;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown memory flag '{other}'"));
            }
            positional => {
                if is_revise && explicit_id.is_none() {
                    explicit_id = Some(positional.to_string());
                } else {
                    return Err(format!("unexpected memory argument '{positional}'"));
                }
            }
        }
        i += 1;
    }

    if has_tags {
        map.insert("tags".into(), json!(tags));
    }
    Ok((explicit_id, has_flags.then_some(Value::Object(map))))
}

fn handle_memory_cmd(store: &Store, args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let items = store.search_memory("", None, 200).map_err(|e| e.to_string())?;
            let records: Vec<Value> = items
                .iter()
                .map(|item| crate::web_api::memory_item_to_record_with_store(store, item))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"records": records})).unwrap()
            );
            Ok(0)
        }
        Some("create") | Some("add") | Some("revise") => {
            let is_revise = args.first().map(String::as_str) == Some("revise");
            let usage = if is_revise {
                "usage: nola memory revise <id> [--title <title>] [--content <content>] [--summary <summary>] [--kind <kind>] [--scope <scope>] [--tags <a,b>] (or pipe JSON on stdin)"
            } else {
                "usage: nola memory create --title <title> --content <content> [--summary <summary>] [--kind <kind>] [--scope <scope>] [--tags <a,b>] [--id <id>] (or pipe JSON on stdin)"
            };
            let (explicit_id, flag_draft) = parse_memory_cli_flags(is_revise, &args[1..])?;
            let draft: Value = if let Some(d) = flag_draft {
                d
            } else {
                if io::stdin().is_terminal() {
                    return Err(usage.to_string());
                }
                let mut raw = String::new();
                io::stdin()
                    .read_to_string(&mut raw)
                    .map_err(|e| format!("failed to read memory JSON from stdin: {e}"))?;
                if raw.trim().is_empty() {
                    return Err(usage.to_string());
                }
                serde_json::from_str(raw.trim())
                    .map_err(|e| format!("invalid memory JSON: {e}"))?
            };
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
                id: id.clone(),
                namespace: format!("{scope}:{kind}"),
                title,
                content,
                tags,
                created_at_ms,
                updated_at_ms: ts,
            };
            store.upsert_memory(&item).map_err(|e| e.to_string())?;

            // Merge & persist extended memory metadata (`memory_meta:{id}`)
            let meta_key = format!("memory_meta:{id}");
            let mut meta = store
                .get_kv(&meta_key)
                .ok()
                .flatten()
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            for field in [
                "confidence",
                "importance",
                "sensitivity",
                "expires_at",
                "source",
                "provenance_kind",
            ] {
                if let Some(val) = draft.get(field) {
                    if !val.is_null() {
                        meta.insert(field.to_string(), val.clone());
                    }
                }
            }
            if let Some(prov_kind) = draft
                .pointer("/provenance/kind")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                meta.entry("provenance_kind".to_string())
                    .or_insert_with(|| json!(prov_kind));
            }
            let mut supersedes: Vec<String> = meta
                .get("supersedes")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(draft_sup) = draft.get("supersedes").and_then(|v| v.as_array()) {
                for s in draft_sup.iter().filter_map(|v| v.as_str()) {
                    if !s.trim().is_empty() && !supersedes.iter().any(|x| x == s) {
                        supersedes.push(s.to_string());
                    }
                }
            }
            if is_revise && !supersedes.iter().any(|x| x == &id) {
                supersedes.push(id.clone());
            }
            if !supersedes.is_empty() {
                meta.insert("supersedes".into(), json!(supersedes));
            }
            if !meta.is_empty() {
                let _ = store.set_kv(&meta_key, &Value::Object(meta));
            }

            let record = crate::web_api::memory_item_to_record_with_store(store, &item);
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

fn handle_host_goal_control_request(
    store: &Store,
    suite: &ModulesSuite,
    req: &Value,
) -> Result<Value, String> {
    let payload = req.get("payload").unwrap_or(req);
    let goal_id = payload
        .get("goal_id")
        .or_else(|| payload.get("goalId"))
        .or_else(|| payload.get("id"))
        .or_else(|| req.get("goal_id"))
        .or_else(|| req.get("goalId"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "goal_control missing 'goal_id'".to_string())?;
    let action = payload
        .get("action")
        .or_else(|| req.get("action"))
        .and_then(|v| v.as_str())
        .unwrap_or("status")
        .trim()
        .to_ascii_lowercase();
    let ts = now_ms();
    let mut goal = suite
        .get_record::<ExtendedGoalRecord>("goal", goal_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("goal '{goal_id}' not found"))?;
    match action.as_str() {
        "start" | "resume" => {
            goal.status = GoalStatus::Running;
            goal.updated_at_ms = ts;
        }
        "pause" => {
            goal.status = GoalStatus::Paused;
            goal.updated_at_ms = ts;
        }
        "cancel" | "stop" => {
            goal.status = GoalStatus::Failed;
            goal.updated_at_ms = ts;
            let _ = store.set_kv(
                &format!("cancel_requested:goal_{goal_id}"),
                &json!({"requested_at_ms": ts, "action": "cancel"}),
            );
        }
        "schedule_resume" => {
            let resume_at = payload
                .get("resume_at_ms")
                .or_else(|| payload.get("scheduled_for_ms"))
                .or_else(|| req.get("resume_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| ts + 60_000);
            goal.status = GoalStatus::Paused;
            goal.updated_at_ms = ts;
            let _ = store.set_kv(
                &format!("goal_scheduled_resume:{goal_id}"),
                &json!({"goal_id": goal_id, "resume_at_ms": resume_at, "scheduled_at_ms": ts}),
            );
        }
        other => {
            return Err(format!(
                "unsupported goal_control action '{other}' (expected start|pause|resume|cancel|schedule_resume)"
            ));
        }
    }
    suite
        .upsert_record("goal", goal_id, &goal)
        .map_err(|e| e.to_string())?;
    let status_str = match goal.status {
        GoalStatus::Pending => "pending",
        GoalStatus::Running => "running",
        GoalStatus::Paused => "paused",
        GoalStatus::Completed => "completed",
        GoalStatus::Failed => "failed",
    };
    Ok(json!({
        "ok": true,
        "operation": "goal_control",
        "goal_id": goal_id,
        "action": action,
        "status": status_str,
        "updated_at_ms": ts
    }))
}

fn upsert_model_connection_module_record(
    store: &Store,
    cfg: &AppliedConfig,
    conn_id: &str,
    source: &str,
) -> Result<(), String> {
    let Some(conn) = cfg.llm.connections.iter().find(|c| c.id == conn_id) else {
        return Ok(());
    };
    let existing = store
        .get_module_record::<Value>("model_connection", conn_id)
        .ok()
        .flatten();
    let prev_rev = existing
        .as_ref()
        .and_then(|r| r.get("revision").and_then(|v| v.as_i64()))
        .unwrap_or(0);
    let display_name = cfg
        .connection_extras
        .get(conn_id)
        .and_then(|e| e.name.as_deref())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            existing
                .as_ref()
                .and_then(|r| r.get("display_name").and_then(|v| v.as_str()))
        })
        .unwrap_or(conn_id)
        .to_string();
    let raw_proto = conn
        .protocol
        .as_deref()
        .or(conn.provider.as_deref())
        .unwrap_or("chat_completions");
    let canonical_proto = crate::daemon::normalize_wire_protocol(raw_proto);
    let ts = now_ms();
    let catalog_models: Vec<Value> = conn
        .models
        .iter()
        .map(|m| {
            let mname = cfg
                .connection_extras
                .get(conn_id)
                .and_then(|ex| ex.model_names.get(&m.id))
                .cloned()
                .unwrap_or_else(|| m.id.clone());
            json!({
                "model_id": m.id,
                "display_name": mname,
                "protocol": canonical_proto,
                "context_window": m.context_window,
                "output_limit": m.max_output_tokens,
                "modalities": ["text"],
                "reasoning_levels": [],
                "summary_supported": Value::Null,
                "execution_capabilities": Value::Null
            })
        })
        .collect();
    let account_id = existing
        .as_ref()
        .and_then(|r| r.get("account_id").cloned())
        .unwrap_or(Value::Null);
    let assessments = existing
        .as_ref()
        .and_then(|r| r.get("capability_assessments").cloned())
        .unwrap_or_else(|| json!([]));
    let rec = json!({
        "id": conn_id,
        "display_name": display_name,
        "base_url": conn.endpoint,
        "protocols": [canonical_proto],
        "account_id": account_id,
        "source": source,
        "source_id": conn_id,
        "enabled": true,
        "revision": prev_rev + 1,
        "updated_at_ms": ts,
        "catalog": {
            "observed_at_ms": ts,
            "source": source,
            "models": catalog_models
        },
        "capability_assessments": assessments
    });
    store
        .put_module_record("model_connection", conn_id, &rec)
        .map_err(|e| e.to_string())
}

fn resolve_local_models_root() -> PathBuf {
    std::env::var("NOLA_MODELS_ROOT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if std::env::consts::OS == "macos" {
                default_home_dir().join("Library/Caches/models")
            } else {
                default_home_dir().join(".nola/models")
            }
        })
}

struct MacLocalCatalogRecordInput<'a> {
    id: &'a str,
    name: &'a str,
    repository: &'a str,
    runtime: &'a str,
    format: &'a str,
    quantization: Option<&'a str>,
    context_window_tokens: i64,
    recommended_device: &'a str,
    snapshot_path: &'a Path,
    artifact_path: Option<&'a Path>,
    artifact_bytes: i64,
}

fn write_local_snapshot_manifest(
    snapshot_dir: &Path,
    artifacts: &[(String, u64)],
    total_bytes: u64,
) -> Result<(), String> {
    fs::create_dir_all(snapshot_dir).map_err(|e| e.to_string())?;
    let items: Vec<Value> = artifacts
        .iter()
        .map(|(p, b)| json!({ "path": p, "bytes": *b as i64 }))
        .collect();
    let manifest = json!({
        "schemaVersion": 1,
        "totalBytes": total_bytes as i64,
        "artifacts": items
    });
    let manifest_path = snapshot_dir.join("snapshot.json");
    let raw = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(&manifest_path, raw).map_err(|e| format!("failed writing snapshot.json: {e}"))
}

fn upsert_mac_local_model_catalog_entry(
    models_root: &Path,
    entry: &MacLocalCatalogRecordInput<'_>,
) -> Result<(), String> {
    fs::create_dir_all(models_root).map_err(|e| e.to_string())?;
    let catalog_path = models_root.join("catalog.json");
    let mut models: Vec<Value> = fs::read_to_string(&catalog_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.get("models").and_then(|m| m.as_array()).cloned())
        .unwrap_or_default();
    let peak_mem = (entry.artifact_bytes as f64 * 1.4) as i64;
    let rec_mem = (entry.artifact_bytes.saturating_mul(2)).max(peak_mem);
    let record = json!({
        "id": entry.id,
        "name": entry.name,
        "repository": entry.repository,
        "runtime": entry.runtime,
        "format": entry.format,
        "quantization": entry.quantization,
        "contextWindowTokens": entry.context_window_tokens,
        "recommendedContextTokens": entry.context_window_tokens,
        "recommendedDevice": entry.recommended_device,
        "snapshotPath": entry.snapshot_path.display().to_string(),
        "artifactPath": entry.artifact_path.map(|p| p.display().to_string()),
        "artifactBytes": entry.artifact_bytes,
        "estimatedPeakMemoryBytes": peak_mem,
        "recommendedMemoryBytes": rec_mem,
        "state": "configured",
        "enabled": true
    });
    models.retain(|m| {
        m.get("id").and_then(|v| v.as_str()) != Some(entry.id)
            && m.get("snapshotPath").and_then(|v| v.as_str())
                != Some(&entry.snapshot_path.display().to_string())
    });
    models.push(record);
    let doc = json!({
        "schemaVersion": 2,
        "models": models
    });
    let raw = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    fs::write(&catalog_path, raw).map_err(|e| format!("failed writing catalog.json: {e}"))
}

fn remove_mac_local_model_catalog_entry(
    models_root: &Path,
    id: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let catalog_path = models_root.join("catalog.json");
    if !catalog_path.exists() {
        return Ok(());
    }
    let Ok(raw) = fs::read_to_string(&catalog_path) else {
        return Ok(());
    };
    let Ok(mut doc) = serde_json::from_str::<Value>(&raw) else {
        return Ok(());
    };
    if let Some(arr) = doc.get_mut("models").and_then(|v| v.as_array_mut()) {
        arr.retain(|m| {
            let mid = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let mname = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let mrepo = m.get("repository").and_then(|v| v.as_str()).unwrap_or("");
            mid != id
                && mname != id
                && mrepo != id
                && name.is_none_or(|n| mname != n && mrepo != n)
        });
    }
    let updated = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    fs::write(&catalog_path, updated).map_err(|e| format!("failed updating catalog.json: {e}"))
}

fn is_mlx_snapshot_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower == "config.json"
        || lower == "tokenizer.json"
        || lower == "tokenizer_config.json"
        || lower == "special_tokens_map.json"
        || lower == "generation_config.json"
        || lower == "chat_template.jinja"
        || lower == "vocab.json"
        || lower == "merges.txt"
        || lower == "model.safetensors.index.json"
        || lower.ends_with(".safetensors")
        || lower.ends_with(".model")
}

fn download_mlx_repo_snapshot(
    source: &str,
    repo: &str,
    rev: &str,
    plan_json: &Value,
    snapshot_dir: &Path,
) -> Result<(Vec<(String, u64)>, u64), String> {
    fs::create_dir_all(snapshot_dir).map_err(|e| e.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("Nola/2.0")
        .build()
        .map_err(|e| e.to_string())?;

    // Build file list: either explicit plan.files (excluding fabricated .gguf) or HF/mirror tree API
    let mut files_to_fetch: Vec<(String, String, Option<u64>)> = Vec::new();
    if let Some(arr) = plan_json.get("files").and_then(|v| v.as_array()) {
        for f in arr {
            if let Some(rel) = f.get("path").and_then(|v| v.as_str()) {
                if rel.to_ascii_lowercase().ends_with(".gguf") {
                    continue;
                }
                let url = f
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| build_hub_download_url(source, repo, rev, rel));
                let sz = f.get("bytes").and_then(|v| v.as_u64());
                files_to_fetch.push((rel.to_string(), url, sz));
            }
        }
    }

    if files_to_fetch.is_empty() {
        let api_base = if source.eq_ignore_ascii_case("modelscope") {
            "https://hf-mirror.com"
        } else {
            "https://huggingface.co"
        };
        let tree_url = format!("{api_base}/api/models/{repo}/tree/{rev}");
        let resp = client
            .get(&tree_url)
            .send()
            .map_err(|e| format!("failed fetching MLX file tree from {tree_url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "failed fetching MLX file tree from {tree_url}: HTTP {}",
                resp.status()
            ));
        }
        let entries: Value = resp
            .json()
            .map_err(|e| format!("invalid MLX file tree JSON: {e}"))?;
        if let Some(arr) = entries.as_array() {
            for entry in arr {
                let kind = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if kind != "file" {
                    continue;
                }
                let Some(rel) = entry.get("path").and_then(|v| v.as_str()) else {
                    continue;
                };
                if rel.contains("..") || rel.starts_with('/') || !is_mlx_snapshot_file(rel) {
                    continue;
                }
                let sz = entry
                    .get("size")
                    .or_else(|| entry.pointer("/lfs/size"))
                    .and_then(|v| v.as_u64());
                let url = build_hub_download_url(source, repo, rev, rel);
                files_to_fetch.push((rel.to_string(), url, sz));
            }
        }
    }

    if files_to_fetch.is_empty() {
        return Err(format!(
            "no MLX snapshot files found for repository '{repo}' (revision '{rev}')"
        ));
    }

    let expected_total: u64 = files_to_fetch.iter().filter_map(|(_, _, sz)| *sz).sum();
    let mut downloaded_total = 0u64;
    let mut artifacts: Vec<(String, u64)> = Vec::new();
    emit_compact_json_line(&json!({
        "downloadedBytes": 0,
        "totalBytes": if expected_total > 0 { json!(expected_total) } else { Value::Null },
        "status": "downloading"
    }));

    for (rel, url, _) in files_to_fetch {
        let target = snapshot_dir.join(&rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut resp = client
            .get(&url)
            .send()
            .map_err(|e| format!("failed downloading MLX file {rel}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "failed downloading MLX file {rel}: HTTP {}",
                resp.status()
            ));
        }
        let mut file = fs::File::create(&target)
            .map_err(|e| format!("failed creating {}: {e}", target.display()))?;
        let mut buf = [0u8; 64 * 1024];
        let mut file_bytes = 0u64;
        loop {
            let n = resp
                .read(&mut buf)
                .map_err(|e| format!("failed reading MLX file {rel}: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("failed writing MLX file {rel}: {e}"))?;
            file_bytes += n as u64;
            downloaded_total += n as u64;
        }
        artifacts.push((rel, file_bytes));
        emit_compact_json_line(&json!({
            "downloadedBytes": downloaded_total,
            "totalBytes": if expected_total > 0 { json!(expected_total.max(downloaded_total)) } else { json!(downloaded_total) },
            "status": "downloading"
        }));
    }

    Ok((artifacts, downloaded_total))
}

fn perform_text_generation_probe(
    conn_id: &str,
    base_url: &str,
    proto_str: &str,
    model_id: &str,
    api_key: &str,
) -> Result<(), String> {
    let protocol = ModelProtocol::parse(proto_str).unwrap_or(ModelProtocol::OpenAiChat);
    let route = ModelRouteConfig {
        connection_id: conn_id.to_string(),
        endpoint: base_url.to_string(),
        protocol,
        api_key: api_key.to_string(),
        model: model_id.to_string(),
        context_window: 32_768,
        max_output_tokens: Some(8),
        temperature: Some(0.0),
        top_p: None,
        reasoning_effort: None,
        prompt_cache_key: None,
        capabilities: nola_types::ModelCapabilities::default(),
    };
    let caller = HttpModelCaller::new(15).map_err(|e| e.to_string())?;
    let req = nola_types::StructuredModelRequest {
        frozen_system_prefix: vec![nola_types::FrozenSystemBlock {
            id: "probe".to_string(),
            text: "Reply with OK.".to_string(),
        }],
        prefix_stable_tools: Vec::new(),
        history: vec![nola_types::HistoryMessage::User {
            content: "ping".to_string(),
            media: Vec::new(),
        }],
        ephemeral_tail_reminders: Vec::new(),
        max_output_tokens: 8,
        json_schema: None,
    };
    let mut noop = |_delta: nola_types::ModelStreamDelta| {};
    caller
        .call(&route, &req, false, &mut noop)
        .map(|_| ())
        .map_err(|e| format!("text generation probe failed: {e}"))
}

fn spawn_siwc_callback_listener(
    expected_state: String,
) -> (u16, SiwcCallbackResult, Arc<AtomicBool>) {
    let result = Arc::new(Mutex::new(None));
    let shutdown = Arc::new(AtomicBool::new(false));
    let listener = std::net::TcpListener::bind("127.0.0.1:1455")
        .or_else(|_| std::net::TcpListener::bind("127.0.0.1:0"));
    let Ok(listener) = listener else {
        return (1455, result, shutdown);
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(1455);
    let _ = listener.set_nonblocking(true);
    let res_clone = Arc::clone(&result);
    let stop_clone = Arc::clone(&shutdown);
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(300);
        while !stop_clone.load(Ordering::SeqCst) && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut buf = [0u8; 4096];
                    if let Ok(n) = stream.read(&mut buf) {
                        let req_str = String::from_utf8_lossy(&buf[..n]);
                        let first_line = req_str.lines().next().unwrap_or("");
                        let path = first_line.split_whitespace().nth(1).unwrap_or("/");
                        if path.starts_with("/auth/callback") {
                            let parsed_url =
                                url::Url::parse(&format!("http://localhost{path}")).ok();
                            let mut code: Option<String> = None;
                            let mut state: Option<String> = None;
                            let mut err: Option<String> = None;
                            if let Some(u) = parsed_url {
                                for (k, v) in u.query_pairs() {
                                    match k.as_ref() {
                                        "code" => code = Some(v.into_owned()),
                                        "state" => state = Some(v.into_owned()),
                                        "error" => err = Some(v.into_owned()),
                                        _ => {}
                                    }
                                }
                            }
                            let (status_line, body, outcome) = if let Some(e) = err {
                                let mapped = if e == "access_denied" {
                                    "native_siwc_authorization_declined".to_string()
                                } else {
                                    format!("native_siwc_authorization_declined:{e}")
                                };
                                (
                                    "HTTP/1.1 200 OK",
                                    "<html><body><h3>Authorization declined. You may close this tab.</h3></body></html>",
                                    Err(mapped),
                                )
                            } else if state.as_deref() != Some(expected_state.as_str()) {
                                (
                                    "HTTP/1.1 400 Bad Request",
                                    "<html><body><h3>Invalid OAuth state.</h3></body></html>",
                                    Err("native_siwc_authorization_expired".to_string()),
                                )
                            } else if let Some(c) = code.filter(|s| !s.trim().is_empty()) {
                                (
                                    "HTTP/1.1 200 OK",
                                    "<html><body><h3>Sign-in complete! You may close this tab and return to Nola.</h3></body></html>",
                                    Ok(c),
                                )
                            } else {
                                (
                                    "HTTP/1.1 400 Bad Request",
                                    "<html><body><h3>Missing authorization code.</h3></body></html>",
                                    Err("native_siwc_authorization_expired".to_string()),
                                )
                            };
                            let resp = format!(
                                "{status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.flush();
                            if let Ok(mut g) = res_clone.lock() {
                                *g = Some(outcome);
                            }
                            break;
                        } else {
                            let _ = stream.write_all(
                                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            );
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => break,
            }
        }
    });
    (port, result, shutdown)
}

fn upsert_siwc_model_account_record(
    store: &Store,
    account_id: &str,
    display_name: &str,
    revision: i64,
    connected: bool,
) {
    let rec = json!({
        "id": account_id,
        "account_id": account_id,
        "display_name": if display_name.trim().is_empty() { "ChatGPT" } else { display_name },
        "source": "nola_chatgpt",
        "credential_ref": if connected { json!(format!("siwc:{account_id}")) } else { Value::Null },
        "state": if connected { "ready" } else { "disconnected" },
        "revision": revision,
        "updated_at_ms": now_ms()
    });
    let _ = store.put_module_record("model_account", account_id, &rec);
}

fn upsert_siwc_model_connection_record(store: &Store, conn_snap: &Value) {
    let Some(conn) = conn_snap.pointer("/document/connection") else {
        return;
    };
    let Some(conn_id) = conn.get("id").and_then(|v| v.as_str()) else {
        return;
    };
    let rev = conn_snap
        .get("revision")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let mut rec = conn.clone();
    if let Some(obj) = rec.as_object_mut() {
        obj.insert("revision".into(), json!(rev));
        obj.insert("updated_at_ms".into(), json!(now_ms()));
        if let Some(cat) = conn_snap.pointer("/document/catalog") {
            obj.insert("catalog".into(), cat.clone());
        }
        if let Some(ass) = conn_snap.pointer("/document/capability_assessments") {
            obj.insert("capability_assessments".into(), ass.clone());
        }
    }
    let _ = store.put_module_record("model_connection", conn_id, &rec);
}

const MAX_HARNESS_PLUGIN_HTML_BYTES: usize = 2 * 1024 * 1024;

struct ScannedHarnessProfile {
    runtime: String,
    profile: String,
    packages: Vec<Value>,
}

fn scan_harness_desktop_plugins(db_path: &Path) -> (Vec<Value>, Vec<ScannedHarnessProfile>) {
    let mut pages = Vec::new();
    let mut profiles = Vec::new();
    let harness_root = db_path.with_extension("plugins").join("harness");
    let Ok(runtime_entries) = fs::read_dir(&harness_root) else {
        return (pages, profiles);
    };

    for rt_entry in runtime_entries.flatten() {
        let rt_dir = rt_entry.path();
        if !rt_dir.is_dir() {
            continue;
        }
        let runtime = rt_entry.file_name().to_string_lossy().to_string();
        let profiles_dir = rt_dir.join("profile").join("profiles");
        let Ok(prof_entries) = fs::read_dir(&profiles_dir) else {
            continue;
        };
        for prof_entry in prof_entries.flatten() {
            let prof_dir = prof_entry.path();
            if !prof_dir.is_dir() {
                continue;
            }
            let profile = prof_entry.file_name().to_string_lossy().to_string();
            let inventory_path = prof_dir.join("package.json");
            let Ok(inv_raw) = fs::read_to_string(&inventory_path) else {
                continue;
            };
            let Ok(inv_json) = serde_json::from_str::<Value>(&inv_raw) else {
                continue;
            };
            let mut dep_names: Vec<String> = Vec::new();
            for field in ["dependencies", "optionalDependencies"] {
                if let Some(map) = inv_json.get(field).and_then(|v| v.as_object()) {
                    for name in map.keys() {
                        if !dep_names.iter().any(|existing| existing == name) {
                            dep_names.push(name.clone());
                        }
                    }
                }
            }
            dep_names.sort();

            let mut prof_packages = Vec::new();
            for dep in dep_names {
                if dep.is_empty()
                    || dep.contains("..")
                    || dep.starts_with('/')
                    || dep.starts_with('\\')
                {
                    continue;
                }
                let pkg_dir = prof_dir.join("node_modules").join(&dep);
                let pkg_manifest_path = pkg_dir.join("package.json");
                let Ok(pkg_raw) = fs::read_to_string(&pkg_manifest_path) else {
                    continue;
                };
                let Ok(pkg_json) = serde_json::from_str::<Value>(&pkg_raw) else {
                    continue;
                };
                let pkg_name = pkg_json
                    .get("name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(&dep)
                    .to_string();
                let raw_pages = pkg_json
                    .pointer("/nola/pages")
                    .or_else(|| pkg_json.get("pages"))
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut pkg_page_refs = Vec::new();
                for item in raw_pages {
                    let Some(raw_id) = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    let title = item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(raw_id);
                    let html = if let Some(inline) = item.get("html").and_then(|v| v.as_str()) {
                        inline.to_string()
                    } else if let Some(rel_file) = item
                        .get("entry")
                        .or_else(|| item.get("html_path"))
                        .or_else(|| item.get("file"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        let candidate = pkg_dir.join(rel_file);
                        if nola_tools::file_pipeline::path_within_workspace(&pkg_dir, &candidate) {
                            fs::read_to_string(&candidate)
                                .ok()
                                .filter(|s| s.len() <= MAX_HARNESS_PLUGIN_HTML_BYTES)
                                .unwrap_or_default()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    let page_id = format!("{runtime}:{profile}:{pkg_name}:{raw_id}");
                    pages.push(json!({
                        "id": page_id,
                        "title": title,
                        "package": pkg_name,
                        "html": html
                    }));
                    pkg_page_refs.push(json!({
                        "id": page_id,
                        "title": title
                    }));
                }
                let links = pkg_json
                    .pointer("/nola/links")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                prof_packages.push(json!({
                    "source": pkg_name,
                    "pages": pkg_page_refs,
                    "links": links
                }));
            }
            profiles.push(ScannedHarnessProfile {
                runtime: runtime.clone(),
                profile,
                packages: prof_packages,
            });
        }
    }

    pages.sort_by(|a, b| {
        a.get("id")
            .and_then(|v| v.as_str())
            .cmp(&b.get("id").and_then(|v| v.as_str()))
    });
    (pages, profiles)
}

fn collect_desktop_plugin_pages(workspace_root: &Path, db_path: &Path) -> (Vec<Value>, Vec<ScannedHarnessProfile>) {
    let mut pages = list_desktop_plugin_pages(workspace_root);
    let (harness_pages, harness_profiles) = scan_harness_desktop_plugins(db_path);
    for page in harness_pages {
        let id = page.get("id").and_then(|v| v.as_str());
        if !pages.iter().any(|p| p.get("id").and_then(|v| v.as_str()) == id) {
            pages.push(page);
        }
    }
    pages.sort_by(|a, b| {
        a.get("id")
            .and_then(|v| v.as_str())
            .cmp(&b.get("id").and_then(|v| v.as_str()))
    });
    (pages, harness_profiles)
}

fn merge_harness_profiles(catalog_profiles: Value, scanned: Vec<ScannedHarnessProfile>) -> Value {
    let mut profiles = catalog_profiles.as_array().cloned().unwrap_or_default();
    for sp in scanned {
        if let Some(existing) = profiles.iter_mut().find(|p| {
            p.get("runtime").and_then(|v| v.as_str()) == Some(sp.runtime.as_str())
                && p.get("profile").and_then(|v| v.as_str()) == Some(sp.profile.as_str())
        }) {
            if let Some(pkgs) = existing.get_mut("packages").and_then(|v| v.as_array_mut()) {
                for pkg in sp.packages {
                    let src = pkg.get("source").and_then(|v| v.as_str());
                    if !pkgs.iter().any(|e| e.get("source").and_then(|v| v.as_str()) == src) {
                        pkgs.push(pkg);
                    }
                }
            }
        } else {
            profiles.push(json!({
                "runtime": sp.runtime,
                "profile": sp.profile,
                "available": true,
                "packages": sp.packages
            }));
        }
    }
    Value::Array(profiles)
}

fn handle_plugin_cmd(
    store: &Store,
    suite: &ModulesSuite,
    db_path: &Path,
    args: &[String],
) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let catalog = list_harness_plugins(Some(store), &suite.workspace_root);
            println!("{}", serde_json::to_string_pretty(&catalog).unwrap());
            Ok(0)
        }
        Some("desktop-pages") => {
            let (pages, _) = collect_desktop_plugin_pages(&suite.workspace_root, db_path);
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
            let (pages, scanned_profiles) =
                collect_desktop_plugin_pages(&suite.workspace_root, db_path);
            let profiles = merge_harness_profiles(
                catalog.get("profiles").cloned().unwrap_or_else(|| json!([])),
                scanned_profiles,
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "plugins": catalog.get("plugins").cloned().unwrap_or_else(|| json!([])),
                    "profiles": profiles,
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
                        return handle_plugin_cmd(store, suite, db_path, sub_args);
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
            let (pages, _) = collect_desktop_plugin_pages(&suite.workspace_root, db_path);
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
           nola memory [list | create [--title <t>] [--content <c>] [--summary <s>] [--kind <k>] [--scope <s>] [--tags <a,b>] [--id <id>] | revise <id> [flags] | delete <id>]\n  \
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
mod tests;
