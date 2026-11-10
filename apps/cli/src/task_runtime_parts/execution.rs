use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use nola_core::context_engine::{
    ContextComponent, ContextComponentKind, ContextComponentSource, ContextPlatform,
};
use serde_json::{json, Map, Value};

use crate::agent_host_driver::{RuntimeTaskPromotion, RuntimeTaskUpdate, RuntimeToolResult};
use crate::agent_registry::AgentRegistry;
use crate::host_invocation_request::HostInvocationRequest;
use crate::core_agent_runtime::{
    cancel_agent_core_task, run_agent_core_with_cancellation, update_agent_core_task,
};
use crate::i18n::{current_language, ui_text, Language, UiText};
use crate::runtime_control::RuntimeExecutionControl;
use crate::runtime_interaction::{PermissionChoice, PermissionPrompt, PermissionPromptKind};
use crate::process_observation::process_id_is_alive;
use crate::session;
use crate::store::{DbBackgroundTask, DbRuntimeTask, NolaStore};
use crate::tools::{self, ToolContext, ToolOutcome};
use nola_core::agent::host::HostRequestEnvelope;
use nola_core::agent::{SopWaitKind, SopWaitRecord};

const DEFAULT_MAX_AGENT_TASKS: usize = 4;
const DEFAULT_WAIT_SECONDS: u64 = 30;
const MAX_WAIT_SECONDS: u64 = 300;
const RESULT_SUMMARY_LIMIT: usize = 24_000;
const DEFAULT_TASK_WALL_TIME_MS: u64 = 2 * 60 * 60 * 1_000;
const DEFAULT_TASK_OUTPUT_BYTES: u64 = 2 * 1024 * 1024;

/// Configurations bound by active Core Hosts. Tool execution may happen on a
/// worker thread rather than the thread that pumps the Host, so this registry
/// deliberately crosses thread boundaries. Entries are scoped by the same
/// permission owner that is persisted on runtime tasks; they are removed when
/// the Host turn exits.
static BOUND_SUPERVISOR_CONFIGS: OnceLock<Mutex<SupervisorBindingRegistry>> = OnceLock::new();

#[derive(Default)]
struct SupervisorBindingRegistry {
    scopes: HashMap<String, Vec<(String, HostInvocationRequest)>>,
}

impl SupervisorBindingRegistry {
    fn insert(&mut self, key: String, id: String, request: HostInvocationRequest) {
        self.scopes.entry(key).or_default().push((id, request));
    }

    fn remove(&mut self, key: &str, id: &str) {
        if let Some(bindings) = self.scopes.get_mut(key) {
            bindings.retain(|(binding_id, _)| binding_id != id);
            if bindings.is_empty() { self.scopes.remove(key); }
        }
    }

    fn select(&self, owner: Option<&str>, project_root: &Path) -> Option<HostInvocationRequest> {
        if let Some(owner) = owner {
            let owner = owner.trim();
            if owner.is_empty() { return None; }
            // An explicit but unavailable owner must not inherit another
            // session's model/permissions merely by sharing a filesystem root.
            return self.scopes.get(&format!("scope:{owner}"))
                .filter(|bindings| bindings.len() == 1)
                .and_then(|bindings| bindings.first()).map(|(_, request)| request.clone());
        }
        let requested_root = canonical_project_root(project_root);
        let mut candidates = self.scopes.values().flatten()
            .filter(|(_, request)| {
                let root = canonical_project_root(&request.tool_settings.project_root);
                requested_root == root || requested_root.starts_with(root)
            });
        let (_, candidate) = candidates.next()?;
        // No identity is available to choose between distinct active owners.
        if candidates.next().is_some() { return None; }
        Some(candidate.clone())
    }
}

pub(crate) struct SupervisorConfigBinding {
    key: String,
    id: String,
}

impl Drop for SupervisorConfigBinding {
    fn drop(&mut self) {
        let registry = BOUND_SUPERVISOR_CONFIGS.get_or_init(|| Mutex::new(SupervisorBindingRegistry::default()));
        if let Ok(mut registry) = registry.lock() {
            registry.remove(&self.key, &self.id);
        }
    }
}

pub(crate) fn bind_supervisor_config(config: &HostInvocationRequest) -> SupervisorConfigBinding {
    let key = supervisor_config_binding_key(config);
    let id = uuid::Uuid::new_v4().to_string();
    let registry = BOUND_SUPERVISOR_CONFIGS.get_or_init(|| Mutex::new(SupervisorBindingRegistry::default()));
    if let Ok(mut registry) = registry.lock() {
        registry.insert(key.clone(), id.clone(), config.clone());
    }
    SupervisorConfigBinding { key, id }
}

/// Give queued runtime tasks a chance to claim a newly released shared slot
/// during an active Core turn. The interactive composer cannot perform this
/// reconciliation while a foreground Tool call owns the Host, so the call's
/// bound immutable configuration is used here. The owner scope is preferred;
/// project-root matching is only a recovery fallback for contexts that do not
/// carry a persisted owner identity.
pub(crate) fn reconcile_bound_supervised_tasks(owner_scope_id: Option<&str>, project_root: &Path) {
    let registry = BOUND_SUPERVISOR_CONFIGS.get_or_init(|| Mutex::new(SupervisorBindingRegistry::default()));
    let config = registry.lock().ok().and_then(|registry| registry.select(owner_scope_id, project_root));
    if let Some(config) = config {
        reconcile_supervised_tasks(&config);
    }
}

fn supervisor_config_binding_key(config: &HostInvocationRequest) -> String {
    config.session.permission_scope_id
        .as_deref()
        .or(config.session.session_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("scope:{value}"))
        .unwrap_or_else(|| {
            format!(
                "project:{}",
                canonical_project_root(&config.tool_settings.project_root).display()
            )
        })
}

fn canonical_project_root(path: &Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod supervisor_binding_registry_tests {
    use super::*;

    fn request(root: &Path, model: &str) -> HostInvocationRequest {
        let mut request = HostInvocationRequest::for_test().unwrap();
        request.tool_settings.project_root = root.to_path_buf();
        request.model_settings.model = model.into();
        request
    }

    #[test]
    fn explicit_missing_owner_never_borrows_same_workspace_authority() {
        let root = tempfile::tempdir().unwrap();
        let mut registry = SupervisorBindingRegistry::default();
        registry.insert("scope:other".into(), "other-binding".into(), request(root.path(), "other-model"));
        assert!(registry.select(Some("missing"), root.path()).is_none());
        assert!(registry.select(Some("  "), root.path()).is_none());
        assert!(registry.select(None, root.path()).is_some());
        registry.insert("scope:another".into(), "another-binding".into(), request(root.path(), "another-model"));
        assert!(registry.select(None, root.path()).is_none());
        assert_eq!(registry.select(Some("other"), root.path()).unwrap().model_settings.model, "other-model");
    }

    #[test]
    fn same_owner_live_bindings_are_ambiguous_and_non_lifo_drop_cannot_revive_them() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(Mutex::new(SupervisorBindingRegistry::default()));
        {
            let mut state = registry.lock().unwrap();
            state.insert("scope:owner".into(), "old".into(), request(root.path(), "old-model"));
            state.insert("scope:owner".into(), "new".into(), request(root.path(), "new-model"));
            assert!(state.select(Some("owner"), root.path()).is_none());
            assert!(state.select(None, root.path()).is_none());
        }
        let drop_old = registry.clone();
        std::thread::spawn(move || drop_old.lock().unwrap().remove("scope:owner", "old")).join().unwrap();
        assert_eq!(registry.lock().unwrap().select(Some("owner"), root.path()).unwrap().model_settings.model, "new-model");
        let drop_new = registry.clone();
        std::thread::spawn(move || drop_new.lock().unwrap().remove("scope:owner", "new")).join().unwrap();
        assert!(registry.lock().unwrap().select(Some("owner"), root.path()).is_none());
        assert!(registry.lock().unwrap().select(None, root.path()).is_none());
    }
}

pub const fn runtime_tool_ids() -> &'static [&'static str] {
    &[
        "spawn_agent",
        "list_tasks",
        "read_task",
        "wait_task",
        "stop_task",
        "send_agent_message",
    ]
}

#[derive(Clone)]
struct AgentTaskHandle {
    control_id: String,
    lease: Option<crate::runtime_task_port::RuntimeTaskLeaseHandle>,
    execution: Option<crate::runtime_task_port::RuntimeTaskExecutionAuthority>,
    permission_resume: Option<ColdPermissionResume>,
    cancellation: RuntimeExecutionControl,
    child_session_id: String,
}

/// An exact permission occurrence transported to its newly granted worker.
/// This is evidence to revalidate, never authority reconstructed from a task ID.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ColdPermissionResume {
    suspension: nola_application::RuntimeTaskPermissionSuspension,
    original: nola_application::RuntimeTaskAttempt,
    grant: nola_application::RuntimeTaskAttempt,
}

fn release_agent_task_handle(
    task_id: &str,
    lease: &crate::runtime_task_port::RuntimeTaskLeaseHandle,
) {
    if let Ok(mut handles) = task_handles().lock() {
        if handles.get(task_id).is_some_and(|handle| {
            handle
                .lease
                .as_ref()
                .is_some_and(|current| current.same_owner(lease))
                || handle
                    .execution
                    .as_ref()
                    .is_some_and(|execution| execution.owns(lease))
        }) {
            handles.remove(task_id);
        }
    }
}

fn release_agent_task_control(task_id: &str, control_id: &str) {
    if let Ok(mut handles) = task_handles().lock() {
        if handles
            .get(task_id)
            .is_some_and(|handle| handle.control_id == control_id)
        {
            handles.remove(task_id);
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AgentTaskProcessRequest {
    task_id: String,
    title: String,
    prompt: String,
    owner_session_id: String,
    input_watermark: i64,
    worker_id: String,
    ownership_token: String,
    runtime_task_attempt: Option<nola_application::RuntimeTaskAttempt>,
    lease_duration_ms: i64,
    config: HostInvocationRequest,
    permission_resume: Option<ColdPermissionResume>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ShellTaskProcessRequest {
    task: DbBackgroundTask,
    worker_id: String,
    ownership_token: String,
    runtime_task_attempt: nola_application::RuntimeTaskAttempt,
    lease_duration_ms: i64,
    database_path: std::path::PathBuf,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProcessGroupUsage {
    pub(crate) cpu_time_ms: u64,
    pub(crate) memory_bytes: u64,
    pub(crate) processes: u32,
    pub(crate) pids: Vec<u32>,
}

#[derive(Debug, Clone)]
pub(crate) enum BackgroundPermissionDecision {
    Allow,
    AllowTask,
    Custom(String),
    ReturnToComposer,
    Deny,
}

/// Ending the waiting transport is not a user authorization decision.
#[derive(Debug, Clone)]
pub(crate) enum BackgroundPermissionWait {
    Decision(BackgroundPermissionDecision),
    StopRequested,
    /// Core consumed this occurrence; no second authorization response is due.
    Settled,
}

static AGENT_TASK_HANDLES: OnceLock<Mutex<HashMap<String, AgentTaskHandle>>> = OnceLock::new();
static TASK_WAKE: OnceLock<(Mutex<u64>, Condvar)> = OnceLock::new();
static TASK_OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

const AGENT_TASK_WORKER_ARG: &str = "--nola-internal-agent-task-worker";
const SHELL_TASK_WORKER_ARG: &str = "--nola-internal-shell-task-worker";

pub fn run_agent_worker_if_requested() -> Option<i32> {
    if !std::env::args().any(|argument| argument == AGENT_TASK_WORKER_ARG) {
        return None;
    }
    let mut encoded = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut encoded) {
        eprintln!("read Agent task worker request failed: {error}");
        return Some(2);
    }
    let request: AgentTaskProcessRequest = match serde_json::from_str(&encoded) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("decode Agent task worker request failed: {error}");
            return Some(2);
        }
    };
    if request.runtime_task_attempt.as_ref().is_none_or(|attempt| {
        attempt.task_id != request.task_id
            || attempt.worker_id != request.worker_id
            || attempt.lease_token != request.ownership_token
    }) {
        eprintln!("Agent task worker request has no matching RuntimeTask attempt identity");
        return Some(2);
    }
    let lease = crate::runtime_task_port::register_runtime_task_attempt(
        request
            .runtime_task_attempt
            .as_ref()
            .expect("validated attempt identity")
            .clone(),
    );
    if let Some(language) = request
        .config
        .runtime
        .language
        .as_deref()
        .and_then(crate::i18n::parse_language_tag)
    {
        crate::i18n::set_language(language);
    }
    let cancellation = RuntimeExecutionControl::default();
    if let Ok(mut handles) = task_handles().lock() {
        handles.insert(
            request.task_id.clone(),
            AgentTaskHandle {
                control_id: uuid::Uuid::new_v4().to_string(),
                lease: Some(lease.clone()),
                execution: None,
                permission_resume: request.permission_resume.clone(),
                cancellation: cancellation.clone(),
                child_session_id: request
                    .config.session.session_id
                    .clone()
                    .unwrap_or_else(|| format!("child-{}", request.task_id)),
            },
        );
    }
    let status = run_agent_task(
        &request.task_id,
        &request.title,
        &request.prompt,
        &request.owner_session_id,
        request.input_watermark,
        request.worker_id,
        lease,
        request.lease_duration_ms,
        request.config,
        cancellation,
        request.permission_resume,
    );
    Some(status)
}

pub fn run_shell_worker_if_requested() -> Option<i32> {
    if !std::env::args().any(|argument| argument == SHELL_TASK_WORKER_ARG) {
        return None;
    }
    let mut encoded = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut encoded) {
        eprintln!("read Shell task worker request failed: {error}");
        return Some(2);
    }
    let request: ShellTaskProcessRequest = match serde_json::from_str(&encoded) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("decode Shell task worker request failed: {error}");
            return Some(2);
        }
    };
    if request.runtime_task_attempt.task_id != request.task.id
        || request.runtime_task_attempt.worker_id != request.worker_id
        || request.runtime_task_attempt.lease_token != request.ownership_token
    {
        eprintln!("Shell task worker request has no matching RuntimeTask attempt identity");
        return Some(2);
    }
    crate::runtime_task_port::register_runtime_task_attempt(request.runtime_task_attempt.clone());
    Some(run_shell_task(request))
}

pub fn queue_background_shell_task(
    task: DbBackgroundTask,
    scheduling: nola_application::RuntimeTaskScheduling,
) -> Result<bool, String> {
    let store = NolaStore::open_default()?;
    queue_background_shell_task_with_launcher(&store, task, scheduling, spawn_shell_task_process)
}

fn queue_background_shell_task_with_launcher(
    store: &NolaStore,
    task: DbBackgroundTask,
    scheduling: nola_application::RuntimeTaskScheduling,
    launch: impl FnOnce(DbBackgroundTask) -> Result<bool, String>,
) -> Result<bool, String> {
    store.save_background_task_with_scheduling(&task, scheduling)?;
    // A launch error is not a rollback of the durable submission. Callers
    // retain the original task identity and files for canonical observation.
    launch(task)
}

pub(crate) fn try_start_queued_background_shell_task(
    task: DbBackgroundTask,
) -> Result<bool, String> {
    spawn_shell_task_process(task)
}

fn runtime_task_concurrency_limits() -> (u32, u32) {
    let max_tasks = std::env::var("NOLA_MAX_CONCURRENT_TASKS")
        .or_else(|_| std::env::var("NOLA_AGENT_MAX_CONCURRENT_TASKS"))
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(DEFAULT_MAX_AGENT_TASKS as u32)
        .clamp(1, 32);
    let max_project_tasks = std::env::var("NOLA_MAX_PROJECT_TASKS")
        .or_else(|_| std::env::var("NOLA_AGENT_MAX_PROJECT_TASKS"))
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(max_tasks)
        .clamp(1, max_tasks);
    (max_tasks, max_project_tasks)
}

pub(crate) struct ExternalExecutionTaskLease {
    task_id: String,
    lease: crate::runtime_task_port::RuntimeTaskLeaseHandle,
    backend_id: String,
    parent_effect_id: String,
    heartbeat_stop: Option<std::sync::mpsc::Sender<()>>,
    heartbeat_handle: Option<std::thread::JoinHandle<()>>,
    terminal: bool,
}

impl ExternalExecutionTaskLease {
    pub(crate) fn acquire(
        config: &crate::external_execution_settings::ExternalExecutionSettings,
        owner_session_id: Option<String>,
        parent_effect_id: &str,
        backend_id: &str,
        cancellation: &RuntimeExecutionControl,
    ) -> Result<Self, String> {
        let store = NolaStore::open_default()?;
        let now = unix_epoch_millis();
        let task_id = format!(
            "executor-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..16]
        );
        let title = config
            .external_execution_model
            .as_deref()
            .map(|model| format!("{backend_id} · {model}"))
            .unwrap_or_else(|| backend_id.to_string());
        let budget = &config.budget;
        let input = nola_application::CreateRuntimeTask {
            id: task_id.clone(),
            kind: "external_executor".to_string(),
            title: title.clone(),
            project_root: config.project_root.to_string_lossy().into_owned(),
            owner_session_id,
            parent_task_id: None,
            parent_tool_call_id: Some(parent_effect_id.to_string()),
            child_session_id: config.external_execution_session_id.clone(),
            agent_type: Some(backend_id.to_string()),
            prompt: None,
            initial_progress_summary: Some(ui_text(UiText::Queued).to_string()),
            initial_activity: None,
            output_path: None,
            initially_unread: false,
            input_watermark: Some(now),
            created_at_ms: now,
            metadata: json!({
                "backend_id": backend_id,
                "model_id": config.external_execution_model,
                "model_provider": config.external_execution_model_provider,
                "parent_effect_id": parent_effect_id,
                "host_pid": std::process::id(),
                "safe_to_retry_before_claim": true,
                "safe_to_retry_after_claim": false
            }),
            scheduling: nola_application::RuntimeTaskScheduling {
                priority: nola_application::RuntimeTaskPriority::Normal,
                queued_at_ms: now,
                resource_budget: nola_application::RuntimeTaskResourceBudget {
                    max_wall_time_ms: Some(
                        budget
                            .max_wall_time_seconds
                            .and_then(|value| u64::try_from(value).ok())
                            .unwrap_or(24 * 60 * 60)
                            .saturating_mul(1_000),
                    ),
                    max_output_bytes: Some(
                        budget
                            .max_output_bytes
                            .and_then(|value| u64::try_from(value).ok())
                            .unwrap_or(64 * 1024 * 1024),
                    ),
                    max_cpu_time_ms: Some(
                        budget
                            .max_cpu_time_seconds
                            .and_then(|value| u64::try_from(value).ok())
                            .unwrap_or(12 * 60 * 60)
                            .saturating_mul(1_000),
                    ),
                    max_memory_bytes: Some(
                        budget
                            .max_memory_bytes
                            .and_then(|value| u64::try_from(value).ok())
                            .unwrap_or(8 * 1024 * 1024 * 1024),
                    ),
                    max_processes: Some(
                        budget
                            .max_processes
                            .and_then(|value| u32::try_from(value).ok())
                            .unwrap_or(128),
                    ),
                },
                output_bytes: 0,
                resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
                budget_exhaustion: None,
            },
        };
        crate::runtime_task_port::create(input)?;
        tools::wake_background_task_notification();

        let worker_id = format!(
            "cli-external:{}:{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        );
        let ownership_token = uuid::Uuid::new_v4().simple().to_string();
        let lease_duration_ms = 45_000_i64;
        let (max_tasks, max_project_tasks) = runtime_task_concurrency_limits();
        let attempt = loop {
            if cancellation.is_requested() {
                let cancelled_at = unix_epoch_millis();
                let _ = store.request_runtime_task_cancel(&task_id, cancelled_at);
                let _ = store.append_runtime_task_event(
                    &task_id,
                    "cancelled_before_start",
                    "cancelled",
                    Some("External executor cancelled while waiting for a Supervisor slot"),
                    "{}",
                    cancelled_at,
                );
                return Err("external executor cancelled before start".to_string());
            }
            if store
                .runtime_task(&task_id)?
                .is_some_and(|task| task.cancel_requested || task.status == "cancelled")
            {
                cancellation.request();
                return Err("external executor cancelled before start".to_string());
            }
            let claim_at = unix_epoch_millis();
            if let Some(attempt) = store.claim_runtime_task_fair(
                &task_id,
                &worker_id,
                crate::runtime_task_port::worker_generation(),
                &ownership_token,
                claim_at,
                claim_at.saturating_add(lease_duration_ms),
                max_tasks,
                max_project_tasks,
            )? {
                break attempt;
            }
            std::thread::sleep(Duration::from_millis(50));
        };

        let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt);
        let started_at = unix_epoch_millis();
        crate::runtime_task_port::report_progress(
            &lease,
            Some(ui_text(UiText::Running).to_string()),
            None,
            None,
            None,
            BTreeMap::new(),
            started_at,
        )?;
        store.append_runtime_task_event(
            &task_id,
            "started",
            "running",
            Some(&title),
            &json!({"backend_id":backend_id,"parent_effect_id":parent_effect_id}).to_string(),
            started_at,
        )?;

        let (heartbeat_stop, heartbeat_rx) = std::sync::mpsc::channel::<()>();
        let heartbeat_lease = lease.clone();
        let heartbeat_cancellation = cancellation.clone();
        let heartbeat_handle = match std::thread::Builder::new()
            .name(format!("nola-external-heartbeat-{task_id}"))
            .spawn(move || loop {
                match heartbeat_rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        let heartbeat_at = unix_epoch_millis();
                        let renewed = NolaStore::open_default().and_then(|store| {
                            store.renew_runtime_task_lease(
                                &heartbeat_lease,
                                heartbeat_at,
                                heartbeat_at.saturating_add(lease_duration_ms),
                            )
                        });
                        match renewed {
                            Ok(_) => {}
                            Err(_) => {
                                heartbeat_cancellation.request();
                                break;
                            }
                        }
                    }
                }
            }) {
            Ok(handle) => handle,
            Err(error) => {
                let reason = format!("start external executor heartbeat failed: {error}");
                let _ = crate::runtime_task_port::finish_detailed_with_event(
                    &lease,
                    "failed",
                    None,
                    Some("start_failed".to_string()),
                    None,
                    Some(reason.clone()),
                    None,
                    true,
                    "start_failed",
                    "failed",
                    Some(reason.clone()),
                    "{}".to_string(),
                    unix_epoch_millis(),
                );
                return Err(reason);
            }
        };

        Ok(Self {
            task_id,
            lease,
            backend_id: backend_id.to_string(),
            parent_effect_id: parent_effect_id.to_string(),
            heartbeat_stop: Some(heartbeat_stop),
            heartbeat_handle: Some(heartbeat_handle),
            terminal: false,
        })
    }

    pub(crate) fn finish_completed(mut self) -> Result<(), String> {
        self.finish("completed", None)
    }

    pub(crate) fn finish_failed(mut self, error: &str) -> Result<(), String> {
        self.finish("failed", Some(error))
    }

    pub(crate) fn finish_cancelled(mut self) -> Result<(), String> {
        self.finish("cancelled", None)
    }

    fn stop_heartbeat(&mut self) {
        if let Some(stop) = self.heartbeat_stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.heartbeat_handle.take() {
            let _ = handle.join();
        }
    }

    fn finish(&mut self, status: &str, error: Option<&str>) -> Result<(), String> {
        self.stop_heartbeat();
        let now = unix_epoch_millis();
        let result_summary =
            (status == "completed").then(|| format!("{} completed", self.backend_id));
        let attempt = self.lease.snapshot()?;
        let detail = json!({
            "backend_id": self.backend_id,
            "parent_effect_id": self.parent_effect_id,
            "runtime_task_attempt_id": attempt.id,
            "runtime_task_attempt_revision": attempt.revision,
            "error": error
        })
        .to_string();
        let finished = crate::runtime_task_port::finish_detailed_with_event(
            &self.lease,
            status,
            None,
            Some(status.to_string()),
            result_summary,
            error.map(str::to_string),
            None,
            true,
            "finished",
            status,
            Some(error.unwrap_or(&self.backend_id).to_string()),
            detail,
            now,
        )?;
        if !finished {
            return Err("external executor lost its Supervisor ownership lease".to_string());
        }
        self.terminal = true;
        wake_task_waiters();
        tools::wake_background_task_notification();
        Ok(())
    }
}

