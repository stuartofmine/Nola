use chrono::{DateTime, Local};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crate::terminal_input::read;
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
#[cfg(debug_assertions)]
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::IsTerminal;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

use agent::{render_markdown_text, Emitter};
use config::{print_help, CommandMode, Config};
use crate::native_host_request::validate_desktop_resume_identity;
use crate::session_projection_output::session_show_json_value;
use core_agent_runtime::{
    compact_agent_core_session,
    session_message_to_agent_items,
};
use i18n::{current_language, format_error, tr, ui_text, Language, Message, UiText};
use nola_core::agent::{GoalRuntimeCommand, GoalStatus};

use serde_json::{json, Value};
use unicode_width::UnicodeWidthChar;

// Respect the terminal's light/dark and high-contrast palette. Hierarchy is
// carried by layout and labels, not fixed pale text on an unknown background.
const INTERACTIVE_DIM: &str = "\x1b[22;39m";
const INTERACTIVE_LABEL: &str = "\x1b[22;39m";
const INTERACTIVE_VALUE: &str = "\x1b[22;39m";
const INTERACTIVE_ACCENT: &str = "\x1b[1;38;5;108m";
const INTERACTIVE_COMPOSER_BG: &str = "\x1b[48;5;236m";
const INTERACTIVE_SELECTION_BG: &str = "\x1b[7m";
const INTERACTIVE_RESET: &str = "\x1b[0m";
const INTERACTIVE_LABEL_WIDTH: usize = 11;
const INTERACTIVE_PROMPT_PREFIX: &str = "  ";
const INTERACTIVE_PANEL_TOP_LEFT: &str = "╭";
const INTERACTIVE_PANEL_TOP_RIGHT: &str = "╮";
const INTERACTIVE_PANEL_BOTTOM_LEFT: &str = "╰";

#[cfg(debug_assertions)]
struct TestBudgetAllocator;

#[cfg(debug_assertions)]
static TEST_ALLOCATION_LIMIT: AtomicUsize = AtomicUsize::new(0);
#[cfg(debug_assertions)]
static TEST_LIVE_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

#[cfg(debug_assertions)]
fn reserve_test_allocation(size: usize) -> bool {
    loop {
        let current = TEST_LIVE_ALLOCATIONS.load(Ordering::Relaxed);
        let Some(next) = current.checked_add(size) else {
            return false;
        };
        let limit = TEST_ALLOCATION_LIMIT.load(Ordering::Relaxed);
        if limit != 0 && next > limit {
            return false;
        }
        if TEST_LIVE_ALLOCATIONS
            .compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return true;
        }
    }
}

#[cfg(debug_assertions)]
fn release_test_allocation(size: usize) {
    TEST_LIVE_ALLOCATIONS.fetch_sub(size, Ordering::Relaxed);
}

#[cfg(debug_assertions)]
unsafe impl GlobalAlloc for TestBudgetAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !reserve_test_allocation(layout.size()) {
            return std::ptr::null_mut();
        }
        let pointer = unsafe { System.alloc(layout) };
        if pointer.is_null() {
            release_test_allocation(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !reserve_test_allocation(layout.size()) {
            return std::ptr::null_mut();
        }
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if pointer.is_null() {
            release_test_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        release_test_allocation(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() && !reserve_test_allocation(new_size - layout.size()) {
            return std::ptr::null_mut();
        }
        let resized = unsafe { System.realloc(pointer, layout, new_size) };
        if resized.is_null() {
            if new_size > layout.size() {
                release_test_allocation(new_size - layout.size());
            }
        } else if new_size < layout.size() {
            release_test_allocation(layout.size() - new_size);
        }
        resized
    }
}

#[cfg(debug_assertions)]
#[global_allocator]
static GLOBAL_ALLOCATOR: TestBudgetAllocator = TestBudgetAllocator;

#[cfg(debug_assertions)]
fn configure_test_allocation_budget() {
    let Some(raw_limit) = std::env::var_os("NOLA_TEST_ALLOCATION_LIMIT_BYTES") else {
        return;
    };
    let Some(limit) = raw_limit
        .to_str()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
    else {
        eprintln!("invalid NOLA_TEST_ALLOCATION_LIMIT_BYTES");
        process::exit(64);
    };
    let live = TEST_LIVE_ALLOCATIONS.load(Ordering::Relaxed);
    if live > limit {
        eprintln!("allocation budget is below current live allocations");
        process::exit(70);
    }
    TEST_ALLOCATION_LIMIT.store(limit, Ordering::Relaxed);
}
const INTERACTIVE_PANEL_BOTTOM_RIGHT: &str = "╯";
const INTERACTIVE_PANEL_SIDE: &str = "│";
const INTERACTIVE_PANEL_BORDER: &str = "─";
const RESUME_SESSION_FIRST_MESSAGE_PREVIEW_LIMIT: usize = 10;
const RESUME_SESSION_LIST_LIMIT: usize = 128;
const INTERACTIVE_REPLAY_NOTICE_THRESHOLD: usize = 96;

#[derive(Clone)]
struct InteractiveProviderSnapshot {
    endpoint: String,
    api_key: String,
    model: String,
    runtime_endpoint: Option<String>,
    runtime_api_key: Option<String>,
    runtime_model: Option<String>,
    runtime_provider: Option<String>,
    context_window_tokens: Option<usize>,
}

static INTERACTIVE_PROVIDER_BEFORE_LOCAL: OnceLock<Mutex<Option<InteractiveProviderSnapshot>>> =
    OnceLock::new();

#[derive(Debug, Clone)]
struct InteractivePastedContent {
    placeholder: String,
    content: String,
}

thread_local! {
    static INTERACTIVE_COMPOSER_CURSOR_ROW: Cell<usize> = const { Cell::new(0) };
    static INTERACTIVE_COMPOSER_FRAME_ROWS: Cell<usize> = const { Cell::new(0) };
}

/// Shared process bootstrap; the binary injects its identity before dispatch.
struct CliApplication;

impl CliApplication {
    fn run(identity: entry::EntryIdentity) {
        let bootstrap_started_at = std::time::Instant::now();
        #[cfg(debug_assertions)]
        configure_test_allocation_budget();

        if let Some(exit_code) = application_service_entry::run_if_requested() {
            process::exit(exit_code);
        }
        if let Some(exit_code) = task_runtime::run_agent_worker_if_requested() {
            process::exit(exit_code);
        }
        if let Some(exit_code) = task_runtime::run_shell_worker_if_requested() {
            process::exit(exit_code);
        }
        configure_process_surface(identity);

        if let Some(exit_code) = external_executor::run_catalog_command_if_requested() {
            process::exit(exit_code);
        }

        if let Some(exit_code) = memory_worker::run_if_requested() {
            process::exit(exit_code);
        }

        #[cfg(target_os = "macos")]
        if let Some(exit_code) = background_scheduler::run_if_requested() {
            process::exit(exit_code);
        }

        if let Some(exit_code) = memory_admin::run_if_requested() {
            process::exit(exit_code);
        }

        let terminal_setup = crate::cli_config_setup::TerminalConfigurationSetup;
        let no_setup = crate::configuration_setup::NoConfigurationSetup;
        let setup: &dyn crate::configuration_setup::ConfigurationSetup = match identity {
            entry::EntryIdentity::Cli => &terminal_setup,
            entry::EntryIdentity::AgentHost => &no_setup,
        };
        let mut config = match Config::from_args_with_setup(std::env::args().skip(1), setup) {
            Ok(config) => config,
            Err(message) => {
                eprintln!("{message}");
                eprintln!();
                print_help();
                process::exit(2);
            }
        };
        let arguments_ready_at = std::time::Instant::now();
        configure_command_surface(&mut config);
        if let Err(message) = validate_agent_host_invocation(identity, &config) {
            eprintln!("{message}");
            process::exit(2);
        }
        let desktop_host_request = config.host_request_stdin;
        if desktop_host_request {
            // Product argv composition ends here. Native stdin assembly and
            // execution never round-trip through Config or CLI dispatch.
            let result = config.host_invocation_request().and_then(
                crate::native_host_entry::run_stdin,
            );
            if let Err(message) = result {
                eprintln!("{message}");
                process::exit(1);
            }
            return;
        }
        if std::env::var_os("NOLA_MODELS_ROOT").is_none() {
            if let Ok(root) = local_models::models_root() {
                std::env::set_var("NOLA_MODELS_ROOT", root);
            }
        }
        let host_request_ready_at = std::time::Instant::now();
        if desktop_host_request {
            let session_id = config
                .session_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_default();
            if let Err(message) = desktop_host_control::bind_session_id(session_id) {
                eprintln!("{message}");
                process::exit(2);
            }
        }
        if let Err(message) = desktop_host_control::start_if_needed() {
            eprintln!("{message}");
            process::exit(2);
        }
        if let Err(message) = config::apply_desktop_host_tool_policy(&mut config) {
            eprintln!("{message}");
            process::exit(2);
        }
        if let Err(message) = configure_default_cli_skills(&mut config) {
            eprintln!("{message}");
            process::exit(2);
        }
        if let Err(message) = crate::conversation_invocation::initialize_or_restore(&mut config) {
            eprintln!("{message}");
            process::exit(2);
        }
        expose_runtime_provider_config(&config);
        llm::configure_local_debug_log(config.debug_mode, &config.project_root);
        let runtime_config_ready_at = std::time::Instant::now();

        if std::env::var_os("NOLA_CLI_DEBUG").is_some() {
            eprintln!(
                "nola_bootstrap_timing arguments_ms={} host_request_ms={} runtime_config_ms={} total_ms={}",
                arguments_ready_at.duration_since(bootstrap_started_at).as_millis(),
                host_request_ready_at.duration_since(arguments_ready_at).as_millis(),
                runtime_config_ready_at.duration_since(host_request_ready_at).as_millis(),
                runtime_config_ready_at.duration_since(bootstrap_started_at).as_millis(),
            );
        }

        if config.show_help {
            print_help();
            return;
        }

        // Standalone CLI consumes the same saved MCP records as the Web Host.
        // Loading metadata does not connect; native stdin requests have already
        // returned above and retain their caller-selected catalog.
        if matches!(config.command, CommandMode::Run | CommandMode::Room { .. }) {
            config.mcp_server_registry = match crate::mcp_catalog_port::load_web_mcp_servers() {
                Ok(servers) => servers,
                Err(error) => { eprintln!("{}", format_error(&error)); process::exit(1); }
            };
        }

        // An explicit recovery must validate its exact Run before any worker
        // can acquire outbox deliveries. Its foreground resume owns the drain;
        // do not start a generic conversation supervisor ahead of that fence.
        let runtime_effect_supervisor = if config.desktop_resume.is_some() { None } else {
            match config.host_invocation_request().and_then(|request| {
                runtime_effect_supervisor::CliRuntimeEffectSupervisor::start_if_persistent(&request)
            }) {
                Ok(supervisor) => supervisor,
                Err(error) => {
                    eprintln!("{}", format_error(&error));
                    local_models::shutdown();
                    process::exit(1);
                }
            }
        };
        let result = run_command(&config);
        let supervisor_result = runtime_effect_supervisor
            .map(|supervisor| {
                if matches!(config.command, CommandMode::Run) && !config.host_request_stdin {
                    supervisor.shutdown_for_cli_exit()
                } else {
                    supervisor.shutdown()
                }
            })
            .transpose()
            .map(|_| ());
        let result = result.and(supervisor_result);
        local_models::shutdown();
        if let Err(err) = result {
            if !err.is_empty() {
                eprintln!("{}", format_error(&err));
            }
            process::exit(1);
        }
    }
}

fn configure_default_cli_skills(config: &mut Config) -> Result<(), String> {
    // `None` is the standalone CLI catalog: embedded metadata is indexed
    // without filesystem scanning, and only the selected built-in Skill is
    // materialized when invoked. Explicit directories (including the empty
    // --no-skills set) remain an embedding/user decision.
    let _ = config;
    Ok(())
}


fn expose_runtime_provider_config(config: &Config) {
    set_runtime_provider_value("NOLA_LLM_ENDPOINT", &config.endpoint);
    set_runtime_provider_value("NOLA_LLM_API_KEY", &config.api_key);
    set_runtime_provider_value("NOLA_LLM_MODEL", &config.model);

    let provider = config
        .runtime
        .provider
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    set_runtime_provider_value("NOLA_LLM_PROVIDER", provider);
    std::env::remove_var("NOLA_CLI_LLM_PROVIDER");
}

fn set_runtime_provider_value(key: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        std::env::remove_var(key);
    } else {
        std::env::set_var(key, value);
    }
}

fn configure_process_surface(identity: entry::EntryIdentity) {
    guard_agent_host_surface(identity);
}

fn configure_command_surface(config: &mut Config) {
    if config.host_request_stdin {
        // `--host-request-stdin` is an authenticated product-Host boundary,
        // not an interactive CLI session. Make the permission profile an
        // invariant of that protocol instead of relying on every native
        // launcher to provide the same environment variables.
        if std::env::var_os("NOLA_CLIENT_SURFACE").is_none() {
            std::env::set_var("NOLA_CLIENT_SURFACE", "agent_host");
        }
        config.permission_profile = Some(resolve_permission_profile(
            config.permission_profile,
            crate::permissions::PermissionProfile::Cautious,
        ));
        return;
    }
    // Every surface needs an explicit identity and permission profile. The
    // interactive CLI defaults to the cautious profile too; `/allow` can
    // still switch it to cautious or allow-all at runtime. Previously CLI `run`
    // returned here, leaving Core in its ask-by-default fallback even though
    // the UI described the cautious profile as the product default.
    if std::env::var_os("NOLA_CLIENT_SURFACE").is_none() {
        std::env::set_var(
            "NOLA_CLIENT_SURFACE",
            match config.command {
                CommandMode::Server => "server",
                CommandMode::Web => "web",
                _ => "cli",
            },
        );
    }
    config.permission_profile = Some(configured_cli_permission_profile(config));
}

/// Resolves the standalone CLI's effective permission profile into its typed
/// request configuration. An explicit environment profile remains
/// authoritative. Without an explicit profile the CLI remains Cautious;
/// `--yes` only approves init/wiki model-network confirmation and never
/// grants Agent tool authority. Resolution must not rewrite the environment because
/// tests and in-process product surfaces may configure independent requests.
fn configured_cli_permission_profile(config: &Config) -> crate::permissions::PermissionProfile {
    resolve_cli_permission_profile(config, config.permission_profile)
}

fn resolve_cli_permission_profile(
    config: &Config,
    configured: Option<crate::permissions::PermissionProfile>,
) -> crate::permissions::PermissionProfile {
    let _ = config;
    resolve_permission_profile(configured, crate::permissions::PermissionProfile::Cautious)
}

fn resolve_permission_profile(
    configured: Option<crate::permissions::PermissionProfile>,
    fallback: crate::permissions::PermissionProfile,
) -> crate::permissions::PermissionProfile {
    configured.unwrap_or(fallback)
}

#[cfg(test)]
#[test]
fn native_profile_admission_rejects_invalid_explicit_configuration() {
    let _environment = crate::store::test_env_lock();
    let previous = std::env::var_os("NOLA_PERMISSION_PROFILE");
    std::env::set_var("NOLA_PERMISSION_PROFILE", "moderate");
    let observed = std::env::var("NOLA_PERMISSION_PROFILE").unwrap();
    let result = Config::for_test();
    match previous {
        Some(value) => std::env::set_var("NOLA_PERMISSION_PROFILE", value),
        None => std::env::remove_var("NOLA_PERMISSION_PROFILE"),
    }
    assert_eq!(observed, "moderate");
    assert!(result.is_err());
}

fn validate_agent_host_invocation(identity: entry::EntryIdentity, config: &Config) -> Result<(), String> {
    if identity != entry::EntryIdentity::AgentHost || config.show_help {
        return Ok(());
    }
    match &config.command {
        CommandMode::Run if config.host_request_stdin => Ok(()),
        CommandMode::Server => Ok(()),
        CommandMode::Run => Err(
            "nola-agent-host accepts run requests only through --host-request-stdin".to_string(),
        ),
        CommandMode::Sessions
        | CommandMode::SessionShow(_)
        | CommandMode::SessionPart { .. }
        | CommandMode::SessionSources { .. }
        | CommandMode::SessionDelete(_) => Ok(()),
        _ => Err(
            "nola-agent-host supports desktop run requests and the persistent Server mode"
                .to_string(),
        ),
    }
}

fn guard_agent_host_surface(identity: entry::EntryIdentity) {
    if identity != entry::EntryIdentity::AgentHost {
        return;
    }
    // The native Host is a product boundary, not a CLI invocation. Never let
    // an inherited shell/IDE environment reclassify it as CLI and re-enable
    // CLI-global `ask` rules for ordinary writes or computer interaction.
    // Preserve a real native/Web caller identity when one was supplied; a
    // standalone Host is an Agent Host, not a pretend Mac client.
    let client_surface = std::env::var("NOLA_CLIENT_SURFACE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| {
            matches!(
                value.as_str(),
                "mac" | "macos" | "mac_server" | "server" | "web" | "ios" | "android"
            )
        })
        .unwrap_or_else(|| "agent_host".to_string());
    std::env::set_var("NOLA_DESKTOP_HOST", "1");
    std::env::set_var("NOLA_CLIENT_SURFACE", client_surface);
}

