// SPDX-License-Identifier: Apache-2.0
//! Real Subagent & Expert Delegation Runtime (`SubagentManager`):
//! Spawns actual child `AgentEngine` executions (foreground or background threads),
//! supports Condvar-signaled parallel `wait` (`any` / `all`), isolated Git worktrees,
//! stall watchdog timeouts, crash/orphan reconciliation, reactive parent inbox delivery,
//! and full custom `ToolRegistry` inheritance.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nola_store::Store;
use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, ExpertPeer, FrozenSystemBlock, HostCapabilities,
    ModelRouteConfig, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::catalog::ForceToolSet;
use crate::hooks::HookRunner;
use crate::loop_runner::{AgentEngine, AgentRunConfig, ModelCaller};
use crate::policy::EffectivePolicy;

const MAX_CONCURRENT_SUBAGENTS: usize = 8;
const MAX_RETAINED_FINISHED_SUBAGENTS: usize = 64;
const DEFAULT_SUBAGENT_TIMEOUT_SECS: u64 = 600;

/// Optional factory creating a child subagent's `ToolRegistry` for its workspace
/// (enabling full inheritance of MCP, memory, and module tools).
pub type SubagentToolFactory = Arc<dyn Fn(&Path) -> ToolRegistry + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitMode {
    Any,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Stopped,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTaskSnapshot {
    pub task_id: String,
    pub role: String,
    pub session_id: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub cancelling: bool,
    pub output: String,
    pub error: Option<String>,
    pub steps: usize,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// Optional extended options for spawning a subagent.
#[derive(Debug, Clone, Default)]
pub struct SubagentSpawnOptions {
    pub model_override: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub worktree: bool,
    pub timeout_secs: Option<u64>,
}

struct SubagentEntry {
    snapshot: Mutex<SubagentTaskSnapshot>,
    cancel: Arc<AtomicBool>,
    timed_out: Arc<AtomicBool>,
    inbox: Arc<Mutex<Vec<String>>>,
    repo_root: PathBuf,
    worktree_path: Option<PathBuf>,
    branch: Option<String>,
}

/// Manages real child subagent executions backed by `AgentEngine`.
pub struct SubagentManager {
    store: Store,
    caller: Arc<dyn ModelCaller>,
    base_route: Mutex<Option<ModelRouteConfig>>,
    base_policy: Mutex<Option<EffectivePolicy>>,
    base_hooks: Mutex<HookRunner>,
    parent_cancel: Mutex<Option<Arc<AtomicBool>>>,
    parent_inbox: Mutex<Option<Arc<Mutex<Vec<String>>>>>,
    tool_factory: Mutex<Option<SubagentToolFactory>>,
    tasks: Mutex<BTreeMap<String, Arc<SubagentEntry>>>,
    completion_signal: Arc<(Mutex<u64>, Condvar)>,
}

impl SubagentManager {
    pub fn new(store: Store, caller: Arc<dyn ModelCaller>) -> Self {
        Self {
            store,
            caller,
            base_route: Mutex::new(None),
            base_policy: Mutex::new(None),
            base_hooks: Mutex::new(HookRunner::default()),
            parent_cancel: Mutex::new(None),
            parent_inbox: Mutex::new(None),
            tool_factory: Mutex::new(None),
            tasks: Mutex::new(BTreeMap::new()),
            completion_signal: Arc::new((Mutex::new(0), Condvar::new())),
        }
    }

    /// Attach or replace the backing store used to persist durable subagent snapshots.
    pub fn with_store(mut self, store: Store) -> Self {
        self.store = store;
        self
    }

    /// Attach a parent inbox that receives structured completion notifications whenever any
    /// background subagent finishes, fails, or stops.
    pub fn with_parent_inbox(self, inbox: Arc<Mutex<Vec<String>>>) -> Self {
        *self.parent_inbox.lock().unwrap() = Some(inbox);
        self
    }

    /// Replace or clear the reactive parent inbox on a shared `SubagentManager`.
    pub fn set_parent_inbox(&self, inbox: Option<Arc<Mutex<Vec<String>>>>) {
        *self.parent_inbox.lock().unwrap() = inbox;
    }

    /// Attach a tool registry factory so spawned subagents inherit MCP/memory/module tools.
    pub fn with_tool_factory(self, factory: SubagentToolFactory) -> Self {
        *self.tool_factory.lock().unwrap() = Some(factory);
        self
    }

    /// Replace or clear the child tool registry factory on a shared `SubagentManager`.
    pub fn set_tool_factory(&self, factory: Option<SubagentToolFactory>) {
        *self.tool_factory.lock().unwrap() = factory;
    }

    /// Bind active parent route & policy before spawning child agents.
    pub fn configure_parent(&self, route: ModelRouteConfig, policy: EffectivePolicy) {
        *self.base_route.lock().unwrap() = Some(route);
        *self.base_policy.lock().unwrap() = Some(policy);
    }

    /// Bind parent hooks and cancellation signal so subagents inherit security hooks and abort on parent cancel.
    pub fn configure_runtime(&self, hooks: HookRunner, parent_cancel: Option<Arc<AtomicBool>>) {
        *self.base_hooks.lock().unwrap() = hooks;
        *self.parent_cancel.lock().unwrap() = parent_cancel;
    }

    /// Reconcile stale `"running"` `durable_task` records in SQLite left over from a prior
    /// process crash/restart by marking them `TaskStatus::Interrupted`.
    pub fn reconcile_orphaned_tasks(&self) -> usize {
        let active_ids: Vec<String> = self
            .tasks
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, e)| {
                (e.snapshot.lock().unwrap().status == TaskStatus::Running).then_some(id.clone())
            })
            .collect();
        let records = self
            .store
            .list_module_records::<SubagentTaskSnapshot>("durable_task")
            .unwrap_or_default();
        let mut reconciled = 0usize;
        for mut snap in records {
            if snap.status == TaskStatus::Running && !active_ids.contains(&snap.task_id) {
                snap.status = TaskStatus::Interrupted;
                snap.cancelling = false;
                snap.ended_at_ms.get_or_insert_with(now_ms);
                if snap.error.is_none() {
                    snap.error = Some("task interrupted after process restart or crash".into());
                }
                if self
                    .store
                    .put_module_record("durable_task", &snap.task_id, &snap)
                    .is_ok()
                {
                    reconciled += 1;
                }
            }
        }
        reconciled
    }

    pub fn spawn(
        self: &Arc<Self>,
        ctx: &ToolContext,
        role: String,
        system_prompt: String,
        task_prompt: String,
        background: bool,
        max_steps: usize,
    ) -> Result<SubagentTaskSnapshot, String> {
        self.spawn_with_full_options(
            ctx,
            role,
            system_prompt,
            task_prompt,
            background,
            max_steps,
            SubagentSpawnOptions::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_options(
        self: &Arc<Self>,
        ctx: &ToolContext,
        role: String,
        system_prompt: String,
        task_prompt: String,
        background: bool,
        max_steps: usize,
        model_override: Option<String>,
        allowed_tools: Option<Vec<String>>,
    ) -> Result<SubagentTaskSnapshot, String> {
        self.spawn_with_full_options(
            ctx,
            role,
            system_prompt,
            task_prompt,
            background,
            max_steps,
            SubagentSpawnOptions {
                model_override,
                allowed_tools,
                worktree: false,
                timeout_secs: None,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_full_options(
        self: &Arc<Self>,
        ctx: &ToolContext,
        role: String,
        system_prompt: String,
        task_prompt: String,
        background: bool,
        max_steps: usize,
        options: SubagentSpawnOptions,
    ) -> Result<SubagentTaskSnapshot, String> {
        let mut route = self
            .base_route
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "parent model route is not configured for subagent spawning".to_string())?;
        if let Some(m) = options.model_override.filter(|s| !s.trim().is_empty()) {
            route.model = m;
        }
        let policy = self
            .base_policy
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(EffectivePolicy {
                profile: ctx.permission_profile,
                workspace_trusted: false,
                network_allowed: ctx.network_allowed,
                rules: Vec::new(),
                plan_mode: false,
            });
        let hooks = self.base_hooks.lock().unwrap().clone();
        let parent_cancel = self.parent_cancel.lock().unwrap().clone();
        let parent_inbox = self.parent_inbox.lock().unwrap().clone();
        let tool_factory = self.tool_factory.lock().unwrap().clone();

        let active_count = self
            .tasks
            .lock()
            .unwrap()
            .values()
            .filter(|e| e.snapshot.lock().unwrap().status == TaskStatus::Running)
            .count();
        if active_count >= MAX_CONCURRENT_SUBAGENTS {
            return Err(format!(
                "maximum {MAX_CONCURRENT_SUBAGENTS} concurrent subagents reached"
            ));
        }

        let task_id = format!("task_{}", &new_id()[..8]);
        let session_id = format!("sub_{task_id}");

        // Optional isolated Git worktree creation when workspace is a git repo
        let (worktree_path, branch) = if options.worktree && is_git_repo(&ctx.workspace_root) {
            let (wt_path, br) = create_task_worktree(&ctx.workspace_root, &task_id)?;
            (Some(wt_path), Some(br))
        } else {
            (None, None)
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let timed_out = Arc::new(AtomicBool::new(false));
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let initial_snap = SubagentTaskSnapshot {
            task_id: task_id.clone(),
            role: role.clone(),
            session_id: session_id.clone(),
            status: TaskStatus::Running,
            cancelling: false,
            output: String::new(),
            error: None,
            steps: 0,
            started_at_ms: now_ms(),
            ended_at_ms: None,
            worktree_path: worktree_path.as_ref().map(|p| p.display().to_string()),
            branch: branch.clone(),
        };
        let _ = self
            .store
            .put_module_record("durable_task", &task_id, &initial_snap);
        let entry = Arc::new(SubagentEntry {
            snapshot: Mutex::new(initial_snap),
            cancel: Arc::clone(&cancel),
            timed_out: Arc::clone(&timed_out),
            inbox: Arc::clone(&inbox),
            repo_root: ctx.workspace_root.clone(),
            worktree_path: worktree_path.clone(),
            branch: branch.clone(),
        });
        {
            let mut tasks_guard = self.tasks.lock().unwrap();
            tasks_guard.insert(task_id.clone(), Arc::clone(&entry));
            let mut finished: Vec<(i64, String)> = tasks_guard
                .iter()
                .filter_map(|(id, e)| {
                    let s = e.snapshot.lock().unwrap();
                    (s.status != TaskStatus::Running)
                        .then_some((s.ended_at_ms.unwrap_or(s.started_at_ms), id.clone()))
                })
                .collect();
            if finished.len() > MAX_RETAINED_FINISHED_SUBAGENTS {
                finished.sort_by_key(|(ts, _)| *ts);
                let remove_count = finished.len() - MAX_RETAINED_FINISHED_SUBAGENTS;
                for (_, old_id) in finished.into_iter().take(remove_count) {
                    tasks_guard.remove(&old_id);
                }
            }
        }

        // Stall watchdog thread waiting on the shared Condvar (zero busy-wait polling)
        let timeout_secs = options.timeout_secs.unwrap_or(DEFAULT_SUBAGENT_TIMEOUT_SECS);
        if timeout_secs > 0 {
            let wd_entry = Arc::clone(&entry);
            let wd_signal = Arc::clone(&self.completion_signal);
            let wd_cancel = Arc::clone(&cancel);
            let wd_timed_out = Arc::clone(&timed_out);
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(timeout_secs);
                let (lock, cvar) = &*wd_signal;
                let mut seq = lock.lock().unwrap();
                loop {
                    let still_running = wd_entry
                        .snapshot
                        .lock()
                        .map(|s| s.status == TaskStatus::Running)
                        .unwrap_or(false);
                    if !still_running || wd_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let now = Instant::now();
                    if now >= deadline {
                        wd_timed_out.store(true, Ordering::SeqCst);
                        wd_cancel.store(true, Ordering::SeqCst);
                        break;
                    }
                    let (guard, timeout_res) = cvar.wait_timeout(seq, deadline - now).unwrap();
                    seq = guard;
                    if timeout_res.timed_out() {
                        let still_running = wd_entry
                            .snapshot
                            .lock()
                            .map(|s| s.status == TaskStatus::Running)
                            .unwrap_or(false);
                        if still_running {
                            wd_timed_out.store(true, Ordering::SeqCst);
                            wd_cancel.store(true, Ordering::SeqCst);
                        }
                        break;
                    }
                }
            });
        }

        let store = self.store.clone();
        let persist_store = self.store.clone();
        let persist_task_id = task_id.clone();
        let caller = Arc::clone(&self.caller);
        let child_ctx = if let Some(ref wt_path) = worktree_path {
            let mut isolated = ToolContext::new(wt_path).with_session_id(session_id.clone());
            isolated.network_allowed = ctx.network_allowed;
            isolated.interactive_user_input = false;
            isolated.permission_profile = ctx.permission_profile;
            isolated.sandbox_mode = ctx.sandbox_mode;
            isolated
        } else {
            ctx.clone().with_session_id(session_id.clone())
        };
        let child_entry = Arc::clone(&entry);
        let panic_entry = Arc::clone(&entry);
        let panic_store = self.store.clone();
        let panic_task_id = task_id.clone();
        let signal_done = Arc::clone(&self.completion_signal);
        let notify_parent_inbox = if background { parent_inbox } else { None };
        let allowed_tools = options.allowed_tools;
        let run_child = move || {
            let unwind_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let mut reg = match &tool_factory {
                    Some(factory) => factory(&child_ctx.workspace_root),
                    None => ToolRegistry::with_builtin_tools(),
                };
                if let Some(allow) = allowed_tools.filter(|a| !a.is_empty()) {
                    let mut filtered = ToolRegistry::default();
                    for spec in reg.all_specs() {
                        if allow.iter().any(|a| a == "*" || a == &spec.schema.name) {
                            if let Some(h) = reg.get(&spec.schema.name) {
                                filtered.register(h);
                            }
                        }
                    }
                    reg = filtered;
                }
                let child_registry = Arc::new(reg);
                let engine = AgentEngine::new(store, child_registry, caller);
                if parent_cancel
                    .as_ref()
                    .is_some_and(|pc| pc.load(Ordering::Relaxed))
                {
                    cancel.store(true, Ordering::SeqCst);
                }
                let cfg = AgentRunConfig {
                    session_id: Some(session_id),
                    resume: false,
                    route,
                    frozen_system_blocks: vec![FrozenSystemBlock {
                        id: "subagent_role".into(),
                        text: if system_prompt.trim().is_empty() {
                            format!(
                                "You are a focused Nola subagent acting as '{role}'. \
                                 Complete the delegated task thoroughly and return a concise, verified report."
                            )
                        } else {
                            system_prompt
                        },
                    }],
                    host_capabilities: HostCapabilities {
                        interactive_user_input: false,
                        network_allowed: child_ctx.network_allowed,
                    },
                    policy,
                    hooks,
                    approval: None,
                    max_steps: if max_steps == 0 { 24 } else { max_steps },
                    force_tool_set: ForceToolSet::Auto,
                    stream: false,
                    json_schema: None,
                    cancel: Arc::clone(&cancel),
                    auto_recall_memory: false,
                    inbox: Some(inbox),
                };
                let parent_c = parent_cancel.clone();
                let child_c = Arc::clone(&cancel);
                let run_res = engine.run(&child_ctx, &task_prompt, cfg, |_| {
                    if parent_c.as_ref().is_some_and(|p| p.load(Ordering::Relaxed)) {
                        child_c.store(true, Ordering::SeqCst);
                    }
                });
                let final_snap = match run_res {
                    Ok(res) => {
                        let mut snap = child_entry.snapshot.lock().unwrap();
                        snap.output = res.final_text;
                        snap.steps = res.steps;
                        snap.cancelling = false;
                        if timed_out.load(Ordering::SeqCst) {
                            snap.status = TaskStatus::Failed;
                            snap.error = Some(format!(
                                "subagent exceeded stall watchdog timeout ({timeout_secs}s)"
                            ));
                        } else if res.cancelled || cancel.load(Ordering::SeqCst) {
                            snap.status = TaskStatus::Stopped;
                        } else {
                            snap.status = TaskStatus::Completed;
                        }
                        snap.ended_at_ms.get_or_insert_with(now_ms);
                        snap.clone()
                    }
                    Err(e) => {
                        let mut snap = child_entry.snapshot.lock().unwrap();
                        snap.cancelling = false;
                        if timed_out.load(Ordering::SeqCst) {
                            snap.status = TaskStatus::Failed;
                            snap.error = Some(format!(
                                "subagent exceeded stall watchdog timeout ({timeout_secs}s)"
                            ));
                        } else if cancel.load(Ordering::SeqCst) {
                            snap.status = TaskStatus::Stopped;
                        } else {
                            snap.error = Some(e.to_string());
                            snap.status = TaskStatus::Failed;
                        }
                        snap.ended_at_ms.get_or_insert_with(now_ms);
                        snap.clone()
                    }
                };
                let _ =
                    persist_store.put_module_record("durable_task", &persist_task_id, &final_snap);
                final_snap
            }));

            let completed_snap = match unwind_res {
                Ok(snap) => snap,
                Err(payload) => {
                    let panic_msg = payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "subagent task panicked".to_string());
                    let mut snap = panic_entry
                        .snapshot
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    snap.cancelling = false;
                    snap.status = TaskStatus::Failed;
                    snap.error = Some(format!("subagent panic: {panic_msg}"));
                    snap.ended_at_ms.get_or_insert_with(now_ms);
                    let snap_clone = snap.clone();
                    drop(snap);
                    let _ =
                        panic_store.put_module_record("durable_task", &panic_task_id, &snap_clone);
                    snap_clone
                }
            };

            if let Some(ref p_inbox) = notify_parent_inbox {
                let preview: String = completed_snap.output.chars().take(400).collect();
                let notice = json!({
                    "type": "subagent_completed",
                    "task_id": completed_snap.task_id,
                    "role": completed_snap.role,
                    "status": completed_snap.status,
                    "steps": completed_snap.steps,
                    "worktree_path": completed_snap.worktree_path,
                    "branch": completed_snap.branch,
                    "output_preview": preview,
                    "error": completed_snap.error,
                });
                if let Ok(mut q) = p_inbox.lock() {
                    q.push(format!(
                        "<subagent_notification>\n{}\n</subagent_notification>",
                        serde_json::to_string(&notice).unwrap_or_default()
                    ));
                }
            }

            let (lock, cvar) = &*signal_done;
            let mut seq = lock.lock().unwrap();
            *seq = seq.wrapping_add(1);
            cvar.notify_all();
        };

        if background {
            thread::spawn(run_child);
        } else {
            run_child();
        }

        let snap = entry.snapshot.lock().unwrap().clone();
        Ok(snap)
    }

    pub fn list(&self) -> Vec<SubagentTaskSnapshot> {
        let mut by_id: BTreeMap<String, SubagentTaskSnapshot> = self
            .store
            .list_module_records::<SubagentTaskSnapshot>("durable_task")
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.task_id.clone(), s))
            .collect();
        for (id, entry) in self.tasks.lock().unwrap().iter() {
            by_id.insert(id.clone(), entry.snapshot.lock().unwrap().clone());
        }
        by_id.into_values().collect()
    }

    pub fn read(&self, task_id: &str) -> Result<SubagentTaskSnapshot, String> {
        if let Some(snap) = self
            .tasks
            .lock()
            .unwrap()
            .get(task_id)
            .map(|e| e.snapshot.lock().unwrap().clone())
        {
            return Ok(snap);
        }
        self.store
            .get_module_record::<SubagentTaskSnapshot>("durable_task", task_id)
            .ok()
            .flatten()
            .ok_or_else(|| format!("unknown task '{task_id}'"))
    }

    pub fn wait(&self, task_id: &str, timeout_ms: u64) -> Result<SubagentTaskSnapshot, String> {
        let snaps = self.wait_many(&[task_id.to_string()], WaitMode::All, timeout_ms)?;
        snaps
            .into_iter()
            .next()
            .ok_or_else(|| format!("unknown task '{task_id}'"))
    }

    /// Wait on one or more subagent tasks via shared `Condvar` (zero busy-wait polling),
    /// returning when `wait_mode` (`WaitMode::Any` or `WaitMode::All`) is satisfied or `timeout_ms` elapses.
    pub fn wait_many(
        &self,
        task_ids: &[String],
        wait_mode: WaitMode,
        timeout_ms: u64,
    ) -> Result<Vec<SubagentTaskSnapshot>, String> {
        if task_ids.is_empty() {
            return Err("wait requires at least one task_id".into());
        }
        let mut entries: Vec<Option<Arc<SubagentEntry>>> = Vec::with_capacity(task_ids.len());
        {
            let map = self.tasks.lock().unwrap();
            for id in task_ids {
                if let Some(e) = map.get(id) {
                    entries.push(Some(Arc::clone(e)));
                } else if self
                    .store
                    .get_module_record::<SubagentTaskSnapshot>("durable_task", id)
                    .ok()
                    .flatten()
                    .is_some()
                {
                    entries.push(None);
                } else {
                    return Err(format!("unknown task '{id}'"));
                }
            }
        }

        let deadline = Instant::now() + Duration::from_millis(timeout_ms.clamp(50, 120_000));
        let (lock, cvar) = &*self.completion_signal;
        let mut seq = lock.lock().unwrap();

        loop {
            let snapshots: Vec<SubagentTaskSnapshot> = task_ids
                .iter()
                .zip(entries.iter())
                .map(|(id, entry)| match entry {
                    Some(e) => e.snapshot.lock().unwrap().clone(),
                    None => self
                        .store
                        .get_module_record::<SubagentTaskSnapshot>("durable_task", id)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| SubagentTaskSnapshot {
                            task_id: id.clone(),
                            role: String::new(),
                            session_id: String::new(),
                            status: TaskStatus::Interrupted,
                            cancelling: false,
                            output: String::new(),
                            error: Some("task record missing".into()),
                            steps: 0,
                            started_at_ms: 0,
                            ended_at_ms: Some(now_ms()),
                            worktree_path: None,
                            branch: None,
                        }),
                })
                .collect();

            let done = match wait_mode {
                WaitMode::All => snapshots.iter().all(|s| s.status != TaskStatus::Running),
                WaitMode::Any => snapshots.iter().any(|s| s.status != TaskStatus::Running),
            };
            let now = Instant::now();
            if done || now >= deadline {
                return Ok(snapshots);
            }
            let (guard, _) = cvar.wait_timeout(seq, deadline - now).unwrap();
            seq = guard;
        }
    }

    pub fn stop(&self, task_id: &str) -> Result<SubagentTaskSnapshot, String> {
        let entry = self
            .tasks
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("unknown task '{task_id}'"))?;
        entry.cancel.store(true, Ordering::SeqCst);
        {
            let mut snap = entry.snapshot.lock().unwrap();
            if snap.status == TaskStatus::Running && snap.ended_at_ms.is_none() {
                snap.cancelling = true;
                let _ = self
                    .store
                    .put_module_record("durable_task", task_id, &*snap);
            }
        }
        // Wait briefly (up to 300ms) on the Condvar for the worker thread to observe cancellation
        // and seal TaskStatus::Stopped without prematurely overwriting status while still running.
        let deadline = Instant::now() + Duration::from_millis(300);
        let (lock, cvar) = &*self.completion_signal;
        let mut seq = lock.lock().unwrap();
        let snap = loop {
            let snap = entry.snapshot.lock().unwrap().clone();
            let now = Instant::now();
            if snap.ended_at_ms.is_some() || snap.status != TaskStatus::Running || now >= deadline {
                break snap;
            }
            let (guard, _) = cvar.wait_timeout(seq, deadline - now).unwrap();
            seq = guard;
        };
        drop(seq);

        // Clean up unmodified isolated git worktree on explicit stop; preserve modified/committed worktrees
        // so the parent can inspect or integrate them via `git worktree_integrate`.
        if let (Some(ref wt_path), Some(ref branch)) = (&entry.worktree_path, &entry.branch) {
            if wt_path.exists() && is_worktree_unmodified(&entry.repo_root, wt_path, branch) {
                let _ = remove_task_worktree(&entry.repo_root, wt_path, branch);
            }
        }
        let _ = entry.timed_out.load(Ordering::Relaxed);
        Ok(snap)
    }

    pub fn send_message(&self, task_id: &str, message: String) -> Result<usize, String> {
        let entry = self
            .tasks
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("unknown task '{task_id}'"))?;
        let mut inbox = entry.inbox.lock().unwrap();
        inbox.push(message);
        Ok(inbox.len())
    }
}