impl Drop for ExternalExecutionTaskLease {
    fn drop(&mut self) {
        self.stop_heartbeat();
        if self.terminal {
            return;
        }
        let now = unix_epoch_millis();
        let reason = "external executor worker ended without a committed terminal state";
        let _ = crate::runtime_task_port::finish_detailed_with_event(
            &self.lease,
            "failed",
            None,
            Some("needs_reconciliation".to_string()),
            None,
            Some(reason.to_string()),
            None,
            true,
            "reconciliation_required",
            "failed",
            Some(reason.to_string()),
            json!({"backend_id":self.backend_id,"safe_to_retry":false}).to_string(),
            now,
        );
        wake_task_waiters();
        tools::wake_background_task_notification();
    }
}

pub(crate) struct McpExecutionTaskLease {
    task_id: String,
    lease: crate::runtime_task_port::RuntimeTaskLeaseHandle,
    server_id: String,
    tool_name: String,
    heartbeat_stop: Option<std::sync::mpsc::Sender<()>>,
    heartbeat_handle: Option<std::thread::JoinHandle<()>>,
    last_progress_at_ms: i64,
    terminal: bool,
}

impl McpExecutionTaskLease {
    fn effect_task_id(project_root: &str, owner: &Option<String>, effect: &str) -> String {
        use sha2::{Digest, Sha256};
        let identity = serde_json::json!(["mcp-effect-v1", project_root, owner, effect]);
        format!("mcp-{:x}", Sha256::digest(identity.to_string().as_bytes()))
    }

    fn arguments_digest(arguments: &Value) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(arguments.to_string().as_bytes()))
    }

    fn recover_task_id(
        observed: Result<Vec<DbRuntimeTask>, String>,
        owner_session_id: &Option<String>,
        parent_effect_id: &str,
        server_id: &str,
        tool_name: &str,
        read_only: bool,
        arguments_digest: &str,
    ) -> Result<Option<String>, String> {
        let tasks = observed.map_err(|error| format!(
            "MCP effect {parent_effect_id}: recovery observation failed; no replacement task created: {error}"
        ))?;
        let mut matching = tasks.into_iter().filter(|task| {
            task.kind == "mcp" && &task.owner_session_id == owner_session_id
                && task.parent_tool_call_id.as_deref() == Some(parent_effect_id)
        });
        let Some(task) = matching.next() else { return Ok(None); };
        if let Some(other) = matching.next() {
            return Err(format!("MCP effect {parent_effect_id}: ambiguous existing tasks {} and {}; reconciliation required", task.id, other.id));
        }
        let metadata: Value = serde_json::from_str(&task.metadata_json).map_err(|error|
            format!("MCP task {} for effect {parent_effect_id}: invalid recovery metadata: {error}", task.id))?;
        if !read_only || metadata.get("read_only").and_then(Value::as_bool) != Some(true)
            || metadata.get("arguments_digest").and_then(Value::as_str) != Some(arguments_digest)
            || metadata.get("server_id").and_then(Value::as_str) != Some(server_id)
            || metadata.get("tool_name").and_then(Value::as_str) != Some(tool_name)
            || metadata.get("parent_effect_id").and_then(Value::as_str) != Some(parent_effect_id)
            || task.agent_type.as_deref() != Some(server_id)
            || task.last_activity.as_deref() != Some(tool_name)
            || task.parent_task_id.is_some() || task.child_session_id.is_some()
            || task.prompt.is_some() || task.output_path.is_some()
            || task.cancel_requested
            || !matches!(task.status.as_str(), "queued" | "retry_waiting")
        {
            return Err(format!("MCP task {} for effect {parent_effect_id} cannot be reacquired (state {}); original task retained", task.id, task.status));
        }
        // A future retry deadline is not absence. The fair claim owns readiness
        // and preserves this task's identity while waiting for its retry slot.
        Ok(Some(task.id))
    }

    pub(crate) fn acquire(
        project_root: &std::path::Path,
        owner_session_id: Option<String>,
        parent_effect_id: &str,
        server_id: &str,
        tool_name: &str,
        read_only: bool,
        arguments: &Value,
        cancellation: &RuntimeExecutionControl,
    ) -> Result<Self, String> {
        let store = NolaStore::open_default()?;
        let now = unix_epoch_millis();
        // Match Store's project grouping identity before deriving the primary
        // key; aliases must not create independent copies of the same effect.
        let project_root = std::fs::canonicalize(project_root)
            .map_err(|error| format!("MCP effect {parent_effect_id}: resolve project identity failed: {error}"))?
            .to_string_lossy().into_owned();
        let arguments_digest = Self::arguments_digest(arguments);
        let reusable_task = Self::recover_task_id(
            store.list_runtime_tasks(&project_root), &owner_session_id,
            parent_effect_id, server_id, tool_name, read_only, &arguments_digest,
        )?;
        let reused_task = reusable_task.is_some();
        let task_id = reusable_task
            .as_ref()
            .cloned()
            .unwrap_or_else(|| Self::effect_task_id(&project_root, &owner_session_id, parent_effect_id));
        let title = format!("{server_id} / {tool_name}");
        if !reused_task {
            let input = nola_application::CreateRuntimeTask {
                id: task_id.clone(),
                kind: "mcp".to_string(),
                title: title.clone(),
                project_root: project_root.clone(),
                owner_session_id: owner_session_id.clone(),
                parent_task_id: None,
                parent_tool_call_id: Some(parent_effect_id.to_string()),
                child_session_id: None,
                agent_type: Some(server_id.to_string()),
                prompt: None,
                initial_progress_summary: Some(ui_text(UiText::Queued).to_string()),
                initial_activity: Some(tool_name.to_string()),
                output_path: None,
                initially_unread: false,
                input_watermark: Some(now),
                created_at_ms: now,
                metadata: json!({
                    "server_id": server_id,
                    "tool_name": tool_name,
                    "parent_effect_id": parent_effect_id,
                    "host_pid": std::process::id(),
                    "read_only": read_only,
                    "arguments_digest": arguments_digest,
                    "safe_to_retry_before_claim": true,
                    "safe_to_retry_after_claim": read_only
                }),
                scheduling: nola_application::RuntimeTaskScheduling {
                    priority: nola_application::RuntimeTaskPriority::Normal,
                    queued_at_ms: now,
                    resource_budget: nola_application::RuntimeTaskResourceBudget {
                        max_wall_time_ms: Some(60 * 60 * 1_000),
                        max_output_bytes: Some(64 * 1_024 * 1_024),
                        max_cpu_time_ms: None,
                        max_memory_bytes: None,
                        max_processes: None,
                    },
                    output_bytes: 0,
                    resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
                    budget_exhaustion: None,
                },
            };
            if let Err(create_error) = crate::runtime_task_port::create(input) {
                // Creation is atomically arbitrated by the canonical Store's
                // primary key. A racing caller may have won after our read;
                // verify its exact identity, without retrying creation or claim.
                let original = store.runtime_task(&task_id).map_err(|error| format!(
                    "MCP task {task_id} for effect {parent_effect_id}: create failed ({create_error}); original task observation failed: {error}"
                ))?.ok_or_else(|| format!("MCP task {task_id} for effect {parent_effect_id}: create failed without a committed original: {create_error}"))?;
                if original.project_root != project_root {
                    return Err(format!("MCP task {task_id}: conflicting original project identity"));
                }
                let recovered = Self::recover_task_id(Ok(vec![original]), &owner_session_id,
                    parent_effect_id, server_id, tool_name, read_only, &arguments_digest)
                    .map_err(|error| format!("{error}; original create error: {create_error}"))?;
                if recovered.as_deref() != Some(task_id.as_str()) {
                    return Err(format!("MCP task {task_id}: conflicting original owner/effect identity"));
                }
                // The product port exposes a String error, not a typed unique
                // conflict. Do not reinterpret an arbitrary storage failure as
                // successful creation merely because a matching row exists.
                return Err(format!("MCP task {task_id} for effect {parent_effect_id}: original invocation retained after create error: {create_error}"));
            }
        } else {
            crate::runtime_task_port::append_event(
                &task_id,
                "takeover_attempted",
                "queued",
                Some(title.clone()),
                json!({"kind":"mcp","server_id":server_id,"tool_name":tool_name,"parent_effect_id":parent_effect_id,"safe_to_retry":true}).to_string(),
                now,
            )?;
        }
        tools::wake_background_task_notification();

        let worker_id = format!(
            "cli-mcp:{}:{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        );
        let ownership_token = uuid::Uuid::new_v4().simple().to_string();
        let lease_duration_ms = 45_000_i64;
        let (max_tasks, max_project_tasks) = runtime_task_concurrency_limits();
        let attempt = loop {
            if cancellation.is_requested() {
                let cancelled_at = unix_epoch_millis();
                let _ = store.request_runtime_task_cancel(&task_id, cancelled_at);
                let _ = store.append_runtime_task_event(
                    &task_id,
                    "cancelled_before_start",
                    "cancelled",
                    Some("MCP call cancelled while waiting for a Supervisor slot"),
                    &json!({"safe_to_retry":true}).to_string(),
                    cancelled_at,
                );
                return Err("MCP call cancelled before transport start".to_string());
            }
            let task = store.runtime_task(&task_id).map_err(|error| format!(
                "MCP task {task_id} for effect {parent_effect_id}: claim observation failed: {error}"
            ))?.ok_or_else(|| format!("MCP task {task_id} for effect {parent_effect_id} disappeared before claim; no replacement created"))?;
            if task.cancel_requested || task.status == "cancelled" {
                cancellation.request();
                return Err(format!("MCP task {task_id} for effect {parent_effect_id} cancelled before transport start"));
            }
            if !matches!(task.status.as_str(), "queued" | "retry_waiting") {
                return Err(format!("MCP task {task_id} for effect {parent_effect_id} changed to {}; claim stopped without replacement", task.status));
            }
            let claim_at = unix_epoch_millis();
            if let Some(attempt) = store.claim_runtime_task_fair(
                &task_id,
                &worker_id,
                crate::runtime_task_port::worker_generation(),
                &ownership_token,
                claim_at,
                claim_at.saturating_add(lease_duration_ms),
                max_tasks,
                max_project_tasks,
            ).map_err(|error| format!("MCP task {task_id} for effect {parent_effect_id}: claim failed: {error}"))? {
                break attempt;
            }
            std::thread::sleep(Duration::from_millis(50));
        };

        let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt);
        let started_at = unix_epoch_millis();
        crate::runtime_task_port::report_progress(
            &lease,
            Some(ui_text(UiText::Running).to_string()),
            None,
            None,
            None,
            BTreeMap::from([
                (
                    "host_pid".to_string(),
                    Some(Value::from(std::process::id())),
                ),
                ("takeover_at_ms".to_string(), Some(Value::from(started_at))),
            ]),
            started_at,
        )?;
        store.append_runtime_task_event(
            &task_id,
            "started",
            "running",
            Some(&title),
            &json!({"server_id":server_id,"tool_name":tool_name,"parent_effect_id":parent_effect_id}).to_string(),
            started_at,
        )?;

        let (heartbeat_stop, heartbeat_rx) = std::sync::mpsc::channel::<()>();
        let heartbeat_task_id = task_id.clone();
        let heartbeat_lease = lease.clone();
        let heartbeat_cancellation = cancellation.clone();
        let heartbeat_handle = match std::thread::Builder::new()
            .name(format!("nola-mcp-heartbeat-{task_id}"))
            .spawn(move || loop {
                match heartbeat_rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        let heartbeat_at = unix_epoch_millis();
                        let store = match NolaStore::open_default() {
                            Ok(store) => store,
                            Err(_) => {
                                heartbeat_cancellation.request();
                                break;
                            }
                        };
                        if store
                            .runtime_task(&heartbeat_task_id)
                            .ok()
                            .flatten()
                            .is_some_and(|task| task.cancel_requested)
                        {
                            heartbeat_cancellation.request();
                            break;
                        }
                        match store.renew_runtime_task_lease(
                            &heartbeat_lease,
                            heartbeat_at,
                            heartbeat_at.saturating_add(lease_duration_ms),
                        ) {
                            Ok(_) => {}
                            Err(_) => {
                                heartbeat_cancellation.request();
                                break;
                            }
                        }
                    }
                }
            }) {
            Ok(handle) => handle,
            Err(error) => {
                let reason = format!("start MCP heartbeat failed: {error}");
                let _ = crate::runtime_task_port::finish_detailed_with_event(
                    &lease,
                    "failed",
                    None,
                    Some("start_failed".to_string()),
                    None,
                    Some(reason.clone()),
                    None,
                    true,
                    "start_failed",
                    "failed",
                    Some(reason.clone()),
                    json!({"server_id":server_id,"tool_name":tool_name,"safe_to_retry":true})
                        .to_string(),
                    unix_epoch_millis(),
                );
                return Err(reason);
            }
        };

        Ok(Self {
            task_id,
            lease,
            server_id: server_id.to_string(),
            tool_name: tool_name.to_string(),
            heartbeat_stop: Some(heartbeat_stop),
            heartbeat_handle: Some(heartbeat_handle),
            last_progress_at_ms: 0,
            terminal: false,
        })
    }

    pub(crate) fn record_progress(&mut self, progress: &Value) -> Result<(), String> {
        let now = unix_epoch_millis();
        if now.saturating_sub(self.last_progress_at_ms) < 250 {
            return Ok(());
        }
        self.last_progress_at_ms = now;
        let summary = progress
            .get("message")
            .or_else(|| progress.get("status"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(240).collect::<String>())
            .unwrap_or_else(|| format!("{} / {}", self.server_id, self.tool_name));
        let detail = serde_json::to_string(progress).unwrap_or_else(|_| "{}".to_string());
        let detail = detail.chars().take(4_000).collect::<String>();
        crate::runtime_task_port::report_progress(
            &self.lease,
            Some(summary.clone()),
            Some(summary.clone()),
            None,
            None,
            std::collections::BTreeMap::new(),
            now,
        )?;
        NolaStore::open_default()?.append_runtime_task_event(
            &self.task_id,
            "progress",
            "running",
            Some(&summary),
            &detail,
            now,
        )?;
        tools::wake_background_task_notification();
        Ok(())
    }

    pub(crate) fn finish_completed(mut self) -> Result<(), String> {
        self.finish("completed", "finished", None, true)
    }

    pub(crate) fn finish_failed(mut self, error: &str) -> Result<(), String> {
        self.finish("failed", "finished", Some(error), true)
    }

    pub(crate) fn finish_reconciliation(mut self, error: &str) -> Result<(), String> {
        self.finish("failed", "reconciliation_required", Some(error), false)
    }

    fn stop_heartbeat(&mut self) {
        if let Some(stop) = self.heartbeat_stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.heartbeat_handle.take() {
            let _ = handle.join();
        }
    }

    fn finish(
        &mut self,
        status: &str,
        event_kind: &str,
        error: Option<&str>,
        safe_to_retry: bool,
    ) -> Result<(), String> {
        self.stop_heartbeat();
        let now = unix_epoch_millis();
        let result_summary = (status == "completed")
            .then(|| format!("{} / {} completed", self.server_id, self.tool_name));
        let attempt = self.lease.snapshot()?;
        let detail = json!({
            "server_id": self.server_id,
            "tool_name": self.tool_name,
            "runtime_task_attempt_id": attempt.id,
            "runtime_task_attempt_revision": attempt.revision,
            "safe_to_retry": safe_to_retry,
            "error": error
        })
        .to_string();
        if !crate::runtime_task_port::finish_detailed_with_event(
            &self.lease,
            status,
            None,
            Some(status.to_string()),
            result_summary,
            error.map(str::to_string),
            None,
            true,
            event_kind,
            status,
            Some(error.unwrap_or(&self.tool_name).to_string()),
            detail,
            now,
        )? {
            return Err("MCP call lost its Supervisor ownership lease".to_string());
        }
        self.terminal = true;
        wake_task_waiters();
        tools::wake_background_task_notification();
        Ok(())
    }
}

impl Drop for McpExecutionTaskLease {
    fn drop(&mut self) {
        self.stop_heartbeat();
        if self.terminal {
            return;
        }
        let now = unix_epoch_millis();
        let reason = "MCP transport ended without a committed terminal state; verify remote side effects before retrying";
        let _ = crate::runtime_task_port::finish_detailed_with_event(
            &self.lease,
            "failed",
            None,
            Some("needs_reconciliation".to_string()),
            None,
            Some(reason.to_string()),
            None,
            true,
            "reconciliation_required",
            "failed",
            Some(reason.to_string()),
            json!({"server_id":self.server_id,"tool_name":self.tool_name,"safe_to_retry":false})
                .to_string(),
            now,
        );
        wake_task_waiters();
        tools::wake_background_task_notification();
    }
}