fn run_command(config: &Config) -> Result<(), String> {
    if config.desktop_cancel.is_some() {
        return crate::native_host_entry::execute(config.host_invocation_request()?);
    }
    let preflight_started_at = std::time::Instant::now();
    // A force-quit, host crash, or machine restart can leave a durable lock
    // behind. Recover all dead owners before any product surface inspects or
    // starts a session; waiting until that exact conversation is reopened
    // makes unrelated UI surfaces report stale running state indefinitely.
    let read_only = matches!(
        &config.command,
        CommandMode::Sessions
            | CommandMode::Room { .. }
            | CommandMode::ModelConnections
            | CommandMode::ModelManage
            | CommandMode::ModelApplyConfig { .. }
            | CommandMode::ModelChatgpt { .. }
            | CommandMode::ModelDiscover { .. }
            | CommandMode::ModelConfigure { .. }
            | CommandMode::ModelProbe { .. }
            | CommandMode::ModelOverride { .. }
            | CommandMode::ModelImport { .. }
            | CommandMode::Plugin(_)
            | CommandMode::SessionShow(_)
            | CommandMode::SessionPart { .. }
            | CommandMode::SessionSources { .. }
    );
    if !read_only && config.desktop_resume.is_none() && config.desktop_cancel.is_none() {
        crate::session_process_observation::cleanup_stale_session_run_locks();
    }
    let stale_locks_ready_at = std::time::Instant::now();
    // Recovery is scoped to each dead lock owner above. A foreground request
    // must never perform a second all-conversation Run sweep: one damaged Run
    // cannot become an admission dependency for unrelated user input.
    if std::env::var_os("NOLA_CLI_DEBUG").is_some() {
        eprintln!(
            "nola_preflight_timing session_lock_recovery_ms={} total_ms={}",
            stale_locks_ready_at
                .duration_since(preflight_started_at)
                .as_millis(),
            stale_locks_ready_at
                .duration_since(preflight_started_at)
                .as_millis(),
        );
    }
    match &config.command {
        CommandMode::Plugin(command) => {
            let result = crate::plugin_manager::run(command)?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?);
            Ok(())
        }
        CommandMode::Run => {
            if config.desktop_resume.is_some() {
                run_prompt_with_native_permissions(config)
            } else if let Some((goal, constraints)) = host_goal_control(config)? {
                let mut goal_config = config.clone();
                run_goal_prompt_cycle(&mut goal_config, &config.message, &goal, &constraints)
                    .map(|_| ())
            } else {
                run_prompt_with_native_permissions(config)
            }
        }
        CommandMode::Init => run_top_level_init_command(config),
        CommandMode::Wiki => run_top_level_wiki_command(config),
        CommandMode::Web => web::run_web_server(config),
        CommandMode::Server => web::run_web_server(config),
        CommandMode::Sessions => {
            let sessions = session::list_sessions()?;
            for saved in &sessions {
                let _ = crate::session_process_observation::session_running_process(&saved.id);
            }
            if config.json_output {
                println!("{}", json!({"type": "sessions", "sessions": sessions}));
            } else if sessions.is_empty() {
                println!("{}", tr(Message::NoSavedSessions));
            } else {
                print_session_table(&sessions);
            }
            Ok(())
        }
        CommandMode::SessionShow(id) => {
            let _ = crate::session_process_observation::session_running_process(id);
            let saved = session::load_session(id)?;
            if config.json_output {
                let trace_runs = session::load_session_trace_runs(id)?;
                let identified_messages = session::load_session_messages_with_ids(id)?;
                let store = store::NolaStore::open_default()?;
                let mut application =
                    application_port::CliApplicationPort::persistent(store.path())?;
                let conversation_projection = application.load_conversation_projection_v1(id)?;
                let session_projection = application.load_session_projection(id)?;
                let skill_projection = session_projection.as_ref().map(|projection| {
                    let runtime = &projection.skill_runtime_projection;
                    session::SessionSkillProjection {
                        session_id: runtime.session_id.clone(),
                        cursor: runtime.cursor,
                        revision: runtime.revision,
                        catalog_revision: runtime.catalog_revision,
                        order: runtime.order.clone(),
                        runs: runtime.runs.clone(),
                    }
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&session_show_json_value(
                        &saved,
                        &identified_messages,
                        &trace_runs,
                        &conversation_projection,
                        session_projection
                            .as_ref()
                            .map(|value| &value.task_projection),
                        session_projection
                            .as_ref()
                            .map(|value| &value.memory_runtime_projection),
                        session_projection
                            .as_ref()
                            .map(|value| &value.goal_projection),
                        session_projection
                            .as_ref()
                            .map(|value| &value.permission_projection),
                        skill_projection.as_ref(),
                    ))
                    .map_err(|err| err.to_string())?
                );
            } else {
                print_session_summary(&saved);
                println!();
                print_interactive_session_replay(&saved);
            }
            Ok(())
        }
        CommandMode::SessionPart {
            conversation_id,
            message_id,
            part_id,
            revision,
        } => {
            let owner = nola_protocol::CanonicalPartOwner::new(
                conversation_id.clone(),
                message_id.clone(),
                part_id.clone(),
            )
            .map_err(|error| error.to_string())?;
            let mut application = application_port::CliApplicationPort::persistent_default()?;
            let detail = historical_part::read(&mut application, owner, *revision)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&detail).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        CommandMode::SessionSources {
            conversation_id,
            compaction_part_id,
        } => {
            let mut application = application_port::CliApplicationPort::persistent_default()?;
            let detail = load_compaction_sources(
                &mut application,
                conversation_id,
                compaction_part_id.as_deref(),
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&detail).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        CommandMode::ModelChatgpt { operation, account_id, revision, label } => {
            #[cfg(unix)]
            { crate::model_siwc_cli::execute(operation, account_id.as_deref(), *revision, label.as_deref(), !config.json_output && io::stdin().is_terminal()) }
            #[cfg(not(unix))]
            { let _ = (operation, account_id, revision, label); Err("Nola ChatGPT credential storage is unavailable on this platform".into()) }
        }
        CommandMode::ModelManage => {
            let url = format!("http://127.0.0.1:{}/models?tab=api", config.web_port);
            let database = crate::platform_paths::application_database_path()?;
            let quoted_database = database.to_string_lossy().replace('\'', "'\\''");
            println!("{}", serde_json::to_string_pretty(&json!({
                "url":url,
                "database_path":database,
                "start_host":format!("NOLA_DB_PATH='{}' nola web --host 127.0.0.1 --port {}", quoted_database, config.web_port),
                "instruction":"Open this address in a browser on the Host computer. Source login manages the source application's account only; it does not enable Nola inference. No login or browser was started.",
                "inference_supported_by_source_login":false,
            })).map_err(|_| "cannot render management address")?);
            Ok(())
        }
        CommandMode::ModelApplyConfig { path } => {
            let selected_path = crate::model_connection_host::file_config::selected_path(path.as_deref())?;
            let store = store::NolaStore::open_default()?;
            let origins = vec![format!("http://localhost:{}", config.web_port), format!("http://127.0.0.1:{}", config.web_port)];
            let result = crate::model_connection_host::file_config::apply(&store, &selected_path, &origins)?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(|_| "cannot render configuration receipt")?);
            Ok(())
        }
        CommandMode::ModelDiscover { connection_id, revision } => {
            let store = store::NolaStore::open_default()?;
            let origins = vec![format!("http://localhost:{}", config.web_port), format!("http://127.0.0.1:{}", config.web_port), format!("http://[::1]:{}", config.web_port)];
            let result = crate::model_connection_host::discover_catalog_request(&store, connection_id, *revision, &origins, &uuid::Uuid::new_v4().to_string())?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(|_| "cannot render model catalog")?);
            Ok(())
        }
        CommandMode::ModelConfigure { connection_id, revision, enabled_model_ids, selected_model_id } => {
            let mut app = application_port::CliApplicationPort::persistent_default()?;
            let result: Value = app.execute(nola_application::ApplicationCommand::Model {
                command: nola_application::ModelCommand::ConfigureConnectionModels {
                    connection_id: connection_id.clone(), expected_revision: *revision,
                    enabled_model_ids: enabled_model_ids.clone(), selected_model_id: selected_model_id.clone(),
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            })?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(|_| "cannot render configured models")?);
            Ok(())
        }
        CommandMode::ModelConnections => {
            let mut app = application_port::CliApplicationPort::persistent_default()?;
            let connections: Value = app.execute(nola_application::ApplicationCommand::Model {
                command: nola_application::ModelCommand::ListConnections })?;
            let mut accounts: Value = app.execute(nola_application::ApplicationCommand::Model {
                command: nola_application::ModelCommand::ListAccounts })?;
            if let Some(values) = accounts.as_array_mut() {
                for value in values { if let Some(account) = value.get_mut("account").and_then(Value::as_object_mut) { account.remove("credential_ref"); } }
            }
            let profiles=store::NolaStore::open_default()?.model_profiles()?.into_iter().filter(|profile|profile.deleted_at_ms.is_none()).map(|profile| {
                let source:Value=serde_json::from_str(&profile.source_json).unwrap_or(Value::Null);
                json!({"id":profile.id,"revision":profile.revision,"display_name":profile.display_name,"model":profile.model_name,"enabled":profile.enabled,"connection_id":source["connectionID"],"protocol":source["wireProtocol"]})
            }).collect::<Vec<_>>();
            println!("{}", serde_json::to_string_pretty(&json!({"connections":connections, "accounts":accounts,"profiles":profiles})).map_err(|error| error.to_string())?);
            Ok(())
        }
        CommandMode::ModelProbe { connection_id, revision, model_id, protocol } => {
            if !io::stdin().is_terminal() || config.json_output {
                return Err("model probe requires an interactive terminal for explicit cost confirmation".into());
            }
            let store = store::NolaStore::open_default()?;
            let protocol = serde_json::from_value(json!(protocol)).map_err(|_| "invalid probe protocol")?;
            let mut confirmation = crate::model_capability_probe::prepare(&store, connection_id, *revision, model_id, protocol)?;
            println!("{}", serde_json::to_string_pretty(&confirmation).map_err(|_| "cannot render probe confirmation")?);
            print!("Send at most ONE model request on this exact account and route? Provider charges may apply. Text success does not prove tool or vision support. [y/N] ");
            io::stdout().flush().map_err(|error| error.to_string())?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer).map_err(|error| error.to_string())?;
            if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") { return Ok(()); }
            confirmation.acknowledge_possible_charges = true;
            let origins = vec![format!("http://localhost:{}", config.web_port), format!("http://127.0.0.1:{}", config.web_port), format!("http://[::1]:{}", config.web_port)];
            let result = crate::model_capability_probe::probe(&store, confirmation, &origins)?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(|_| "cannot render probe result")?);
            Ok(())
        }
        CommandMode::ModelOverride { connection_id, revision, model_id, protocol, capabilities } => {
            let protocol = serde_json::from_value(json!(protocol)).map_err(|_| "invalid override protocol")?;
            let mut application = application_port::CliApplicationPort::persistent_default()?;
            let result: Value = application.execute(nola_application::ApplicationCommand::Model {
                command:nola_application::ModelCommand::SetConnectionCapabilityOverride {
                    connection_id:connection_id.clone(), expected_revision:*revision, model_id:model_id.clone(),
                    protocol, capabilities:*capabilities, updated_at_ms:chrono::Utc::now().timestamp_millis(),
                }
            })?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(|_| "cannot render capability override")?);
            Ok(())
        }
        CommandMode::ModelImport { source, path, credential_path, include_credentials, allow_environment } => {
            if !io::stdin().is_terminal() || config.json_output { return Err("model import needs an interactive terminal to review and confirm the plan".into()); }
            let store = store::NolaStore::open_default()?;
            let source = serde_json::from_value(json!(source)).map_err(|_| "invalid import source")?;
            let origins = vec![format!("http://localhost:{}", config.web_port), format!("http://127.0.0.1:{}", config.web_port), format!("http://[::1]:{}", config.web_port)];
            let plan = crate::model_connection_host::plan_import(&store, crate::model_connection_host::ImportPlanRequest {
                source, path: path.clone(), credential_path: credential_path.clone(),
                include_credentials: *include_credentials, allow_environment: *allow_environment,
            }, &origins)?;
            println!("{}", serde_json::to_string_pretty(&plan).map_err(|_| "cannot render import plan")?);
            let mut selections = vec![];
            for candidate in &plan.candidates {
                if candidate.issue.is_some() { continue; }
                print!("Import {}{}? [y/N] ", candidate.connection.display_name,
                    if candidate.existing_revision.is_some() { " (update existing connection)" } else { "" });
                io::stdout().flush().map_err(|error| error.to_string())?;
                let mut answer = String::new(); io::stdin().read_line(&mut answer).map_err(|error| error.to_string())?;
                if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") { continue; }
                selections.push(crate::model_connection_host::ImportSelection { candidate_id: candidate.candidate_id.clone(),
                    display_name: None, update_existing: candidate.existing_revision.is_some(), enable_models: false });
            }
            if selections.is_empty() { return Ok(()); }
            let result = crate::model_connection_host::commit_import(&store, crate::model_connection_host::ImportCommitRequest { plan_id: plan.plan_id, selections }, &origins)?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(|_| "cannot render import results")?);
            Ok(())
        }
        CommandMode::Room { action, value, agent_id, text } => {
            let mut application = application_port::CliApplicationPort::persistent_default()?;
            if action == "recover" {
                let room_id = value.clone().ok_or("room ID required")?;
                let mut offset = 0;
                loop {
                    let values: Vec<nola_application::agent_room::RoomSubmission> = application.execute(nola_application::ApplicationCommand::AgentRoom {
                        command:nola_application::agent_room::AgentRoomCommand::Submissions {room_id:room_id.clone(),limit:200,offset},
                    })?;
                    for value in &values {
                        if matches!(value.state,nola_application::agent_room::RoomSubmissionState::Accepted | nola_application::agent_room::RoomSubmissionState::Selected | nola_application::agent_room::RoomSubmissionState::Executing) {
                            crate::web::run_room_submission(config,value.clone())?;
                        }
                    }
                    if values.len() < 200 { break; }
                    offset += values.len();
                }
                return Ok(());
            }
            let command = match action.as_str() {
                "list" => json!({"type":"list", "limit":100, "offset":0}),
                "directory" => json!({"type":"directory"}),
                "create" => json!({"type":"create", "title":value, "at_ms":chrono::Utc::now().timestamp_millis()}),
                "show" => json!({"type":"read", "room_id":value}),
                "send" => {
                    let room: Value = application.execute(nola_application::ApplicationCommand::AgentRoom {
                        command: nola_application::agent_room::AgentRoomCommand::Read { room_id: value.clone().ok_or("room ID required")? },
                    })?;
                    if room.is_null() { return Err("room not found".into()); }
                    json!({"type":"submit","input":{"room_id":value,"submission_id":uuid::Uuid::new_v4().to_string(),
                        "expected_room_revision":room["revision"],"text":text,"participant_id":agent_id,"at_ms":chrono::Utc::now().timestamp_millis()}})
                }
                "join" => {
                    let room: Value = application.execute(nola_application::ApplicationCommand::AgentRoom {
                        command: nola_application::agent_room::AgentRoomCommand::Read { room_id: value.clone().ok_or("room ID required")? },
                    })?;
                    let directory: Value = application.execute(nola_application::ApplicationCommand::AgentRoom {
                        command: nola_application::agent_room::AgentRoomCommand::Directory,
                    })?;
                    let agent = directory["agents"].as_array().and_then(|agents| agents.iter().find(|agent| agent["id"].as_str() == agent_id.as_deref()))
                        .ok_or("agent not found; use room directory")?;
                    if room.is_null() { return Err("room not found".into()); }
                    json!({"type":"join", "input":{"room_id":value, "expected_room_revision":room["revision"],
                        "agent_id":agent_id, "expected_agent_revision":agent["revision"], "at_ms":chrono::Utc::now().timestamp_millis()}})
                }
                _ => return Err("unsupported room action".into()),
            };
            let command = serde_json::from_value(command).map_err(|_| "invalid room command")?;
            let result: Value = application.execute(nola_application::ApplicationCommand::AgentRoom { command })?;
            println!("{}", serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?);
            if action == "send" {
                crate::web::run_room_submission(config, serde_json::from_value(result).map_err(|_| "invalid room receipt")?)?;
            }
            Ok(())
        }
        CommandMode::SessionDelete(id) => {
            core_agent_runtime::end_agent_core_session(id)?;
            session::delete_session(id)?;
            if config.json_output {
                println!("{}", json!({"type": "session_deleted", "id": id}));
            } else {
                println!("{} {id}", tr(Message::DeletedSessionPrefix));
            }
            Ok(())
        }
    }
}