fn is_git_repo(workspace_root: &Path) -> bool {
    Command::new("git")
        .current_dir(workspace_root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

fn create_task_worktree(workspace_root: &Path, task_id: &str) -> Result<(PathBuf, String), String> {
    let wt_parent = workspace_root.join(".nola").join("worktrees");
    std::fs::create_dir_all(&wt_parent)
        .map_err(|e| format!("failed to create worktree parent dir: {e}"))?;
    let wt_path = wt_parent.join(task_id);
    let branch = format!("nola-task-{task_id}");
    let wt_str = wt_path.to_string_lossy().into_owned();
    let out = Command::new("git")
        .current_dir(workspace_root)
        .args(["worktree", "add", "-b", &branch, "--", &wt_str])
        .output()
        .map_err(|e| format!("failed to spawn git worktree add: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("git worktree add failed: {stderr}"));
    }
    let canonical = std::fs::canonicalize(&wt_path).unwrap_or(wt_path);
    Ok((canonical, branch))
}

fn is_worktree_unmodified(repo_root: &Path, wt_path: &Path, branch: &str) -> bool {
    let clean_status = Command::new("git")
        .current_dir(wt_path)
        .args(["status", "--porcelain"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);
    if !clean_status {
        return false;
    }
    let no_new_commits = Command::new("git")
        .current_dir(repo_root)
        .args(["rev-list", "--count", &format!("HEAD..{branch}")])
        .output()
        .map(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u64>()
                    .unwrap_or(1)
                    == 0
        })
        .unwrap_or(false);
    no_new_commits
}

fn remove_task_worktree(repo_root: &Path, wt_path: &Path, branch: &str) -> Result<(), String> {
    let wt_str = wt_path.to_string_lossy().into_owned();
    let _ = Command::new("git")
        .current_dir(repo_root)
        .args(["worktree", "remove", "--force", "--", &wt_str])
        .output();
    let _ = Command::new("git")
        .current_dir(repo_root)
        .args(["branch", "-D", branch])
        .output();
    Ok(())
}

pub fn register_subagent_tools(mgr: &Arc<SubagentManager>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(SpawnAgentTool {
        mgr: Arc::clone(mgr),
    }));
    registry.register(Arc::new(DelegateExpertTool {
        mgr: Arc::clone(mgr),
    }));
    registry.register(Arc::new(TaskControlTool {
        mgr: Arc::clone(mgr),
    }));
}

struct SpawnAgentTool {
    mgr: Arc<SubagentManager>,
}

impl ToolHandler for SpawnAgentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "spawn_agent".into(),
                description: "Spawn a real child subagent (foreground or background=true) to execute an independent subtask, optionally inside an isolated git worktree.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "role": { "type": "string", "description": "Subagent role (e.g. 'Code Researcher', 'Test Fixer')" },
                        "prompt": { "type": "string", "description": "Actionable task instructions" },
                        "system_prompt": { "type": "string", "description": "Optional custom system instructions" },
                        "background": { "type": "boolean", "description": "Run asynchronously (default false)" },
                        "max_steps": { "type": "integer", "description": "Step cap (default 24)" },
                        "worktree": { "type": "boolean", "description": "Isolate execution in a dedicated git worktree (default false)" },
                        "timeout_secs": { "type": "integer", "description": "Watchdog timeout in seconds (default 600)" }
                    },
                    "required": ["role", "prompt"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(role) = args.get("role").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'role'", None);
        };
        let Some(prompt) = args.get("prompt").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'prompt'", None);
        };
        let sys = args
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let bg = args
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_steps = args.get("max_steps").and_then(|v| v.as_u64()).unwrap_or(24) as usize;
        let worktree = args
            .get("worktree")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let timeout_secs = args.get("timeout_secs").and_then(|v| v.as_u64());
        match self.mgr.spawn_with_full_options(
            ctx,
            role.to_string(),
            sys,
            prompt.to_string(),
            bg,
            max_steps,
            SubagentSpawnOptions {
                model_override: None,
                allowed_tools: None,
                worktree,
                timeout_secs,
            },
        ) {
            Ok(snap) => ToolExecutionResult::ok_structured(
                serde_json::to_value(&snap).unwrap_or(json!({})),
                format!("Subagent {} ({}) -> {:?}", snap.task_id, snap.role, snap.status),
            ),
            Err(e) => ToolExecutionResult::err_structured("subagent_error", e, None),
        }
    }
}