fn spawn_shell_task_process(mut task: DbBackgroundTask) -> Result<bool, String> {
    let now = unix_epoch_millis();
    let lease_duration_ms = std::env::var("NOLA_SHELL_TASK_LEASE_MS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(45_000)
        .clamp(1_000, 10 * 60 * 1_000);
    let (max_tasks, max_project_tasks) = runtime_task_concurrency_limits();
    let worker_id = format!(
        "cli-shell-launch:{}:{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    );
    let ownership_token = uuid::Uuid::new_v4().simple().to_string();
    let store = NolaStore::open_default()?;
    let Some(attempt) = store.claim_runtime_task_fair(
        &task.id,
        &worker_id,
        crate::runtime_task_port::worker_generation(),
        &ownership_token,
        now,
        now.saturating_add(lease_duration_ms),
        max_tasks,
        max_project_tasks,
    )?
    else {
        return Ok(false);
    };
    #[cfg_attr(test, allow(unused_variables))]
    let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt.clone());
    task.pid = 0;
    #[allow(unused_mut)]
    let mut request = ShellTaskProcessRequest {
        task: task.clone(),
        worker_id,
        ownership_token: ownership_token.clone(),
        runtime_task_attempt: attempt,
        lease_duration_ms,
        database_path: store.path().to_path_buf(),
    };
    // Deterministic fault-injection window for the real process-recovery
    // contract. The durable task owns a lease, but neither the isolated
    // worker nor the command has started. Production never sets this value.
    if let Some(delay_ms) = std::env::var("NOLA_TEST_SHELL_PRE_SPAWN_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
    {
        std::thread::sleep(Duration::from_millis(delay_ms.min(30_000)));
    }
    // Prepared was initialized with this claim in the Store transaction.
    // Invalidate that non-delivery proof durably BEFORE any worker can exist
    // or receive a request; PID publication happens later and is not a fence.
    request.runtime_task_attempt = mark_shell_worker_dispatch(&lease)?.attempt;
    #[cfg(test)]
    {
        std::thread::Builder::new()
            .name(format!("nola-test-shell-worker-{}", task.id))
            .spawn(move || {
                let _ = run_shell_task(request);
            })
            .map_err(|error| format!("start test Shell worker failed: {error}"))?;
        return Ok(true);
    }
    #[cfg(not(test))]
    {
        let executable = std::env::current_exe()
            .map_err(|error| format!("resolve Shell task worker executable failed: {error}"))?;
        let mut command = Command::new(executable);
        command
            .arg(SHELL_TASK_WORKER_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        detach_worker_session(&mut command);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = store.retry_runtime_task(
                    &lease,
                    now.saturating_add(1_000),
                    &format!("start isolated Shell task worker failed: {error}"),
                    unix_epoch_millis(),
                );
                return Err(format!("start isolated Shell task worker failed: {error}"));
            }
        };
        let worker_pid = child.id();
        // Publish the worker identity while the isolated process is still blocked
        // reading its request. Writing the request first allowed the worker to
        // publish the command PID and the parent to overwrite it with an older
        // metadata snapshot whose `pid` was still zero.
        let task_id = task.id.clone();
        let progress_ack = match crate::runtime_task_port::report_progress_now(
            &lease,
            None,
            None,
            None,
            None,
            BTreeMap::from([
                ("workerPid".to_string(), Some(Value::from(worker_pid))),
                (
                    "workerIsolation".to_string(),
                    Some(Value::String("process_group".to_string())),
                ),
            ]),
        ) {
            Ok(ack) => ack,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = store.retry_runtime_task(
                    &lease,
                    now.saturating_add(1_000),
                    &format!("persist isolated Shell worker identity failed: {error}"),
                    unix_epoch_millis(),
                );
                return Err(format!(
                    "persist isolated Shell worker identity failed: {error}"
                ));
            }
        };
        request.runtime_task_attempt = progress_ack.attempt;
        let encoded = serde_json::to_vec(&request)
            .map_err(|error| format!("encode Shell task worker request failed: {error}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "isolated Shell task worker has no stdin pipe".to_string())?;
        if let Err(error) = stdin.write_all(&encoded) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = store.retry_runtime_task(
                &lease,
                now.saturating_add(1_000),
                &format!("send Shell task worker request failed: {error}"),
                unix_epoch_millis(),
            );
            return Err(format!("send Shell task worker request failed: {error}"));
        }
        drop(stdin);
        std::thread::Builder::new()
            .name(format!("nola-shell-worker-reaper-{task_id}"))
            .spawn(move || {
                let status = child.wait();
                if !status.as_ref().is_ok_and(|status| status.success()) {
                    let detail = status
                        .map(|status| format!("isolated Shell worker exited with {status}"))
                        .unwrap_or_else(|error| format!("wait for Shell worker failed: {error}"));
                    let _ = append_task_event(
                        &task_id,
                        "worker_process_exited",
                        "suspended",
                        Some(&detail),
                        json!({"worker_pid": worker_pid}),
                    );
                }
                crate::runtime_task_port::release_runtime_task_handle(&lease);
                wake_task_waiters();
                tools::wake_background_task_notification();
            })
            .map_err(|error| format!("start Shell worker reaper failed: {error}"))?;
        Ok(true)
    }
}

fn mark_shell_worker_dispatch(lease: &crate::runtime_task_port::RuntimeTaskLeaseHandle)
    -> Result<nola_application::RuntimeTaskProgressAck,String> {
    use nola_domain::runtime_task_shell_delivery::*;
    let attempt=lease.snapshot()?;
    let witness=ShellWorkerDispatchWitness::new(&attempt.task_id,&attempt.id,&attempt.worker_id,
        &attempt.worker_generation,&attempt.lease_token,ShellWorkerDispatchPhase::MayHaveExecuted)?;
    crate::runtime_task_port::report_progress_now(lease,None,None,None,None,
        BTreeMap::from([(SHELL_WORKER_DISPATCH_METADATA_KEY.into(),Some(serde_json::to_value(witness).map_err(|error|error.to_string())?))]))
}

// Execution consumes the budget frozen at creation; recovery must never
// fabricate a replacement budget when the durable observation is unavailable.
fn require_task_execution_scheduling(
    observed: Result<Option<DbRuntimeTask>, String>,
) -> Result<(DbRuntimeTask, nola_application::RuntimeTaskScheduling), String> {
    let task = observed?.ok_or_else(|| "runtime task is missing before execution".to_string())?;
    let scheduling = task.scheduling()?
        .ok_or_else(|| "runtime task scheduling is missing before execution".to_string())?;
    Ok((task, scheduling))
}

// A successful progress CAS proves ownership, not permission to spend more
// resources. Consume the authoritative post-CAS observation before effects.
fn require_task_execution_budget(
    task: &DbRuntimeTask,
    now_ms: i64,
) -> Result<nola_application::RuntimeTaskScheduling, String> {
    let scheduling = task.scheduling()?
        .ok_or("runtime task scheduling is missing before execution")?;
    if let Some(failure) = scheduling.execution_budget_failure(task.started_at_ms, now_ms)? {
        return Err(format!("runtime task budget exhausted: {} (limit {}, observed {})",
            failure.resource.code(), failure.limit, failure.observed));
    }
    Ok(scheduling)
}

fn remaining_task_wall_budget(task: &DbRuntimeTask, now_ms: i64) -> Result<Option<Duration>, String> {
    let scheduling = require_task_execution_budget(task, now_ms)?;
    scheduling.resource_budget.max_wall_time_ms.map(|limit| {
        let start = task.started_at_ms.ok_or("task wall-clock origin is missing")?;
        let elapsed = u64::try_from(now_ms.checked_sub(start)
            .ok_or("task wall-clock elapsed overflow")?).map_err(|_| "task wall-clock moved backwards")?;
        Ok(Duration::from_millis(limit.saturating_sub(elapsed)))
    }).transpose()
}

// Revalidate at the effect boundary, after potentially blocking preparation.
// The Store observation also checks cancellation; the monotonic deadline
// prevents a wall-clock adjustment from granting time back to this worker.
fn check_task_effect_admission(
    lease: &crate::runtime_task_port::RuntimeTaskLeaseHandle,
    started: Instant,
    remaining: Option<Duration>,
) -> Result<(), String> {
    let ack = crate::runtime_task_port::report_progress_now(
        lease, None, None, None, None, BTreeMap::new(),
    )?;
    if ack.task.cancel_requested {
        return Err("runtime task cancellation requested before effect start".into());
    }
    require_task_execution_budget(&ack.task, unix_epoch_millis())?;
    check_task_preparation_deadline(started.elapsed(), remaining)
}

fn check_task_preparation_deadline(elapsed: Duration, remaining: Option<Duration>) -> Result<(), String> {
    if remaining.is_some_and(|remaining| elapsed >= remaining) {
        return Err("runtime task wall-clock budget exhausted during preparation".into());
    }
    Ok(())
}

#[cfg(test)]
mod execution_scheduling_admission_tests {
    use super::*;

    fn task() -> DbRuntimeTask {
        serde_json::from_value(json!({
            "id":"scheduling-admission", "kind":"shell", "status":"queued",
            "title":"Frozen budget", "project_root":"/fixture",
            "unread":false, "cancel_requested":false, "retry_count":0,
            "created_at_ms":1, "updated_at_ms":1, "metadata_json":"{}"
        })).unwrap()
    }

    fn scheduling() -> nola_application::RuntimeTaskScheduling {
        nola_application::RuntimeTaskScheduling {
            priority: nola_application::RuntimeTaskPriority::Normal,
            queued_at_ms: 1,
            resource_budget: nola_application::RuntimeTaskResourceBudget {
                max_wall_time_ms: Some(1_000), max_output_bytes: Some(1_024),
                max_cpu_time_ms: Some(500), max_memory_bytes: Some(4_096), max_processes: Some(2),
            },
            output_bytes: 0,
            resource_usage: Default::default(),
            budget_exhaustion: None,
        }
    }

    #[test]
    fn preparation_must_not_spend_the_remaining_budget_before_effect_start() {
        let limit = Duration::from_millis(10);
        assert!(check_task_preparation_deadline(Duration::from_millis(9), Some(limit)).is_ok());
        assert!(check_task_preparation_deadline(limit, Some(limit)).is_err());
        assert!(check_task_preparation_deadline(Duration::from_millis(11), Some(limit)).is_err());
        assert!(check_task_preparation_deadline(Duration::from_secs(100), None).is_ok());
    }

    #[test]
    fn worker_budget_admission_uses_original_wall_origin_and_current_ack_usage() {
        let mut persisted = task();
        persisted.started_at_ms = Some(100);
        persisted.set_scheduling(scheduling()).unwrap();
        assert_eq!(remaining_task_wall_budget(&persisted, 900).unwrap(), Some(Duration::from_millis(200)));
        assert!(require_task_execution_budget(&persisted, 1_100).unwrap_err().contains("wall_time_ms"));
        assert!(remaining_task_wall_budget(&persisted, 99).is_err());
        persisted.started_at_ms = None;
        assert!(require_task_execution_budget(&persisted, 900).is_err());
        persisted.started_at_ms = Some(100);
        let mut policy = scheduling();
        policy.resource_usage.cpu_time_ms = 501;
        persisted.set_scheduling(policy.clone()).unwrap();
        assert!(require_task_execution_budget(&persisted, 101).unwrap_err().contains("cpu_time_ms"));
        policy.resource_budget.max_wall_time_ms = None;
        policy.resource_usage.cpu_time_ms = 0;
        persisted.set_scheduling(policy).unwrap();
        persisted.started_at_ms = None;
        assert_eq!(remaining_task_wall_budget(&persisted, 900).unwrap(), None);
    }

    #[test]
    fn worker_admission_refuses_unknown_or_damaged_budget_instead_of_defaulting() {
        assert_eq!(require_task_execution_scheduling(Err("store unavailable".into())).unwrap_err(), "store unavailable");
        assert!(require_task_execution_scheduling(Ok(None)).unwrap_err().contains("task is missing"));
        assert!(require_task_execution_scheduling(Ok(Some(task()))).unwrap_err().contains("scheduling is missing"));
        for metadata in ["null", "[]", "{", r#"{"_nola_scheduler":{"priority":"urgent"}}"#] {
            let mut invalid = task();
            invalid.metadata_json = metadata.into();
            assert!(require_task_execution_scheduling(Ok(Some(invalid))).is_err());
        }
        let mut valid = task();
        valid.cancel_requested = true;
        valid.set_scheduling(scheduling()).unwrap();
        let (observed, budget) = require_task_execution_scheduling(Ok(Some(valid.clone()))).unwrap();
        assert_eq!(observed, valid, "admission must retain cancellation and original task identity");
        assert_eq!(budget, scheduling(), "all frozen resource caps must survive recovery");
    }

    #[test]
    fn shell_worker_rejects_damaged_persisted_policy_before_spawning_command() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("tasks.sqlite");
        let store = NolaStore::open(&database).unwrap();
        for metadata in ["{}", "[]", r#"{"_nola_scheduler":{"priority":"urgent"}}"#] {
            let now = unix_epoch_millis();
            let id = format!("policy-{}", uuid::Uuid::new_v4());
            let marker = root.path().join(format!("{id}.effect"));
            let task = DbBackgroundTask {
        process_sandbox: None,
                terminal: None,
                id: id.clone(),
                project_root: root.path().to_string_lossy().into_owned(),
                command: format!("printf ran > '{}'", marker.display()),
                cwd: root.path().to_string_lossy().into_owned(),
                pid: 0,
                stdout_path: root.path().join(format!("{id}.stdout")).to_string_lossy().into_owned(),
                stderr_path: root.path().join(format!("{id}.stderr")).to_string_lossy().into_owned(),
                status_path: root.path().join(format!("{id}.status")).to_string_lossy().into_owned(),
                started_at_ms: now,
                owner_session_id: None,
                output_contract_json: None,
                sop_invocation: None,
            };
            store.save_background_task(&task).unwrap();
            let attempt = store.claim_runtime_task(&id, "policy-worker", "policy-generation",
                &format!("token-{id}"), now, now + 60_000).unwrap().unwrap();
            // Simulate damaged durable recovery input, not a malformed create
            // command (which Application correctly refuses before persistence).
            let connection = rusqlite::Connection::open(&database).unwrap();
            connection.execute("UPDATE durable_tasks SET metadata_json=?2 WHERE id=?1",
                rusqlite::params![id, metadata]).unwrap();
            drop(connection);
            let before = store.runtime_task(&id).unwrap().unwrap();
            let events = store.runtime_task_events(&id, 100).unwrap();
            assert_eq!(run_shell_task(ShellTaskProcessRequest {
                task: task.clone(), worker_id: attempt.worker_id.clone(),
                ownership_token: attempt.lease_token.clone(), runtime_task_attempt: attempt,
                lease_duration_ms: 60_000, database_path: database.clone(),
            }), 2);
            assert!(!marker.exists(), "policy observation failure must not launch the command");
            assert!(!Path::new(&task.stdout_path).exists());
            assert!(!Path::new(&task.status_path).exists());
            assert_eq!(store.runtime_task(&id).unwrap().unwrap(), before,
                "observation failure must not fabricate a failed or cancelled task");
            assert_eq!(store.runtime_task_events(&id, 100).unwrap(), events);
            assert!(crate::runtime_task_port::runtime_task_lease_state(&id).is_err(),
                "refused worker must release its in-process lease handle");
        }
    }

    #[test]
    fn isolated_store_missing_and_reopened_budget_are_distinct_observations() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tasks.sqlite");
        let store = NolaStore::open(&path).unwrap();
        assert!(require_task_execution_scheduling(store.runtime_task("absent")).is_err());
        let mut original = task();
        original.set_scheduling(scheduling()).unwrap();
        store.create_runtime_task(&original).unwrap();
        drop(store);
        let reopened = NolaStore::open(&path).unwrap();
        let (observed, budget) = require_task_execution_scheduling(reopened.runtime_task(&original.id)).unwrap();
        assert_eq!(observed, original);
        assert_eq!(budget, scheduling());
    }
}