fn load_compaction_sources(
    application: &mut application_port::CliApplicationPort,
    conversation_id: &str,
    requested_part_id: Option<&str>,
) -> Result<Value, String> {
    application.load_compaction_lineage(conversation_id, requested_part_id)
}

fn host_goal_control(config: &Config) -> Result<Option<(String, String)>, String> {
    crate::goal_runtime::host_goal_control(&config.host_invocation_request()?)
}


fn run_prompt_with_native_permissions(config: &Config) -> Result<(), String> {
    let native_mode = std::env::var("NOLA_NATIVE_PERMISSION_MODE")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false);
    if !native_mode && config.desktop_resume.is_none() {
        return run_prompt(config);
    }

    let session_id = config
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "native permission mode requires an explicit session id".to_string())?;
    let cancellation = crate::runtime_control::RuntimeExecutionControl::default();
    let _signals = crate::execution_signals::ExecutionSignals::install(
        cancellation.clone(), Some(crate::desktop_host_control::interrupt_pending_waits),
    )?;
    let _permission_session = tools::enter_host_permission_session(session_id);
    if let Some(resume) = &config.desktop_resume {
        // Reject assistant/run drift before the resume function is allowed to
        // recover deliveries. It rechecks the exact Run after acquiring Host.
        validate_desktop_resume_identity(
            &mut crate::application_port::CliApplicationPort::persistent_default()?, resume)?;
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::new();
        let outcome = core_agent_runtime::resume_agent_core_expected_with_cancellation(
            &config.host_invocation_request()?, &mut emitter, cancellation, &resume.expected_run_id)?;
        if outcome.is_none() { return Err("expected Run is no longer resumable".into()); }
        Ok(())
    } else {
        let mut output = crate::native_runtime_output::NativeRuntimeOutput::new();
        // Host input is already parsed by the typed request adapter. Never
        // interpret user text as a CLI command or reopen stdin/terminal here.
        let outcome = core_agent_runtime::run_agent_core_with_cancellation(
            &config.host_invocation_request()?, &config.message, &mut output, cancellation,
        )?;
        if outcome.turn_status == nola_core::agent::AgentTurnStatus::Failed {
            return Err(String::new());
        }
        Ok(())
    }
}

fn run_top_level_init_command(config: &Config) -> Result<(), String> {
    if config.json_output || !io::stdout().is_terminal() {
        return init::run_init(config);
    }
    println!("{}", interactive_layer_rule(Some("init")));
    println!();
    let mut active_progress = None;
    let result = init::run_init_with_progress(config, |progress| {
        update_interactive_init_progress(&mut active_progress, &progress);
    });
    if let Some(progress) = active_progress.take() {
        progress.clear();
    }
    result
}

fn run_top_level_wiki_command(config: &Config) -> Result<(), String> {
    if config.json_output || !io::stdout().is_terminal() {
        return init::run_wiki(config);
    }
    println!("{}", interactive_layer_rule(Some("wiki")));
    println!();
    let mut active_progress = None;
    let result = init::run_wiki_with_progress(config, |progress| {
        update_interactive_init_progress(&mut active_progress, &progress);
    });
    if let Some(progress) = active_progress.take() {
        progress.clear();
    }
    result
}

fn print_session_table(sessions: &[session::SessionInfo]) {
    let session_label = table_label(tr(Message::SessionLabel));
    let updated_label = table_label(tr(Message::UpdatedLabel));
    let messages_label = tr(Message::MessagesUnit).to_string();
    let id_width = sessions
        .iter()
        .map(|item| display_width(&item.id))
        .max()
        .unwrap_or(7)
        .max(display_width(&session_label));
    let updated_width = sessions
        .iter()
        .map(|item| display_width(&item.updated_at))
        .max()
        .unwrap_or(19)
        .max(display_width(&updated_label));
    let messages_width = sessions
        .iter()
        .map(|item| display_width(item.message_count.to_string().as_str()))
        .max()
        .unwrap_or(1)
        .max(display_width(&messages_label));
    println!(
        "{}  {}  {}",
        pad_right_width(&session_label, id_width),
        pad_right_width(&updated_label, updated_width),
        pad_left_width(&messages_label, messages_width)
    );
    println!(
        "{}  {}  {}",
        "-".repeat(id_width),
        "-".repeat(updated_width),
        "-".repeat(messages_width)
    );
    for item in sessions {
        let message_count = item.message_count.to_string();
        println!(
            "{}  {}  {}",
            pad_right_width(&item.id, id_width),
            pad_right_width(&item.updated_at, updated_width),
            pad_left_width(&message_count, messages_width)
        );
    }
}

fn display_width(text: &str) -> usize {
    if !text.contains('\u{1b}') {
        return UnicodeWidthStr::width(text);
    }
    let stripped = strip_ansi_sequences(text);
    UnicodeWidthStr::width(stripped.as_str())
}

fn strip_ansi_sequences(text: &str) -> String {
    if !text.contains('\u{1b}') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn pad_right_width(text: &str, width: usize) -> String {
    let current_width = display_width(text);
    if current_width >= width {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + (width - current_width));
    out.push_str(text);
    out.push_str(&" ".repeat(width - current_width));
    out
}

fn pad_left_width(text: &str, width: usize) -> String {
    let current_width = display_width(text);
    if current_width >= width {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + (width - current_width));
    out.push_str(&" ".repeat(width - current_width));
    out.push_str(text);
    out
}

fn table_label(label: &str) -> String {
    label
        .trim()
        .trim_end_matches(':')
        .trim_end_matches('：')
        .to_string()
}

fn display_role(role: &str) -> &'static str {
    match role {
        "assistant" => "assistant",
        "user" => "user",
        "tool" => "tool",
        "system" => "system",
        _ => "message",
    }
}

fn print_session_summary(saved: &session::SavedSession) {
    println!("{} {}", tr(Message::SessionLabel), saved.id);
    println!("{} {}", tr(Message::CreatedLabel), saved.created_at);
    println!("{} {}", tr(Message::UpdatedLabel), saved.updated_at);
    println!("{} {}", tr(Message::MessagesLabel), saved.messages.len());
}

fn run_prompt(config: &Config) -> Result<(), String> {
    run_prompt_with_optional_cancellation(config, None)
}

