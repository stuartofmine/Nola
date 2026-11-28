// SPDX-License-Identifier: Apache-2.0
//! Autonomous Goal Execution Worker (`GoalWorker`):
//! Durable lease acquisition/renewal, exponential retry backoff (`30s * 2^attempt`, capped at 30m),
//! `needs_user` pause/resume transitions, and autonomous background polling via
//! `spawn_background_goal_worker`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use nola_engine::{
    AgentEngine, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller,
    ModelCaller,
};
use nola_store::Store;
use nola_tools::{path_within_workspace, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{now_ms, FrozenSystemBlock, GoalStatus, HostCapabilities, ModelRouteConfig};

use crate::goals_sops::{GoalRecord, KIND_GOAL};
use crate::ModulesSuite;

pub const DEFAULT_GOAL_LEASE_MS: i64 = 120_000;
pub const BASE_RETRY_BACKOFF_MS: i64 = 30_000;
pub const MAX_RETRY_BACKOFF_MS: i64 = 30 * 60 * 1_000;
pub const MAX_GOAL_RETRIES: u32 = 5;

/// Exponential retry backoff: `30s * 2^attempt`, capped at `30m` (1,800,000 ms).
pub fn retry_backoff_ms(attempt: u32) -> i64 {
    let shift = attempt.min(20);
    let factor = 1i64.checked_shl(shift).unwrap_or(i64::MAX);
    BASE_RETRY_BACKOFF_MS
        .saturating_mul(factor)
        .min(MAX_RETRY_BACKOFF_MS)
}

pub struct GoalWorker;

impl GoalWorker {
    /// Attempt to durably acquire an execution lease on a goal (`Pending` whose retry window
    /// has elapsed, or `Running` whose prior lease expired).
    pub fn try_acquire_lease(
        store: &Store,
        goal_id: &str,
        lease_ms: i64,
        now: i64,
    ) -> Result<Option<GoalRecord>, String> {
        store
            .update_module_record(KIND_GOAL, goal_id, |goal: &mut GoalRecord| {
                if !Self::is_runnable(goal, now) {
                    return false;
                }
                goal.status = GoalStatus::Running;
                goal.lease_until_ms = Some(now.saturating_add(lease_ms.max(1_000)));
                goal.updated_at_ms = now;
                true
            })
            .map_err(|e| e.to_string())
    }

    fn is_runnable(goal: &GoalRecord, now: i64) -> bool {
        match goal.status {
            GoalStatus::Pending => {
                goal.needs_user_question.is_none()
                    && goal.next_retry_at_ms.is_none_or(|retry| retry <= now)
            }
            GoalStatus::Running => goal.lease_until_ms.is_none_or(|until| until <= now),
            _ => false,
        }
    }

    pub fn renew_lease(
        store: &Store,
        goal_id: &str,
        lease_ms: i64,
        now: i64,
    ) -> Result<GoalRecord, String> {
        let goal = store
            .get_module_record::<GoalRecord>(KIND_GOAL, goal_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("unknown goal '{goal_id}'"))?;
        Self::renew_owned_lease(store, goal_id, goal.lease_until_ms, lease_ms, now)
    }

    fn renew_owned_lease(
        store: &Store,
        id: &str,
        deadline: Option<i64>,
        lease_ms: i64,
        now: i64,
    ) -> Result<GoalRecord, String> {
        store
            .update_module_record(KIND_GOAL, id, |goal: &mut GoalRecord| {
                if goal.status != GoalStatus::Running
                    || goal.lease_until_ms != deadline
                    || deadline.is_none_or(|d| d <= now)
                {
                    return false;
                }
                goal.lease_until_ms = Some(now.saturating_add(lease_ms.max(1_000)));
                goal.updated_at_ms = now;
                true
            })
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("goal '{id}' lease no longer active"))
    }

    /// Pause a goal in `needs_user` state with an explicit question awaiting user input.
    pub fn mark_needs_user(
        store: &Store,
        goal_id: &str,
        question: &str,
    ) -> Result<GoalRecord, String> {
        let q = question.trim();
        if q.is_empty() {
            return Err("needs_user question must not be empty".into());
        }
        let mut goal = store
            .get_module_record::<GoalRecord>(KIND_GOAL, goal_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("goal '{goal_id}' not found"))?;
        let ts = now_ms();
        goal.status = GoalStatus::Paused;
        goal.lease_until_ms = None;
        goal.needs_user_question = Some(q.to_string());
        goal.progress_notes.push(format!("[needs_user] {q}"));
        goal.updated_at_ms = ts;
        store
            .put_module_record(KIND_GOAL, &goal.id, &goal)
            .map_err(|e| e.to_string())?;
        Ok(goal)
    }

    /// Resume a paused/`needs_user` goal with the user's answer so the worker picks it up again.
    pub fn resume_from_user_input(
        store: &Store,
        goal_id: &str,
        user_answer: &str,
    ) -> Result<GoalRecord, String> {
        let mut goal = store
            .get_module_record::<GoalRecord>(KIND_GOAL, goal_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("goal '{goal_id}' not found"))?;
        let ts = now_ms();
        if !user_answer.trim().is_empty() {
            goal.progress_notes
                .push(format!("[user_answer] {}", user_answer.trim()));
        }
        goal.needs_user_question = None;
        goal.next_retry_at_ms = None;
        goal.lease_until_ms = None;
        goal.status = GoalStatus::Pending;
        goal.updated_at_ms = ts;
        store
            .put_module_record(KIND_GOAL, &goal.id, &goal)
            .map_err(|e| e.to_string())?;
        Ok(goal)
    }

    /// Record a failed execution attempt and schedule exponential retry backoff (`30s * 2^attempt`, capped at `30m`).
    pub fn schedule_retry_backoff(
        store: &Store,
        goal_id: &str,
        error: &str,
        now: i64,
    ) -> Result<GoalRecord, String> {
        store
            .update_module_record(KIND_GOAL, goal_id, |goal: &mut GoalRecord| {
                Self::record_retry(goal, error, now);
                true
            })
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("unknown goal '{goal_id}'"))
    }

    fn record_retry(goal: &mut GoalRecord, error: &str, now: i64) {
        let attempt = goal.retry_count;
        goal.retry_count = goal.retry_count.saturating_add(1);
        goal.lease_until_ms = None;
        if goal.retry_count >= MAX_GOAL_RETRIES {
            goal.status = GoalStatus::Failed;
            goal.next_retry_at_ms = None;
            goal.progress_notes.push(format!(
                "[failed after {} retries] {error}",
                goal.retry_count
            ));
        } else {
            let delay = retry_backoff_ms(attempt);
            goal.status = GoalStatus::Pending;
            goal.next_retry_at_ms = Some(now.saturating_add(delay));
            goal.progress_notes.push(format!(
                "[retry #{} scheduled in {}s] {error}",
                goal.retry_count,
                delay / 1000
            ));
        }
        goal.updated_at_ms = now;
    }

    /// List goals currently eligible for autonomous execution (due pending goals or expired leases).
    pub fn find_runnable_goals(store: &Store, now: i64) -> Vec<GoalRecord> {
        let mut goals: Vec<GoalRecord> = store
            .list_module_records(KIND_GOAL)
            .unwrap_or_default()
            .into_iter()
            .filter(|g| Self::is_runnable(g, now))
            .collect();
        goals.sort_by_key(|g| g.updated_at_ms);
        goals
    }

    /// Backward-compatible single-goal executor returning `nola_types::GoalRecord`.
    pub fn execute_goal(
        suite: &Arc<ModulesSuite>,
        caller: Arc<dyn ModelCaller>,
        route: ModelRouteConfig,
        policy: EffectivePolicy,
        goal_id: &str,
        max_steps: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<nola_types::GoalRecord, String> {
        Self::execute_goal_ext(suite, caller, route, policy, goal_id, max_steps, cancel)
            .map(GoalRecord::into_base)
    }

    /// Execute a persisted goal end-to-end with durable lease acquisition/renewal,
    /// `goal_complete` / `needs_user` preservation, and exponential retry backoff on failure.
    pub fn execute_goal_ext(
        suite: &Arc<ModulesSuite>,
        caller: Arc<dyn ModelCaller>,
        route: ModelRouteConfig,
        policy: EffectivePolicy,
        goal_id: &str,
        max_steps: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<GoalRecord, String> {
        let now = now_ms();
        let goal = suite
            .get_record::<GoalRecord>(KIND_GOAL, goal_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("goal '{goal_id}' not found"))?;
        let goal_ws_buf = PathBuf::from(&goal.workspace_root);
        let goal_ws = goal_ws_buf.as_path();
        if !path_within_workspace(&suite.workspace_root, goal_ws) {
            return Err(format!(
                "goal workspace '{}' escapes active workspace root '{}'",
                goal.workspace_root,
                suite.workspace_root.display()
            ));
        }

        let goal = Self::try_acquire_lease(&suite.store, goal_id, DEFAULT_GOAL_LEASE_MS, now)?
            .ok_or_else(|| format!("goal '{goal_id}' is not ready to run"))?;
        let run_cancel = Arc::new(AtomicBool::new(cancel.load(Ordering::Acquire)));

        let mut registry = ToolRegistry::with_builtin_tools();
        suite.register_tools(&mut registry);
        let engine = AgentEngine::new(suite.store.clone(), Arc::new(registry), caller);

        let mut ctx = ToolContext::new(goal_ws);
        ctx.permission_profile = policy.profile;
        ctx.network_allowed = policy.network_allowed;
        ctx.sandbox_mode = SandboxMode::from_env();

        let criteria_block = if goal.acceptance_criteria.is_empty() {
            String::new()
        } else {
            format!(
                "\nAcceptance criteria:\n- {}",
                goal.acceptance_criteria.join("\n- ")
            )
        };
        let hooks = HookRunner::load(
            goal_ws,
            suite.user_home.as_deref(),
            policy.workspace_trusted,
        );
        let run_cfg = AgentRunConfig {
            session_id: Some(format!("goal_{}", goal.id)),
            resume: true,
            route,
            frozen_system_blocks: vec![FrozenSystemBlock {
                id: "goal_worker".into(),
                text: format!(
                    "You are executing an autonomous Nola Goal.\n\
                     Goal ID: {}\n\
                     Title: {}\n\
                     Workspace: {}{}\n\
                     Verify all changes with real commands/tests and call `goal_complete` when done (or `goal_manage(action: \"update\", status: \"needs_user\")` if blocked on user input).",
                    goal.id, goal.title, goal.workspace_root, criteria_block
                ),
            }],
            host_capabilities: HostCapabilities {
                interactive_user_input: false,
                network_allowed: policy.network_allowed,
            },
            policy,
            hooks,
            approval: None,
            max_steps,
            force_tool_set: ForceToolSet::Auto,
            stream: false,
            json_schema: None,
            cancel: Arc::clone(&run_cancel),
            auto_recall_memory: true,
            inbox: None,
        };

        // Renew independently of model/tool events: a single quiet call may outlast the lease.
        let mut deadline = goal.lease_until_ms;
        let result = std::thread::scope(|scope| {
            let (done, wait) = std::sync::mpsc::channel::<()>();
            let mut until = deadline;
            let run_cancel = &run_cancel;
            let cancel = &cancel;
            let heartbeat = scope.spawn(move || {
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                    wait.recv_timeout(Duration::from_millis(250))
                {
                    if cancel.load(Ordering::Acquire) {
                        run_cancel.store(true, Ordering::Release);
                        break;
                    }
                    let ts = now_ms();
                    if until.is_some_and(|end| end - ts > DEFAULT_GOAL_LEASE_MS / 2) {
                        continue;
                    }
                    match Self::renew_owned_lease(
                        &suite.store,
                        goal_id,
                        until,
                        DEFAULT_GOAL_LEASE_MS,
                        ts,
                    ) {
                        Ok(record) => until = record.lease_until_ms,
                        Err(_) => {
                            run_cancel.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
                until
            });
            let result = engine.run(&ctx, &goal.objective, run_cfg, |_| {});
            drop(done);
            deadline = heartbeat.join().expect("goal lease heartbeat panicked");
            result
        });
        let finished = suite
            .store
            .update_module_record(KIND_GOAL, goal_id, |current: &mut GoalRecord| {
                if current.status != GoalStatus::Running || current.lease_until_ms != deadline {
                    return false;
                }
                let ts = now_ms();
                current.lease_until_ms = None;
                current.updated_at_ms = ts;
                match &result {
                    Ok(run) if run.cancelled => current.status = GoalStatus::Paused,
                    Ok(run) if max_steps > 0 && run.steps >= max_steps => {
                        current.status = GoalStatus::Pending;
                        current.next_retry_at_ms = None;
                    }
                    Ok(_) => Self::record_retry(current, "turn ended without goal acceptance", ts),
                    Err(e) => Self::record_retry(current, &e.to_string(), ts),
                }
                if let Ok(run) = &result {
                    let summary: String = run.final_text.chars().take(400).collect();
                    current
                        .progress_notes
                        .push(format!("[steps={}] {summary}", run.steps));
                }
                true
            })
            .map_err(|e| e.to_string())?;
        if let Err(e) = result {
            return Err(e.to_string());
        }
        match finished {
            Some(record) => Ok(record),
            None => suite
                .get_record(KIND_GOAL, goal_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("unknown goal '{goal_id}'")),
        }
    }
}

/// Spawn a durable autonomous background worker thread that polls due/resumable goals,
/// acquires leases, executes turns, and exits promptly when `shutdown` is signaled.
pub fn spawn_background_goal_worker<F>(
    store: Store,
    route_provider: F,
    workspace_root: PathBuf,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()>
where
    F: Fn() -> Option<ModelRouteConfig> + Send + 'static,
{
    std::thread::Builder::new()
        .name("nola-goal-worker".into())
        .spawn(move || {
            let suite = Arc::new(ModulesSuite::new(store.clone(), workspace_root, None));
            while !shutdown.load(Ordering::Relaxed) {
                let runnable = GoalWorker::find_runnable_goals(&store, now_ms());
                if !runnable.is_empty() {
                    if let Some(route) = route_provider() {
                        if let Ok(client) = HttpModelCaller::new(120) {
                            let caller: Arc<dyn ModelCaller> = Arc::new(client);
                            for goal in runnable {
                                if shutdown.load(Ordering::Relaxed) {
                                    break;
                                }
                                let _ = GoalWorker::execute_goal_ext(
                                    &suite,
                                    Arc::clone(&caller),
                                    route.clone(),
                                    EffectivePolicy::default(),
                                    &goal.id,
                                    24,
                                    Arc::clone(&shutdown),
                                );
                            }
                        }
                    }
                }
                // Sleep 5s in 100ms slices for fast shutdown responsiveness.
                for _ in 0..50 {
                    if shutdown.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        })
        .expect("spawn nola-goal-worker thread")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_worker_leases_exponential_backoff_needs_user_and_background_shutdown() {
        assert_eq!(retry_backoff_ms(0), 30_000);
        assert_eq!(retry_backoff_ms(1), 60_000);
        assert_eq!(retry_backoff_ms(2), 120_000);
        assert_eq!(retry_backoff_ms(3), 240_000);
        assert_eq!(retry_backoff_ms(10), MAX_RETRY_BACKOFF_MS);

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let base_ts = 1_700_000_000_000i64;
        let goal = GoalRecord {
            id: "g-lease".into(),
            title: "Durable Goal".into(),
            objective: "Run checks".into(),
            workspace_root: dir.path().display().to_string(),
            status: GoalStatus::Pending,
            progress_notes: Vec::new(),
            created_at_ms: base_ts,
            updated_at_ms: base_ts,
            lease_until_ms: None,
            retry_count: 0,
            next_retry_at_ms: None,
            needs_user_question: None,
            acceptance_criteria: vec!["tests pass".into()],
            accepted: false,
        };
        store.put_module_record(KIND_GOAL, &goal.id, &goal).unwrap();

        let gate = std::sync::Barrier::new(2);
        let acquired = std::thread::scope(|scope| {
            let claim = || {
                gate.wait();
                GoalWorker::try_acquire_lease(&store, "g-lease", 60_000, base_ts).unwrap()
            };
            let other = scope.spawn(claim);
            let claims = [claim(), other.join().unwrap()];
            let mut winners = claims.into_iter().flatten();
            let acquired = winners.next().expect("one worker must acquire the goal");
            assert!(winners.next().is_none());
            acquired
        });
        assert_eq!(acquired.status, GoalStatus::Running);
        assert_eq!(acquired.lease_until_ms, Some(base_ts + 60_000));

        // Active lease prevents duplicate acquisition
        let dup =
            GoalWorker::try_acquire_lease(&store, "g-lease", 60_000, base_ts + 10_000).unwrap();
        assert!(dup.is_none());

        // Renew lease
        let renewed = GoalWorker::renew_lease(&store, "g-lease", 60_000, base_ts + 20_000).unwrap();
        assert_eq!(renewed.lease_until_ms, Some(base_ts + 80_000));

        // Expired lease can be re-acquired
        let reacquired = GoalWorker::try_acquire_lease(&store, "g-lease", 60_000, base_ts + 90_000)
            .unwrap()
            .unwrap();
        assert_eq!(reacquired.lease_until_ms, Some(base_ts + 150_000));

        // Retry backoff schedules next_retry_at_ms = now + 30s
        let retried = GoalWorker::schedule_retry_backoff(
            &store,
            "g-lease",
            "transient timeout",
            base_ts + 100_000,
        )
        .unwrap();
        assert_eq!(retried.retry_count, 1);
        assert_eq!(retried.status, GoalStatus::Pending);
        assert_eq!(retried.next_retry_at_ms, Some(base_ts + 130_000));
        assert!(GoalWorker::find_runnable_goals(&store, base_ts + 110_000).is_empty());
        assert_eq!(
            GoalWorker::find_runnable_goals(&store, base_ts + 130_000).len(),
            1
        );

        // needs_user pause and resume
        let paused =
            GoalWorker::mark_needs_user(&store, "g-lease", "Which target region?").unwrap();
        assert!(paused.is_needs_user());
        assert!(GoalWorker::find_runnable_goals(&store, base_ts + 200_000).is_empty());

        let resumed = GoalWorker::resume_from_user_input(&store, "g-lease", "us-west-2").unwrap();
        assert!(!resumed.is_needs_user());
        assert_eq!(resumed.status, GoalStatus::Pending);
        assert_eq!(
            GoalWorker::find_runnable_goals(&store, base_ts + 200_000).len(),
            1
        );

        // Background worker spawns and shuts down cleanly
        let shutdown = Arc::new(AtomicBool::new(false));
        let handle = spawn_background_goal_worker(
            store,
            || None,
            dir.path().to_path_buf(),
            Arc::clone(&shutdown),
        );
        shutdown.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }
}