#[cfg(unix)]
fn drain_shell_terminal(master: &mut nola_runtime_supervisor::terminal::TerminalMaster,
    output: &mut std::fs::File) -> std::io::Result<()> {
    // Bounded work per worker tick preserves cancellation and lease checks.
    for _ in 0..4 {
        match master.read_available(nola_runtime_supervisor::terminal::MAX_IO_BYTES) {
            Ok(bytes) if bytes.is_empty() => break,
            Ok(bytes) => std::io::Write::write_all(output, &bytes)?,
            Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => break,
            // Linux signals closure of the slave side using EIO. This only
            // ends this read; child.try_wait remains the process authority.
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn consume_shell_terminal_input(master: &mut nola_runtime_supervisor::terminal::TerminalMaster,
    task: &DbBackgroundTask, runtime: &DbRuntimeTask, lease: &crate::runtime_task_port::RuntimeTaskLeaseHandle)
    -> Result<(),String> {
    use nola_domain::terminal_input::*;
    let config = task.terminal.as_ref().ok_or("missing terminal configuration")?;
    let metadata: Value = serde_json::from_str(&runtime.metadata_json).map_err(|error|error.to_string())?;
    let queue: BTreeMap<String,TerminalInputRequest> = metadata.get(TERMINAL_INPUT_QUEUE_METADATA_KEY)
        .cloned().map(serde_json::from_value).transpose().map_err(|error|error.to_string())?.unwrap_or_default();
    let receipts: BTreeMap<String,TerminalInputReceipt> = metadata.get(TERMINAL_INPUT_RECEIPTS_METADATA_KEY)
        .cloned().map(serde_json::from_value).transpose().map_err(|error|error.to_string())?.unwrap_or_default();
    let attempt = lease.snapshot()?;
    for (id,input) in queue {
        let receipt = receipts.get(&id).ok_or("queued terminal input has no receipt")?;
        if receipt.phase == TerminalInputPhase::Delivered { continue; }
        if input.identity.host_id != config.host_id || input.identity.task_id != task.id
            || Some(input.identity.owner_session_id.as_str()) != task.owner_session_id.as_deref()
            || input.identity.attempt_id != attempt.id || input.identity.worker_generation != attempt.worker_generation {
            return Err("queued terminal input does not belong to this worker".into());
        }
        let next = master.deliver_input(receipt,&input.identity,input.text.as_bytes(),
            |previous,next| crate::runtime_task_port::advance_terminal_input(lease,previous,next))?;
        if next.phase == TerminalInputPhase::Unknown {
            return Err("terminal input outcome unknown; no automatic resend".into());
        }
        break; // Bounded input work; the next tick checks cancellation again.
    }
    Ok(())
}

#[cfg(all(test,any(target_os="macos",target_os="linux")))]
#[test]
fn durable_required_process_sandbox_missing_never_launches_or_drops_policy() {
    let _environment=crate::store::test_env_lock();
    let fixture=tempfile::tempdir().unwrap();let root=fixture.path().canonicalize().unwrap();
    let database=root.join("tasks.sqlite");
    struct RestoreDatabase(Option<std::ffi::OsString>);
    impl Drop for RestoreDatabase {fn drop(&mut self) {match &self.0 {Some(value)=>std::env::set_var("NOLA_DB_PATH",value),None=>std::env::remove_var("NOLA_DB_PATH")}}}
    let _restore=RestoreDatabase(std::env::var_os("NOLA_DB_PATH"));std::env::set_var("NOLA_DB_PATH",&database);
    let store=NolaStore::open(&database).unwrap();
    let marker=root.join("must-not-run");let now=unix_epoch_millis();
    let spec=crate::process_sandbox::unavailable_process_sandbox_fixture(&root);
    let task=DbBackgroundTask {process_sandbox:Some(spec.clone()),terminal:None,id:"required-sandbox".into(),
        project_root:root.to_string_lossy().into_owned(),command:format!("printf unsafe > '{}'",marker.display()),
        cwd:root.to_string_lossy().into_owned(),pid:0,stdout_path:root.join("stdout").to_string_lossy().into_owned(),
        stderr_path:root.join("stderr").to_string_lossy().into_owned(),status_path:root.join("status").to_string_lossy().into_owned(),
        started_at_ms:now,owner_session_id:None,output_contract_json:None,sop_invocation:None};
    store.save_background_task(&task).unwrap();
    let loaded=crate::store::background_task_from_runtime(store.runtime_task(&task.id).unwrap().unwrap()).unwrap();
    assert_eq!(loaded.process_sandbox,Some(spec));
    let attempt=store.claim_runtime_task(&task.id,"sandbox-worker","sandbox-generation","sandbox-token",now,now+60_000).unwrap().unwrap();
    let lease=crate::runtime_task_port::register_runtime_task_attempt(attempt);
    let attempt=mark_shell_worker_dispatch(&lease).unwrap().attempt;
    crate::runtime_task_port::release_runtime_task_handle(&lease);
    assert_eq!(run_shell_task(ShellTaskProcessRequest {task:loaded,worker_id:attempt.worker_id.clone(),ownership_token:attempt.lease_token.clone(),
        runtime_task_attempt:attempt,lease_duration_ms:60_000,database_path:database}),2);
    assert!(!marker.exists());assert!(!Path::new(&task.stdout_path).exists());
    let result=store.runtime_task(&task.id).unwrap().unwrap();
    assert_eq!(result.status,"failed");
    assert!(result.error.as_deref().unwrap_or_default().contains("Required process sandbox unavailable"));
}

/// The admitted worker is the sole writer of each output stream. Windows
/// MSYS children need a writable handle; append-only FILE_APPEND_DATA handles
/// are not usable as their inherited stdout/stderr (rust-lang/rust#92136).
/// Seek the exclusive writer to EOF without truncating existing output.
pub(crate) fn open_shell_output_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    #[cfg(windows)]
    {
        use std::io::Seek;
        let mut file = options.write(true).truncate(false).open(path)?;
        file.seek(std::io::SeekFrom::End(0))?;
        Ok(file)
    }
    #[cfg(not(windows))]
    { options.append(true).open(path) }
}

#[test]
fn shell_output_writer_preserves_prior_output() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("output.txt");
    std::fs::write(&path, b"prior-").unwrap();
    let mut file = open_shell_output_file(&path).unwrap();
    file.write_all(b"next").unwrap();
    drop(file);
    assert_eq!(std::fs::read(&path).unwrap(), b"prior-next");
}

fn run_shell_task(request: ShellTaskProcessRequest) -> i32 {
    let task = request.task.clone();
    let lease = crate::runtime_task_port::register_runtime_task_attempt(
        request.runtime_task_attempt.clone(),
    );
    let store = match NolaStore::open(&request.database_path) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("open Shell task store failed: {error}");
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            return 2;
        }
    };
    let (persisted_task, _) = match require_task_execution_scheduling(store.runtime_task(&task.id)) {
        Ok(admission) => admission,
        Err(error) => {
            eprintln!("Shell task {} refused before command start: {error}", task.id);
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            return 2;
        }
    };
    let dispatch_is_bound = (|| -> Result<bool,String> {
        let durable = crate::store::background_task_from_runtime(persisted_task.clone())?;
        if durable.command != task.command || durable.cwd != task.cwd || durable.terminal != task.terminal
            || durable.process_sandbox!=task.process_sandbox
            || durable.owner_session_id != task.owner_session_id || durable.stdout_path != task.stdout_path
            || durable.stderr_path != task.stderr_path || durable.status_path != task.status_path {
            return Err("Shell launch payload differs from the durable task".into());
        }
        use nola_domain::runtime_task_shell_delivery::*;
        let metadata:Value=serde_json::from_str(&persisted_task.metadata_json).map_err(|error|error.to_string())?;
        let witness:ShellWorkerDispatchWitness=serde_json::from_value(metadata.get(SHELL_WORKER_DISPATCH_METADATA_KEY)
            .cloned().ok_or("Shell worker has no durable dispatch admission")?).map_err(|error|error.to_string())?;
        let attempt=&request.runtime_task_attempt;
        Ok(witness.phase==ShellWorkerDispatchPhase::MayHaveExecuted && witness.matches(&attempt.task_id,
            &attempt.id,&attempt.worker_id,&attempt.worker_generation,&attempt.lease_token))
    })();
    if !matches!(dispatch_is_bound,Ok(true)) {
        eprintln!("Shell task {} refused without exact durable worker dispatch admission",task.id);
        crate::runtime_task_port::release_runtime_task_handle(&lease);
        return 2;
    }
    if persisted_task.cancel_requested {
        let finished = crate::runtime_task_port::finish_detailed_with_event_now(
            &lease,
            "cancelled",
            None,
            Some("cancelled".to_string()),
            None,
            None,
            Some(task.status_path.clone()),
            true,
            "cancelled",
            "cancelled",
            Some("Cancelled before process start".to_string()),
            json!({"phase":"before_process_start"}).to_string(),
            None,
        )
        .unwrap_or(false);
        if finished {
            let _ = std::fs::write(&task.status_path, "cancelled\n");
            tools::wake_background_task_notification();
        }
        return 1;
    }
    let mut prepared_sandbox=match task.process_sandbox.as_ref().map(|spec|
        spec.prepare_shell(Path::new(&task.cwd),&task.command,false)).transpose() {
        Ok(prepared)=>prepared,
        Err(error)=>return finish_shell_start_failure(&request,&lease,&format!("Required process sandbox unavailable: {error}; no unrestricted fallback")),
    };
    let execution_command=match prepared_sandbox.as_ref().map(|prepared|prepared.shell_invocation()).transpose() {
        Ok(command)=>command.unwrap_or_else(||task.command.clone()),
        Err(error)=>return finish_shell_start_failure(&request,&lease,&error),
    };
    if let Some(prepared)=prepared_sandbox.as_ref() {
        if let Err(error)=crate::runtime_task_port::report_progress_now(&lease,None,None,None,None,
            BTreeMap::from([("processSandboxExecution".into(),Some(json!({"attempt_id":request.runtime_task_attempt.id,"resources":prepared.descriptor()})))])) {
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            eprintln!("Shell sandbox resource binding was not acknowledged: {error}");
            return 2;
        }
    }
    // Process startup consumes the transported lease. Renew the same attempt
    // before launching effects, and count heartbeat time from this renewal,
    // not from the later completion of command-start bookkeeping. Renewal's
    // CAS rejects expired/replaced attempts rather than reviving them.
    let mut heartbeat = match process_control::ShellLeaseHeartbeat::start(
        &request.database_path, lease.clone(), request.lease_duration_ms,
    ) {
        Ok(heartbeat) => heartbeat,
        Err(error) => {
            eprintln!("Shell task {} refused before command start: {error}", task.id);
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            return 2;
        }
    };
    // Transported attempt identity must pass fresh CAS before any command runs.
    let admission = crate::runtime_task_port::report_progress_now(
        &lease,
        Some(ui_text(UiText::Running).to_string()),
        None,
        None,
        None,
        BTreeMap::new(),
    ).and_then(|ack| {
        let now = unix_epoch_millis();
        Ok((require_task_execution_budget(&ack.task, now)?, remaining_task_wall_budget(&ack.task, now)?))
    });
    let (scheduling, remaining_wall_budget) = match admission {
        Ok(admission) => admission,
        Err(error) => {
            eprintln!("Shell task {} refused before command start: {error}", task.id);
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            return 2;
        }
    };
    let started = Instant::now();
    let stdout = match open_shell_output_file(Path::new(&task.stdout_path))
    {
        Ok(file) => file,
        Err(error) => {
            return finish_shell_start_failure(
                &request,
                &lease,
                &format!("open stdout failed: {error}"),
            )
        }
    };
    let stderr = match open_shell_output_file(Path::new(&task.stderr_path))
    {
        Ok(file) => file,
        Err(error) => {
            return finish_shell_start_failure(
                &request,
                &lease,
                &format!("open stderr failed: {error}"),
            )
        }
    };
    let evidence_path = std::path::PathBuf::from(format!("{}.exit-evidence", task.status_path));
    #[cfg(not(unix))]
    if task.terminal.is_some() {
        return finish_shell_start_failure(&request, &lease, "PTY execution is unavailable on this platform");
    }
    #[cfg(unix)]
    let mut terminal = match task.terminal.as_ref().map(|config| {
        config.validate().map_err(std::io::Error::other)?;
        let size = config.size;
        size.validate().map_err(std::io::Error::other)?;
        let (master, slave) = nola_runtime_supervisor::terminal::open(
            nola_runtime_supervisor::terminal::TerminalSize { rows:size.rows, columns:size.columns })?;
        Ok::<_, std::io::Error>((master, slave, stdout.try_clone()?))
    }).transpose() {
        Ok(terminal) => terminal,
        Err(error) => return finish_shell_start_failure(&request, &lease, &format!("PTY setup failed: {error}")),
    };
    let _ = std::fs::remove_file(&evidence_path);
    let mut command = Command::new(crate::platform_paths::bash_program());
    if prepared_sandbox.is_some() {
        crate::process_sandbox::isolate_shell_supervisor_environment(&mut command);
        command.args(["--noprofile","--norc"]).env_remove("BASH_ENV").env_remove("ENV");
        for (name,_) in std::env::vars_os().filter(|(name,_)|name.to_string_lossy().starts_with("BASH_FUNC_")) {command.env_remove(name);}
    }
    command
        .arg("-o")
        .arg("pipefail")
        .arg(if prepared_sandbox.is_some() {"-c"}else{"-lc"})
        .arg(tools::shell_evidence_script(&execution_command))
        .env("NOLA_SHELL_EVIDENCE_PATH", crate::platform_paths::shell_path(&evidence_path))
        .current_dir(&task.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if let Err(error) = check_task_effect_admission(&lease, started, remaining_wall_budget)
        .and_then(|()| if heartbeat.lost() { Err("Shell task lost its ownership lease".into()) } else { Ok(()) }) {
        eprintln!("Shell task {} refused before command start: {error}", task.id);
        crate::runtime_task_port::release_runtime_task_handle(&lease);
        return 2;
    }
    if let Some(prepared)=prepared_sandbox.as_mut() {prepared.mark_dispatched();}
    #[cfg(unix)]
    let (spawned, mut terminal_io) = match terminal.take() {
        Some((master, slave, output)) => (heartbeat.spawn_terminal(&mut command, slave), Some((master, output))),
        None => (heartbeat.spawn(&mut command), None),
    };
    #[cfg(not(unix))]
    let spawned = heartbeat.spawn(&mut command);
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            if let Some(prepared)=prepared_sandbox.as_mut() {prepared.settle();}
            return finish_shell_start_failure(&request, &lease, &format!("exec failed: {error}"))
        }
    };
    let command_pid = child.id();
    let started_at_ms = unix_epoch_millis();
    let mut process_metadata = std::collections::BTreeMap::from([
        ("pid".to_string(), Some(json!(command_pid))),
        ("runnerPid".to_string(), Some(json!(std::process::id()))),
        ("processStartedAtMs".to_string(), Some(json!(started_at_ms))),
    ]);
    if let Some(prepared)=prepared_sandbox.as_ref() {
        process_metadata.insert("processSandboxExecution".into(),Some(json!({"attempt_id":request.runtime_task_attempt.id,"resources":prepared.descriptor()})));
    }
    if let Err(error) = crate::runtime_task_port::report_progress_now(
        &lease,
        Some("Background process running".to_string()),
        Some(task.cwd.clone()),
        None,
        None,
        process_metadata,
    ).and_then(|ack| {
        if ack.task.cancel_requested {
            return Err("runtime task cancellation requested during process startup".into());
        }
        require_task_execution_budget(&ack.task, unix_epoch_millis()).map(|_| ())
    }) {
        child.terminate();
        let _ = child.wait();
        // The child has started: a failed observation is not proof that its
        // command failed or had no side effects. Preserve execution evidence
        // and let the exact task/attempt reconcile, rather than invent a seal.
        eprintln!("Shell task {} stopped after startup observation failed: {error}", task.id);
        crate::runtime_task_port::release_runtime_task_handle(&lease);
        return 2;
    }
    let _ = store.append_runtime_task_event(
        &task.id,
        "started",
        "running",
        Some(&task.command),
        &json!({"pid": command_pid, "worker_id": request.worker_id}).to_string(),
        unix_epoch_millis(),
    );
    let mut observed = ProcessGroupUsage::default();
    let mut has_resource_observation = false;
    let mut budget_failure = None::<String>;
    let mut cancelled = false;
    let exit_status = loop {
        if heartbeat.lost() {
            child.terminate();
            budget_failure = Some("Shell task lost its ownership lease".to_string());
            break child.wait().ok();
        }
        #[cfg(unix)]
        if let Some((master, output)) = terminal_io.as_mut() {
            if let Err(error) = drain_shell_terminal(master, output) {
                budget_failure = Some(format!("PTY output persistence failed: {error}"));
                child.terminate();
                break child.wait().ok();
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if child.residual_processes_stopped() != Some(0) {
                    budget_failure = Some(format!("Shell root exited with untransferred group members or unknown prior group observation; confirmed stop count={:?}",
                        child.residual_processes_stopped()));
                }
                break Some(status);
            }
            Ok(None) => {}
            Err(error) => {
                budget_failure = Some(format!("wait for Shell command failed: {error}"));
                child.terminate();
                break child.wait().ok();
            }
        }
        let runtime_task = store.runtime_task(&task.id).ok().flatten();
        if runtime_task
            .as_ref()
            .is_some_and(|value| value.cancel_requested)
        {
            cancelled = true;
            child.terminate();
            break child.wait().ok();
        }
        #[cfg(unix)]
        if let Some((master, _)) = terminal_io.as_mut() {
            let delivered = runtime_task.as_ref().ok_or_else(||"terminal task could not be read".to_string())
                .and_then(|runtime|consume_shell_terminal_input(master,&task,runtime,&lease));
            if let Err(error) = delivered {
                budget_failure = Some(error);
                child.terminate();
                break child.wait().ok();
            }
        }
        let output_bytes = match shell_output_bytes(&task.stdout_path, &task.stderr_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                budget_failure = Some(format!("Task output observation failed: {error}"));
                child.terminate();
                break child.wait().ok();
            }
        };
        let usage = match current_process_tree_usage() {
            Ok(usage) => usage,
            Err(error) => {
                budget_failure = Some(format!("Task resource monitor failed: {error}"));
                child.terminate();
                break child.wait().ok();
            }
        };
        {
            has_resource_observation = true;
            observed.cpu_time_ms = observed.cpu_time_ms.max(usage.cpu_time_ms);
            observed.memory_bytes = observed.memory_bytes.max(usage.memory_bytes);
            observed.processes = observed.processes.max(usage.processes.saturating_sub(1));
        }
        let budget = &scheduling.resource_budget;
        budget_failure = if remaining_wall_budget
            .is_some_and(|remaining| started.elapsed() >= remaining)
        {
            Some(format!(
                "Task exceeded its wall-clock budget of {} ms",
                budget.max_wall_time_ms.unwrap_or_default()
            ))
        } else if budget
            .max_output_bytes
            .is_some_and(|limit| output_bytes > limit)
        {
            Some(format!(
                "Task exceeded its output budget of {} bytes (observed {output_bytes} bytes)",
                budget.max_output_bytes.unwrap_or_default()
            ))
        } else if budget
            .max_cpu_time_ms
            .is_some_and(|limit| observed.cpu_time_ms > limit)
        {
            Some(format!(
                "Task exceeded its CPU-time budget of {} ms (observed {} ms)",
                budget.max_cpu_time_ms.unwrap_or_default(),
                observed.cpu_time_ms
            ))
        } else if budget
            .max_memory_bytes
            .is_some_and(|limit| observed.memory_bytes > limit)
        {
            Some(format!(
                "Task exceeded its memory budget of {} bytes (observed {} bytes)",
                budget.max_memory_bytes.unwrap_or_default(),
                observed.memory_bytes
            ))
        } else if budget
            .max_processes
            .is_some_and(|limit| observed.processes > limit)
        {
            Some(format!(
                "Task exceeded its process-count budget of {} (observed {})",
                budget.max_processes.unwrap_or_default(),
                observed.processes
            ))
        } else {
            None
        };
        if budget_failure.is_some() {
            child.terminate();
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    #[cfg(unix)]
    if let Some((master, output)) = terminal_io.as_mut() {
        if let Err(error) = drain_shell_terminal(master, output) {
            budget_failure.get_or_insert_with(|| format!("PTY final output persistence failed: {error}"));
        }
    }
    let output_bytes = match shell_output_bytes(&task.stdout_path, &task.stderr_path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            budget_failure.get_or_insert_with(|| format!("Task final output observation failed: {error}"));
            None
        }
    };
    // The child may exit before the next live budget check, and PTY draining
    // can add bytes after that check. Final output must obey the same budget.
    if let (Some(bytes), Some(limit)) = (output_bytes, scheduling.resource_budget.max_output_bytes) {
        if bytes > limit {
            budget_failure.get_or_insert_with(|| format!(
                "Task exceeded its output budget of {limit} bytes (observed {bytes})"));
        }
    }
    let tree_stop_confirmed = matches!(child.stop_fact(), process_control::TreeStopFact::Confirmed { .. });
    if tree_stop_confirmed {if let Some(prepared)=prepared_sandbox.as_mut() {prepared.settle();}}
    if !tree_stop_confirmed {
        budget_failure.get_or_insert_with(|| format!("Shell process tree stop is unconfirmed: {:?}; side effects remain unresolved; not safe to retry", child.stop_fact()));
    }
    let exit_code = exit_status
        .as_ref()
        .and_then(|status| status.code())
        .unwrap_or(-1);
    #[cfg(unix)]
    let signal = exit_status.as_ref().and_then(|status| {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    });
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    let exit_evidence = match prepared_sandbox.as_ref() {
        Some(prepared)=>prepared.exit_evidence(exit_code),None=>tools::read_shell_exit_evidence(&evidence_path,exit_code),
    };
    let (semantic_error, semantic_note) = tools::command_semantic(&task.command, exit_code, exit_evidence.as_ref());
    let status_label = if !tree_stop_confirmed {
        "failed(process_tree_stop_unknown)".to_string()
    } else if cancelled {
        "cancelled".to_string()
    } else if let Some(error) = budget_failure.as_ref() {
        format!("failed({})", error.replace('\n', " "))
    } else if let Some(signal) = signal {
        format!("signaled({signal})")
    } else {
        format!("exited({exit_code})")
    };
    let terminal_status = if !tree_stop_confirmed {
        "failed"
    } else if cancelled {
        "cancelled"
    } else if budget_failure.is_some() || signal.is_some() || semantic_error {
        "failed"
    } else {
        "completed"
    };
    let terminal_error = budget_failure.clone().or_else(|| {
        (signal.is_some() || semantic_error).then(|| format!("Shell command finished with {status_label}: {semantic_note}"))
    });
    let terminal_at_ms = unix_epoch_millis();
    let final_observation = nola_application::RuntimeTaskFinalObservation {
        output_bytes,
        resource_usage: has_resource_observation.then_some(nola_application::RuntimeTaskResourceUsage {
            cpu_time_ms: observed.cpu_time_ms,
            peak_memory_bytes: observed.memory_bytes,
            peak_processes: observed.processes,
        }),
        metadata_patch: std::collections::BTreeMap::from([("shell_execution".to_string(), Some(json!({
            "exit_code": exit_status.as_ref().and_then(|status| status.code()),
            "signal": signal,
            "exit_evidence": exit_evidence,
            "semantic_note": semantic_note,
            "residual_processes_stopped": child.residual_processes_stopped(),
            "process_tree_stop": child.stop_fact(),
            "process_tree_stop_request_error": child.stop_request_error(),
            "safe_to_retry": false,
        })))]),
    };
    #[cfg(test)]
    if exit_code != 0 || budget_failure.is_some() {
        eprintln!("shell fixture settlement: task={} code={} budget={:?} tree={:?}",
            task.id, exit_code, budget_failure, child.stop_fact());
    }
    let finished = crate::runtime_task_port::finish_detailed_with_event_now(
        &lease,
        terminal_status,
        None,
        Some(status_label.clone()),
        Some(format!(
            "Shell command finished with {status_label}. Use read_task for bounded stdout and stderr."
        )),
        terminal_error,
        Some(task.stdout_path.clone()),
        true,
        if tree_stop_confirmed { "finished" } else { "reconciliation_required" },
        terminal_status,
        Some(status_label.clone()),
        json!({
            "exit_code": exit_code,
            "signal": signal,
            "exit_evidence": exit_evidence,
            "semantic_note": semantic_note,
            "output_bytes": output_bytes,
            "budget_exhaustion": budget_failure,
            "process_observed_at_ms": terminal_at_ms,
            "safe_to_retry": false,
            "process_tree_stop": child.stop_fact(),
        })
        .to_string(),
        Some(final_observation),
    ).unwrap_or_else(|error| {
        crate::runtime_task_port::task_recovery_diagnostic(&task.id, "shell_final_observation", error);
        false
    });
    heartbeat.stop();
    if !finished {
        return 1;
    }
    // Raw process evidence remains recoverable if the atomic final commit
    // fails. A process exit alone never authorizes deleting that evidence.
    if tree_stop_confirmed { let _ = std::fs::remove_file(&evidence_path); }
    let _ = std::fs::write(&task.status_path, if tree_stop_confirmed {
        format!("{status_label}\n")
    } else { "needs_reconciliation\n".into() });
    tools::wake_background_task_notification();
    if terminal_status == "completed" {
        0
    } else {
        1
    }
}

fn finish_shell_start_failure(
    request: &ShellTaskProcessRequest,
    lease: &crate::runtime_task_port::RuntimeTaskLeaseHandle,
    error: &str,
) -> i32 {
    let observed_at_ms = unix_epoch_millis();
    let finished = crate::runtime_task_port::finish_detailed_with_event_now(
        lease,
        "failed",
        None,
        Some("Shell task failed to start".to_string()),
        None,
        Some(error.to_string()),
        Some(request.task.stdout_path.clone()),
        true,
        "start_failed",
        "failed",
        Some(error.to_string()),
        json!({"error": error, "process_observed_at_ms": observed_at_ms}).to_string(),
        None,
    )
    .unwrap_or(false);
    if finished {
        let _ = std::fs::write(&request.task.status_path, format!("failed({error})\n"));
    }
    2
}

fn shell_output_bytes(stdout: &str, stderr: &str) -> Result<u64, String> {
    let observe = |path| std::fs::metadata(path).map(|metadata| metadata.len())
        .map_err(|error| format!("observe Shell output length failed: {error}"));
    observe(stdout)?.checked_add(observe(stderr)?).ok_or_else(|| "Shell output byte count overflow".into())
}

fn terminate_child_process_group(child: &mut std::process::Child) {
    #[cfg(any(unix, windows))]
    terminate_external_process_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn detach_worker_session(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeTaskNotification {
    pub delivery: nola_application::RuntimeTaskNotificationDelivery,
    pub id: String,
    pub title: String,
    pub status: String,
    pub agent_type: Option<String>,
    pub elapsed_secs: f64,
}

pub fn spawn_user_agent(config: &HostInvocationRequest, agent_type: &str, prompt: &str,
    emitter:&mut dyn crate::runtime_output::RuntimeOutputPort) -> Result<crate::core_agent_runtime::CoreRunOutcome,String> {
    let submission_id=format!("user-delegation:{}",uuid::Uuid::new_v4());
    let mut request=config.clone();
    let submission=nola_core::agent::AgentSubmission::UserDelegation {
        agent_type:agent_type.trim().into(),prompt:prompt.trim().into(),submission_id:submission_id.clone(),
        submitted_at_epoch_ms:unix_epoch_millis(),
    };
    let item=nola_core::agent::protocol::user_delegation_item(agent_type.trim(),prompt.trim(),&submission_id)?;
    request.session.host_submission_id=Some(submission_id.clone());
    request.session.host_user_message_id=Some(format!("{submission_id}:user"));
    request.session.user_delegation=Some(submission);
    crate::core_agent_runtime::run_agent_core_items(&request,Vec::new(),vec![item],emitter)
}

pub fn available_agent_profiles(config: &HostInvocationRequest) -> Vec<(String, String)> {
    let registry = AgentRegistry::from_profiles(&config.runtime.agents);
    registry
        .names()
        .into_iter()
        .filter_map(|name| {
            registry
                .get(&name)
                .map(|profile| (profile.name.clone(), profile.description.clone()))
        })
        .collect()
}

pub fn running_task_count(config: &HostInvocationRequest) -> usize {
    let root = config.tool_settings.project_root.to_string_lossy();
    NolaStore::open_default()
        .and_then(|store| store.list_runtime_tasks(&root))
        .unwrap_or_default()
        .iter()
        .filter(|task| task_scope_matches(config, task) && is_live_status(&task.status))
        .count()
}

/// Reconciles every Host-owned execution into the canonical RuntimeTask
/// projection. The supervisor owns leases and lifecycle only; Core continues
/// to own model decisions, tool selection, and task semantics.
pub fn reconcile_supervised_tasks(config: &HostInvocationRequest) {
    for workspace_config in authorized_supervisor_workspaces(config) {
        recover_orphaned_shell_tasks(&workspace_config);
        recover_orphaned_agent_tasks(&workspace_config);
        recover_orphaned_external_executor_tasks(&workspace_config);
    }
}

fn recover_orphaned_external_executor_tasks(config: &HostInvocationRequest) {
    let root = config.tool_settings.project_root.to_string_lossy();
    let Ok(store) = NolaStore::open_default() else {
        return;
    };
    let Ok(tasks) = store.list_runtime_tasks(&root) else {
        return;
    };
    let now = unix_epoch_millis();
    for task in tasks {
        if !matches!(task.kind.as_str(), "external_executor" | "mcp") {
            continue;
        }
        if task.status == "queued" {
            let host_pid = serde_json::from_str::<Value>(&task.metadata_json)
                .ok()
                .and_then(|value| value.get("host_pid").and_then(Value::as_u64))
                .and_then(|value| u32::try_from(value).ok());
            if host_pid.is_some_and(|pid| pid != std::process::id() && !process_id_is_alive(pid)) {
                let reason = if task.kind == "mcp" {
                    "MCP Host exited before the transport started; no remote request was begun"
                } else {
                    "External executor Host exited before the process started; no external side effect was begun"
                };
                if store
                    .request_runtime_task_cancel(&task.id, now)
                    .unwrap_or(false)
                {
                    let _ = store.append_runtime_task_event(
                        &task.id,
                        "abandoned_before_start",
                        "cancelled",
                        Some(reason),
                        &json!({"safe_to_retry":true,"host_pid":host_pid}).to_string(),
                        now,
                    );
                    wake_task_waiters();
                    tools::wake_background_task_notification();
                }
            }
            continue;
        }
        if task.status != "running" || task.lease_until_ms.is_none_or(|lease| lease >= now) {
            continue;
        }
        let metadata = serde_json::from_str::<Value>(&task.metadata_json).unwrap_or(Value::Null);
        let retryable_read_only_mcp = task.kind == "mcp"
            && metadata.get("read_only").and_then(Value::as_bool) == Some(true)
            && metadata
                .get("safe_to_retry_after_claim")
                .and_then(Value::as_bool)
                == Some(true);
        if retryable_read_only_mcp {
            let reason =
                "Read-only MCP ownership expired; the same invocation may be safely reclaimed";
            if store
                .recover_expired_runtime_task(&task.id, reason, now, now)
                .unwrap_or(false)
            {
                let _ = store.append_runtime_task_event(
                    &task.id,
                    "retry_scheduled",
                    "queued",
                    Some(reason),
                    &json!({"server_id":task.agent_type,"safe_to_retry":true,"parent_effect_id":task.parent_tool_call_id}).to_string(),
                    now,
                );
                wake_task_waiters();
                tools::wake_background_task_notification();
            }
            continue;
        }
        let reason = if task.kind == "mcp" {
            "MCP ownership expired; verify remote side effects before starting a new invocation"
        } else {
            "External executor ownership expired; verify remote and filesystem side effects before starting a new run"
        };
        if store
            .fail_expired_runtime_task_with_event(
                &task.id,
                reason,
                "reconciliation_required",
                "failed",
                Some(reason),
                &json!({"backend_id":task.agent_type,"safe_to_retry":false}).to_string(),
                now,
            )
            .unwrap_or(false)
        {
            wake_task_waiters();
            tools::wake_background_task_notification();
        }
    }
}

fn authorized_supervisor_workspaces(config: &HostInvocationRequest) -> Vec<HostInvocationRequest> {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let authorized_roots = config.tool_settings.allowed_dirs
        .iter()
        .map(|path| canonical(path))
        .collect::<Vec<_>>();
    let current_root = canonical(&config.tool_settings.project_root);
    let mut project_roots = vec![current_root.clone()];
    if let Ok(tasks) = NolaStore::open_default().and_then(|store| store.list_all_runtime_tasks()) {
        for task in tasks {
            if !is_live_status(&task.status) {
                continue;
            }
            let project_root = canonical(Path::new(&task.project_root));
            let authorized = project_root == current_root
                || authorized_roots
                    .iter()
                    .any(|root| project_root == *root || project_root.starts_with(root));
            if authorized && !project_roots.contains(&project_root) {
                project_roots.push(project_root);
            }
        }
    }
    project_roots
        .into_iter()
        .map(|project_root| {
            let mut workspace_config = config.clone();
            workspace_config.tool_settings.project_root = project_root;
            workspace_config.session.session_id = None;
            workspace_config
        })
        .collect()
}

/// Wake only after the original App exported its real commit and the durable
/// Task owner admitted it. Neither this callback nor a UI marker grants work.
pub fn reconcile_committed_agent_task(config: &HostInvocationRequest, task_id: &str) {
    if !crate::runtime_task_port::agent_authority(task_id)
        .ok()
        .flatten()
        .is_some_and(|leaf| {
            leaf.phase
                == nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Admitted
        })
    {
        return;
    }
    reconcile_supervised_tasks(config);
}

/// Durable cross-process scheduler high-water mark. Interactive Hosts compare
/// this cursor while blocked for terminal input and reconcile only after a
/// persisted task event advances it. Event payloads are deliberately not
/// interpreted here; the Store and fair claim transaction remain authoritative.
pub fn latest_scheduler_event_cursor() -> Result<i64, String> {
    NolaStore::open_default()?.latest_runtime_task_event_id()
}

/// Persists guidance before acknowledging admission. Process-local handles
/// and cancellation queues are not delivery or ownership evidence.
pub fn queue_agent_guidance(
    owner_session_id: &str,
    parent_turn_id: &str,
    task_id: &str,
    content: &str,
) -> Result<String, String> {
    queue_agent_guidance_intent(owner_session_id, parent_turn_id, task_id, content,
        &uuid::Uuid::new_v4().to_string(), unix_epoch_millis()).and_then(|record| {
            if record.status == "rejected" { Err(format!("Guidance {} was rejected: {}", record.message_id,
                record.reason.as_deref().unwrap_or("task cannot accept guidance"))) }
            else { Ok(record.message_id) }
        })
}

pub fn queue_agent_guidance_intent(
    owner_session_id: &str, parent_turn_id: &str, task_id: &str, content: &str,
    message_id: &str, submitted_at_epoch_ms: i64,
) -> Result<nola_application::RuntimeTaskGuidance, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("Agent guidance cannot be empty".to_string());
    }
    let task = NolaStore::open_default()?
        .runtime_task(task_id)?
        .ok_or_else(|| format!("Unknown task: {task_id}"))?;
    if task.owner_session_id.as_deref() != Some(owner_session_id) || task.kind != "agent" {
        return Err("Agent guidance does not belong to the active parent task".to_string());
    }
    let message_id = message_id.to_string();
    let guidance: nola_application::RuntimeTaskGuidance =
        crate::application_port::CliApplicationPort::persistent_default()?.execute(
            nola_application::ApplicationCommand::SubmitTaskGuidance {
                task_id: task_id.to_string(),
                message_id: message_id.clone(),
                parent_turn_id: parent_turn_id.to_string(),
                content: content.to_string(),
                submitted_at_ms: submitted_at_epoch_ms,
            },
        )?;
    if guidance.message_id != message_id
        || guidance.task_id != task_id
        || guidance.owner_session_id != owner_session_id
    {
        return Err("guidance acknowledgement does not match the submitted identity".into());
    }
    // A failed local wake cannot undo an already committed submission. The
    // durable parent/child FIFO remains the recovery source across processes.
    let _ = crate::core_agent_runtime::drain_registered_agent_task_inbox(owner_session_id);
    wake_task_waiters();
    Ok(guidance)
}