fn run_prompt_with_optional_cancellation(
    config: &Config,
    cancellation: Option<interaction::ExecutionCancellation>,
) -> Result<(), String> {
    if config.resume_session {
        if !config.message.trim().is_empty() {
            let text = crate::i18n::interaction_text();
            return Err(format!(
                "{}: {}",
                text.resume_title, text.resume_inline_unsupported
            ));
        }
        return run_resume_session_interactive_prompt(config);
    }
    if config.continue_last && config.message.trim().is_empty() && io::stdin().is_terminal() {
        return run_resume_session_interactive_prompt(config);
    }

    let prompt = match read_prompt(&config.message) {
        Ok(prompt) => prompt,
        Err(err) => return Err(format!("{} {err}", tr(Message::FailedReadPromptPrefix))),
    };

    if prompt.trim().is_empty() {
        if io::stdin().is_terminal() {
            return run_interactive_prompt(config);
        }
        return Err(tr(Message::PromptRequired).to_string());
    }

    // Product control commands must never become model input. Interactive
    // mode already dispatches `/sources` locally, but one-shot and piped CLI
    // invocations previously sent the same command to the LLM. Besides being
    // slow and misleading, that made data-source setup impossible to automate.
    // Keep the command in the shared CLI control layer for every input path.
    if prompt
        .split_whitespace()
        .next()
        .and_then(normalize_interactive_command)
        == Some("/sources")
    {
        return handle_data_source_command(&prompt, config);
    }

    let result = match cancellation {
        Some(cancellation) => {
            run_single_prompt_with_execution_cancellation(config, &prompt, cancellation)?
        }
        None => run_single_prompt(config, &prompt)?,
    };
    if !result.run_outcome.is_terminal() && !config.json_output {
        eprintln!(
            "Run remains pending ({:?}); no completion or permission decision was recorded.",
            result.run_outcome.turn_status
        );
    }
    if result.terminal_reason.as_deref() == Some("failed") {
        // The emitter already rendered the structured or human-readable
        // failure. Preserve the non-zero process status without duplicating it.
        return Err(String::new());
    }
    Ok(())
}

fn run_resume_session_interactive_prompt(config: &Config) -> Result<(), String> {
    if let Some(session_id) = select_interactive_resume_session(config)? {
        run_resumed_session_interactive_prompt(config, &session_id)?;
    }
    Ok(())
}

fn select_interactive_resume_session(config: &Config) -> Result<Option<String>, String> {
    let text = crate::i18n::interaction_text();
    if config.json_output {
        return Err(format!(
            "{}: {}",
            text.resume_title, text.resume_json_unsupported
        ));
    }
    if !io::stdin().is_terminal() {
        return Err(format!(
            "{}: {}",
            text.resume_title, text.resume_terminal_only
        ));
    }

    if config.continue_last {
        let session_id = resolve_continue_session_id(config)?
            .ok_or_else(|| format!("{}: {}", text.resume_title, tr(Message::NoSavedSessions)))?;
        return Ok(Some(session_id));
    }

    let project_root = config.project_root.to_string_lossy().to_string();
    let raw_sessions = session::list_sessions_with_first_user_message_for_project_with_limit(
        &project_root,
        RESUME_SESSION_LIST_LIMIT,
    )?;

    let mut sessions: Vec<ResumeSessionOption> = Vec::new();
    for raw_session in raw_sessions {
        let first_user_message = raw_session.first_user_message;
        sessions.push(ResumeSessionOption {
            id: raw_session.id,
            updated_at: raw_session.updated_at,
            last_user_message_at: raw_session.last_user_message_at,
            first_user_message: first_user_message.clone(),
            first_user_message_preview: resume_session_message_preview(
                &first_user_message,
                RESUME_SESSION_FIRST_MESSAGE_PREVIEW_LIMIT,
            ),
        });
    }
    if sessions.is_empty() {
        return Err(format!(
            "{}: {}",
            text.resume_title,
            tr(Message::NoSavedSessions)
        ));
    }

    select_resume_session_with_arrow(&sessions).map_err(|err| err.to_string())
}

fn run_resumed_session_interactive_prompt(config: &Config, session_id: &str) -> Result<(), String> {
    let mut resumed = config.clone();
    resumed.session_id = Some(session_id.to_string());
    resumed.continue_last = false;
    resumed.resume_session = false;
    crate::conversation_invocation::initialize_or_restore(&mut resumed)?;
    run_interactive_prompt_with_recovery(&resumed, true)
}

#[derive(Clone)]
struct ResumeSessionOption {
    id: String,
    last_user_message_at: String,
    updated_at: String,
    first_user_message: String,
    first_user_message_preview: String,
}

fn select_resume_session_with_arrow(
    sessions: &[ResumeSessionOption],
) -> io::Result<Option<String>> {
    let text = crate::i18n::interaction_text();
    if sessions.is_empty() {
        return Ok(None);
    }

    let Ok(_raw_mode) = InteractiveRawModeGuard::new() else {
        let mut choice = String::new();
        loop {
            println!("{} ({})", text.resume_title, text.session_list);
            println!("  {}", text.recent_sessions_hint);
            for (index, session) in sessions.iter().enumerate() {
                let first = if session.first_user_message.is_empty() {
                    text.empty_value.to_string()
                } else {
                    session.first_user_message_preview.clone()
                };
                println!("  {:>3}) {}  {}", index + 1, session.updated_at, first);
            }
            print!("{} ", text.resume_input_prompt);
            io::stdout()
                .flush()
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
            choice.clear();
            io::stdin()
                .read_line(&mut choice)
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
            let trimmed = choice.trim();
            match trimmed {
                "" => continue,
                "q" | "quit" | "exit" | "esc" => return Ok(None),
                _ => {
                    if let Ok(index) = trimmed.parse::<usize>() {
                        if (1..=sessions.len()).contains(&index) {
                            return Ok(Some(sessions[index - 1].id.clone()));
                        }
                    }
                }
            }
            println!(
                "{}{}{}",
                text.resume_invalid_prefix,
                sessions.len(),
                text.resume_invalid_suffix
            );
        }
    };

    let _list_guard = ResumeSessionSelectionUiStateGuard;
    enter_resume_session_screen()?;

    let mut selected = 0usize;
    let mut page_start = 0usize;
    loop {
        let page_size = resume_session_list_page_size();
        page_start =
            resolve_resume_session_list_window(selected, page_start, page_size, sessions.len());
        print!(
            "\r{INTERACTIVE_RESET}\x1b[0m\x1b[?25l\x1b[2J\x1b[H{}",
            resume_session_list_lines(sessions, selected, page_start, page_size)
        );
        io::stdout()
            .flush()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;

        let event = read().map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("{} {err}", tr(Message::FailedReadKeyEventPrefix)),
            )
        })?;
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    if selected + 1 < sessions.len() {
                        selected += 1;
                    }
                }
                KeyCode::PageUp => {
                    if page_size > 0 {
                        selected = selected.saturating_sub(page_size);
                    }
                }
                KeyCode::PageDown => {
                    if page_size > 0 {
                        selected = (selected + page_size).min(sessions.len().saturating_sub(1));
                    }
                }
                KeyCode::Home => {
                    selected = 0;
                }
                KeyCode::End => {
                    selected = sessions.len().saturating_sub(1);
                }
                KeyCode::Enter => {
                    return Ok(Some(sessions[selected].id.clone()));
                }
                KeyCode::Esc => {
                    return Ok(None);
                }
                KeyCode::Char('q') => {
                    return Ok(None);
                }
                KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if ch == 'c' || ch == 'C' {
                        return Ok(None);
                    }
                    continue;
                }
                _ => {}
            }
        }
    }
}

fn resume_session_list_lines(
    sessions: &[ResumeSessionOption],
    selected: usize,
    page_start: usize,
    page_size: usize,
) -> String {
    let text = crate::i18n::interaction_text();
    let width = terminal_columns().saturating_sub(1).max(20);
    let mut lines = String::new();
    if interactive_color_enabled() {
        lines.push_str(&format!(
            "  {INTERACTIVE_ACCENT}{}{INTERACTIVE_RESET}\r\n",
            text.resume_title
        ));
        lines.push_str(&format!(
            "  {INTERACTIVE_DIM}{}{INTERACTIVE_RESET}\r\n\r\n",
            text.recent_sessions_hint
        ));
    } else {
        lines.push_str(&format!("  {}\r\n", text.resume_title));
        lines.push_str(&format!("  {}\r\n\r\n", text.recent_sessions_hint));
    }

    let page_end = (page_start + page_size).min(sessions.len());
    for index in page_start..page_end {
        let session = &sessions[index];
        let is_selected = index == selected;
        let marker = if is_selected { "›" } else { " " };
        let first = if session.first_user_message.is_empty() {
            "<empty>".to_string()
        } else {
            session.first_user_message_preview.clone()
        };
        let row = format!(
            "  {} {:>3}  {:<8}  {}",
            marker,
            index + 1,
            resume_session_display_time(&session.last_user_message_at),
            first
        );
        let row = truncate_display_width_no_ellipsis(&row, width);
        let row = pad_right_width(&row, width);
        if interactive_color_enabled() {
            if is_selected {
                lines.push_str(INTERACTIVE_SELECTION_BG);
                lines.push_str(INTERACTIVE_VALUE);
                lines.push_str(&row);
                lines.push_str(INTERACTIVE_RESET);
            } else {
                lines.push_str(INTERACTIVE_DIM);
                lines.push_str(&row);
                lines.push_str(INTERACTIVE_RESET);
            }
        } else {
            lines.push_str(&row);
        }
        lines.push_str("\r\n");
    }

    if sessions.is_empty() {
        lines.push_str("\r\n暂无可恢复的会话。\r\n");
    } else {
        let pagination = format!("  {}–{} / {}", page_start + 1, page_end, sessions.len());
        if interactive_color_enabled() {
            lines.push_str(&format!(
                "\r\n{INTERACTIVE_DIM}{pagination}{INTERACTIVE_RESET}\r\n"
            ));
        } else {
            lines.push_str(&format!("\r\n{pagination}\r\n"));
        }
    }
    lines
}

fn resume_session_display_time(updated_at: &str) -> String {
    resume_session_relative_time(updated_at).unwrap_or_else(|| {
        updated_at
            .split_once('.')
            .map(|(prefix, _)| prefix)
            .unwrap_or(updated_at)
            .replace('T', " ")
    })
}

fn resume_session_relative_time(timestamp: &str) -> Option<String> {
    let parsed = DateTime::parse_from_rfc3339(timestamp).ok()?;
    let now = Local::now();
    let duration = now.signed_duration_since(parsed.with_timezone(&Local));
    let total_minutes = duration.num_minutes().max(0);
    let days = total_minutes / (24 * 60);
    let hours = (total_minutes % (24 * 60)) / 60;
    let minutes = total_minutes % 60;
    if days > 0 {
        if hours > 0 && minutes > 0 {
            return Some(format!("{days}d {hours}h {minutes}m"));
        }
        if hours > 0 {
            return Some(format!("{days}d {hours}h"));
        }
        if minutes > 0 {
            return Some(format!("{days}d {minutes}m"));
        }
        return Some(format!("{days}d"));
    }
    if hours > 0 {
        if minutes > 0 {
            return Some(format!("{hours}h {minutes}m"));
        }
        return Some(format!("{hours}h"));
    }
    if minutes == 0 {
        Some("<1m".to_string())
    } else {
        Some(format!("{minutes}m"))
    }
}

fn resume_session_message_preview(text: &str, limit: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= limit {
        return normalized;
    }
    let mut preview = normalized.chars().take(limit).collect::<String>();
    preview.push('…');
    preview
}

fn resume_session_list_page_size() -> usize {
    let rows = match crossterm::terminal::size() {
        Ok((_, rows)) => rows as usize,
        Err(_) => 24,
    };
    rows.saturating_sub(8).max(1)
}

fn enter_resume_session_screen() -> io::Result<()> {
    if !crate::terminal_ui::owns_terminal() { print!("\x1b[?1049h"); }
    print!("\r{INTERACTIVE_RESET}\x1b[0m\x1b[?25l\x1b[2J\x1b[H");
    io::stdout()
        .flush()
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
}

fn restore_resume_session_screen() -> io::Result<()> {
    if !crate::terminal_ui::owns_terminal() { print!("\x1b[?1049l"); }
    print!("\r{INTERACTIVE_RESET}\x1b[0m\x1b[?25h\x1b[2J\x1b[H");
    io::stdout()
        .flush()
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
}

fn resolve_resume_session_list_window(
    selected: usize,
    page_start: usize,
    page_size: usize,
    sessions_len: usize,
) -> usize {
    if sessions_len == 0 || page_size == 0 {
        return 0;
    }
    if selected < page_start {
        return selected;
    }
    if selected >= page_start + page_size {
        return selected.saturating_sub(page_size).saturating_add(1);
    }
    page_start
}

fn print_interactive_session_plan(config: &Config) -> Result<(), String> {
    let text = crate::i18n::interaction_text();
    let Some(saved) = load_interactive_session_for_command(config)? else {
        println!("  {}", text.no_execution_plan);
        return Ok(());
    };
    let Some(plan) = agent::session_plan_summary_for_session(&saved.id, &saved.messages) else {
        println!("  {}", text.no_execution_plan);
        return Ok(());
    };
    let stored_plan = crate::store::NolaStore::open_default()?.session_plan_record(&saved.id)?;
    let revision = stored_plan.as_ref().map(|plan| plan.revision);
    let frozen_revision = stored_plan.as_ref().and_then(|plan| plan.frozen_revision);

    println!("{}", interactive_layer_rule(Some("plan")));
    if interactive_color_enabled() {
        println!(
            "  {INTERACTIVE_ACCENT}{}{INTERACTIVE_RESET}{INTERACTIVE_DIM}{} · {}/{} {}{INTERACTIVE_RESET}",
            text.execution_plan,
            revision
                .map(|revision| format!(
                    " r{revision}{}",
                    frozen_revision
                        .map(|contract| format!(" · frozen@r{contract}"))
                        .unwrap_or_default()
                ))
                .unwrap_or_default(),
            plan.completed,
            plan.total,
            text.completed
        );
    } else {
        println!(
            "  {}{} · {}/{} {}",
            text.execution_plan,
            revision
                .map(|revision| format!(
                    " r{revision}{}",
                    frozen_revision
                        .map(|contract| format!(" · frozen@r{contract}"))
                        .unwrap_or_default()
                ))
                .unwrap_or_default(),
            plan.completed,
            plan.total,
            text.completed
        );
    }
    let width = terminal_columns().saturating_sub(12).max(24);
    for step in &plan.steps {
        let (icon, label, color) = match step.status.as_str() {
            "completed" => ("✓", text.complete, "\x1b[38;5;108m"),
            "in_progress" => ("●", text.in_progress, INTERACTIVE_ACCENT),
            _ => ("○", text.pending, INTERACTIVE_DIM),
        };
        let content = if step.status == "in_progress" && !step.active_form.trim().is_empty() {
            step.active_form.as_str()
        } else {
            step.content.as_str()
        };
        let content = truncate_display_width(content, width);
        let approval = match step.approval_status.as_str() {
            "approved" => "◆",
            "pending" => "◇",
            "rejected" => "×",
            _ => "–",
        };
        let dependencies = if step.dependency_ids.is_empty() {
            String::new()
        } else {
            format!(" ← {}", step.dependency_ids.join(","))
        };
        if interactive_color_enabled() {
            println!(
                "  {color}{icon} {label:<6}{INTERACTIVE_RESET} {content}{INTERACTIVE_DIM} · {approval} {} · #{}{}{INTERACTIVE_RESET}",
                step.priority, step.id, dependencies
            );
        } else {
            println!(
                "  {icon} {label:<6} {content} · {approval} {} · #{}{}",
                step.priority, step.id, dependencies
            );
        }
    }
    Ok(())
}