struct DelegateExpertTool {
    mgr: Arc<SubagentManager>,
}

impl ToolHandler for DelegateExpertTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "delegate_expert".into(),
                description: "Delegate a task to a saved ExpertPeer persona stored in Nola, optionally in an isolated git worktree.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "expert_name": { "type": "string" },
                        "prompt": { "type": "string" },
                        "background": { "type": "boolean" },
                        "worktree": { "type": "boolean", "description": "Isolate execution in a dedicated git worktree (default false)" },
                        "timeout_secs": { "type": "integer", "description": "Watchdog timeout in seconds (default 600)" }
                    },
                    "required": ["expert_name", "prompt"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(name) = args.get("expert_name").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'expert_name'", None);
        };
        let Some(prompt) = args.get("prompt").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'prompt'", None);
        };
        let experts = self
            .mgr
            .store
            .list_module_records::<ExpertPeer>("expert_peer")
            .unwrap_or_default();
        let Some(expert) = experts
            .into_iter()
            .find(|e| e.name.eq_ignore_ascii_case(name) || e.id == name)
        else {
            return ToolExecutionResult::err_structured(
                "expert_not_found",
                format!("expert peer '{name}' not found"),
                None,
            );
        };
        let bg = args
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let worktree = args
            .get("worktree")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let timeout_secs = args.get("timeout_secs").and_then(|v| v.as_u64());
        match self.mgr.spawn_with_full_options(
            ctx,
            expert.name.clone(),
            expert.system_prompt,
            prompt.to_string(),
            bg,
            24,
            SubagentSpawnOptions {
                model_override: expert.model_override,
                allowed_tools: Some(expert.allowed_tools),
                worktree,
                timeout_secs,
            },
        ) {
            Ok(snap) => ToolExecutionResult::ok_structured(
                serde_json::to_value(&snap).unwrap_or(json!({})),
                format!("Delegated to expert '{}' -> {:?}", expert.name, snap.status),
            ),
            Err(e) => ToolExecutionResult::err_structured("delegate_error", e, None),
        }
    }
}