pub fn running_core_task_count(session_id: Option<&str>) -> usize {
    session_id
        .and_then(|id| {
            crate::session::load_session_task_projection(id)
                .ok()
                .flatten()
        })
        .map(|projection| {
            projection
                .tasks
                .values()
                .filter(|task| !task.state.is_terminal())
                .count()
        })
        .unwrap_or(0)
}

pub fn core_task_footer_summary(session_id: Option<&str>) -> Option<String> {
    let projection = session_id.and_then(|id| {
        crate::session::load_session_task_projection(id)
            .ok()
            .flatten()
    })?;
    let live = projection
        .order
        .iter()
        .filter_map(|id| projection.tasks.get(id))
        .filter(|task| !task.state.is_terminal())
        .collect::<Vec<_>>();
    let current = live.last()?;
    let step = current
        .current_step
        .as_deref()
        .unwrap_or(current.id.as_str());
    let progress = current
        .progress
        .map(|value| format!(" · {:.0}%", value * 100.0))
        .unwrap_or_default();
    Some(format!(
        "● {} · {}{} · /tasks",
        live.len(),
        truncate_chars(step, 72),
        progress
    ))
}

pub fn manage_runtime_tasks(config: &HostInvocationRequest, action: &str, task_id: Option<&str>) -> ToolOutcome {
    let skills =
        crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(config.tool_settings.skill_dirs.clone());
    match action {
        "list" => render_task_list(config),
        "status" => {
            let Some(task_id) = task_id else {
                return task_error("task_id is required");
            };
            if runtime_task_kind(config, task_id).as_deref() == Some("agent") {
                return render_task_detail(config, task_id, &skills);
            }
            if let Some(task) = core_task(config, task_id) {
                return render_core_task_detail(&task);
            }
            tools::manage_background_bash_tasks(
                "status",
                Some(task_id),
                &config.tool_settings.allowed_dirs,
                config.session.session_id.as_deref(),
                &skills,
            )
        }
        "stop" => {
            let Some(task_id) = task_id else {
                return task_error("task_id is required");
            };
            if core_task(config, task_id).is_some() {
                let Some(session_id) = config.session.session_id.as_deref() else {
                    return task_error("Core task cancellation requires an active session");
                };
                return match cancel_agent_core_task(session_id, task_id) {
                    Ok(_) => task_ok(
                        format!("Task `{task_id}` cancellation requested"),
                        format!("Task {task_id} cancellation requested"),
                    ),
                    Err(error) => task_error(error),
                };
            }
            if runtime_task_kind(config, task_id).as_deref() == Some("agent") {
                let mut args = Map::new();
                args.insert("task_id".to_string(), Value::String(task_id.to_string()));
                stop_task(config, &args, &skills)
            } else {
                tools::manage_background_bash_tasks(
                    "stop",
                    Some(task_id),
                    &config.tool_settings.allowed_dirs,
                    config.session.session_id.as_deref(),
                    &skills,
                )
            }
        }
        _ => task_error("tasks action must be list, status, or stop"),
    }
}

pub fn take_runtime_task_notifications(
    config: &HostInvocationRequest,
) -> Result<Vec<RuntimeTaskNotification>, String> {
    if !config.session.save_session {
        return Ok(Vec::new());
    }
    let store = NolaStore::open_default()?;
    let root = config.tool_settings.project_root.to_string_lossy();
    let mut tasks = store.list_runtime_tasks(&root)?;
    tasks.sort_by_key(|task| task.ended_at_ms.unwrap_or(task.updated_at_ms));
    let mut notifications = Vec::new();
    for task in tasks {
        if !task_scope_matches(config, &task) || task.kind != "agent" {
            continue;
        }
        // Neither terminal nor permission presentation depends on the model
        // mailbox's unread bit. Only this consumer's occurrence ACK matters.
        if is_live_status(&task.status) && !matches!(task.status.as_str(), "waiting_permission" | "waiting_input") {
            continue;
        }
        let permission = if task.status == "waiting_permission" {
            let suspension: Option<nola_application::RuntimeTaskPermissionSuspension> =
                crate::application_port::CliApplicationPort::persistent_default()?.execute(
                    nola_application::ApplicationCommand::RuntimeTask {
                        command: nola_application::RuntimeTaskCommand::PendingPermissionSuspension {
                            task_id: task.id.clone(),
                        },
                    },
                )?;
            let Some(suspension) = suspension else {
                continue;
            };
            Some(suspension.permission_request_id)
        } else {
            None
        };
        let occurrences = if task.status == "waiting_input" {
            crate::user_input_wait::owned(task.owner_session_id.as_deref().ok_or("Task notification has no owner")?, Some(task.id.clone()))?
                .into_iter().map(|record| nola_store_port::runtime_task::user_input_notification_occurrence_id(
                    &record.stored.run_id, &record.stored.wait.wait_id, record.stored.wait.revision,
                    &record.stored.wait.response_schema_digest)).collect::<Vec<_>>()
        } else { vec![match permission {
            Some(id) => format!("permission:{id}"),
            None => format!("terminal:{}", crate::runtime_task_port::result_delivery_cursor(&task)?.terminal_receipt_id),
        }] };
        for occurrence_id in occurrences {
        let delivery = nola_application::RuntimeTaskNotificationDelivery {
            owner_conversation_id: Some(task.owner_session_id.clone().ok_or("Task notification has no owner")?),
            task_id: task.id.clone(),
            command_id: format!("cli-presentation:{}:{occurrence_id}", task.id),
            occurrence_id,
            consumer_id: "cli-interactive".into(),
        };
        let delivered: bool = crate::application_port::CliApplicationPort::persistent_default()?.execute(
            nola_application::ApplicationCommand::RuntimeTask { command: nola_application::RuntimeTaskCommand::NotificationDelivered { delivery: delivery.clone() } })?;
        if delivered {
            continue;
        }
        let elapsed_ms = task
            .ended_at_ms
            .unwrap_or(task.updated_at_ms)
            .saturating_sub(task.started_at_ms.unwrap_or(task.created_at_ms));
        notifications.push(RuntimeTaskNotification {
            delivery,
            id: task.id.clone(),
            title: task.title.clone(),
            status: task.status.clone(),
            agent_type: task.agent_type.clone(),
            elapsed_secs: elapsed_ms.max(0) as f64 / 1000.0,
        });
        }
    }
    Ok(notifications)
}

pub fn acknowledge_runtime_task_notification_delivery(notification: &RuntimeTaskNotification) -> Result<nola_application::RuntimeTaskNotificationAcknowledgement, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        nola_application::ApplicationCommand::RuntimeTask { command: nola_application::RuntimeTaskCommand::AcknowledgeDelivery { delivery: notification.delivery.clone() } })
}

pub fn deliver_runtime_task_notification(
    notification: &RuntimeTaskNotification,
    present: impl FnOnce() -> std::io::Result<NotificationPresentation>,
) -> Result<Option<nola_application::RuntimeTaskNotificationAcknowledgement>, String> {
    match present().map_err(|error| format!("Task notification was not presented: {error}"))? {
        NotificationPresentation::Deferred => Ok(None),
        NotificationPresentation::Presented => acknowledge_runtime_task_notification_delivery(notification).map(Some),
    }
}

pub fn execute_runtime_task_tool(
    config: &HostInvocationRequest,
    owner_session_id: &str,
    request: &HostRequestEnvelope,
    name: &str,
    call_id: &str,
    args: &Map<String, Value>,
    context: &ToolContext,
) -> Option<RuntimeToolResult> {
    match name {
        "spawn_agent" => Some(spawn_agent(config, request, owner_session_id, call_id, args)),
        "list_tasks" => Some(RuntimeToolResult::completed(list_tasks(config))),
        "read_task" => Some(RuntimeToolResult::completed(read_task(
            config,
            args,
            &context.skills,
        ))),
        "wait_task" => Some(RuntimeToolResult::completed(wait_task(
            config, args, context,
        ))),
        "stop_task" => Some(RuntimeToolResult::completed(stop_task(
            config,
            args,
            &context.skills,
        ))),
        "send_agent_message" => Some(RuntimeToolResult::completed(send_agent_message(
            owner_session_id,
            request,
            args,
        ))),
        _ => None,
    }
}

fn send_agent_message(
    owner_session_id: &str,
    request: &HostRequestEnvelope,
    args: &Map<String, Value>,
) -> ToolOutcome {
    if request.session_id != owner_session_id {
        return task_error("Agent guidance request belongs to another conversation");
    }
    let Some(parent_turn_id) = request.turn_id.as_deref() else {
        return task_error("Agent guidance requires an active Core Turn");
    };
    let Some(task_id) = string_arg(args, "task_id") else {
        return task_error("send_agent_message requires task_id");
    };
    let Some(message) = string_arg(args, "message") else {
        return task_error("send_agent_message requires message");
    };
    match queue_agent_guidance(owner_session_id, parent_turn_id, task_id, message) {
        Ok(message_id) => task_ok(
            json!({
                "task_id": task_id,
                "message_id": message_id,
                "status": "queued"
            })
            .to_string(),
            format!("Guidance queued · {task_id}"),
        ),
        Err(error) => task_error(error),
    }
}

fn runtime_task_update(task: &DbRuntimeTask) -> Result<RuntimeTaskUpdate, String> {
    let state = match task.status.as_str() {
        "queued" => nola_core::agent::TaskState::Queued,
        "running" => nola_core::agent::TaskState::Running,
        "waiting_permission" | "waiting_input" | "retry_waiting" => nola_core::agent::TaskState::Waiting,
        "suspended" | "awaiting_reconciliation" => nola_core::agent::TaskState::Suspended,
        "sealed" => nola_core::agent::TaskState::Sealed,
        "cancelled" => nola_core::agent::TaskState::Cancelled,
        "failed" => nola_core::agent::TaskState::Failed,
        status => {
            return Err(format!(
                "Agent task {} has unsupported persisted state {status}",
                task.id
            ))
        }
    };
    let progress = None;
    let result_ref = task.output_path.clone().or_else(|| {
        state
            .is_terminal()
            .then(|| format!("nola-task://{}", task.id))
    });
    Ok(RuntimeTaskUpdate {
        task_id: task.id.clone(),
        state,
        progress,
        current_step: task.progress_summary.clone(),
        result_ref,
        cancel_requested: task.cancel_requested,
        ownership_token: task.ownership_token.clone(),
        completion_watermark: task.completion_watermark,
    })
}

/// Resolves a redelivered durable runtime call without executing it again.
/// Only capabilities whose tool-call identity becomes a durable task boundary
/// participate; ordinary runtime tools continue through normal transactions.
pub fn replay_runtime_task_tool(name: &str, call_id: &str) -> Option<RuntimeTaskUpdate> {
    if name != "spawn_agent" {
        return None;
    }
    let store = NolaStore::open_default().ok()?;
    store
        .list_all_runtime_tasks()
        .ok()?
        .iter()
        .find(|task| task.parent_tool_call_id.as_deref() == Some(call_id))
        // Terminal Agent state is imported only through its source-validated
        // receipt. Falling through reaches spawn's durable replay below, not
        // a new Task or a worker-authored TaskUpdated(Sealed).
        .filter(|task| is_live_status(&task.status))
        .and_then(|task| runtime_task_update(task).ok())
}

fn require_isolated_agent_model_channel(config: &HostInvocationRequest) -> Result<(), String> {
    // Isolated workers currently receive one launch payload, then stdin is
    // closed. They cannot attest a native channel through the owning Mac.
    // RuntimeConfig deliberately skips this trusted identity in generic serde;
    // reject before serialization, never downgrade it into ordinary HTTP.
    if config.runtime.native_model_route.is_some() {
        return Err("Native-model subagents require an authenticated Mac control channel; this isolated worker has none. Continue the work in the current foreground Run.".into());
    }
    Ok(())
}