fn update_interactive_session_plan_approval(
    config: &Config,
    step_ids: Vec<String>,
    approved: bool,
) -> Result<(), String> {
    let Some(saved) = load_interactive_session_for_command(config)? else {
        return Err("an active session is required".to_string());
    };
    let state = tools::AgentPlanState::default();
    if !state.bind_session(&saved.id)? {
        return Err("the current session does not have a persisted execution plan".to_string());
    }
    let changed = state.set_plan_approval(&step_ids, approved)?;
    println!("  {} · {changed}", if approved { "✓" } else { "×" });
    print_interactive_session_plan(config)
}

fn edit_interactive_session_plan_step(
    config: &Config,
    step_id: &str,
    content: &str,
) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("plan step content must not be empty".to_string());
    }
    let Some(saved) = load_interactive_session_for_command(config)? else {
        return Err("an active session is required".to_string());
    };
    let store = crate::store::NolaStore::open_default()?;
    let current = store.session_plan_record(&saved.id)?.ok_or_else(|| {
        "the current session does not have a persisted execution plan".to_string()
    })?;
    store.edit_session_plan_step(
        &saved.id,
        current.revision,
        step_id,
        nola_application::ConversationPlanStepEdit {
            content: Some(content.trim().to_string()),
            ..nola_application::ConversationPlanStepEdit::default()
        },
    )?;
    print_interactive_session_plan(config)
}

fn freeze_interactive_session_plan(config: &Config) -> Result<(), String> {
    let Some(saved) = load_interactive_session_for_command(config)? else {
        return Err("an active session is required".to_string());
    };
    let store = crate::store::NolaStore::open_default()?;
    let current = store.session_plan_record(&saved.id)?.ok_or_else(|| {
        "the current session does not have a persisted execution plan".to_string()
    })?;
    store.freeze_session_plan(&saved.id, current.revision)?;
    print_interactive_session_plan(config)
}

fn replan_interactive_session_plan_step(
    config: &Config,
    step_id: &str,
    content: &str,
) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("replanned step content must not be empty".to_string());
    }
    let Some(saved) = load_interactive_session_for_command(config)? else {
        return Err("an active session is required".to_string());
    };
    let store = crate::store::NolaStore::open_default()?;
    let current = store.session_plan_record(&saved.id)?.ok_or_else(|| {
        "the current session does not have a persisted execution plan".to_string()
    })?;
    if current.frozen_revision.is_none() {
        return Err("freeze the current execution contract before creating a replan".to_string());
    }
    let mut todos = current.todos.clone();
    let step = todos
        .iter_mut()
        .find(|step| step.id == step_id)
        .ok_or_else(|| format!("unknown plan step: {step_id}"))?;
    step.content = content.trim().to_string();
    let todos = serde_json::to_value(&todos)
        .map_err(|error| format!("encode replanned steps failed: {error}"))?;
    let next_sop = nola_application::ConversationPlanSopIdentity {
        run_id: uuid::Uuid::new_v4().to_string(),
        sop_id: current.sop.sop_id.clone(),
        sop_version: current.sop.sop_version,
    };
    store.replan_session_plan(
        &saved.id,
        current.revision,
        &next_sop,
        &todos,
        &format!("user revised step {step_id}"),
    )?;
    print_interactive_session_plan(config)
}

fn print_interactive_session_plan_history(config: &Config) -> Result<(), String> {
    let Some(saved) = load_interactive_session_for_command(config)? else {
        return Err("an active session is required".to_string());
    };
    let revisions = crate::store::NolaStore::open_default()?.session_plan_history(&saved.id)?;
    println!("{}", interactive_layer_rule(Some("plan history")));
    for revision in revisions {
        let summary = if revision.change_summary.is_empty() {
            revision.change_kind.clone()
        } else {
            format!("{} · {}", revision.change_kind, revision.change_summary)
        };
        println!(
            "  r{} · {} · {} steps · {}",
            revision.revision,
            summary,
            revision.todos.len(),
            chrono::DateTime::from_timestamp_millis(revision.updated_at_ms)
                .map(|value| value.with_timezone(&Local).to_rfc3339())
                .unwrap_or_else(|| revision.updated_at_ms.to_string())
        );
    }
    Ok(())
}

fn print_interactive_session_plan_diff(
    config: &Config,
    from_revision: i64,
    to_revision: i64,
) -> Result<(), String> {
    let Some(saved) = load_interactive_session_for_command(config)? else {
        return Err("an active session is required".to_string());
    };
    let diff = crate::store::NolaStore::open_default()?.session_plan_diff(
        &saved.id,
        from_revision,
        to_revision,
    )?;
    println!("{}", interactive_layer_rule(Some("plan diff")));
    println!("  r{} → r{}", diff.from_revision, diff.to_revision);
    for step in diff.added {
        println!("  + #{} {}", step.id, step.content);
    }
    for step in diff.removed {
        println!("  - #{} {}", step.id, step.content);
    }
    for change in diff.changed {
        println!("  ~ #{}", change.id);
        println!("    - {}", change.before.content);
        println!("    + {}", change.after.content);
    }
    Ok(())
}

fn print_interactive_session_history(config: &Config) -> Result<(), String> {
    let text = crate::i18n::interaction_text();
    let session_label = config
        .session_id
        .as_deref()
        .unwrap_or(text.current_project_session);
    let mut active_progress = (!config.json_output)
        .then(|| InteractiveProgressSpinner::start(text.loading_session_history, session_label));
    let saved = match load_interactive_session_for_command(config) {
        Ok(Some(saved)) => saved,
        Ok(None) => {
            if let Some(progress) = active_progress.take() {
                progress.clear();
            }
            return Ok(());
        }
        Err(err) => {
            if let Some(progress) = active_progress.take() {
                progress.clear();
            }
            eprintln!("{err}");
            return Ok(());
        }
    };
    if !config.json_output {
        crate::terminal_ui::set_restoring_view(true);
        crate::terminal_ui::clear_replayed_history();
    }
    if !config.json_output {
        let detail = format!("{} {}", saved.messages.len(), text.messages_suffix);
        if let Some(progress) = active_progress.take() {
            progress.finish(text.loading_session_history, &detail);
        } else {
            print_interactive_progress_completion(text.loading_session_history, &detail);
        }
        println!("{}", interactive_layer_rule(Some("history")));
    }
    println!();
    print_interactive_session_replay(&saved);
    if !config.json_output { crate::terminal_ui::set_restoring_view(false); }
    Ok(())
}

fn print_interactive_session_replay(saved: &session::SavedSession) {
    let started = Instant::now();
    let commits = interactive_session_replay_commits(saved);
    let mut user_commits = 0usize;
    let mut assistant_commits = 0usize;
    let mut tool_commits = 0usize;
    let mut tool_items = 0usize;
    let mut rendered_chars = 0usize;
    let mut seen_user_turn = false;
    for commit in commits {
        let mut output = String::new();
        match commit {
            InteractiveReplayCommit::Empty => output.push_str("  暂无消息\n"),
            InteractiveReplayCommit::User(lines) => {
                user_commits = user_commits.saturating_add(1);
                if seen_user_turn {
                    output.push_str(&interactive_layer_rule(None));
                    output.push('\n');
                }
                seen_user_turn = true;
                for line in lines {
                    let rendered = interactive_session_replay_user_line(&line);
                    rendered_chars = rendered_chars.saturating_add(rendered.chars().count());
                    output.push_str(&rendered);
                    output.push('\n');
                }
                output.push('\n');
            }
            InteractiveReplayCommit::Assistant(text) => {
                assistant_commits = assistant_commits.saturating_add(1);
                let rendered = render_markdown_text(&text);
                if !rendered.is_empty() {
                    rendered_chars = rendered_chars.saturating_add(rendered.chars().count());
                    output.push_str(&rendered);
                    if !rendered.ends_with('\n') {
                        output.push('\n');
                    }
                    output.push('\n');
                }
            }
            InteractiveReplayCommit::Tools(items) => {
                tool_commits = tool_commits.saturating_add(1);
                tool_items = tool_items.saturating_add(items.len());
                for line in interactive_session_tool_block_lines(&items) {
                    rendered_chars = rendered_chars.saturating_add(line.chars().count());
                    output.push_str(&line);
                    output.push('\n');
                }
                output.push('\n');
            }
        }
        crate::terminal_ui::commit_history(&output);
    }
    crate::terminal_ui::establish_restored_history_tail();
    llm::log_local_debug_payload(
        "perf_replay_render",
        &json!({
            "type": "perf_replay_render",
            "session_id": saved.id,
            "message_count": saved.messages.len(),
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "user_commits": user_commits,
            "assistant_commits": assistant_commits,
            "tool_commits": tool_commits,
            "tool_items": tool_items,
            "rendered_chars": rendered_chars,
        }),
    );
}

fn print_interactive_session_replay_notice(visible: usize, total: usize) {
    if total <= visible || total < INTERACTIVE_REPLAY_NOTICE_THRESHOLD {
        return;
    }
    let notice = match current_language() {
        Language::ZhHans => format!("已显示最近 {visible} 条消息，共 {total} 条；完整记录将在继续对话时使用。"),
        Language::De => format!("Die neuesten {visible} von {total} Nachrichten werden angezeigt; beim Fortsetzen steht der vollständige Verlauf bereit."),
        Language::Es => format!("Se muestran los {visible} mensajes más recientes de {total}; el historial completo seguirá disponible al continuar."),
        Language::Fr => format!("Affichage des {visible} messages les plus récents sur {total} ; l’historique complet reste disponible pour la suite."),
        Language::Ja => format!("直近 {visible} 件を表示しています（全 {total} 件）。会話の続行時には完全な履歴が使用されます。"),
        Language::Ru => format!("Показаны последние {visible} из {total} сообщений; полная история будет доступна при продолжении диалога."),
        Language::En => format!("Showing the latest {visible} of {total} messages; the complete transcript remains available to the next turn."),
    };
    let rendered = if interactive_color_enabled() {
        format!("  {INTERACTIVE_DIM}{notice}{INTERACTIVE_RESET}\n\n")
    } else {
        format!("  {notice}\n\n")
    };
    crate::terminal_ui::commit_history(&rendered);
}

fn resolve_continue_session_id(config: &Config) -> Result<Option<String>, String> {
    let project_root = config.project_root.to_string_lossy().to_string();
    session::latest_session_id_for_project(&project_root)
        .map_err(|err| format!("{} {err}", tr(Message::FailedReadSessionsPrefix)))
}

