// SPDX-License-Identifier: Apache-2.0
//! Real Subagent & Expert Delegation Runtime (`SubagentManager`):
//! Spawns actual child `AgentEngine` executions (foreground or background threads),
//! tracks task lifecycle, supports `wait`/`read`/`stop`/`list`/`send_message`,
//! and enforces bounded concurrency and clean cancellation propagation.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Stopped,
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
}

struct SubagentEntry {
    snapshot: Mutex<SubagentTaskSnapshot>,
    cancel: Arc<AtomicBool>,
    inbox: Arc<Mutex<Vec<String>>>,
}

/// Manages real child subagent executions backed by `AgentEngine`.
pub struct SubagentManager {
    store: Store,
    caller: Arc<dyn ModelCaller>,
    base_route: Mutex<Option<ModelRouteConfig>>,
    base_policy: Mutex<Option<EffectivePolicy>>,
    base_hooks: Mutex<HookRunner>,
    parent_cancel: Mutex<Option<Arc<AtomicBool>>>,
    tasks: Mutex<BTreeMap<String, Arc<SubagentEntry>>>,
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
            tasks: Mutex::new(BTreeMap::new()),
        }
    }

    /// Attach or replace the backing store used to persist durable subagent snapshots.
    pub fn with_store(mut self, store: Store) -> Self {
        self.store = store;
        self
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

    pub fn spawn(
        self: &Arc<Self>,
        ctx: &ToolContext,
        role: String,
        system_prompt: String,
        task_prompt: String,
        background: bool,
        max_steps: usize,
    ) -> Result<SubagentTaskSnapshot, String> {
        self.spawn_with_options(
            ctx,
            role,
            system_prompt,
            task_prompt,
            background,
            max_steps,
            None,
            None,
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
        let mut route = self
            .base_route
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "parent model route is not configured for subagent spawning".to_string())?;
        if let Some(m) = model_override.filter(|s| !s.trim().is_empty()) {
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
            });
        let hooks = self.base_hooks.lock().unwrap().clone();
        let parent_cancel = self.parent_cancel.lock().unwrap().clone();

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
        let cancel = Arc::new(AtomicBool::new(false));
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
        };
        let _ = self
            .store
            .put_module_record("durable_task", &task_id, &initial_snap);
        let entry = Arc::new(SubagentEntry {
            snapshot: Mutex::new(initial_snap),
            cancel: Arc::clone(&cancel),
            inbox: Arc::clone(&inbox),
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

        let store = self.store.clone();
        let persist_store = self.store.clone();
        let persist_task_id = task_id.clone();
        let caller = Arc::clone(&self.caller);
        let child_ctx = ctx.clone().with_session_id(session_id.clone());
        let child_entry = Arc::clone(&entry);
        let panic_entry = Arc::clone(&entry);
        let panic_store = self.store.clone();
        let panic_task_id = task_id.clone();
        let run_child = move || {
            let unwind_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let mut reg = ToolRegistry::with_builtin_tools();
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
                        snap.status = if res.cancelled || cancel.load(Ordering::SeqCst) {
                            TaskStatus::Stopped
                        } else {
                            TaskStatus::Completed
                        };
                        snap.ended_at_ms.get_or_insert_with(now_ms);
                        snap.clone()
                    }
                    Err(e) => {
                        let mut snap = child_entry.snapshot.lock().unwrap();
                        snap.cancelling = false;
                        if cancel.load(Ordering::SeqCst) {
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
            }));
            if let Err(payload) = unwind_res {
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
                let _ = panic_store.put_module_record("durable_task", &panic_task_id, &snap_clone);
            }
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
        let entry = self
            .tasks
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .ok_or_else(|| format!("unknown task '{task_id}'"))?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.clamp(50, 120_000));
        loop {
            let snap = entry.snapshot.lock().unwrap().clone();
            if snap.status != TaskStatus::Running || Instant::now() >= deadline {
                return Ok(snap);
            }
            thread::sleep(Duration::from_millis(25));
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
        // Wait briefly (up to 300ms) for the worker thread to observe cancellation and seal
        // TaskStatus::Stopped without prematurely overwriting status while it is still executing.
        let deadline = Instant::now() + Duration::from_millis(300);
        loop {
            let snap = entry.snapshot.lock().unwrap().clone();
            if snap.ended_at_ms.is_some()
                || snap.status != TaskStatus::Running
                || Instant::now() >= deadline
            {
                return Ok(snap);
            }
            thread::sleep(Duration::from_millis(15));
        }
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
                description: "Spawn a real child subagent (foreground or background=true) to execute an independent subtask.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "role": { "type": "string", "description": "Subagent role (e.g. 'Code Researcher', 'Test Fixer')" },
                        "prompt": { "type": "string", "description": "Actionable task instructions" },
                        "system_prompt": { "type": "string", "description": "Optional custom system instructions" },
                        "background": { "type": "boolean", "description": "Run asynchronously (default false)" },
                        "max_steps": { "type": "integer", "description": "Step cap (default 24)" }
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
        match self
            .mgr
            .spawn(ctx, role.to_string(), sys, prompt.to_string(), bg, max_steps)
        {
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
                description: "Delegate a task to a saved ExpertPeer persona stored in Nola.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "expert_name": { "type": "string" },
                        "prompt": { "type": "string" },
                        "background": { "type": "boolean" }
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
        match self.mgr.spawn_with_options(
            ctx,
            expert.name.clone(),
            expert.system_prompt,
            prompt.to_string(),
            bg,
            24,
            expert.model_override,
            Some(expert.allowed_tools),
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
                description: "Inspect, wait on, stop, or message spawned subagent tasks (action: list|read|wait|stop|send_message).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["list", "read", "wait", "stop", "send_message"] },
                        "task_id": { "type": "string" },
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
                let Some(id) = args.get("task_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'task_id'", None);
                };
                let wait_ms = args.get("wait_ms").and_then(|v| v.as_u64()).unwrap_or(5_000);
                match self.mgr.wait(id, wait_ms) {
                    Ok(s) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&s).unwrap_or(json!({})),
                        format!("Waited for task {id} ({:?})", s.status),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("task_error", e, None),
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