struct TaskControlTool {
    mgr: Arc<SubagentManager>,
}

impl ToolHandler for TaskControlTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "task_control".into(),
                description: "Inspect, wait on (single task_id or parallel task_ids with wait_mode=any|all), stop, or message spawned subagent tasks (action: list|read|wait|stop|send_message).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["list", "read", "wait", "stop", "send_message"] },
                        "task_id": { "type": "string" },
                        "task_ids": { "type": "array", "items": { "type": "string" }, "description": "Parallel task IDs for action=wait" },
                        "wait_mode": { "type": "string", "enum": ["any", "all"], "description": "Parallel wait mode (default 'all')" },
                        "wait_ms": { "type": "integer", "description": "Default 5000" },
                        "message": { "type": "string" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("list") {
            "list" => {
                let tasks = self.mgr.list();
                let count = tasks.len();
                ToolExecutionResult::ok_structured(
                    json!({"count": count, "tasks": tasks}),
                    format!("Listed {count} subagent task(s)"),
                )
            }
            "read" => {
                let Some(id) = args.get("task_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'task_id'", None);
                };
                match self.mgr.read(id) {
                    Ok(s) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&s).unwrap_or(json!({})),
                        format!("Read task {id} ({:?})", s.status),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("task_error", e, None),
                }
            }
            "wait" => {
                let wait_ms = args.get("wait_ms").and_then(|v| v.as_u64()).unwrap_or(5_000);
                let wait_mode = match args.get("wait_mode").and_then(|v| v.as_str()).unwrap_or("all")
                {
                    "any" => WaitMode::Any,
                    "all" => WaitMode::All,
                    other => {
                        return ToolExecutionResult::err_structured(
                            "invalid_param",
                            format!("unsupported wait_mode '{other}'; expected 'any' or 'all'"),
                            None,
                        );
                    }
                };
                if let Some(arr) = args.get("task_ids").and_then(|v| v.as_array()) {
                    let ids: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect();
                    if ids.is_empty() {
                        return ToolExecutionResult::err_structured(
                            "missing_param",
                            "'task_ids' array must not be empty",
                            None,
                        );
                    }
                    match self.mgr.wait_many(&ids, wait_mode, wait_ms) {
                        Ok(tasks) => {
                            let finished = tasks
                                .iter()
                                .filter(|s| s.status != TaskStatus::Running)
                                .count();
                            let total = tasks.len();
                            ToolExecutionResult::ok_structured(
                                json!({
                                    "wait_mode": wait_mode,
                                    "completed_count": finished,
                                    "total": total,
                                    "tasks": tasks,
                                }),
                                format!("Waited ({wait_mode:?}) for {total} task(s): {finished} finished"),
                            )
                        }
                        Err(e) => ToolExecutionResult::err_structured("task_error", e, None),
                    }
                } else {
                    let Some(id) = args.get("task_id").and_then(|v| v.as_str()) else {
                        return ToolExecutionResult::err_structured(
                            "missing_param",
                            "missing 'task_id' or 'task_ids'",
                            None,
                        );
                    };
                    match self.mgr.wait(id, wait_ms) {
                        Ok(s) => ToolExecutionResult::ok_structured(
                            serde_json::to_value(&s).unwrap_or(json!({})),
                            format!("Waited for task {id} ({:?})", s.status),
                        ),
                        Err(e) => ToolExecutionResult::err_structured("task_error", e, None),
                    }
                }
            }
            "stop" => {
                let Some(id) = args.get("task_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'task_id'", None);
                };
                match self.mgr.stop(id) {
                    Ok(s) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&s).unwrap_or(json!({})),
                        format!("Stopped task {id}"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("task_error", e, None),
                }
            }
            "send_message" => {
                let Some(id) = args.get("task_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'task_id'", None);
                };
                let msg = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                match self.mgr.send_message(id, msg) {
                    Ok(queued) => ToolExecutionResult::ok_structured(
                        json!({"task_id": id, "queued_messages": queued}),
                        format!("Queued message to task {id}"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("task_error", e, None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported task_control action '{other}'"),
                None,
            ),
        }
    }
}