#[cfg(test)]
fn interactive_session_replay_lines(saved: &session::SavedSession) -> Vec<String> {
    let mut lines = Vec::new();
    for commit in interactive_session_replay_commits(saved) {
        match commit {
            InteractiveReplayCommit::Empty => lines.push("  暂无消息".to_string()),
            InteractiveReplayCommit::User(inputs) => {
                for line in inputs {
                    lines.push(format!("{INTERACTIVE_PROMPT_PREFIX}{line}"));
                }
                lines.push(String::new());
            }
            InteractiveReplayCommit::Assistant(text) => {
                lines.extend(split_rendered_markdown_lines(&text));
                lines.push(String::new());
            }
            InteractiveReplayCommit::Tools(items) => {
                lines.extend(interactive_session_tool_block_lines_for_language(
                    &items,
                    Language::En,
                ));
                lines.push(String::new());
            }
        }
    }
    lines
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum InteractiveReplayBlock {
    Empty,
    User(Vec<String>),
    Assistant(String),
    Tools(Vec<ReplayToolItem>),
}

#[derive(Debug, Clone)]
enum InteractiveReplayCommit {
    Empty,
    User(Vec<String>),
    Assistant(String),
    Tools(Vec<ReplayToolItem>),
}

fn interactive_session_replay_commits(
    saved: &session::SavedSession,
) -> Vec<InteractiveReplayCommit> {
    let events = interactive_session_replay_events(saved);
    if events.is_empty() {
        return vec![InteractiveReplayCommit::Empty];
    }

    let mut state = InteractiveReplayState::default();
    for event in events {
        state.apply(event);
    }
    state.finish()
}

#[derive(Debug, Clone)]
enum InteractiveReplayEvent {
    User(Vec<String>),
    AssistantText(String),
    ToolItems(Vec<ReplayToolItem>),
}

#[derive(Debug, Default)]
struct InteractiveReplayState {
    commits: Vec<InteractiveReplayCommit>,
    pending_tools: Vec<ReplayToolItem>,
}

impl InteractiveReplayState {
    fn apply(&mut self, event: InteractiveReplayEvent) {
        match event {
            InteractiveReplayEvent::ToolItems(items) => {
                self.pending_tools.extend(items);
            }
            InteractiveReplayEvent::User(lines) => {
                self.flush_tools();
                if !lines.is_empty() {
                    self.commits.push(InteractiveReplayCommit::User(lines));
                }
            }
            InteractiveReplayEvent::AssistantText(text) => {
                self.flush_tools();
                if !text.trim().is_empty() {
                    self.commits.push(InteractiveReplayCommit::Assistant(text));
                }
            }
        }
    }

    fn flush_tools(&mut self) {
        if self.pending_tools.is_empty() {
            return;
        }
        let items = finalize_tool_items(std::mem::take(&mut self.pending_tools));
        if !items.is_empty() {
            self.commits.push(InteractiveReplayCommit::Tools(items));
        }
    }

    fn finish(mut self) -> Vec<InteractiveReplayCommit> {
        self.flush_tools();
        if self.commits.is_empty() {
            self.commits.push(InteractiveReplayCommit::Empty);
        }
        self.commits
    }
}

fn interactive_session_replay_events(saved: &session::SavedSession) -> Vec<InteractiveReplayEvent> {
    let messages: Vec<&Value> = saved
        .messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) != Some("system"))
        .collect();
    let mut events = Vec::new();
    let mut tool_name_by_call_id: HashMap<String, String> = HashMap::new();
    let mut index = 0usize;

    while index < messages.len() {
        let message = messages[index];
        if let Some(lines) = replay_user_decision_lines(message) {
            events.push(InteractiveReplayEvent::User(lines));
            index = index.saturating_add(1);
            continue;
        }
        if is_replay_tool_only_message(message) {
            let block_start = index;
            let mut next = block_start.saturating_add(1);
            while next < messages.len()
                && is_replay_tool_only_message(messages[next])
                && replay_user_decision_lines(messages[next]).is_none()
            {
                next = next.saturating_add(1);
            }
            let tool_items = collect_replay_tool_items_in_range(
                messages.as_slice(),
                block_start,
                next,
                &mut tool_name_by_call_id,
            );
            if !tool_items.is_empty() {
                events.push(InteractiveReplayEvent::ToolItems(tool_items));
            }
            index = next;
            continue;
        }

        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "user" => {
                let lines = interactive_session_input_message_lines(message.get("content"));
                if !lines.is_empty() {
                    events.push(InteractiveReplayEvent::User(lines));
                }
            }
            "assistant" => {
                let terminal_status = match message["display_status"].as_str() {
                    Some("cancelled") => Some(UiText::Cancelled),
                    Some("failed") => Some(UiText::Failed),
                    _ => None,
                };
                if let Some(status) = terminal_status {
                    events.push(InteractiveReplayEvent::AssistantText(ui_text(status).to_string()));
                }
                if let Some(parts) = message.get("display_text_parts").and_then(Value::as_array) {
                    for part in parts {
                        if !replay_text_part_is_visible(part) { continue; }
                        let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
                        if text.is_empty() { continue; }
                        events.push(InteractiveReplayEvent::AssistantText(text.to_string()));
                    }
                    if let Some(summaries) = message.get("display_ui_surface_summaries").and_then(Value::as_array) {
                        for summary in summaries.iter().filter_map(Value::as_str).filter(|text| !text.trim().is_empty()) {
                            events.push(InteractiveReplayEvent::AssistantText(crate::terminal_journal_presentation::readonly_surface_summary(summary)));
                        }
                    }
                } else if let Some(text) = interactive_session_assistant_content_text(message) {
                    if !text.trim().is_empty() {
                        events.push(InteractiveReplayEvent::AssistantText(text));
                    }
                }
                let tool_items = collect_replay_tool_items_optional(
                    messages.as_slice(),
                    index,
                    index.saturating_add(1),
                    &mut tool_name_by_call_id,
                );
                if !tool_items.is_empty() {
                    events.push(InteractiveReplayEvent::ToolItems(tool_items));
                }
            }
            _ => {
                if let Some(text) = interactive_session_message_content_text(message) {
                    if !text.trim().is_empty() {
                        events.push(InteractiveReplayEvent::AssistantText(text));
                    }
                }
                let tool_items = collect_replay_tool_items_optional(
                    messages.as_slice(),
                    index,
                    index.saturating_add(1),
                    &mut tool_name_by_call_id,
                );
                if !tool_items.is_empty() {
                    events.push(InteractiveReplayEvent::ToolItems(tool_items));
                }
            }
        }
        index = index.saturating_add(1);
    }

    events
}

fn replay_user_decision_lines(message: &Value) -> Option<Vec<String>> {
    if message.get("nola_kind").and_then(Value::as_str) != Some("user_decision") {
        return None;
    }
    let text = message
        .get("decision_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let lines = split_input_text_lines(text);
    (!lines.is_empty()).then_some(lines)
}

#[cfg(test)]
fn interactive_session_replay_blocks(saved: &session::SavedSession) -> Vec<InteractiveReplayBlock> {
    interactive_session_replay_commits(saved)
        .into_iter()
        .map(|commit| match commit {
            InteractiveReplayCommit::Empty => InteractiveReplayBlock::Empty,
            InteractiveReplayCommit::User(lines) => InteractiveReplayBlock::User(lines),
            InteractiveReplayCommit::Assistant(text) => InteractiveReplayBlock::Assistant(text),
            InteractiveReplayCommit::Tools(items) => InteractiveReplayBlock::Tools(items),
        })
        .collect()
}

fn collect_replay_tool_items_optional(
    messages: &[&Value],
    start: usize,
    end: usize,
    call_name_map: &mut HashMap<String, String>,
) -> Vec<ReplayToolItem> {
    let tool_items = messages
        .get(start..end)
        .unwrap_or(&[])
        .iter()
        .flat_map(|message| collect_tool_items_from_message(message, call_name_map).into_iter())
        .collect::<Vec<_>>();
    finalize_tool_items(tool_items)
}

fn interactive_session_replay_user_line(line: &str) -> String {
    // Submitted input is durable conversation history, not another composer.
    // Give it a stable role marker and neutral foreground so it cannot be
    // confused with the live input row or assistant Markdown.
    let line_width = terminal_columns().saturating_sub(1).max(1);
    interactive_user_history_at_width(line, line_width, interactive_color_enabled())
}

fn interactive_user_history_at_width(line: &str, line_width: usize, color: bool) -> String {
    let prefix = "    › ";
    let (rows, _, _) = composer_input_multiline(line, line.len(),
        line_width.saturating_sub(display_width(prefix)).max(1), prefix);
    rows.iter().enumerate().map(|(index, (text, _))| {
        interactive_input_bar_line_with_color(text, false, false,
            if index == 0 { prefix } else { "      " }, line_width, color)
    }).collect::<Vec<_>>().join("\n")
}

fn interactive_layer_rule(label: Option<&str>) -> String {
    let width = terminal_columns().saturating_sub(4).clamp(20, 64);
    let raw = if let Some(label) = label.filter(|value| !value.trim().is_empty()) {
        let label = label.trim();
        let prefix = format!("─ {label} ");
        let remaining = width.saturating_sub(display_width(&prefix)).max(1);
        format!("{prefix}{}", "─".repeat(remaining))
    } else {
        "─".repeat(width)
    };
    if interactive_color_enabled() {
        format!("  {INTERACTIVE_DIM}{raw}{INTERACTIVE_RESET}")
    } else {
        format!("  {raw}")
    }
}

#[derive(Debug, Clone)]
struct ReplayToolItem {
    call_id: Option<String>,
    name: String,
    detail: Option<String>,
    result: Option<String>,
}

impl ReplayToolItem {
    fn new(name: String) -> Self {
        Self {
            call_id: None,
            name,
            detail: None,
            result: None,
        }
    }
}

fn collect_replay_tool_items_in_range(
    messages: &[&Value],
    start: usize,
    end: usize,
    call_name_map: &mut HashMap<String, String>,
) -> Vec<ReplayToolItem> {
    let mut tool_items = messages
        .get(start..end)
        .unwrap_or(&[])
        .iter()
        .flat_map(|message| collect_tool_items_from_message(message, call_name_map).into_iter())
        .collect::<Vec<_>>();
    if tool_items.is_empty() {
        tool_items.push(ReplayToolItem::new("tool".to_string()));
    }
    finalize_tool_items(tool_items)
}

fn is_replay_tool_only_message(message: &Value) -> bool {
    // Missing envelope metadata is not evidence of a tool-only message. In
    // particular, its intentionally empty preview must not create a fake tool.
    if message.get("metadata_deferred").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if is_replay_tool_message_role(role) {
        return true;
    }
    if role == "assistant" {
        if message.get("display_ui_surface_summaries").and_then(Value::as_array)
            .is_some_and(|summaries| summaries.iter().any(|value| value.as_str().is_some_and(|text| !text.trim().is_empty()))) {
            return false;
        }
        if message.get("display_text_parts").and_then(Value::as_array)
            .is_some_and(|parts| parts.iter().any(|part| replay_text_part_is_visible(part)
                && part["text"].as_str().is_some_and(|text| !text.is_empty()))) {
            return false;
        }
        let content = message.get("content").unwrap_or(&Value::Null);
        if replay_content_is_tool_only(content)
            || (!replay_content_has_visible_text(content)
                && replay_message_has_tool_payload(message))
        {
            return true;
        }
    }
    if role == "user" {
        if let Some(content) = message.get("content") {
            return replay_content_is_tool_only(content);
        }
        return false;
    }

    replay_message_has_tool_payload(message)
        || replay_content_is_tool_only(message.get("content").unwrap_or(&Value::Null))
}

fn replay_text_part_is_visible(part: &Value) -> bool {
    part.get("metadata_deferred").and_then(Value::as_bool) != Some(true)
        && matches!(part.get("review").and_then(Value::as_str), None | Some("approved"))
}

fn is_replay_tool_message_role(role: &str) -> bool {
    matches!(role, "tool" | "tool_call" | "tool_result" | "function")
}

fn replay_message_has_tool_payload(message: &Value) -> bool {
    message.get("tool").is_some()
        || message.get("tool_name").is_some()
        || message.get("tool_call_id").is_some()
        || message.get("tool_use_id").is_some()
        || message.get("tool_result").is_some()
        || message.get("tool_calls").is_some()
        || message.get("input").is_some()
        || message.get("output").is_some()
        || message.get("result").is_some()
        || message.get("arguments").is_some()
        || message.get("function").is_some()
        || message.get("name").is_some()
        || message
            .get("metadata")
            .and_then(Value::as_object)
            .is_some_and(|metadata| {
                metadata.contains_key("tool")
                    || metadata.contains_key("name")
                    || metadata.contains_key("tool_name")
                    || metadata.contains_key("tool_call_id")
                    || metadata.contains_key("tool_use_id")
            })
}

fn interactive_session_tool_block_lines(tool_items: &[ReplayToolItem]) -> Vec<String> {
    interactive_session_tool_block_lines_for_language(tool_items, current_language())
}

fn interactive_session_tool_block_lines_for_language(
    tool_items: &[ReplayToolItem],
    language: Language,
) -> Vec<String> {
    if tool_items.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for tool in tool_items {
        let result = tool
            .result
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        let suffix = result
            .map(|value| format!(" · {value}"))
            .unwrap_or_default();
        let line = if let Some(detail) = tool
            .detail
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            format!(
                "    › {}  {detail}{suffix}",
                crate::i18n::localize_tool_display_name_for(language, &tool.name)
            )
        } else {
            format!(
                "    › {}{suffix}",
                crate::i18n::localize_tool_display_name_for(language, &tool.name)
            )
        };
        if interactive_color_enabled() {
            lines.push(format!("{INTERACTIVE_DIM}{line}{INTERACTIVE_RESET}"));
        } else {
            lines.push(line);
        }
    }
    lines
}

fn interactive_session_assistant_content_text(message: &Value) -> Option<String> {
    let text = message.get("content").and_then(extract_replay_content_text).or_else(|| message
        .get("reasoning_content")
        .and_then(extract_replay_content_text));
    let mut chunks: Vec<String> = text.into_iter().collect();
    if let Some(summaries) = message.get("display_ui_surface_summaries").and_then(Value::as_array) {
        for summary in summaries.iter().filter_map(Value::as_str).filter(|text| !text.trim().is_empty()) {
            chunks.push(crate::terminal_journal_presentation::readonly_surface_summary(summary));
        }
    }
    if chunks.is_empty() { None } else { Some(chunks.join("\n\n")) }
}

fn interactive_session_message_content_text(message: &Value) -> Option<String> {
    if let Some(content) = message.get("content") {
        if let Some(text) = extract_replay_content_text(content) {
            return Some(text);
        }
    }

    message
        .get("reasoning_content")
        .and_then(extract_replay_content_text)
}

fn collect_tool_items_from_message(
    message: &Value,
    call_name_map: &mut HashMap<String, String>,
) -> Vec<ReplayToolItem> {
    let mut items = collect_tool_names_from_message(message, call_name_map)
        .into_iter()
        .map(ReplayToolItem::new)
        .collect::<Vec<_>>();

    merge_replay_tool_call_details(message.get("tool_calls"), call_name_map, &mut items);
    merge_replay_tool_value_details(message.get("content"), call_name_map, &mut items);
    merge_replay_tool_value_details(message.get("input"), call_name_map, &mut items);
    merge_replay_tool_value_details(message.get("value"), call_name_map, &mut items);

    if is_replay_tool_message_role(
        message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    ) {
        let name = extract_tool_name_from_tool_message(message, call_name_map)
            .unwrap_or_else(|| "tool".to_string());
        let result = replay_tool_result_summary(message);
        let role_item = ReplayToolItem {
            call_id: replay_tool_call_id(message),
            name,
            detail: replay_tool_detail_from_object(message.as_object()),
            result,
        };
        if role_item.call_id.is_none() {
            if let Some(existing) = items
                .iter_mut()
                .find(|item| item.call_id.is_none() && item.name == role_item.name)
            {
                if existing.detail.is_none() {
                    existing.detail = role_item.detail;
                }
                if existing.result.is_none() {
                    existing.result = role_item.result;
                }
            } else {
                items.push(role_item);
            }
        } else {
            upsert_replay_tool_item(&mut items, role_item);
        }
    }

    finalize_tool_items(items)
}