fn spawn_agent(
    config: &HostInvocationRequest,
    request: &HostRequestEnvelope,
    owner_session_id: &str,
    call_id: &str,
    args: &Map<String, Value>,
) -> RuntimeToolResult {
    let agent_type = string_arg(args, "agent_type").unwrap_or("explore");
    let description = string_arg(args, "description")
        .or_else(|| string_arg(args, "name"))
        .unwrap_or_else(|| ui_text(UiText::Task))
        .trim();
    let Some(prompt) = string_arg(args, "prompt")
        .or_else(|| string_arg(args, "message"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return RuntimeToolResult::completed(task_error("spawn_agent requires a non-empty prompt"));
    };
    let registry = AgentRegistry::from_profiles(&config.runtime.agents);
    let Some(profile) = registry.get(agent_type).cloned() else {
        return RuntimeToolResult::completed(task_error(format!(
            "Unknown agent type: {agent_type}. Available agents: {}",
            registry.names().join(", ")
        )));
    };
    let requested_read_only = match args.get("permission_mode") {
        None => false,
        Some(Value::String(mode)) if mode == "ask" => false,
        Some(Value::String(mode)) if mode == "read_only" => true,
        _ => return RuntimeToolResult::completed(task_error(
            "permission_mode must be read_only or ask; delegation arguments do not grant tool authority")),
    };
    let can_mutate = registry.can_mutate_or_execute(agent_type) && !requested_read_only;
    let max_turns = args
        .get("max_turns")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, 20) as usize)
        .or(profile.max_turns);
    let project_root = config.tool_settings.project_root.canonicalize().unwrap_or_else(|_|config.tool_settings.project_root.clone()).to_string_lossy().into_owned();
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(error) => return RuntimeToolResult::completed(task_error(error)),
    };
    let resumed_task = match string_arg(args, "task_id") {
        Some(task_id) => match store.runtime_task(task_id) {
            Ok(Some(task)) => {
                match resumable_child_session(&task, &project_root, owner_session_id, agent_type) {
                    Ok(child_session_id) => Some((task, child_session_id)),
                    Err(error) => return RuntimeToolResult::completed(task_error(error)),
                }
            }
            Ok(None) => {
                return RuntimeToolResult::completed(task_error(format!(
                    "Agent task not found: {task_id}"
                )))
            }
            Err(error) => return RuntimeToolResult::completed(task_error(error)),
        },
        None => None,
    };

    let backend = if let Some((previous, _)) = &resumed_task {
        let frozen = serde_json::from_str::<Value>(&previous.metadata_json)
            .ok().and_then(|v| serde_json::from_value::<nola_host_protocol::delegation::DelegatedInvocationSnapshot>(
                v["delegated_invocation"].clone()).ok());
        match frozen {
            Some(snapshot) => snapshot.execution_backend,
            None => return RuntimeToolResult::completed(task_error("Original delegated execution snapshot missing")),
        }
    } else {
        profile.executor.as_ref().map(|e| e.backend.id().to_owned())
            .unwrap_or_else(|| config.execution.execution_backend.clone())
    };
    let observed_worktree = (|| -> Result<_,String> {
        let inherited = if let Some((previous,_))=&resumed_task {
            let metadata:Value=serde_json::from_str(&previous.metadata_json).map_err(|e|e.to_string())?;
            let snapshot:nola_host_protocol::delegation::DelegatedInvocationSnapshot=
                serde_json::from_value(metadata["delegated_invocation"].clone()).map_err(|e|e.to_string())?;
            snapshot.worktree
        } else if let Some(value)=args.get("worktree") {
            let selection=serde_json::from_value(value.clone()).map_err(|_|"Invalid task worktree selection")?;
            return crate::tools::capture_task_worktree(&selection,&config.tool_settings.allowed_dirs).map(Some);
        } else {config.tool_settings.task_worktree.clone()};
        if let Some(binding)=&inherited {crate::tools::verify_task_worktree(binding)?;}
        Ok(inherited)
    })();
    let observed_worktree=match observed_worktree {Ok(value)=>value,Err(error)=>return RuntimeToolResult::completed(task_error(error))};
    if backend == "nola" {
        if let Err(error) = require_isolated_agent_model_channel(config) {
            return RuntimeToolResult::completed(task_error(error));
        }
    } else {
        match crate::external_executor::discover_execution_backend(&backend, false) {
            Some(descriptor) if descriptor.availability == nola_domain::execution_backend::ExecutionBackendAvailability::Available => {},
            Some(descriptor) => return RuntimeToolResult::completed(task_error(format!(
                "External subagent {backend} is not ready: {:?}. Use its normal installed CLI login/setup first.", descriptor.availability))),
            None => return RuntimeToolResult::completed(task_error("Unknown subagent executor")),
        }
    }
    let now = unix_epoch_millis();
    let task_id = format!("task-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let resumed_from_task_id = resumed_task.as_ref().map(|(task, _)| task.id.clone());
    let parent_task_id = resumed_task.as_ref().and_then(|(task, _)| task.parent_task_id.clone());
    let child_session_id = resumed_task
        .as_ref()
        .map(|(_, child_session_id)| child_session_id.clone())
        .unwrap_or_else(|| format!("agent-{}", &uuid::Uuid::new_v4().simple().to_string()[..16]));
    let priority = match string_arg(args, "priority").unwrap_or("normal") {
        "low" => nola_application::RuntimeTaskPriority::Low,
        "normal" => nola_application::RuntimeTaskPriority::Normal,
        "high" => nola_application::RuntimeTaskPriority::High,
        value => {
            return RuntimeToolResult::completed(task_error(format!(
                "Invalid task priority: {value}"
            )))
        }
    };
    let max_wall_time_ms = args
        .get("max_wall_time_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TASK_WALL_TIME_MS / 1_000)
        .clamp(1, 86_400)
        .saturating_mul(1_000);
    let max_output_bytes = args
        .get("max_output_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TASK_OUTPUT_BYTES)
        .clamp(1_024, 16 * 1024 * 1024);
    let max_cpu_time_ms = args
        .get("max_cpu_time_seconds")
        .and_then(Value::as_u64)
        .map(|seconds| seconds.clamp(1, 86_400).saturating_mul(1_000));
    let max_memory_bytes = args
        .get("max_memory_bytes")
        .and_then(Value::as_u64)
        .map(|bytes| bytes.clamp(16 * 1024 * 1024, 64 * 1024 * 1024 * 1024));
    let max_processes = args
        .get("max_processes")
        .and_then(Value::as_u64)
        .map(|count| count.clamp(1, 256) as u32);
    let sop_invocation = tools::current_sop_invocation_context();
    let input = nola_application::CreateRuntimeTask {
        id: task_id.clone(),
        kind: "agent".to_string(),
        title: description.to_string(),
        project_root: project_root.clone(),
        owner_session_id: Some(owner_session_id.to_string()),
        parent_task_id: parent_task_id.clone(),
        parent_tool_call_id: Some(call_id.to_string()),
        child_session_id: Some(child_session_id.clone()),
        agent_type: Some(agent_type.to_string()),
        prompt: Some(prompt.to_string()),
        initial_progress_summary: Some(ui_text(UiText::Queued).to_string()),
        initial_activity: None,
        output_path: None,
        initially_unread: false,
        input_watermark: Some(now),
        created_at_ms: now,
        metadata: json!({
            "max_turns": max_turns,
            "model_notified": false,
            "can_mutate": can_mutate,
            "requested_read_only": requested_read_only,
            "execution_backend": config.execution.execution_backend,
            "execution_model": config.execution.external_execution_model,
            "execution_model_provider": config.execution.external_execution_model_provider,
            "resumed_from_task_id": resumed_from_task_id.clone(),
            "_nolaExecution": sop_invocation
        }),
        scheduling: nola_application::RuntimeTaskScheduling {
            priority,
            queued_at_ms: now,
            resource_budget: nola_application::RuntimeTaskResourceBudget {
                max_wall_time_ms: Some(max_wall_time_ms),
                max_output_bytes: Some(max_output_bytes),
                max_cpu_time_ms,
                max_memory_bytes,
                max_processes,
            },
            output_bytes: 0,
            resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        },
    };
    let prepared: nola_application::delegation_preparation::PreparedDelegatedTask = match crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut app| app.execute(nola_application::ApplicationCommand::PrepareDelegatedTask {
            input: nola_application::delegation_preparation::PrepareDelegatedTask { request:request.clone(), input, now_ms:unix_epoch_millis(), observed_worktree },
        })) {
        Ok(value)=>value,
        Err(error)=>return RuntimeToolResult::completed(task_error(error)),
    };
    if prepared.replayed && prepared.source_admitted {
        // The shared preparation returns the original task. Reconcile terminal
        // receipts through the existing path; never create a second worker.
        let existing=match store.runtime_task(&prepared.task.id) {Ok(Some(value))=>value,Ok(None)=>return RuntimeToolResult::completed(task_error("Prepared delegation disappeared")),Err(error)=>return RuntimeToolResult::completed(task_error(error))};
        if !is_live_status(&existing.status) {
            if let Err(error)=crate::runtime_task_port::enqueue_terminal_reconciliation(&existing.id) {return RuntimeToolResult::completed(task_error(error));}
            return RuntimeToolResult::completed(task_ok(json!({"task_id":existing.id,"status":existing.status,"replayed":true,"terminal_receipt_delivery":"queued"}).to_string(),format!("{} · {}",existing.status,existing.id)));
        }
        let update=match runtime_task_update(&existing) {Ok(value)=>value,Err(error)=>return RuntimeToolResult::completed(task_error(error))};
        return RuntimeToolResult::task_updated(task_ok(json!({"task_id":existing.id,"status":existing.status,"replayed":true}).to_string(),format!("{} · {}",existing.status,existing.id)),update);
    }

    let task_id=prepared.task.id.clone();
    let child_session_id=prepared.task.child_session_id.clone().expect("Core prepared agent child identity");
    let now=prepared.task.input_watermark.expect("Core prepared agent input watermark");
    // TaskPromoted is the transaction boundary that establishes the Core task
    // projection. Starting a worker before the Host response is committed can
    // race a Running/Completed update ahead of the queued promotion. Persist
    // and wake here; the shared Supervisor claims the task after promotion.
    tools::wake_background_task_notification();

    RuntimeToolResult {
        outcome: task_ok(
            json!({
                "task_id": task_id,
                "agent_id": child_session_id,
                "agent_type": agent_type,
                "execution_backend": prepared.snapshot.execution_backend,
                "task_worktree_selection": prepared.snapshot.worktree.as_ref().map(|binding| &binding.selection),
                "delivery_review": prepared.snapshot.worktree.as_ref().map(|_| "Review all changed paths and verify the child delivery. Use git_worktree_integrate only after reviewing/committing it and confirming the source baseline; no automatic acceptance or merge."),
                "execution_model": prepared.snapshot.external_execution_model,
                "worktree": prepared.snapshot.worktree,
                "sandbox_requirement": prepared.snapshot.sandbox_requirement,
                "status": "queued",
                "description": description,
                "permission_mode": if requested_read_only { "read_only" } else { "ask" },
                "capability_scope": if requested_read_only {
                    "Write and command tools are excluded. Do not assign steps requiring those tools."
                } else {
                    "Profile-eligible tools; each invocation remains subject to Core permission checks."
                },
                "resumed_from_task_id": resumed_from_task_id.clone(),
                "next": "Use wait_task when this result blocks progress; otherwise continue independent work. Completion is delivered as a task notification."
            })
            .to_string(),
            format!("{} · {agent_type} · {task_id}", ui_text(UiText::Queued)),
        ),
        task_promotion: Some(RuntimeTaskPromotion {
            task_id: task_id.clone(),
            input_watermark: Some(now),
            kind: nola_core::agent::TaskKind::Agent,
            state: nola_core::agent::TaskState::Queued,
            progress: Some(0.0),
            current_step: Some(ui_text(UiText::Queued).to_string()),
            lineage: nola_core::agent::TaskLineage {
                parent_task_id,
                resumed_from_task_id,
                child_session_id: Some(nola_core::agent::SessionId::from_string(child_session_id)),
                parent_call_id: Some(nola_core::agent::CallId::from_string(call_id.to_string())),
                permission_scope_id: Some(format!("task:{task_id}")),
                cancellation_scope_id: Some(format!("task:{task_id}")),
                ..nola_core::agent::TaskLineage::default()
            },
        }),
        task_update: None,
    }
}

fn spawn_agent_task_process(mut request: AgentTaskProcessRequest) -> Result<bool, String> {
    require_isolated_agent_model_channel(&request.config)?;
    let owner_task=NolaStore::open_default()?.runtime_task(&request.task_id)?.ok_or("Agent task disappeared before worker creation")?;
    crate::delegated_invocation::validate_task_workspace_execution(&request.config,&owner_task.metadata_json)?;
    let now = unix_epoch_millis();
    let configured_lease_duration_ms = std::env::var("NOLA_AGENT_TASK_LEASE_MS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(45_000)
        .clamp(1_000, 10 * 60 * 1_000);
    let attempt = if let Some(attempt) = request.runtime_task_attempt.take() {
        if attempt.task_id != request.task_id
            || attempt.state != "leased"
            || attempt.lease_expires_at_ms <= now
            || (attempt.worker_generation != crate::runtime_task_port::worker_generation()
                && request
                    .permission_resume
                    .as_ref()
                    .is_none_or(|resume| resume.grant != attempt))
        {
            return Err(
                "preclaimed Agent task attempt is stale or does not match this worker".to_string(),
            );
        }
        crate::runtime_task_port::register_runtime_task_attempt(attempt.clone());
        attempt
    } else {
        let (max_tasks, max_project_tasks) = runtime_task_concurrency_limits();
        let worker_id = format!(
            "cli-launch:{}:{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        );
        let ownership_token = uuid::Uuid::new_v4().simple().to_string();
        let claimed = NolaStore::open_default()?.claim_runtime_task_fair(
            &request.task_id,
            &worker_id,
            crate::runtime_task_port::worker_generation(),
            &ownership_token,
            now,
            now.saturating_add(configured_lease_duration_ms),
            max_tasks,
            max_project_tasks,
        )?;
        let Some(attempt) = claimed else {
            return Ok(false);
        };
        nola_runtime_driver::task_worker::adopt_task_worker_claim(
            &request.task_id, &worker_id, crate::runtime_task_port::worker_generation(),
            &ownership_token, now, attempt,
        )?.snapshot()?
    };
    let worker_id = attempt.worker_id.clone();
    let ownership_token = attempt.lease_token.clone();
    let lease_duration_ms = attempt
        .lease_expires_at_ms
        .saturating_sub(attempt.claimed_at_ms);
    let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt.clone());
    // Atomic parent preparation consumes time before process launch. Renew
    // only a still-live grant and preserve the duration originally admitted.
    crate::runtime_task_port::refresh_lease_before_execution(&lease, lease_duration_ms)?;
    if let Ok(mut handles) = task_handles().lock() {
        let current = handles
            .entry(request.task_id.clone())
            .or_insert_with(|| AgentTaskHandle {
                control_id: uuid::Uuid::new_v4().to_string(),
                lease: None,
                execution: None,
                permission_resume: request.permission_resume.clone(),
                cancellation: RuntimeExecutionControl::default(),
                child_session_id: request.config.session.session_id.clone().unwrap_or_default(),
            });
        if current
            .lease
            .as_ref()
            .is_some_and(|current| !current.same_owner(&lease))
        {
            *current = AgentTaskHandle {
                control_id: uuid::Uuid::new_v4().to_string(),
                lease: Some(lease.clone()),
                execution: None,
                permission_resume: request.permission_resume.clone(),
                cancellation: RuntimeExecutionControl::default(),
                child_session_id: request.config.session.session_id.clone().unwrap_or_default(),
            };
        } else {
            current.lease = Some(lease.clone());
        }
    }
    request.worker_id = worker_id;
    request.ownership_token = ownership_token.clone();
    request.runtime_task_attempt = Some(lease.snapshot()?);
    request.lease_duration_ms = lease_duration_ms;
    request.config.runtime.language =
        Some(crate::i18n::browser_locale_for(crate::i18n::current_language()).to_string());
    // Deterministic fault-injection window for real process-recovery E2E.
    // The task is already durably claimed, but no worker process exists yet.
    // Production runs never set this test-only variable.
    if let Some(delay_ms) = std::env::var("NOLA_TEST_AGENT_PRE_SPAWN_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
    {
        std::thread::sleep(Duration::from_millis(delay_ms.min(30_000)));
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("resolve Agent task worker executable failed: {error}"))?;
    let mut command = Command::new(executable);
    command
        .arg(AGENT_TASK_WORKER_ARG)
        .stdin(Stdio::piped())
        // The isolated worker emits opaque wake bytes on this private pipe
        // after committing task state. The parent never interprets payload
        // text; it re-reads the authoritative SQLite projection on wake.
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    detach_worker_session(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let retry_at_ms = unix_epoch_millis().saturating_add(1_000);
            if request.permission_resume.is_none() {
                let _ = NolaStore::open_default().and_then(|store| {
                    store.retry_runtime_task(
                        &lease,
                        retry_at_ms,
                        &format!("start isolated Agent task worker failed: {error}"),
                        unix_epoch_millis(),
                    )
                });
            }
            return Err(format!("start isolated Agent task worker failed: {error}"));
        }
    };
    let pid = child.id();
    let progress_ack = match crate::runtime_task_port::report_progress(
        &lease,
        None,
        None,
        None,
        None,
        BTreeMap::from([
            ("runner_pid".to_string(), Some(Value::from(pid))),
            (
                "worker_isolation".to_string(),
                Some(Value::String("process_group".into())),
            ),
        ]),
        unix_epoch_millis(),
    ) {
        Ok(ack) => ack,
        Err(error) => {
            terminate_child_process_group(&mut child);
            let _ = child.wait();
            release_agent_task_handle(&request.task_id, &lease);
            return Err(error);
        }
    };
    request.runtime_task_attempt = Some(progress_ack.attempt);
    let encoded = serde_json::to_vec(&request)
        .map_err(|error| format!("encode Agent task worker request failed: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "isolated Agent task worker has no stdin pipe".to_string())?;
    if let Err(error) = stdin.write_all(&encoded) {
        let _ = child.kill();
        let _ = child.wait();
        let retry_at_ms = unix_epoch_millis().saturating_add(1_000);
        if request.permission_resume.is_none() {
            let _ = NolaStore::open_default().and_then(|store| {
                store.retry_runtime_task(
                    &lease,
                    retry_at_ms,
                    &format!("send Agent task worker request failed: {error}"),
                    unix_epoch_millis(),
                )
            });
        }
        return Err(format!("send Agent task worker request failed: {error}"));
    }
    drop(stdin);

    let task_id = request.task_id.clone();
    let owner_session_id = request.owner_session_id.clone();
    let project_root = std::path::PathBuf::from(owner_task.project_root);
    let worker_attempt_id = lease.snapshot()?.id;
    let mut worker_wakes = child.stdout.take();
    std::thread::Builder::new()
        .name(format!("nola-worker-reaper-{task_id}"))
        .spawn(move || {
            if let Some(stream) = worker_wakes.as_mut() {
                let mut signal = [0_u8; 64];
                loop {
                    match stream.read(&mut signal) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => wake_task_waiters(),
                    }
                }
            }
            let status = child.wait();
            if !status.as_ref().is_ok_and(|status| status.success()) {
                let detail = status
                    .map(|status| format!("isolated Agent worker exited with {status}"))
                    .unwrap_or_else(|error| {
                        format!("wait for isolated Agent worker failed: {error}")
                    });
                let _ = append_task_event(
                    &task_id,
                    "worker_process_exited",
                    "suspended",
                    Some(&detail),
                    json!({"runner_pid": pid, "attempt_id": worker_attempt_id}),
                );
            }
            release_agent_task_handle(&task_id, &lease);
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            // A freed slot must advance the queue even while the parent is
            // blocked on its next model request rather than in the composer.
            // Reuse the parent's bound supervisor policy, not the child's
            // narrower execution configuration or a new scheduler authority.
            // Wake before recovery can wait on the parent's active Host lock.
            wake_task_waiters();
            reconcile_bound_supervised_tasks(Some(&owner_session_id), &project_root);
            wake_task_waiters();
            tools::wake_background_task_notification();
        })
        .map_err(|error| format!("start Agent worker reaper failed: {error}"))?;
    Ok(true)
}

fn task_command_reconciliation_input(
    error: &str,
    attempt: &nola_application::RuntimeTaskAttempt,
    now_ms: i64,
) -> Option<nola_application::SuspendRuntimeTaskForCommandReconciliation> {
    use nola_service_client::ApplicationServiceCommandError;
    let unresolved = ApplicationServiceCommandError::unresolved_from_boundary_message(error)?;
    let failure_code = match &unresolved {
        ApplicationServiceCommandError::OutcomeUnknown { code, .. } => code.clone(),
        ApplicationServiceCommandError::Pending { .. } => "command_outcome_pending".into(),
        ApplicationServiceCommandError::Transport {
            outcome_unknown: true,
            ..
        } => "command_transport_outcome_unknown".into(),
        _ => return None,
    };
    Some(
        nola_application::SuspendRuntimeTaskForCommandReconciliation {
            lease: nola_application::RuntimeTaskLeaseWitness::from(attempt),
            client_id: unresolved.client_id().into(),
            request_id: unresolved.request_id().into(),
            failure_code,
            progress_summary: Some(format!(
                "Application command {} awaits receipt reconciliation",
                unresolved.request_id()
            )),
            now_ms,
        },
    )
}

fn matches_original_command_reconciliation(
    pending: &nola_application::RuntimeTaskCommandReconciliation,
    expected: &nola_application::SuspendRuntimeTaskForCommandReconciliation,
) -> bool {
    pending.task_id == expected.lease.task_id
        && pending.attempt_id == expected.lease.attempt_id
        && pending.client_id == expected.client_id
        && pending.request_id == expected.request_id
        && pending.failure_code == expected.failure_code
        && pending.state == "pending"
        && pending.revision > 0
}

#[cfg(test)]
mod core_run_outcome_calling_tests {
    use crate::core_agent_runtime::CoreRunOutcome;
    use nola_core::agent::AgentTurnStatus;

    #[test]
    fn native_subagent_is_rejected_before_task_creation_or_http_downgrade() {
        let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
        assert!(super::require_isolated_agent_model_channel(&config).is_ok());
        config.runtime.native_model_route = Some(serde_json::from_value(serde_json::json!({
            "adapter":"mlx", "adapter_contract_revision":1, "model_id":"native-model",
            "profile_ref":{"profile_id":"native-model", "admitted_revision":1}
        })).unwrap());
        assert!(super::require_isolated_agent_model_channel(&config).unwrap_err().contains("authenticated Mac control channel"));
        assert!(super::require_isolated_agent_model_channel(&config).is_err());
        assert!(config.runtime.native_model_route.is_some());
    }

    #[test]
    fn pending_core_pump_cannot_finish_background_task() {
        for turn_status in [
            AgentTurnStatus::Idle,
            AgentTurnStatus::AwaitingPermission,
            AgentTurnStatus::AwaitingInput,
            AgentTurnStatus::AwaitingModel,
            AgentTurnStatus::AwaitingTools,
            AgentTurnStatus::Compacting,
            AgentTurnStatus::Cancelling,
        ] {
            let outcome = CoreRunOutcome { turn_status, run_result: None, goal_signal: None };
            assert!(!outcome.is_terminal());
        }
        for turn_status in [
            AgentTurnStatus::Completed,
            AgentTurnStatus::Cancelled,
            AgentTurnStatus::Failed,
        ] {
            let outcome = CoreRunOutcome { turn_status, run_result: None, goal_signal: None };
            assert!(outcome.is_terminal());
        }
    }

    #[test]
    fn uncertain_command_keeps_the_original_attempt_and_receipt_identity() {
        use nola_service_client::ApplicationServiceCommandError;
        let attempt = nola_application::RuntimeTaskAttempt {
            id: "original-attempt".into(),
            task_id: "task".into(),
            attempt_number: 3,
            worker_id: "original-worker".into(),
            worker_generation: "original-generation".into(),
            lease_token: "original-token".into(),
            lease_expires_at_ms: 60_000,
            state: "leased".into(),
            revision: 7,
            claimed_at_ms: 10,
        };
        for error in [
            ApplicationServiceCommandError::OutcomeUnknown {
                client_id: "original-client".into(),
                request_id: "original-request".into(),
                code: "command_transaction_commit_outcome_unknown".into(),
                message: "fixture acknowledgement lost".into(),
                reopen_required: true,
            },
            ApplicationServiceCommandError::Pending {
                client_id: "original-client".into(),
                request_id: "original-request".into(),
            },
            ApplicationServiceCommandError::Transport {
                client_id: "original-client".into(),
                request_id: "original-request".into(),
                message: "response disconnected".into(),
                outcome_unknown: true,
            },
        ] {
            let input = super::task_command_reconciliation_input(
                &format!("Agent Core failed: {error}"),
                &attempt,
                20,
            )
            .unwrap();
            assert_eq!(
                input.lease,
                nola_application::RuntimeTaskLeaseWitness::from(&attempt)
            );
            assert_eq!(input.client_id, "original-client");
            assert_eq!(input.request_id, "original-request");
        }
        let before_send = ApplicationServiceCommandError::Transport {
            client_id: "original-client".into(),
            request_id: "original-request".into(),
            message: "connection unavailable before send".into(),
            outcome_unknown: false,
        };
        assert!(
            super::task_command_reconciliation_input(&before_send.to_string(), &attempt, 20)
                .is_none()
        );
        assert!(super::task_command_reconciliation_input(
            "application_rejected: fixture business failure",
            &attempt,
            20
        )
        .is_none());

        let expected = super::task_command_reconciliation_input(
            &ApplicationServiceCommandError::Pending {
                client_id: "original-client".into(),
                request_id: "original-request".into(),
            }
            .to_string(),
            &attempt,
            20,
        )
        .unwrap();
        let pending = nola_application::RuntimeTaskCommandReconciliation {
            task_id: attempt.task_id.clone(),
            attempt_id: attempt.id.clone(),
            client_id: expected.client_id.clone(),
            request_id: expected.request_id.clone(),
            failure_code: expected.failure_code.clone(),
            state: "pending".into(),
            revision: 1,
            awaiting_at_ms: 20,
        };
        assert!(super::matches_original_command_reconciliation(
            &pending, &expected
        ));
        for (task, attempt_id, client, request) in [
            (
                "foreign-task",
                "original-attempt",
                "original-client",
                "original-request",
            ),
            (
                "task",
                "successor-attempt",
                "original-client",
                "original-request",
            ),
            (
                "task",
                "original-attempt",
                "foreign-client",
                "original-request",
            ),
            (
                "task",
                "original-attempt",
                "original-client",
                "successor-request",
            ),
        ] {
            let mut foreign = pending.clone();
            foreign.task_id = task.into();
            foreign.attempt_id = attempt_id.into();
            foreign.client_id = client.into();
            foreign.request_id = request.into();
            assert!(!super::matches_original_command_reconciliation(
                &foreign, &expected
            ));
        }
    }
}

#[cfg(test)]
mod bound_worker_authority_tests {
    use super::*;

    #[test]
    fn blocked_agent_observer_renews_then_late_events_cannot_touch_its_successor() {
        let _environment_lock = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("observer.db");
        struct Environment(Option<std::ffi::OsString>);
        impl Drop for Environment { fn drop(&mut self) {
            if let Some(previous)=self.0.take() { std::env::set_var("NOLA_DB_PATH",previous); }
            else { std::env::remove_var("NOLA_DB_PATH"); }
        }}
        let _environment = Environment(std::env::var_os("NOLA_DB_PATH"));
        std::env::set_var("NOLA_DB_PATH",&database);
        let store=NolaStore::open(&database).unwrap();
        let now=unix_epoch_millis();
        let task=DbBackgroundTask {
        process_sandbox: None,id:"blocked-agent-observer".into(),
            terminal: None,
            project_root:root.path().to_string_lossy().into_owned(),command:"not-executed".into(),
            cwd:root.path().to_string_lossy().into_owned(),pid:0,
            stdout_path:root.path().join("stdout").to_string_lossy().into_owned(),
            stderr_path:root.path().join("stderr").to_string_lossy().into_owned(),
            status_path:root.path().join("status").to_string_lossy().into_owned(),
            started_at_ms:now,owner_session_id:None,output_contract_json:None,sop_invocation:None};
        store.save_background_task(&task).unwrap();
        let attempt=store.claim_runtime_task_fair(&task.id,"observer-worker",
            crate::runtime_task_port::worker_generation(),"isolated-observer-token",now,now+1_000,1,1)
            .unwrap().unwrap();
        let lease=crate::runtime_task_port::register_runtime_task_attempt(attempt.clone());
        let execution=crate::runtime_task_port::RuntimeTaskExecutionAuthority::new(lease.clone());
        let cancellation=RuntimeExecutionControl::default();
        let heartbeat=AgentTaskLeaseHeartbeat::start(&task.id,"owner",&execution,1_000,&cancellation).unwrap();
        let (entered,observing)=std::sync::mpsc::channel();
        let (release,blocked)=std::sync::mpsc::channel();
        let observation_authority=execution.clone();
        let original_cancellation=cancellation.clone();
        let observed_task=task.id.clone();
        let observation=std::thread::spawn(move || with_agent_task_observation_lease(&observation_authority,|lease| {
            entered.send(lease.snapshot().unwrap().id).unwrap();
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            let output=Arc::new(Mutex::new(String::new()));
            let bytes=std::sync::atomic::AtomicU64::new(0);
            let failure=Arc::new(Mutex::new(None)); let mut journal=None;
            observe_agent_event(&observed_task,lease,&output,&bytes,
                &json!({"type":"agent_trace","trace":{"kind":"tool.started","payload":{"tool_name":"late-tool"}}}),
                4096,&failure,&original_cancellation,&mut journal);
            // A late journal error is delivered to the old callback's own
            // cancellation control, never a task-ID lookup of its successor.
            observe_agent_event(&observed_task,lease,&output,&bytes,
                &json!({"type":"assistant_journal_committed_v1"}),
                4096,&failure,&original_cancellation,&mut journal);
        }));
        assert_eq!(observing.recv_timeout(Duration::from_secs(2)).unwrap(),attempt.id);
        std::thread::sleep(Duration::from_millis(1_400));
        let renewed=lease.snapshot().unwrap();
        assert_eq!(renewed.id,attempt.id);
        assert_eq!(renewed.claimed_at_ms,attempt.claimed_at_ms);
        assert!(renewed.lease_expires_at_ms>attempt.lease_expires_at_ms);
        assert!(renewed.lease_expires_at_ms>unix_epoch_millis());
        assert!(!cancellation.is_requested());
        execution.revoke();
        let current=lease.snapshot().unwrap();
        let handoff_at=unix_epoch_millis();
        assert!(store.retry_runtime_task(&lease,
            handoff_at,"explicit observer handoff",handoff_at).unwrap());
        let successor=store.claim_runtime_task_fair(&task.id,"successor-worker","successor-generation",
            "isolated-successor-token",handoff_at+1,handoff_at+10_001,1,1).unwrap().unwrap();
        assert_ne!(successor.id,current.id);
        let successor_control=RuntimeExecutionControl::default();
        let successor_lease=crate::runtime_task_port::register_runtime_task_attempt(successor);
        task_handles().lock().unwrap().insert(task.id.clone(),AgentTaskHandle {
            control_id:"successor-observer-control".into(),lease:Some(successor_lease.clone()),
            execution:None,permission_resume:None,cancellation:successor_control.clone(),
            child_session_id:"unbound-observer-fixture".into(),
        });
        let before=store.runtime_task(&task.id).unwrap();
        let events=store.runtime_task_events(&task.id,100).unwrap();
        release.send(()).unwrap(); observation.join().unwrap();
        assert_eq!(store.runtime_task(&task.id).unwrap(),before);
        assert_eq!(store.runtime_task_events(&task.id,100).unwrap(),events);
        assert!(cancellation.is_requested());
        assert!(!successor_control.is_requested());
        let called=std::sync::atomic::AtomicBool::new(false);
        with_agent_task_observation_lease(&execution,|_| {called.store(true,std::sync::atomic::Ordering::SeqCst);});
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        drop(heartbeat);
        release_agent_task_handle(&task.id,&successor_lease);
        crate::runtime_task_port::release_runtime_task_handle(&successor_lease);
        crate::runtime_task_port::release_runtime_task_handle(&lease);
    }

    #[test]
    fn durable_shell_save_survives_launcher_ack_failure_with_original_files() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("nola.db");
        let store = NolaStore::open(&database).unwrap();
        let task = DbBackgroundTask {
        process_sandbox: None,
            terminal: None,
            id: "original-shell-task".into(),
            project_root: root.path().to_string_lossy().into_owned(),
            command: "fixture-never-executed".into(),
            cwd: root.path().to_string_lossy().into_owned(), pid: 0,
            stdout_path: root.path().join("stdout").to_string_lossy().into_owned(),
            stderr_path: root.path().join("stderr").to_string_lossy().into_owned(),
            status_path: root.path().join("status").to_string_lossy().into_owned(),
            started_at_ms: unix_epoch_millis(), owner_session_id: None,
            output_contract_json: None, sop_invocation: None,
        };
        for path in [&task.stdout_path, &task.stderr_path, &task.status_path] {
            std::fs::write(path, "retained evidence").unwrap();
        }
        let scheduling = nola_application::RuntimeTaskScheduling {
            priority: nola_application::RuntimeTaskPriority::Normal,
            queued_at_ms: task.started_at_ms,
            resource_budget: nola_application::RuntimeTaskResourceBudget::default(),
            output_bytes: 0, resource_usage: Default::default(), budget_exhaustion: None,
        };
        let error = queue_background_shell_task_with_launcher(&store, task.clone(), scheduling, |received| {
            assert_eq!(received.id, task.id);
            assert!(store.runtime_task(&task.id).unwrap().is_some(), "save precedes launch");
            Err("injected launch acknowledgment failure".into())
        }).unwrap_err();
        assert!(error.contains("injected"));
        drop(store);
        let reopened = NolaStore::open(&database).unwrap();
        assert_eq!(reopened.runtime_task(&task.id).unwrap().unwrap().id, task.id);
        for path in [&task.stdout_path, &task.stderr_path, &task.status_path] {
            assert_eq!(std::fs::read_to_string(path).unwrap(), "retained evidence");
        }
    }

    #[test]
    fn old_agent_cleanup_cannot_remove_successor_control() {
        let task_id = format!("control-aba-{}", uuid::Uuid::new_v4());
        let attempt = |id: &str, number: i64| nola_application::RuntimeTaskAttempt {
            id: id.into(),
            task_id: task_id.clone(),
            attempt_number: number,
            worker_id: format!("worker-{number}"),
            worker_generation: format!("generation-{number}"),
            lease_token: "reused-token".into(),
            lease_expires_at_ms: 60_000,
            state: "leased".into(),
            revision: 1,
            claimed_at_ms: number,
        };
        let original =
            crate::runtime_task_port::register_runtime_task_attempt(attempt("original", 1));
        let successor =
            crate::runtime_task_port::register_runtime_task_attempt(attempt("successor", 2));
        let cancellation = RuntimeExecutionControl::default();
        task_handles().lock().unwrap().insert(
            task_id.clone(),
            AgentTaskHandle {
                control_id: "successor-control".into(),
                lease: Some(successor.clone()),
                execution: None,
                permission_resume: None,
                cancellation: cancellation.clone(),
                child_session_id: "successor-child".into(),
            },
        );
        release_agent_task_handle(&task_id, &original);
        release_agent_task_control(&task_id, "old-control");
        assert!(task_handles()
            .lock()
            .unwrap()
            .get(&task_id)
            .is_some_and(|handle| handle.lease.as_ref().unwrap().same_owner(&successor)));
        assert!(!cancellation.is_requested());
        release_agent_task_handle(&task_id, &successor);
        assert!(!task_handles().lock().unwrap().contains_key(&task_id));
        crate::runtime_task_port::release_runtime_task_handle(&successor);
    }

    #[test]
    fn shell_worker_refuses_expired_or_replaced_transport_before_side_effects() {
        let _environment_lock = crate::store::test_env_lock();
        let root = std::env::temp_dir().join(format!("nola-bound-shell-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        struct TestEnvironment {
            previous_database: Option<std::ffi::OsString>,
            root: std::path::PathBuf,
        }
        impl Drop for TestEnvironment {
            fn drop(&mut self) {
                if let Some(previous) = self.previous_database.take() {
                    std::env::set_var("NOLA_DB_PATH", previous);
                } else {
                    std::env::remove_var("NOLA_DB_PATH");
                }
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
        let _environment = TestEnvironment {
            previous_database: std::env::var_os("NOLA_DB_PATH"),
            root: root.clone(),
        };
        let database = root.join("nola.db");
        std::env::set_var("NOLA_DB_PATH", &database);
        let store = NolaStore::open(&database).unwrap();
        for (replacement, fresh, fail_final_commit) in [
            (false, false, false), (true, false, false),
            (false, true, false), (false, true, true),
        ] {
            let now = unix_epoch_millis();
            let task_id = format!("stale-transport-{}", uuid::Uuid::new_v4());
            let marker = root.join(format!("{task_id}.side-effect"));
            let task = DbBackgroundTask {
        process_sandbox: None,
                terminal: None,
                id: task_id.clone(),
                project_root: root.to_string_lossy().into_owned(),
                command: if fresh {
                    // The command outlives the transported lease; the worker
                    // must renew that attempt before effects, not wait until
                    // its ordinary 20-second heartbeat interval.
                    format!("sleep 3; printf ran > '{}'", marker.display())
                } else {
                    format!("printf ran > '{}'", marker.display())
                },
                cwd: root.to_string_lossy().into_owned(),
                pid: 0,
                stdout_path: root
                    .join(format!("{task_id}.stdout"))
                    .to_string_lossy()
                    .into_owned(),
                stderr_path: root
                    .join(format!("{task_id}.stderr"))
                    .to_string_lossy()
                    .into_owned(),
                status_path: root
                    .join(format!("{task_id}.status"))
                    .to_string_lossy()
                    .into_owned(),
                started_at_ms: now - 100,
                owner_session_id: None,
                output_contract_json: None,
                sop_invocation: None,
            };
            store.save_background_task(&task).unwrap();
            let attempt = store
                .claim_runtime_task(
                    &task_id,
                    "old-worker",
                    "old-generation",
                    "reused-token",
                    now - 90,
                    if fresh { now + 2_000 } else { now - 10 },
                )
                .unwrap()
                .unwrap();
            let original = crate::runtime_task_port::register_runtime_task_attempt(attempt.clone());
            let successor = if replacement {
                assert!(store
                    .recover_expired_runtime_task(&task_id, "test takeover", now - 5, now - 5)
                    .unwrap());
                assert!(
                    store
                        .claim_runtime_task(
                            &task_id,
                            "new-worker",
                            "new-generation",
                            "reused-token",
                            now - 4,
                            now + 60_000
                        )
                        .is_err(),
                    "historical nonce reuse must be refused"
                );
                Some(
                    store
                        .claim_runtime_task(
                            &task_id,
                            "new-worker",
                            "new-generation",
                            "fresh-successor-token",
                            now - 4,
                            now + 60_000,
                        )
                        .unwrap()
                        .unwrap(),
                )
            } else {
                None
            };
            std::fs::write(&task.status_path, "successor-owned\n").unwrap();
            let mut request = ShellTaskProcessRequest {
                task: task.clone(),
                worker_id: attempt.worker_id.clone(),
                ownership_token: attempt.lease_token.clone(),
                runtime_task_attempt: attempt,
                lease_duration_ms: 60_000,
                database_path: database.clone(),
            };
            if fresh {
                request.runtime_task_attempt = mark_shell_worker_dispatch(&original).unwrap().attempt;
                if fail_final_commit {
                    let db = rusqlite::Connection::open(&database).unwrap();
                    db.execute_batch("CREATE TRIGGER reject_shell_finish BEFORE INSERT ON durable_task_events
                        WHEN NEW.kind='finished' BEGIN SELECT RAISE(ABORT,'injected final commit failure'); END;").unwrap();
                    assert_ne!(run_shell_task(request), 0);
                    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "ran");
                    assert_eq!(store.runtime_task(&task_id).unwrap().unwrap().status, "running");
                    assert_eq!(std::fs::read_to_string(&task.status_path).unwrap(), "successor-owned\n");
                    assert!(std::path::Path::new(&format!("{}.exit-evidence", task.status_path)).exists());
                    let receipts: i64 = db.query_row("SELECT COUNT(*) FROM runtime_task_terminal_receipts WHERE task_id=?1",
                        [&task_id], |row| row.get(0)).unwrap();
                    assert_eq!(receipts, 0);
                    assert!(!store.runtime_task_events(&task_id, 20).unwrap().iter().any(|event| event.kind == "finished"));
                    crate::runtime_task_port::release_runtime_task_handle(&original);
                    continue;
                }
                assert_eq!(
                    run_shell_task(request),
                    0,
                    "fresh original transport was refused"
                );
                assert_eq!(std::fs::read_to_string(&marker).unwrap(), "ran");
                let completed = store.runtime_task(&task_id).unwrap().unwrap();
                assert_eq!(completed.status, "completed");
                assert!(completed.ended_at_ms.is_some());
                let metadata: serde_json::Value = serde_json::from_str(&completed.metadata_json).unwrap();
                assert_eq!(metadata["shell_execution"]["exit_code"], json!(0));
                assert!(metadata["shell_execution"]["exit_evidence"].is_object());
                assert!(completed.scheduling().unwrap().unwrap().resource_usage.peak_memory_bytes > 0);
                assert_eq!(
                    std::fs::read_to_string(&task.status_path).unwrap(),
                    "exited(0)\n"
                );
                assert_eq!(
                    store
                        .runtime_task_events(&task_id, 20)
                        .unwrap()
                        .iter()
                        .filter(|event| event.kind == "finished" && event.status == "completed")
                        .count(),
                    1
                );
                assert!(crate::runtime_task_port::runtime_task_lease_state(&task_id).is_err());
                continue;
            }
            assert_ne!(run_shell_task(request.clone()), 0);
            assert!(!marker.exists(), "stale transport launched the command");
            assert_ne!(
                finish_shell_start_failure(&request, &original, "old-worker failure"),
                0
            );
            assert_eq!(
                std::fs::read_to_string(&task.status_path).unwrap(),
                "successor-owned\n"
            );
            let events_before = store.runtime_task_events(&task_id, 20).unwrap().len();
            let mut config = HostInvocationRequest::for_test().unwrap();
            config.tool_settings.project_root = root.clone();
            assert_eq!(
                run_agent_task(
                    &task_id,
                    "stale Agent transport",
                    "must not execute",
                    "fixture-owner",
                    now,
                    request.worker_id.clone(),
                    original.clone(),
                    60_000,
                    config,
                    RuntimeExecutionControl::default(),
                    None,
                ),
                2
            );
            assert_eq!(
                store.runtime_task_events(&task_id, 20).unwrap().len(),
                events_before,
                "stale Agent transport reached Core claim or execution"
            );
            let current = store.runtime_task(&task_id).unwrap().unwrap();
            assert_eq!(current.status, "running");
            assert!(!store
                .runtime_task_events(&task_id, 20)
                .unwrap()
                .iter()
                .any(|event| matches!(event.kind.as_str(), "finished" | "start_failed")));
            if let Some(successor) = successor {
                assert_eq!(
                    current.worker_id.as_deref(),
                    Some(successor.worker_id.as_str())
                );
                let handle = crate::runtime_task_port::register_runtime_task_attempt(successor);
                crate::runtime_task_port::release_runtime_task_handle(&handle);
            } else {
                crate::runtime_task_port::release_runtime_task_handle(&original);
            }
        }
    }
}

struct AgentTaskLeaseHeartbeat {
    stop: std::sync::mpsc::Sender<()>,
    handle: Option<std::thread::JoinHandle<()>>,
}

fn with_agent_task_observation_lease(
    execution: &crate::runtime_task_port::RuntimeTaskExecutionAuthority,
    observe: impl FnOnce(&crate::runtime_task_port::RuntimeTaskLeaseHandle),
) {
    if let Ok(Some(lease)) = execution.active_lease() {
        observe(&lease);
    }
}

impl AgentTaskLeaseHeartbeat {
    fn start(
        task_id: &str,
        _owner: &str,
        execution: &crate::runtime_task_port::RuntimeTaskExecutionAuthority,
        duration_ms: i64,
        cancellation: &RuntimeExecutionControl,
    ) -> Result<Self, String> {
        execution
            .with_renewal(|lease| {
                crate::runtime_task_port::refresh_lease_before_execution(lease, duration_ms)
            })?
            .ok_or("task grant is not held during parent confirmation")?;
        let (stop, receiver) = std::sync::mpsc::channel();
        let task_id = task_id.to_string();
        let execution = execution.clone();
        let cancellation = cancellation.clone();
        let interval =
            Duration::from_millis(u64::try_from((duration_ms / 3).clamp(1, 10_000)).unwrap());
        let handle = std::thread::Builder::new()
            .name(format!("nola-heartbeat-{task_id}"))
            .spawn(move || loop {
                match receiver.recv_timeout(interval) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
                let renewed_attempt = execution.active_lease().ok().flatten()
                    .and_then(|lease| lease.snapshot().ok()).map(|attempt| attempt.id);
                match execution.with_renewal(|lease| {
                    crate::runtime_task_port::refresh_lease_before_execution(lease, duration_ms)
                }) {
                    Ok(Some(())) => {}
                    Ok(None) => continue,
                    Err(_) => {
                        // Commit can retire the Task just before its local
                        // control callback revokes renewal. Exact durable
                        // retirement is not cancellation of unrelated effects.
                        let retired = renewed_attempt.is_some_and(|id|
                            NolaStore::open_default().ok().and_then(|store|
                                store.runtime_task_attempt_retired_for_user_input(&task_id,&id).ok()).unwrap_or(false));
                        if retired { execution.revoke(); break; }
                        cancellation.request();
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for AgentTaskLeaseHeartbeat {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn prepare_owned_task_submission_boundary(config: &mut HostInvocationRequest) -> Result<(), String> {
    config.session.expected_configuration_revision = None;
    config.session.expected_parent_message_id = None;
    let child = config.session.session_id.as_deref().ok_or("Task submission requires its child owner")?;
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    if application.active_run(child)?.is_some() {
        // Exact active-Run recovery uses its immutable invocation, not a new
        // caller version or an independently admitted user submission.
        return Ok(());
    }
    let source: Option<nola_application::ConversationTailSnapshot> = application.execute(
        nola_application::ApplicationCommand::LoadConversationTailSnapshotV1 {
            conversation_id: child.into(), message_limit: 1, part_text_limit: 1, part_json_limit: 1,
        },
    )?;
    if let Some(source) = source {
        if source.conversation.id != child { return Err("Task preparation source owner mismatch".into()); }
        // This is the lease-owning Task caller's preparation read, before Host
        // construction/dispatch. It does not repair another caller's input.
        config.session.expected_configuration_revision = Some(source.conversation.configuration_revision);
        config.session.expected_parent_message_id = source.conversation.messages.last().map(|message| message.id.clone());
    }
    Ok(())
}

struct CliTaskChildAuthority;
impl nola_runtime_driver::task_worker::TaskChildAuthorityPort for CliTaskChildAuthority {
    fn load_binding(&mut self, task_id: &str) -> Result<Option<nola_domain::task_execution::RuntimeTaskRunBinding>, String> {
        crate::runtime_task_port::run_binding(task_id)
    }
    fn reconcile_bound_terminal(&mut self, task_id: &str) -> Result<bool, String> {
        crate::runtime_task_port::reconcile_bound_run_terminal(task_id, unix_epoch_millis())
    }
}

struct CliTaskChildRunPort<'a> {
    config: &'a HostInvocationRequest,
    prompt: &'a str,
    emitter: &'a mut dyn crate::runtime_output::RuntimeOutputPort,
    cancellation: RuntimeExecutionControl,
}
impl nola_runtime_driver::task_worker::TaskChildAuthorityPort for CliTaskChildRunPort<'_> {
    fn load_binding(&mut self, task_id: &str) -> Result<Option<nola_domain::task_execution::RuntimeTaskRunBinding>, String> {
        crate::runtime_task_port::run_binding(task_id)
    }
    fn reconcile_bound_terminal(&mut self, task_id: &str) -> Result<bool, String> {
        crate::runtime_task_port::reconcile_bound_run_terminal(task_id, unix_epoch_millis())
    }
}
impl nola_runtime_driver::task_worker::TaskChildRunPort for CliTaskChildRunPort<'_> {
    type Outcome = crate::core_agent_runtime::CoreRunOutcome;
    fn begin_and_execute(&mut self, authority: &crate::runtime_task_port::RuntimeTaskExecutionAuthority) -> Result<Self::Outcome, String> {
        let mut config = self.config.clone();
        config.session.host_task_execution = Some(authority.clone());
        // Existing admission selects BeginTaskAssistantTurn; its committed
        // HostRequests then run through the shared CommittedRuntimeDriver.
        run_agent_core_with_cancellation(&config, self.prompt, self.emitter, self.cancellation.clone())
    }
    fn resume_and_execute(&mut self, run_id: &str, authority: &crate::runtime_task_port::RuntimeTaskExecutionAuthority) -> Result<Self::Outcome, String> {
        let mut config = self.config.clone();
        config.session.host_task_execution = Some(authority.clone());
        crate::core_agent_runtime::resume_agent_core_expected_with_cancellation(
            &config, self.emitter, self.cancellation.clone(), run_id,
        )?.ok_or_else(|| "bound task Run is not active; reconcile its canonical seal rather than starting a new Turn".into())
    }
}

fn run_agent_task(
    task_id: &str,
    title: &str,
    prompt: &str,
    owner_session_id: &str,
    input_watermark: i64,
    worker_id: String,
    lease: crate::runtime_task_port::RuntimeTaskLeaseHandle,
    lease_duration_ms: i64,
    mut config: HostInvocationRequest,
    cancellation: RuntimeExecutionControl,
    permission_resume: Option<ColdPermissionResume>,
) -> i32 {
    let child_identity = nola_runtime_driver::task_worker::TaskChildIdentity {
        task_id: task_id.into(), owner_conversation_id: owner_session_id.into(),
        child_conversation_id: config.session.session_id.clone().unwrap_or_default(), input_watermark,
    };
    match nola_runtime_driver::task_worker::reconcile_task_child(&child_identity, &mut CliTaskChildAuthority) {
        Ok(true) => {
            release_agent_task_handle(task_id, &lease);
            wake_task_waiters();
            tools::wake_background_task_notification();
            return 0;
        }
        Err(error) => {
            crate::runtime_task_port::task_recovery_diagnostic(task_id, "child_binding_reconciliation", error);
            release_agent_task_handle(task_id, &lease);
            return 2;
        }
        Ok(false) => {}
    }
    let record=NolaStore::open_default().and_then(|store| store.runtime_task(task_id));
    if let Ok(Some(task))=&record {
        if let Err(error)=crate::delegated_invocation::validate_task_workspace_execution(&config,&task.metadata_json) {
            crate::runtime_task_port::task_recovery_diagnostic(task_id,"task_workspace_unavailable",error);
            release_agent_task_handle(task_id,&lease);
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            return 2;
        }
    }
    match require_task_execution_scheduling(record) {
        Ok(_) => {},
        Err(error) => {
            crate::runtime_task_port::task_recovery_diagnostic(task_id, "scheduling_admission", error);
            release_agent_task_handle(task_id, &lease);
            // The control handle and lease registry are separate process-local
            // resources. Release only this exact registration, never a newer
            // worker that may have replaced it while recovery was reading.
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            return 2;
        }
    };
    let execution = crate::runtime_task_port::RuntimeTaskExecutionAuthority::new(lease.clone());
    let heartbeat = match AgentTaskLeaseHeartbeat::start(
        task_id,
        owner_session_id,
        &execution,
        lease_duration_ms,
        &cancellation,
    ) {
        Ok(heartbeat) => heartbeat,
        Err(error) => {
            crate::runtime_task_port::task_recovery_diagnostic(task_id, "heartbeat_start", error);
            release_agent_task_handle(task_id, &lease);
            return 2;
        }
    };
    config.session.permission_scope_id = Some(format!("host:task:{task_id}"));
    let now = unix_epoch_millis();
    let admission = crate::runtime_task_port::report_progress(
        &lease,
        Some(ui_text(UiText::Running).to_string()),
        None,
        None,
        None,
        BTreeMap::new(),
        now,
    ).and_then(|ack| {
        let now = unix_epoch_millis();
        Ok((require_task_execution_budget(&ack.task, now)?, remaining_task_wall_budget(&ack.task, now)?))
    });
    let (scheduling, remaining_wall_budget) = match admission {
        Ok(admission) => admission,
        Err(error) => {
            crate::runtime_task_port::task_recovery_diagnostic(task_id, "initial_progress", error);
            cancellation.request();
            drop(heartbeat);
            let settled = settle_worker_budget_stop(task_id);
            release_agent_task_handle(task_id, &lease);
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            return if settled { 0 } else { 2 };
        }
    };
    let budget_started = Instant::now();
    let core_claim = if let Some(resume) = &permission_resume {
        validate_cold_permission_execution(
            owner_session_id,
            task_id,
            input_watermark,
            resume,
            &lease,
            &cancellation,
        )
    } else {
        validate_durable_task_execution(owner_session_id, task_id, input_watermark, &lease)
    }.and_then(|()| prepare_owned_task_submission_boundary(&mut config));
    if let Err(error) = core_claim {
        crate::runtime_task_port::task_recovery_diagnostic(task_id, "core_authority", error.clone());
        if permission_resume.is_some() {
            let _ = append_task_event(
                task_id,
                "permission_recovery_blocked",
                "running",
                Some(&error),
                json!({"safe_to_retry": false}),
            );
            release_agent_task_handle(task_id, &lease);
            return 2;
        }
        // The atomic grant cannot be split into a Runtime-only retry. Stop
        // renewing it; exact expiry recovery will retire both authorities.
        let _ = append_task_event(
            task_id,
            "core_claim_failed",
            "running",
            Some(&error),
            json!({"worker_id": worker_id, "safe_to_retry":false}),
        );
        release_agent_task_handle(task_id, &lease);
        wake_task_waiters();
        tools::wake_background_task_notification();
        return 2;
    }
    if let Err(error) = crate::runtime_task_port::refresh_lease_before_execution(&lease, lease_duration_ms) {
        crate::runtime_task_port::task_recovery_diagnostic(task_id, "lease_refresh", error);
        release_agent_task_handle(task_id, &lease);
        return 2;
    }
    if cancellation.is_requested() {
        release_agent_task_handle(task_id, &lease);
        return 2;
    }
    let _ = append_task_event(task_id, "started", "running", Some(title), json!({}));

    config.session.host_task_execution = Some(execution.clone());
    if let Ok(mut handles) = task_handles().lock() {
        let Some(handle) = handles.get_mut(task_id).filter(|handle| {
            handle
                .lease
                .as_ref()
                .is_some_and(|current| current.same_owner(&lease))
        }) else {
            cancellation.request();
            return 2;
        };
        handle.execution = Some(execution.clone());
    } else {
        cancellation.request();
        return 2;
    }

    let max_output_bytes = scheduling
        .resource_budget.max_output_bytes
        .unwrap_or(DEFAULT_TASK_OUTPUT_BYTES);
    let max_wall_time_ms = scheduling.resource_budget.max_wall_time_ms;
    let max_cpu_time_ms = scheduling.resource_budget.max_cpu_time_ms;
    let max_memory_bytes = scheduling.resource_budget.max_memory_bytes;
    let max_processes = scheduling.resource_budget.max_processes;
    let budget_failure = Arc::new(Mutex::new(None::<String>));
    let budget_persistence_error = Arc::new(Mutex::new(None::<String>));
    let resource_usage = Arc::new(Mutex::new(ProcessGroupUsage::default()));
    let (budget_stop, budget_receiver) = std::sync::mpsc::channel::<()>();
    let budget_cancellation = cancellation.clone();
    let budget_failure_watchdog = Arc::clone(&budget_failure);
    let resource_usage_watchdog = Arc::clone(&resource_usage);
    let budget_task_id = task_id.to_string();
    let budget_execution = execution.clone();
    let persistence_error_watchdog = Arc::clone(&budget_persistence_error);
    let budget_handle = match std::thread::Builder::new()
        .name(format!("nola-budget-{task_id}"))
        .spawn(move || {
            loop {
                match budget_receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
                let externally_cancelled = NolaStore::open_default()
                    .ok()
                    .and_then(|store| store.runtime_task(&budget_task_id).ok().flatten())
                    .is_some_and(|task| task.cancel_requested);
                if externally_cancelled {
                    budget_cancellation.request();
                    terminate_process_tree_descendants();
                    break;
                }
                let failure = if remaining_wall_budget.is_some_and(|remaining| budget_started.elapsed() >= remaining) {
                    Some(format!(
                        "Task exceeded its wall-clock budget of {} ms", max_wall_time_ms.unwrap_or_default()
                    ))
                } else if max_cpu_time_ms.is_some()
                    || max_memory_bytes.is_some()
                    || max_processes.is_some()
                {
                    match current_process_tree_usage() {
                        Ok(observed) => {
                            if let Ok(mut usage) = resource_usage_watchdog.lock() {
                                usage.cpu_time_ms = usage.cpu_time_ms.max(observed.cpu_time_ms);
                                usage.memory_bytes = usage.memory_bytes.max(observed.memory_bytes);
                                usage.processes = usage.processes.max(observed.processes);
                                usage.pids = observed.pids.clone();
                            }
                            if max_cpu_time_ms.is_some_and(|limit| observed.cpu_time_ms > limit) {
                                Some(format!(
                                    "Task exceeded its CPU-time budget of {} ms (observed {} ms)",
                                    max_cpu_time_ms.unwrap_or_default(),
                                    observed.cpu_time_ms
                                ))
                            } else if max_memory_bytes
                                .is_some_and(|limit| observed.memory_bytes > limit)
                            {
                                Some(format!(
                                    "Task exceeded its memory budget of {} bytes (observed {} bytes)",
                                    max_memory_bytes.unwrap_or_default(),
                                    observed.memory_bytes
                                ))
                            } else if max_processes
                                .is_some_and(|limit| observed.processes > limit)
                            {
                                Some(format!(
                                    "Task exceeded its process-count budget of {} (observed {})",
                                    max_processes.unwrap_or_default(),
                                    observed.processes
                                ))
                            } else {
                                None
                            }
                        }
                        Err(error) => Some(format!(
                            "Task resource monitor failed before it could enforce the configured budget: {error}"
                        )),
                    }
                } else {
                    None
                };
                if let Some(failure_message) = failure {
                    // Commit the actual measurements before cancelling the
                    // child. Its cancelled seal is not the Task's failure cause.
                    let usage = resource_usage_watchdog.lock().ok().filter(|usage| !usage.pids.is_empty()).map(|usage|
                        nola_application::RuntimeTaskResourceUsage { cpu_time_ms: usage.cpu_time_ms,
                            peak_memory_bytes: usage.memory_bytes, peak_processes: usage.processes });
                    if let Err(error) = budget_execution.with_active(|lease| {
                        crate::runtime_task_port::report_progress(lease, None, None, None, usage,
                            BTreeMap::new(), unix_epoch_millis()).map(|_| ())
                    }) {
                        if let Ok(mut pending) = persistence_error_watchdog.lock() { *pending = Some(error); }
                    }
                    if let Ok(mut failure) = budget_failure_watchdog.lock() {
                        if failure.is_none() {
                            *failure = Some(failure_message);
                        }
                    }
                    budget_cancellation.request();
                    terminate_process_tree_descendants();
                    break;
                }
            }
        }) {
        Ok(handle) => Some(handle),
        Err(error) => {
            crate::runtime_task_port::task_recovery_diagnostic(task_id, "budget_monitor_start", error.to_string());
            cancellation.request();
            release_agent_task_handle(task_id, &lease);
            crate::runtime_task_port::release_runtime_task_handle(&lease);
            return 2;
        }
    };
    let result = Arc::new(Mutex::new(String::new()));
    let result_sink = Arc::clone(&result);
    let observed_output_bytes = Arc::new(std::sync::atomic::AtomicU64::new(
        scheduling.output_bytes,
    ));
    let observed_output_sink = Arc::clone(&observed_output_bytes);
    let output_budget_failure = Arc::clone(&budget_failure);
    let output_budget_cancellation = cancellation.clone();
    let progress_task_id = task_id.to_string();
    let progress_execution = execution.clone();
    let mut source_journal = None;
    let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(move |event| {
        // Journal reads can span multiple Service requests. Freeze only the
        // exact observation lease; never hold the authority mutex across that
        // read and starve the independent heartbeat. Store still fences every
        // progress mutation against this original lease, not a successor.
        with_agent_task_observation_lease(&progress_execution, |lease| {
            observe_agent_event(
                &progress_task_id,
                lease,
                &result_sink,
                &observed_output_sink,
                &event,
                max_output_bytes,
                &output_budget_failure,
                &output_budget_cancellation,
                &mut source_journal,
            );
        });
    });
    if let Err(error) = check_task_effect_admission(&lease, budget_started, remaining_wall_budget) {
        crate::runtime_task_port::task_recovery_diagnostic(task_id, "effect_admission", error);
        cancellation.request();
        let _ = budget_stop.send(());
        if let Some(handle) = budget_handle {
            let _ = handle.join();
        }
        drop(heartbeat);
        let settled = settle_worker_budget_stop(task_id);
        release_agent_task_handle(task_id, &lease);
        crate::runtime_task_port::release_runtime_task_handle(&lease);
        return if settled { 0 } else { 2 };
    }
    let expected_permission_run = permission_resume.as_ref().map(|resume| {
        serde_json::from_str::<Value>(&resume.suspension.continuation_json)
            .map_err(|error| error.to_string())?
            .get("run_id").and_then(Value::as_str).filter(|id| !id.is_empty())
            .map(str::to_owned).ok_or_else(|| "permission continuation has no exact Run".to_string())
    }).transpose();
    let mut outcome = expected_permission_run.and_then(|expected_run| {
        nola_runtime_driver::task_worker::execute_task_child(
            &child_identity, &execution, expected_run.as_deref(),
            &mut CliTaskChildRunPort { config: &config, prompt, emitter: &mut emitter,
                cancellation: cancellation.clone() },
        )
    });
    let result = result
        .lock()
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let _ = budget_stop.send(());
    if let Some(handle) = budget_handle {
        let _ = handle.join();
    }
    if let Some(error) = budget_persistence_error
        .lock()
        .ok()
        .and_then(|pending| pending.clone())
    {
        if nola_service_client::ApplicationServiceCommandError::unresolved_from_boundary_message(
            &error,
        )
        .is_some()
        {
            outcome = Err(error);
        }
    }
    drop(heartbeat);
    // Persist observed worker usage before a canonical terminal reconciliation
    // retires its lease. Recovery never invents these process observations.
    if let Ok(Some(active)) = execution.active_lease() {
        let usage = resource_usage
            .lock()
            .ok()
            .filter(|usage| !usage.pids.is_empty())
            .map(|usage| nola_application::RuntimeTaskResourceUsage {
                cpu_time_ms: usage.cpu_time_ms,
                peak_memory_bytes: usage.memory_bytes,
                peak_processes: usage.processes,
            });
        let mut metadata = std::collections::BTreeMap::new();
        if usage.is_some() {
            metadata.insert(
                "terminal_usage_observed_at_ms".into(),
                Some(json!(unix_epoch_millis())),
            );
        }
        let _ = crate::runtime_task_port::report_progress(
            &active,
            None,
            None,
            Some(observed_output_bytes.load(std::sync::atomic::Ordering::Relaxed)),
            usage,
            metadata,
            unix_epoch_millis(),
        );
    }
    // A terminal commit can race the initial recovery read. Its canonical
    // result wins over local cancellation, budgets, or "Run no longer active".
    // Unknown command receipts retain their separate boundary below.
    if outcome.as_ref().err().is_none_or(|error| {
        nola_service_client::ApplicationServiceCommandError::unresolved_from_boundary_message(error)
            .is_none()
    }) && matches!(
        nola_runtime_driver::task_worker::reconcile_task_child(&child_identity, &mut CliTaskChildAuthority),
        Ok(true)
    ) {
        release_agent_task_handle(task_id, &lease);
        wake_task_waiters();
        tools::wake_background_task_notification();
        return 0;
    }
    // Permission continuation replaces the attempt. After the pump and its
    // heartbeat have stopped, settle using this worker's activated authority,
    // never its initial lease or a task-ID lookup of another worker's lease.
    let lease = match execution.active_lease() {
        Ok(Some(current)) => current,
        Ok(None) | Err(_) => {
            release_agent_task_handle(task_id, &lease);
            wake_task_waiters();
            return 1;
        }
    };
    let ownership_token = match lease.snapshot() {
        Ok(attempt) => attempt.lease_token,
        Err(_) => {
            release_agent_task_handle(task_id, &lease);
            wake_task_waiters();
            return 1;
        }
    };
    let terminal_status;
    let terminal_error;
    let resource_failure = budget_failure.lock().ok().and_then(|value| value.clone());
    let observed_usage = resource_usage
        .lock()
        .ok()
        .filter(|usage| !usage.pids.is_empty())
        .map(|usage| usage.clone());
    let reported_usage = observed_usage.map(|usage| nola_application::RuntimeTaskResourceUsage {
        cpu_time_ms: usage.cpu_time_ms,
        peak_memory_bytes: usage.memory_bytes,
        peak_processes: usage.processes,
    });
    let _ = crate::runtime_task_port::report_progress(
        &lease,
        None,
        None,
        // Report observed output, not the UTF-8-truncated retained preview.
        // Application derives the typed budget exhaustion from this fact.
        Some(observed_output_bytes.load(std::sync::atomic::Ordering::Relaxed)),
        reported_usage,
        std::collections::BTreeMap::new(),
        unix_epoch_millis(),
    );
    // A local budget/cancellation signal cannot prove the outcome of an
    // already-sent Application command. Establish its receipt boundary before
    // interpreting any local terminal condition.
    if let Err(error) = &outcome {
        if nola_service_client::ApplicationServiceCommandError::unresolved_from_boundary_message(
            error,
        )
        .is_some()
        {
            let expected = lease.snapshot().ok().and_then(|attempt| {
                task_command_reconciliation_input(error, &attempt, unix_epoch_millis())
            });
            let mut suspended = lease.with_attempt(|attempt| {
                let input = task_command_reconciliation_input(error, attempt, unix_epoch_millis())
                    .ok_or_else(|| {
                        "unresolved command lost its typed receipt identity".to_string()
                    })?;
                crate::application_port::CliApplicationPort::persistent_default()?.execute::<Option<
                    nola_application::RuntimeTaskCommandReconciliation,
                >>(
                    nola_application::ApplicationCommand::RuntimeTask {
                        command:
                            nola_application::RuntimeTaskCommand::SuspendForCommandReconciliation {
                                input,
                            },
                    },
                )
            });
            if suspended.is_err() {
                // A lost acknowledgement of the suspension itself must be
                // reconciled by its canonical fact, without reissuing it or
                // borrowing a successor attempt's authority.
                if let Some(expected) = expected.as_ref() {
                    let pending = crate::application_port::CliApplicationPort::persistent_default()
                        .and_then(|mut port| port.execute::<Option<nola_application::RuntimeTaskCommandReconciliation>>(
                            nola_application::ApplicationCommand::RuntimeTask {
                                command: nola_application::RuntimeTaskCommand::PendingCommandReconciliation {
                                    task_id: expected.lease.task_id.clone(),
                                },
                            },
                        ));
                    if let Ok(Some(pending)) = pending {
                        if matches_original_command_reconciliation(&pending, expected) {
                            suspended = Ok(Some(pending));
                        }
                    }
                }
            }
            match suspended {
                Ok(Some(reconciliation)) => {
                    let _ = update_agent_core_task(
                        owner_session_id,
                        task_id,
                        nola_core::agent::TaskState::Suspended,
                        None,
                        Some("awaiting_command_reconciliation".into()),
                        None,
                        false,
                        &ownership_token,
                        None,
                        format!(
                            "command-reconciliation:{task_id}:{}",
                            reconciliation.request_id
                        ),
                    );
                }
                // A terminal receipt can appear between the client's lookup
                // and the atomic suspension check. It is not permission to
                // finish the task or start another turn.
                Ok(None) => {
                    let _ = append_task_event(
                        task_id,
                        "command_receipt_recheck_required",
                        "suspended",
                        Some(error),
                        json!({"safe_to_retry": false}),
                    );
                }
                Err(suspension_error) => {
                    let _ = append_task_event(
                        task_id,
                        "command_reconciliation_suspend_failed",
                        "suspended",
                        Some(error),
                        json!({"error": suspension_error, "safe_to_retry": false}),
                    );
                }
            }
            release_agent_task_handle(task_id, &lease);
            wake_task_waiters();
            return 1;
        }
    }
    if let Some(error) = resource_failure {
        terminal_status = "failed";
        terminal_error = Some(error);
    } else if cancellation.is_requested() {
        terminal_status = "cancelled";
        terminal_error = None;
    } else {
        match outcome {
            Ok(outcome) if !outcome.is_terminal() => {
                let summary = format!("Core Run remains pending ({:?})", outcome.turn_status);
                // The committed wait projection already persisted waiting_input
                // (or the permission resolver persisted waiting_permission).
                // Preserve its resolution flag: another client may have just
                // approved while this transport was unwinding.
                let _ = crate::runtime_task_port::report_progress(
                    &lease,
                    Some(summary.clone()),
                    None,
                    None,
                    None,
                    std::collections::BTreeMap::new(),
                    unix_epoch_millis(),
                );
                let _ = update_agent_core_task(
                    owner_session_id,
                    task_id,
                    nola_core::agent::TaskState::Waiting,
                    None,
                    Some(summary),
                    None,
                    false,
                    &ownership_token,
                    None,
                    format!("pending:{task_id}:{ownership_token}"),
                );
                release_agent_task_handle(task_id, &lease);
                wake_task_waiters();
                return 0;
            }
            Ok(outcome) => {
                // A stopped pump is not a Task result. The exact Run seal
                // reconciliation above is the only successful settlement path.
                // Retain recovery authority rather than inventing completion.
                let _ = append_task_event(task_id, "result_reconciliation_required", "suspended",
                    Some("Core stopped without an available canonical Run result"),
                    json!({"turn_status": format!("{:?}", outcome.turn_status)}));
                release_agent_task_handle(task_id, &lease);
                wake_task_waiters();
                return 2;
            }
            Err(error) => {
                let _ = append_task_event(
                    task_id, "worker_execution_failed", "suspended",
                    Some("Child execution stopped before canonical terminal reconciliation"),
                    json!({"error": crate::session::redact_sensitive_text(&error)}),
                );
                terminal_status = "failed";
                terminal_error = Some(error);
            }
        }
    }
    let result_summary = nonempty(
        result
            .chars()
            .take(RESULT_SUMMARY_LIMIT)
            .collect::<String>(),
    );
    let finished = crate::runtime_task_port::finish_detailed_with_event(
        &lease,
        terminal_status,
        Some(input_watermark),
        Some(terminal_status.to_string()),
        result_summary,
        terminal_error.clone(),
        None,
        true,
        "finished",
        terminal_status,
        Some(title.to_string()),
        json!({
            "result_bytes": result.len(),
            "error": terminal_error,
        })
        .to_string(),
        unix_epoch_millis(),
    );
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            let _ = append_task_event(
                task_id, "worker_settlement_failed", "suspended",
                Some("Task settlement was rejected; canonical recovery is required"),
                json!({"error": crate::session::redact_sensitive_text(&error)}),
            );
            release_agent_task_handle(task_id, &lease);
            wake_task_waiters();
            return 1;
        }
    };
    if finished {
        if let Err(error) = crate::runtime_task_port::enqueue_terminal_reconciliation(task_id) {
            let _ = append_task_event(
                task_id,
                "core_reconciliation_failed",
                "suspended",
                Some(&error),
                json!({"terminal_status": terminal_status}),
            );
        }
    } else {
        let _ = append_task_event(
            task_id,
            "ownership_lost",
            "suspended",
            Some("Task result was not committed because this worker no longer owns the lease"),
            json!({"worker_id": worker_id}),
        );
    }
    release_agent_task_handle(task_id, &lease);
    wake_task_waiters();
    recover_orphaned_agent_tasks(&config);
    if finished {
        0
    } else {
        1
    }
}

fn observe_agent_event(
    task_id: &str,
    lease: &crate::runtime_task_port::RuntimeTaskLeaseHandle,
    result: &Arc<Mutex<String>>,
    observed_output_bytes: &std::sync::atomic::AtomicU64,
    event: &Value,
    max_output_bytes: u64,
    budget_failure: &Arc<Mutex<Option<String>>>,
    cancellation: &RuntimeExecutionControl,
    source_journal: &mut Option<crate::source_journal::SourceRunJournal>,
) {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "assistant_journal_committed_v1" => {
            let observed = (|| -> Result<String, String> {
                let binding = crate::runtime_task_port::run_binding(task_id)?
                    .ok_or("task has no Run binding")?;
                let through: nola_domain::assistant_journal::AssistantJournalCursor =
                    serde_json::from_value(event["through"].clone())
                        .map_err(|error| error.to_string())?;
                if through.identity.conversation_id != binding.child_session_id
                    || through.identity.run_id != binding.run_id
                {
                    return Err("task journal wakeup belongs to another Run".into());
                }
                let mut port = crate::application_port::CliApplicationPort::persistent_default()?;
                if source_journal.is_none() {
                    let seal: Option<
                        nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
                    > = port.execute(
                        nola_application::ApplicationCommand::LocalAssistantRunTerminalSeal {
                            conversation_id: binding.child_session_id.clone(),
                            run_id: binding.run_id.clone(),
                        },
                    )?;
                    let root = if let Some(seal) = seal {
                        seal.identity.assistant_message_id
                    } else {
                        let active = port
                            .active_run(&binding.child_session_id)?
                            .ok_or("bound task Run is unavailable")?;
                        if active.run_id != binding.run_id {
                            return Err("task active Run changed".into());
                        }
                        active
                            .assistant_message_id
                            .ok_or("task Run has no assistant root")?
                    };
                    *source_journal =
                        Some(crate::source_journal::SourceRunJournal::new(
                            binding.child_session_id,
                            binding.run_id,
                            root,
                        ));
                }
                let reader = source_journal.as_mut().unwrap();
                reader.catch_up(&mut port)?;
                Ok(reader.public_text())
            })();
            match observed {
                Ok(text) => {
                    if let Ok(mut output) = result.lock() {
                        output.clear();
                        append_bounded_output(
                            &mut output,
                            &text,
                            observed_output_bytes,
                            max_output_bytes,
                            budget_failure,
                            cancellation,
                        );
                        let mut metadata = std::collections::BTreeMap::new();
                        metadata.insert("public_output_observed".into(), Some(json!(true)));
                        let _ = crate::runtime_task_port::report_progress(
                            lease,
                            None,
                            None,
                            Some(observed_output_bytes.load(std::sync::atomic::Ordering::Relaxed)),
                            None,
                            metadata,
                            unix_epoch_millis(),
                        );
                    }
                }
                Err(error) => {
                    if let Ok(mut failure) = budget_failure.lock() {
                        if failure.is_none() {
                            *failure = Some(format!("Task journal observation failed: {error}"));
                        }
                    }
                    cancellation.request();
                }
            }
        }
        "agent_trace" => {
            let trace_kind = event.pointer("/trace/kind").and_then(Value::as_str);
            if trace_kind == Some("external.session_started") {
                let external_session_id = event
                    .pointer("/trace/payload/external_session_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let Some(external_session_id) = external_session_id {
                    let mut metadata_patch = std::collections::BTreeMap::new();
                    metadata_patch.insert(
                        "external_session_id".to_string(),
                        Some(Value::String(external_session_id.to_string())),
                    );
                    metadata_patch.insert(
                        "runner_pid".to_string(),
                        Some(Value::from(std::process::id())),
                    );
                    let _ = crate::runtime_task_port::report_progress(
                        lease,
                        None,
                        None,
                        None,
                        None,
                        metadata_patch,
                        unix_epoch_millis(),
                    );
                }
            }
            if trace_kind == Some("tool.started") {
                let tool = event
                    .pointer("/trace/payload/tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let localized_tool = crate::i18n::localize_tool_display_name(tool);
                let summary = if current_language() == Language::ZhHans {
                    format!("{} {localized_tool}", ui_text(UiText::Using))
                } else {
                    format!("Using {localized_tool}")
                };
                let _ = crate::runtime_task_port::report_activity_now(
                    lease,summary,tool.to_string(),json!({"tool":tool}).to_string());
            }
        }
        "status" => {
            if let Some(message) = event.get("message").and_then(Value::as_str) {
                let message = message.to_string();
                let _ = crate::runtime_task_port::report_progress(
                    lease,
                    Some(message),
                    None,
                    None,
                    None,
                    std::collections::BTreeMap::new(),
                    unix_epoch_millis(),
                );
            }
        }
        _ => {}
    }
}

fn append_bounded_output(
    output: &mut String,
    delta: &str,
    observed_output_bytes: &std::sync::atomic::AtomicU64,
    max_output_bytes: u64,
    budget_failure: &Arc<Mutex<Option<String>>>,
    cancellation: &RuntimeExecutionControl,
) {
    observed_output_bytes.fetch_max(
        output.len().saturating_add(delta.len()) as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    let limit = usize::try_from(max_output_bytes).unwrap_or(usize::MAX);
    let remaining = limit.saturating_sub(output.len());
    if delta.len() <= remaining {
        output.push_str(delta);
        return;
    }
    let mut boundary = remaining.min(delta.len());
    while boundary > 0 && !delta.is_char_boundary(boundary) {
        boundary -= 1;
    }
    output.push_str(&delta[..boundary]);
    if let Ok(mut failure) = budget_failure.lock() {
        if failure.is_none() {
            *failure = Some(format!(
                "Task exceeded its output budget of {max_output_bytes} bytes"
            ));
        }
    }
    cancellation.request();
}