fn collect_tool_names_from_message(
    message: &Value,
    call_name_map: &mut HashMap<String, String>,
) -> Vec<String> {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let mut tool_names = Vec::new();
    let has_tool_calls = matches!(message.get("tool_calls"), Some(Value::Array(items)) if !items.is_empty())
        || matches!(message.get("tool_calls"), Some(Value::Object(_)));

    collect_tool_names_from_tool_calls(message.get("tool_calls"), call_name_map, &mut tool_names);
    collect_tool_names_from_value(message.get("content"), call_name_map, &mut tool_names);
    collect_tool_names_from_value(message.get("input"), call_name_map, &mut tool_names);
    collect_tool_names_from_value(message.get("value"), call_name_map, &mut tool_names);

    if is_replay_tool_message_role(role) {
        if let Some(tool_name) = extract_tool_name_from_tool_message(message, call_name_map) {
            if !tool_names.iter().any(|name| name == &tool_name) {
                tool_names.push(tool_name);
            }
        }

        if let Some(content) = message.get("content") {
            collect_tool_names_from_value(Some(content), call_name_map, &mut tool_names);
        }

        if tool_names.is_empty() {
            tool_names.push("tool".to_string());
        }

        return finalize_tool_names(tool_names);
    }

    if role != "assistant" {
        return tool_names;
    }

    if tool_names.is_empty() && has_tool_calls {
        tool_names.push("tool".to_string());
    }

    finalize_tool_names(tool_names)
}

fn collect_tool_names_from_tool_calls(
    tool_calls: Option<&Value>,
    call_name_map: &mut HashMap<String, String>,
    out: &mut Vec<String>,
) {
    let calls: Vec<&Value> = match tool_calls {
        Some(Value::Array(calls)) => calls.iter().collect(),
        Some(call @ Value::Object(_)) => vec![call],
        _ => return,
    };

    for call in calls {
        if let Some(tool_name) = extract_tool_name_from_tool_call(call, call_name_map) {
            if !tool_name.trim().is_empty() {
                out.push(tool_name);
            }
        }
    }
}

fn merge_replay_tool_call_details(
    tool_calls: Option<&Value>,
    call_name_map: &mut HashMap<String, String>,
    out: &mut Vec<ReplayToolItem>,
) {
    let calls: Vec<&Value> = match tool_calls {
        Some(Value::Array(calls)) => calls.iter().collect(),
        Some(call @ Value::Object(_)) => vec![call],
        _ => return,
    };

    for call in calls {
        let name = extract_tool_name_from_tool_call(call, call_name_map)
            .unwrap_or_else(|| "tool".to_string());
        let detail = replay_tool_detail_from_object(call.as_object());
        upsert_replay_tool_item(
            out,
            ReplayToolItem {
                call_id: replay_tool_call_id(call),
                name,
                detail,
                result: None,
            },
        );
    }
}

fn merge_replay_tool_value_details(
    value: Option<&Value>,
    call_name_map: &mut HashMap<String, String>,
    out: &mut Vec<ReplayToolItem>,
) {
    let Some(value) = value else {
        return;
    };
    if let Some(text) = value.as_str() {
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            merge_replay_tool_value_details(Some(&parsed), call_name_map, out);
        }
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                merge_replay_tool_value_details(Some(item), call_name_map, out);
            }
        }
        Value::Object(object) => {
            if let Some(name) = extract_single_tool_name_from_object(object, call_name_map) {
                upsert_replay_tool_item(
                    out,
                    ReplayToolItem {
                        call_id: replay_tool_call_id(value),
                        name,
                        detail: replay_tool_detail_from_object(Some(object)),
                        result: replay_tool_result_summary(value),
                    },
                );
            }
            merge_replay_tool_call_details(object.get("tool_calls"), call_name_map, out);
            for key in ["content", "input", "value", "result", "parts"] {
                merge_replay_tool_value_details(object.get(key), call_name_map, out);
            }
        }
        _ => {}
    }
}

fn replay_tool_detail_from_object(
    object: Option<&serde_json::Map<String, Value>>,
) -> Option<String> {
    let object = object?;
    let mut parts = Vec::new();
    for (label, keys) in [
        ("path", &["path", "file", "notebook_path", "root"][..]),
        ("pattern", &["pattern", "query", "symbol"][..]),
        ("url", &["url", "target", "item"][..]),
        ("selector", &["selector", "label"][..]),
        ("cmd", &["command", "cmd"][..]),
    ] {
        if let Some(value) = first_replay_tool_arg(object, keys) {
            parts.push(format!("{label}={value}"));
        }
        if parts.len() >= 3 {
            break;
        }
    }
    if parts.is_empty() {
        if let Some(function) = object.get("function").and_then(Value::as_object) {
            return replay_tool_detail_from_object(Some(function));
        }
        return None;
    }
    Some(truncate_display_width(&parts.join(" "), 120))
}

fn first_replay_tool_arg(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if replay_tool_arg_key_is_hidden(key) {
            continue;
        }
        if let Some(value) = object.get(*key).and_then(compact_replay_tool_arg_value) {
            return Some(value);
        }
    }
    if let Some(arguments) = object.get("arguments") {
        if let Some(raw) = arguments.as_str() {
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                if let Some(parsed_object) = parsed.as_object() {
                    return first_replay_tool_arg(parsed_object, keys);
                }
            }
        } else if let Some(parsed_object) = arguments.as_object() {
            return first_replay_tool_arg(parsed_object, keys);
        }
    }
    None
}

fn compact_replay_tool_arg_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            if replay_value_looks_sensitive(text) {
                Some("[redacted]".to_string())
            } else {
                let text = text.replace('\n', "\\n");
                let text = text.trim();
                if text.is_empty() {
                    None
                } else {
                    Some(truncate_display_width(text, 72))
                }
            }
        }
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Array(items) => {
            if items.is_empty() {
                None
            } else if items.len() == 1 {
                compact_replay_tool_arg_value(&items[0])
            } else {
                Some(format!("{} items", items.len()))
            }
        }
        Value::Object(object) => {
            if let Some(path) = object.get("path").and_then(compact_replay_tool_arg_value) {
                Some(path)
            } else {
                Some(format!("{} fields", object.len()))
            }
        }
        Value::Null => None,
    }
}

fn replay_tool_arg_key_is_hidden(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "content"
            | "contents"
            | "text"
            | "input"
            | "value"
            | "old_text"
            | "new_text"
            | "replacement"
            | "patch"
            | "diff"
            | "headers"
            | "cookies"
            | "password"
            | "token"
            | "api_key"
            | "api-key"
            | "authorization"
    )
}

fn replay_tool_result_summary(value: &Value) -> Option<String> {
    let text = match value {
        Value::Object(object) => object
            .get("display")
            .or_else(|| object.get("summary"))
            .or_else(|| object.get("status"))
            .or_else(|| object.get("error"))
            .and_then(Value::as_str),
        Value::String(text) => Some(text.as_str()),
        _ => None,
    }?;
    let line = text.lines().find(|line| !line.trim().is_empty())?.trim();
    if line.is_empty() {
        None
    } else if line == "(no matches)" {
        Some("无匹配".to_string())
    } else if line == "(no matches in requested window)" {
        Some("当前窗口无匹配".to_string())
    } else if replay_value_looks_sensitive(line) {
        Some("[redacted]".to_string())
    } else if replay_result_line_is_noisy(line) {
        None
    } else {
        Some(truncate_display_width(line, 100))
    }
}

fn replay_result_line_is_noisy(line: &str) -> bool {
    line.chars().count() > 140
        || line.starts_with('{')
        || line.starts_with('[')
        || line.starts_with("diff ")
        || line.starts_with("@@")
}

fn replay_value_looks_sensitive(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("authorization:")
        || lower.contains("bearer ")
        || lower.contains("api_key")
        || lower.contains("api-key")
        || lower.contains("password")
        || lower.contains("access_token")
        || lower.contains("secret")
        || lower.contains("set-cookie")
}

fn finalize_tool_names(tool_names: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for tool_name in tool_names {
        if tool_name.trim().is_empty() {
            continue;
        }
        if seen.insert(tool_name.clone()) {
            deduped.push(tool_name);
        }
    }

    deduped
}

fn finalize_tool_items(tool_items: Vec<ReplayToolItem>) -> Vec<ReplayToolItem> {
    let mut finalized = Vec::<ReplayToolItem>::new();
    for mut item in tool_items {
        item.name = item.name.trim().to_string();
        if item.name.is_empty() || item.name == "request_user_input" {
            continue;
        }
        upsert_replay_tool_item(&mut finalized, item);
    }
    let names_with_call_ids = finalized
        .iter()
        .filter(|item| item.call_id.is_some())
        .map(|item| item.name.clone())
        .collect::<HashSet<_>>();
    finalized.retain(|item| {
        item.call_id.is_some()
            || item.detail.is_some()
            || item.result.is_some()
            || !names_with_call_ids.contains(&item.name)
    });
    finalized
}

fn upsert_replay_tool_item(items: &mut Vec<ReplayToolItem>, mut item: ReplayToolItem) {
    if item.name.trim().is_empty() {
        return;
    }
    let exact_call = item.call_id.as_deref().and_then(|call_id| {
        items
            .iter()
            .position(|existing| existing.call_id.as_deref() == Some(call_id))
    });
    if let Some(index) = exact_call {
        let existing = &mut items[index];
        if existing.call_id.is_none() {
            existing.call_id = item.call_id.take();
        }
        if existing.detail.is_none() {
            existing.detail = item.detail;
        }
        if existing.result.is_none() {
            existing.result = item.result;
        }
    } else {
        items.push(item);
    }
}

fn replay_tool_call_id(value: &Value) -> Option<String> {
    value
        .get("id")
        .or_else(|| value.get("tool_call_id"))
        .or_else(|| value.get("tool_use_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extract_tool_name_from_tool_message(
    message: &Value,
    call_name_map: &mut HashMap<String, String>,
) -> Option<String> {
    let message_tool_name = message
        .get("name")
        .or_else(|| message.get("tool_name"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(|name| name.to_string());
    let tool_call_id = message
        .get("tool_call_id")
        .or_else(|| message.get("tool_use_id"))
        .and_then(Value::as_str);

    if let Some(tool_name) = message_tool_name {
        if let Some(tool_call_id) = tool_call_id {
            call_name_map.insert(tool_call_id.to_string(), tool_name.clone());
        }
        return Some(tool_name);
    }

    if let Some(tool_call_id) = tool_call_id {
        if let Some(tool_name) = call_name_map.get(tool_call_id) {
            return Some(tool_name.clone());
        }
    }

    if let Some(content) = message.get("content") {
        if let Some(tool_name) = extract_tool_name_from_value(content) {
            if let Some(tool_call_id) = tool_call_id {
                call_name_map.insert(tool_call_id.to_string(), tool_name.clone());
            }
            return Some(tool_name);
        }
        if let Some(raw_content) = content.as_str() {
            if let Ok(value) = serde_json::from_str::<Value>(raw_content) {
                if let Some(tool_name) = extract_tool_name_from_value(&value) {
                    if let Some(tool_call_id) = tool_call_id {
                        call_name_map.insert(tool_call_id.to_string(), tool_name.clone());
                    }
                    return Some(tool_name);
                }
            }
        }
    }

    if let Some(result_name) = message.get("result").and_then(extract_tool_name_from_value) {
        if let Some(tool_call_id) = tool_call_id {
            call_name_map.insert(tool_call_id.to_string(), result_name.clone());
        }
        return Some(result_name);
    }

    if let Some(tool_call_id) = tool_call_id {
        if let Some(tool_name) = call_name_map.get(tool_call_id) {
            return Some(tool_name.clone());
        }
    }

    message
        .get("metadata")
        .and_then(extract_tool_name_from_value)
        .or_else(|| message.get("result").and_then(extract_tool_name_from_value))
}

fn extract_tool_name_from_tool_call(
    call: &Value,
    call_name_map: &mut HashMap<String, String>,
) -> Option<String> {
    let call_name = call
        .get("function")
        .and_then(|item| item.get("name"))
        .or_else(|| call.get("name"))
        .or_else(|| call.get("tool"))
        .or_else(|| call.get("tool_name"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(|name| name.to_string());
    let call_name_from_metadata = call
        .get("metadata")
        .and_then(|value| value.get("tool"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(|name| name.to_string());
    let call_name = call_name.or(call_name_from_metadata);
    let call_id = call
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| call.get("tool_call_id").and_then(Value::as_str))
        .or_else(|| call.get("tool_use_id").and_then(Value::as_str));

    if let Some(tool_name) = call_name {
        if let Some(call_id) = call_id {
            call_name_map.insert(call_id.to_string(), tool_name.clone());
        }
        return Some(tool_name);
    }

    call_id.and_then(|tool_id| call_name_map.get(tool_id).cloned())
}

fn collect_tool_names_from_value(
    value: Option<&Value>,
    call_name_map: &mut HashMap<String, String>,
    out: &mut Vec<String>,
) {
    let Some(value) = value else {
        return;
    };

    if let Some(text) = value.as_str() {
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            collect_tool_names_from_value(Some(&parsed), call_name_map, out);
        }
        return;
    }

    match value {
        Value::Array(items) => {
            for item in items {
                collect_tool_names_from_value(Some(item), call_name_map, out);
            }
        }
        Value::Object(object) => {
            if let Some(tool_name) = extract_single_tool_name_from_object(object, call_name_map) {
                if !out.iter().any(|name| name == &tool_name) {
                    out.push(tool_name);
                }
            }
            collect_tool_names_from_tool_calls(object.get("tool_calls"), call_name_map, out);
            collect_tool_names_from_value(object.get("content"), call_name_map, out);
            collect_tool_names_from_value(object.get("input"), call_name_map, out);
            collect_tool_names_from_value(object.get("value"), call_name_map, out);
            collect_tool_names_from_value(object.get("result"), call_name_map, out);
            collect_tool_names_from_value(object.get("parts"), call_name_map, out);
            if let Some(metadata) = object.get("metadata") {
                collect_tool_names_from_value(Some(metadata), call_name_map, out);
            }
            if let Some(function) = object.get("function") {
                collect_tool_names_from_value(Some(function), call_name_map, out);
            }
        }
        _ => {}
    }
}

fn extract_single_tool_name_from_object(
    object: &serde_json::Map<String, Value>,
    call_name_map: &mut HashMap<String, String>,
) -> Option<String> {
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let is_tool_like = matches!(
        kind,
        "tool_use" | "tool_result" | "tool_call" | "tool" | "function"
    ) || object.contains_key("tool_calls")
        || object.contains_key("function")
        || object.contains_key("tool")
        || object.contains_key("tool_name")
        || object.contains_key("tool_call_id")
        || object.contains_key("tool_use_id")
        || object.contains_key("id");

    let call_id = object
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| object.get("tool_call_id").and_then(Value::as_str))
        .or_else(|| object.get("tool_use_id").and_then(Value::as_str));

    if kind == "text" && !is_tool_like {
        return None;
    }

    if let Some(name) = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
    {
        let name = name.to_string();
        if !name.is_empty() {
            if let Some(call_id) = call_id {
                call_name_map.insert(call_id.to_string(), name.clone());
            }
            return Some(name);
        }
    }

    if let Some(name) = object
        .get("function")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
    {
        let name = name.to_string();
        if let Some(call_id) = call_id {
            call_name_map.insert(call_id.to_string(), name.clone());
        }
        return Some(name);
    }

    if let Some(name) = object
        .get("tool")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
    {
        let name = name.to_string();
        if let Some(call_id) = call_id {
            call_name_map.insert(call_id.to_string(), name.clone());
        }
        return Some(name);
    }

    if let Some(name) = object
        .get("tool_name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
    {
        let name = name.to_string();
        if let Some(call_id) = call_id {
            call_name_map.insert(call_id.to_string(), name.clone());
        }
        return Some(name);
    }

    if let Some(metadata) = object.get("metadata").and_then(Value::as_object) {
        if let Some(name) = metadata
            .get("tool")
            .and_then(Value::as_str)
            .or_else(|| metadata.get("name").and_then(Value::as_str))
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            let name = name.to_string();
            if let Some(call_id) = call_id {
                call_name_map.insert(call_id.to_string(), name.clone());
            }
            return Some(name);
        }
    }

    if let Some(tool_name) = call_id.and_then(|tool_id| call_name_map.get(tool_id)) {
        return Some(tool_name.clone());
    }

    None
}

fn extract_tool_name_from_value(value: &Value) -> Option<String> {
    let mut names = Vec::new();
    collect_tool_names_from_value(Some(value), &mut HashMap::new(), &mut names);
    names.into_iter().next()
}

fn replay_value_has_hidden_tool_payload(object: &serde_json::Map<String, Value>) -> bool {
    if let Some(value_type) = object.get("type").and_then(Value::as_str) {
        if matches!(
            value_type,
            "tool_use" | "tool_result" | "tool_call" | "tool"
        ) {
            return true;
        }
    }

    if object.get("tool").is_some()
        || object.get("tool_name").is_some()
        || object.get("tool_call_id").is_some()
        || object.get("tool_use_id").is_some()
        || object.get("tool_result").is_some()
        || object.get("tool_calls").is_some()
        || object.get("arguments").is_some()
        || object.get("result").is_some()
        || object.get("input").is_some()
        || object.get("output").is_some()
    {
        return true;
    }

    if object.get("metadata").is_some_and(|metadata| {
        metadata
            .get("tool")
            .and_then(Value::as_object)
            .is_some_and(|metadata| metadata.contains_key("name"))
    }) {
        return true;
    }

    if object.get("name").is_some()
        && (object.get("arguments").is_some()
            || object.get("input").is_some()
            || object.get("result").is_some())
    {
        return true;
    }

    if object.get("metadata").is_some_and(|metadata| {
        metadata.as_object().is_some_and(|metadata| {
            metadata.contains_key("tool")
                || metadata.contains_key("tool_name")
                || metadata.contains_key("tool_call_id")
                || metadata.contains_key("tool_use_id")
        })
    }) {
        return true;
    }

    if let Some(function) = object.get("function") {
        if let Some(name) = function.get("name") {
            return name.is_string();
        }
        return true;
    }

    false
}

fn replay_content_has_visible_text(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(replay_content_has_visible_text),
        Value::Object(object) => {
            if replay_value_has_hidden_tool_payload(object) {
                return false;
            }
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    return true;
                }
            }
            if let Some(text) = object.get("content").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    return true;
                }
            }
            if let Some(text) = object.get("value").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    return true;
                }
            }
            if let Some(text) = object.get("input").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    return true;
                }
            }
            if let Some(parts) = object.get("parts").and_then(Value::as_array) {
                if parts.iter().any(replay_content_has_visible_text) {
                    return true;
                }
            }
            if let Some(items) = object.get("items").and_then(Value::as_array) {
                if items.iter().any(replay_content_has_visible_text) {
                    return true;
                }
            }
            if let Some(content) = object.get("content") {
                return replay_content_has_visible_text(content);
            }
            false
        }
        _ => false,
    }
}

fn replay_content_is_tool_only(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            if text.trim().is_empty() {
                return false;
            }
            if let Ok(json) = serde_json::from_str::<Value>(text) {
                return replay_content_is_tool_only(&json);
            }
            false
        }
        Value::Array(items) => !items.is_empty() && items.iter().all(replay_content_is_tool_only),
        Value::Object(object) => {
            if replay_value_has_hidden_tool_payload(object) {
                return true;
            }
            if let Some(parts) = object.get("parts").and_then(Value::as_array) {
                return !parts.is_empty() && parts.iter().all(replay_content_is_tool_only);
            }
            if let Some(content) = object.get("content") {
                return replay_content_is_tool_only(content);
            }
            if let Some(items) = object.get("items").and_then(Value::as_array) {
                return !items.is_empty() && items.iter().all(replay_content_is_tool_only);
            }
            false
        }
        _ => false,
    }
}

fn collect_replay_content_text(value: &Value, chunks: &mut Vec<String>) {
    if replay_content_is_tool_only(value) {
        return;
    }

    match value {
        Value::String(text) => {
            if !text.trim().is_empty() {
                chunks.push(text.to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_replay_content_text(item, chunks);
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    chunks.push(text.to_string());
                }
                return;
            }
            if let Some(text) = object.get("content").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    chunks.push(text.to_string());
                }
                return;
            }
            if let Some(text) = object.get("value").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    chunks.push(text.to_string());
                }
                return;
            }
            if let Some(text) = object.get("input").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    chunks.push(text.to_string());
                }
                return;
            }

            if let Some(parts) = object.get("parts").and_then(Value::as_array) {
                for part in parts {
                    collect_replay_content_text(part, chunks);
                }
                return;
            }

            if let Some(items) = object.get("items").and_then(Value::as_array) {
                for item in items {
                    collect_replay_content_text(item, chunks);
                }
                return;
            }

            if let Some(content) = object.get("content") {
                collect_replay_content_text(content, chunks);
            }
        }
        _ => {}
    }
}

fn extract_replay_content_text(value: &Value) -> Option<String> {
    let mut chunks = Vec::new();
    collect_replay_content_text(value, &mut chunks);
    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join("\n"))
    }
}

#[cfg(test)]
fn split_rendered_markdown_lines(text: &str) -> Vec<String> {
    let rendered = render_markdown_text(text);
    if rendered.is_empty() {
        return Vec::new();
    }
    rendered
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect()
}

fn interactive_session_replay_user_inputs(saved: &session::SavedSession) -> Vec<String> {
    let mut out = Vec::new();
    for message in &saved.messages {
        if !is_replay_user_input_message(message) {
            continue;
        }
        for line in interactive_session_input_message_lines(message.get("content")) {
            let line = line.trim_end();
            if !line.trim().is_empty() {
                out.push(line.to_string());
            }
        }
    }
    out
}

fn is_replay_user_input_message(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user")
        && message.get("metadata_deferred").and_then(Value::as_bool) != Some(true)
        && !is_replay_tool_only_message(message)
}

fn interactive_session_input_message_lines(content: Option<&Value>) -> Vec<String> {
    let Some(content) = content else {
        return Vec::new();
    };
    interactive_session_input_value_lines(content)
}

fn interactive_session_input_value_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    match value {
        Value::String(text) => {
            lines.extend(split_input_text_lines(text));
        }
        Value::Array(items) => {
            for item in items {
                lines.extend(interactive_session_input_value_lines(item));
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                lines.extend(split_input_text_lines(text));
                return lines;
            }
            if let Some(text) = object.get("content").and_then(Value::as_str) {
                lines.extend(split_input_text_lines(text));
                return lines;
            }
            if let Some(parts) = object.get("parts").and_then(Value::as_array) {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        lines.extend(split_input_text_lines(text));
                        continue;
                    }
                    lines.extend(interactive_session_input_value_lines(part));
                }
                if !lines.is_empty() {
                    return lines;
                }
            }
            if let Some(items) = object.get("items").and_then(Value::as_array) {
                for item in items {
                    lines.extend(interactive_session_input_value_lines(item));
                }
                if !lines.is_empty() {
                    return lines;
                }
            }
            if let Some(value) = object.get("value").or_else(|| object.get("input")) {
                if let Some(text) = value.as_str() {
                    lines.extend(split_input_text_lines(text));
                } else {
                    lines.extend(interactive_session_input_value_lines(value));
                }
                if !lines.is_empty() {
                    return lines;
                }
            }
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "text" | "content" | "parts" | "items" | "value" | "input"
                ) {
                    continue;
                }
                lines.extend(interactive_session_input_value_lines(value));
            }
        }
        _ => {}
    }
    lines
}

fn split_input_text_lines(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n");
    normalized
        .lines()
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.trim().is_empty())
        .collect()
}

struct ResumeSessionSelectionUiStateGuard;

impl Drop for ResumeSessionSelectionUiStateGuard {
    fn drop(&mut self) {
        let _ = restore_resume_session_screen();
    }
}

fn restore_interactive_session_messages(
    interactive_config: &Config,
    started: &mut bool,
    turn: &mut usize,
    input_history: &mut Vec<String>,
) -> Result<Option<session::SavedSession>, String> {
    let restore_started = Instant::now();
    let load_started = Instant::now();
    let explicit_session = interactive_config.session_id.is_some();
    let session_id = if let Some(id) = interactive_config.session_id.as_ref() {
        Some(id.clone())
    } else if interactive_config.continue_last {
        resolve_continue_session_id(interactive_config)?
    } else {
        None
    };
    let Some(session_id) = session_id else {
        return Ok(None);
    };
    let snapshot = match session::load_interactive_session_snapshot(&session_id) {
        Ok(snapshot) => snapshot,
        Err(error) if explicit_session => {
            if !interactive_config.json_output {
                eprintln!("{error}");
            }
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let saved = snapshot.saved;
    let total_message_count = snapshot.total_message_count;
    let total_user_turns = snapshot.total_user_turns;
    if saved.messages.is_empty() && total_message_count == 0 {
        *started = false;
        *turn = 1;
        return Ok(Some(saved));
    }
    let load_elapsed_ms = load_started.elapsed().as_millis() as u64;
    let input_history_started = Instant::now();
    input_history.clear();
    for input in interactive_session_replay_user_inputs(&saved) {
        if !input.trim().is_empty() {
            input_history.push(input);
            if input_history.len() > 128 {
                input_history.drain(0..(input_history.len() - 128));
            }
        }
    }
    let input_history_elapsed_ms = input_history_started.elapsed().as_millis() as u64;
    let turn_started = Instant::now();
    *started = true;
    *turn = total_user_turns.saturating_add(1);
    let turn_elapsed_ms = turn_started.elapsed().as_millis() as u64;

    llm::log_local_debug_payload(
        "perf_interactive_restore",
        &json!({
            "type": "perf_interactive_restore",
            "session_id": saved.id,
            "message_count": total_message_count,
            "visible_message_count": saved.messages.len(),
            "input_history_count": input_history.len(),
            "next_turn": *turn,
            "load_elapsed_ms": load_elapsed_ms,
            "input_history_elapsed_ms": input_history_elapsed_ms,
            "turn_elapsed_ms": turn_elapsed_ms,
            "elapsed_ms": restore_started.elapsed().as_millis() as u64,
        }),
    );

    Ok(Some(saved))
}

fn load_interactive_session_for_command(
    config: &Config,
) -> Result<Option<session::SavedSession>, String> {
    let explicit_session = config.session_id.is_some();
    let session_id = if let Some(id) = config.session_id.as_ref() {
        id.clone()
    } else if config.continue_last {
        let Some(latest) = resolve_continue_session_id(config)? else {
            return Ok(None);
        };
        latest
    } else {
        return Ok(None);
    };

    match session::load_session(&session_id) {
        Ok(saved) => Ok(Some(saved)),
        Err(err) if explicit_session => {
            if !config.json_output {
                eprintln!("{err}");
            }
            Ok(None)
        }
        Err(err) => Err(err.to_string()),
    }
}
