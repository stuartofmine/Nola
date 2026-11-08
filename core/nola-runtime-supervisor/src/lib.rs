// SPDX-License-Identifier: Apache-2.0
//! Process-level supervision for committed Runtime effect work.
//!
//! This crate owns waiting, cancellation and worker lifecycle. It talks only
//! to the Application Service client and never opens or writes canonical Store.

#[cfg(unix)]
pub mod terminal;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound::{Excluded, Included, Unbounded};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nola_protocol::{
    RuntimeEffectWorkRunHead, RuntimeEffectWorkSchedule, RuntimeEffectWorkWaitRequest,
};
use nola_service_client::{
    ApplicationServiceClient, ApplicationServiceRequestCancellation,
    APPLICATION_SERVICE_REQUEST_CANCELLED,
};

pub trait RuntimeEffectWorkWaitClient: Send + Sync + 'static {
    fn wait_for_runtime_effect_work(
        &self,
        request: &RuntimeEffectWorkWaitRequest,
        cancellation: &ApplicationServiceRequestCancellation,
    ) -> Result<RuntimeEffectWorkSchedule, String>;
}

impl RuntimeEffectWorkWaitClient for ApplicationServiceClient {
    fn wait_for_runtime_effect_work(
        &self,
        request: &RuntimeEffectWorkWaitRequest,
        cancellation: &ApplicationServiceRequestCancellation,
    ) -> Result<RuntimeEffectWorkSchedule, String> {
        self.runtime_effect_work_wait_cancellable(request, cancellation)
    }
}

/// Executes independently owned Run heads. Implementations that can block must observe
/// `cancellation` and release owned processes/resources promptly; supervisor
/// shutdown and `Drop` join this worker to prevent orphaned execution.
pub trait RuntimeEffectWorker: Send + Sync + 'static {
    fn run(
        &self,
        work: &RuntimeEffectWorkRunHead,
        cancellation: &RuntimeEffectCancellation,
    ) -> Result<RuntimeEffectWorkerOutcome, String>;
}

/// A successful worker call must say whether it actually drained the durable
/// head. Temporary absence of a matching Host is not completion: it keeps the
/// head eligible after a bounded per-conversation backoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEffectWorkerOutcome {
    Drained,
    RetryableUnavailable {
        reason: String,
    },
    /// The committed Run cannot be reconstructed from the Host configuration
    /// currently available in this process. The durable head remains pending;
    /// the supervisor applies bounded backoff and may retry after registration
    /// supplies an exact frozen configuration match.
    BlockedConfiguration {
        reason: String,
    },
}

impl RuntimeEffectWorkerOutcome {
    pub fn retryable_unavailable(reason: impl Into<String>) -> Self {
        Self::RetryableUnavailable {
            reason: reason.into(),
        }
    }

    pub fn blocked_configuration(reason: impl Into<String>) -> Self {
        Self::BlockedConfiguration {
            reason: reason.into(),
        }
    }
}

struct RuntimeEffectCancellationState {
    cancelled: AtomicBool,
    callback_registered: AtomicBool,
    callback: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

#[derive(Clone)]
pub struct RuntimeEffectCancellation(Arc<RuntimeEffectCancellationState>);

impl Default for RuntimeEffectCancellation {
    fn default() -> Self {
        Self(Arc::new(RuntimeEffectCancellationState {
            cancelled: AtomicBool::new(false),
            callback_registered: AtomicBool::new(false),
            callback: Mutex::new(None),
        }))
    }
}

impl RuntimeEffectCancellation {
    pub fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::AcqRel) {
            let callback = self
                .0
                .callback
                .lock()
                .ok()
                .and_then(|mut callback| callback.take());
            if let Some(callback) = callback {
                callback();
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    /// Registers the one physical-execution abort hook. Registration after
    /// cancellation invokes it immediately; concurrent cancel/register still
    /// invokes it exactly once.
    pub fn on_cancel(&self, callback: Arc<dyn Fn() + Send + Sync>) -> Result<(), &'static str> {
        if self
            .0
            .callback_registered
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("Runtime effect cancellation callback is already registered");
        }
        let mut slot = self
            .0
            .callback
            .lock()
            .map_err(|_| "Runtime effect cancellation callback is poisoned")?;
        if self.is_cancelled() {
            drop(slot);
            callback();
        } else {
            *slot = Some(callback);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSupervisorLifecycle {
    Running,
    Draining,
    Stopped,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeSupervisorConfig {
    /// Bounds physical workers; a conversation still has at most one worker.
    pub maximum_concurrent_runs: usize,
    pub schedule_limit: u32,
    pub long_wait: Duration,
    pub initial_backoff: Duration,
    pub maximum_backoff: Duration,
}

impl Default for RuntimeSupervisorConfig {
    fn default() -> Self {
        Self {
            maximum_concurrent_runs: 4,
            schedule_limit: 64,
            long_wait: Duration::from_secs(30),
            initial_backoff: Duration::from_millis(100),
            maximum_backoff: Duration::from_secs(10),
        }
    }
}

impl RuntimeSupervisorConfig {
    fn validate(self) -> Result<Self, String> {
        if !(1..=64).contains(&self.maximum_concurrent_runs)
            || !(1..=64).contains(&self.schedule_limit)
            || self.long_wait < Duration::from_millis(1)
            || self.long_wait > Duration::from_secs(30)
            || self.initial_backoff.is_zero()
            || self.maximum_backoff < self.initial_backoff
        {
            return Err("invalid Runtime Effect Supervisor configuration".into());
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSupervisorSnapshot {
    pub lifecycle: RuntimeSupervisorLifecycle,
    pub conversation_ids: Vec<String>,
    pub active_runs: Vec<RuntimeSupervisorRun>,
    /// A scheduling observation, not a Core user-input/permission wait.
    pub waiting_for_capacity: Vec<RuntimeSupervisorRun>,
    pub blocked_workers: BTreeMap<String, String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSupervisorRun {
    pub conversation_id: String,
    pub run_id: String,
}

struct ActiveExecution {
    run_id: String,
    cancellation: RuntimeEffectCancellation,
}

struct Backoff {
    failures: u32,
    eligible_at: Instant,
}

struct State {
    lifecycle: RuntimeSupervisorLifecycle,
    conversations: BTreeSet<String>,
    /// Last scope boundary observed, never an execution/admission identity.
    scope_cursor: Option<String>,
    scope_window: Option<usize>,
    dirty: bool,
    active_wait: Option<ApplicationServiceRequestCancellation>,
    active_executions: BTreeMap<String, ActiveExecution>,
    waiting_for_capacity: BTreeMap<String, String>,
    blocked_workers: BTreeMap<String, String>,
    backoff: HashMap<String, Backoff>,
    wait_failures: u32,
    wait_eligible_at: Option<Instant>,
    last_error: Option<String>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl State {
    fn new() -> Self {
        Self {
            lifecycle: RuntimeSupervisorLifecycle::Running,
            conversations: BTreeSet::new(),
            scope_cursor: None,
            scope_window: None,
            dirty: false,
            active_wait: None,
            active_executions: BTreeMap::new(),
            waiting_for_capacity: BTreeMap::new(),
            blocked_workers: BTreeMap::new(),
            backoff: HashMap::new(),
            wait_failures: 0,
            wait_eligible_at: None,
            last_error: None,
        }
    }
}

#[derive(Clone)]
pub struct RuntimeEffectSupervisorHandle(Arc<Shared>);

impl RuntimeEffectSupervisorHandle {
    pub fn register_conversation(
        &self,
        conversation_id: impl Into<String>,
    ) -> Result<bool, String> {
        let conversation_id = conversation_id.into();
        if conversation_id.trim().is_empty()
            || conversation_id.trim() != conversation_id
            || conversation_id.chars().any(char::is_control)
        {
            return Err("Runtime supervisor conversation identity is not canonical".into());
        }
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| "Runtime supervisor state is poisoned")?;
        if state.lifecycle != RuntimeSupervisorLifecycle::Running {
            return Err("Runtime supervisor is not accepting registrations".into());
        }
        let inserted = state.conversations.insert(conversation_id.clone());
        let refreshed = state.blocked_workers.remove(&conversation_id).is_some();
        if inserted || refreshed {
            state.dirty = true;
            cancel_wait(&state);
            self.0.changed.notify_all();
        }
        Ok(inserted)
    }

    pub fn unregister_conversation(&self, conversation_id: &str) -> bool {
        let Ok(mut state) = self.0.state.lock() else {
            return false;
        };
        let removed = state.conversations.remove(conversation_id);
        let mut active_cancellation = None;
        if removed {
            state.backoff.remove(conversation_id);
            state.dirty = true;
            cancel_wait(&state);
            state.waiting_for_capacity.remove(conversation_id);
            state.blocked_workers.remove(conversation_id);
            active_cancellation = state
                .active_executions
                .get(conversation_id)
                .map(|execution| execution.cancellation.clone());
            self.0.changed.notify_all();
        }
        drop(state);
        if let Some(cancellation) = active_cancellation {
            cancellation.cancel();
        }
        removed
    }

    pub fn drain(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            if state.lifecycle == RuntimeSupervisorLifecycle::Running {
                state.lifecycle = RuntimeSupervisorLifecycle::Draining;
                cancel_wait(&state);
                self.0.changed.notify_all();
            }
        }
    }

    pub fn cancel(&self, conversation_id: &str) -> bool {
        let Ok(state) = self.0.state.lock() else {
            return false;
        };
        let cancellation = state
            .active_executions
            .get(conversation_id)
            .map(|execution| execution.cancellation.clone());
        drop(state);
        cancellation.is_some_and(|cancellation| {
            cancellation.cancel();
            true
        })
    }

    pub fn snapshot(&self) -> RuntimeSupervisorSnapshot {
        let state = self.0.state.lock().unwrap();
        RuntimeSupervisorSnapshot {
            lifecycle: state.lifecycle,
            conversation_ids: state.conversations.iter().cloned().collect(),
            active_runs: state
                .active_executions
                .iter()
                .map(|(id, execution)| RuntimeSupervisorRun {
                    conversation_id: id.clone(),
                    run_id: execution.run_id.clone(),
                })
                .collect(),
            waiting_for_capacity: state
                .waiting_for_capacity
                .iter()
                .map(|(id, run)| RuntimeSupervisorRun {
                    conversation_id: id.clone(),
                    run_id: run.clone(),
                })
                .collect(),
            blocked_workers: state.blocked_workers.clone(),
            last_error: state.last_error.clone(),
        }
    }
}

pub struct RuntimeEffectSupervisor {
    handle: RuntimeEffectSupervisorHandle,
    worker: Option<JoinHandle<()>>,
}

impl RuntimeEffectSupervisor {
    pub fn spawn<C, W>(
        client: C,
        worker: W,
        config: RuntimeSupervisorConfig,
    ) -> Result<Self, String>
    where
        C: RuntimeEffectWorkWaitClient,
        W: RuntimeEffectWorker,
    {
        let config = config.validate()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State::new()),
            changed: Condvar::new(),
        });
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("nola-runtime-effect-supervisor".into())
            .spawn(move || run_loop(thread_shared, client, worker, config))
            .map_err(|error| format!("start Runtime Effect Supervisor failed: {error}"))?;
        Ok(Self {
            handle: RuntimeEffectSupervisorHandle(shared),
            worker: Some(thread),
        })
    }

    pub fn handle(&self) -> RuntimeEffectSupervisorHandle {
        self.handle.clone()
    }

    pub fn drain(mut self) -> Result<RuntimeSupervisorSnapshot, String> {
        self.handle.drain();
        self.join()
    }

    pub fn shutdown(mut self) -> Result<RuntimeSupervisorSnapshot, String> {
        let active_cancellation = if let Ok(mut state) = self.handle.0.state.lock() {
            state.lifecycle = RuntimeSupervisorLifecycle::Stopped;
            cancel_wait(&state);
            let cancellation = state
                .active_executions
                .values()
                .map(|execution| execution.cancellation.clone())
                .collect::<Vec<_>>();
            self.handle.0.changed.notify_all();
            cancellation
        } else {
            Vec::new()
        };
        for cancellation in active_cancellation {
            cancellation.cancel();
        }
        self.join()
    }

    fn join(&mut self) -> Result<RuntimeSupervisorSnapshot, String> {
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "Runtime Effect Supervisor worker panicked".to_string())?;
        }
        Ok(self.handle.snapshot())
    }
}

impl Drop for RuntimeEffectSupervisor {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let active_cancellation = if let Ok(mut state) = self.handle.0.state.lock() {
                state.lifecycle = RuntimeSupervisorLifecycle::Stopped;
                cancel_wait(&state);
                let cancellation = state
                    .active_executions
                    .values()
                    .map(|execution| execution.cancellation.clone())
                    .collect::<Vec<_>>();
                self.handle.0.changed.notify_all();
                cancellation
            } else {
                Vec::new()
            };
            for cancellation in active_cancellation {
                cancellation.cancel();
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

fn cancel_wait(state: &State) {
    if let Some(cancellation) = &state.active_wait {
        cancellation.cancel();
    }
}

fn backoff_delay(config: RuntimeSupervisorConfig, failures: u32) -> Duration {
    let multiplier = 1_u32
        .checked_shl(failures.saturating_sub(1).min(30))
        .unwrap_or(u32::MAX);
    config
        .initial_backoff
        .saturating_mul(multiplier)
        .min(config.maximum_backoff)
}

fn wait_milliseconds(duration: Duration) -> u32 {
    duration
        .as_nanos()
        .saturating_add(999_999)
        .checked_div(1_000_000)
        .unwrap_or(u128::MAX)
        .clamp(1, 30_000) as u32
}

fn record_worker_backoff(
    state: &mut State,
    conversation_id: &str,
    error: String,
    config: RuntimeSupervisorConfig,
) {
    let failures = state
        .backoff
        .get(conversation_id)
        .map_or(1, |backoff| backoff.failures.saturating_add(1));
    state.backoff.insert(
        conversation_id.to_string(),
        Backoff {
            failures,
            eligible_at: Instant::now() + backoff_delay(config, failures),
        },
    );
    state.last_error = Some(error);
}

/// Keep query scopes bounded without bounding the durable owner registry.
/// Stable keyset ranges tolerate an unregistered cursor and newly inserted IDs.
fn next_scope(state: &State, now: Instant, limit: usize) -> (Vec<String>, bool) {
    let cursor = state.scope_cursor.as_deref().unwrap_or("");
    let mut scope = state
        .conversations
        .range::<str, _>((Excluded(cursor), Unbounded))
        .chain(
            state
                .conversations
                .range::<str, _>((Unbounded, Included(cursor))),
        )
        .filter(|id| {
            !state.active_executions.contains_key(*id)
                && !state.blocked_workers.contains_key(*id)
                && state
                    .backoff
                    .get(*id)
                    .is_none_or(|backoff| backoff.eligible_at <= now)
        })
        .take(limit + 1)
        .cloned()
        .collect::<Vec<_>>();
    let more = scope.len() > limit;
    scope.truncate(limit);
    (scope, more)
}

fn advance_scope(
    state: &mut State,
    request: &RuntimeEffectWorkWaitRequest,
    exhausted: Option<bool>,
) {
    // Saturation may hide another owner's head behind one owner's many Runs.
    // Subdivide before advancing; aborts move on without assuming exhaustion.
    // A sparse response proves this whole batch was inspected and can skip it.
    if exhausted == Some(false) && request.conversation_ids.len() > 1 {
        // Subdivide from the same boundary: advancing first would skip the
        // very owner hidden by the saturated result. Even a scope containing
        // fewer than 64 owners must eventually query each owner alone.
        state.scope_window = Some((request.conversation_ids.len() / 2).max(1));
        return;
    }
    if exhausted == Some(true) {
        state.scope_window = None;
    }
    state.scope_cursor = if exhausted == Some(true) {
        request.conversation_ids.last()
    } else {
        request.conversation_ids.first()
    }
    .cloned();
}

fn run_loop<C: RuntimeEffectWorkWaitClient, W: RuntimeEffectWorker>(
    shared: Arc<Shared>,
    client: C,
    worker: W,
    config: RuntimeSupervisorConfig,
) {
    let worker = Arc::new(worker);
    let mut executions: Vec<JoinHandle<()>> = Vec::new();
    'scheduling: loop {
        let (finished, pending): (Vec<_>, Vec<_>) = executions
            .into_iter()
            .partition(|execution| execution.is_finished());
        executions = pending;
        for execution in finished {
            let _ = execution.join();
        }
        let (scope, wait_duration, cancellation) = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if state.lifecycle != RuntimeSupervisorLifecycle::Running {
                    break 'scheduling;
                }
                let now = Instant::now();
                let wait_backoff = state.wait_eligible_at.filter(|deadline| *deadline > now);
                let scope_limit = state
                    .scope_window
                    .unwrap_or(config.schedule_limit as usize)
                    .min(config.schedule_limit as usize);
                let (eligible, more_scopes) = next_scope(&state, now, scope_limit);
                if !eligible.is_empty() && wait_backoff.is_none() {
                    let next_excluded_eligibility = state
                        .backoff
                        .iter()
                        .filter(|(id, backoff)| {
                            state.conversations.contains(*id) && backoff.eligible_at > now
                        })
                        .map(|(_, backoff)| backoff.eligible_at)
                        .min();
                    let mut wait_duration = next_excluded_eligibility
                        .map(|deadline| {
                            config
                                .long_wait
                                .min(deadline.saturating_duration_since(now))
                        })
                        .unwrap_or(config.long_wait);
                    if more_scopes {
                        // The service can only observe this batch. An idle
                        // first batch must not long-poll ahead of ready tails.
                        wait_duration = wait_duration.min(Duration::from_millis(200));
                    }
                    let cancellation = ApplicationServiceRequestCancellation::new();
                    state.active_wait = Some(cancellation.clone());
                    state.dirty = false;
                    break (eligible, wait_duration, cancellation);
                }
                let next = wait_backoff
                    .into_iter()
                    .chain(
                        state
                            .backoff
                            .values()
                            .map(|backoff| backoff.eligible_at)
                            .filter(|at| *at > now),
                    )
                    .min();
                state = match next {
                    Some(deadline) => {
                        shared
                            .changed
                            .wait_timeout(state, deadline.saturating_duration_since(now))
                            .unwrap()
                            .0
                    }
                    None => shared.changed.wait(state).unwrap(),
                };
            }
        };
        let request = RuntimeEffectWorkWaitRequest {
            conversation_ids: scope,
            limit: config.schedule_limit,
            wait_ms: wait_milliseconds(wait_duration),
        };
        let result = client
            .wait_for_runtime_effect_work(&request, &cancellation)
            .and_then(|schedule| {
                schedule
                    .validate_for_request(&request)
                    .map_err(|error| format!("invalid Runtime effect-work schedule: {error}"))?;
                Ok(schedule)
            });
        let changed_while_waiting = {
            let mut state = shared.state.lock().unwrap();
            state.active_wait = None;
            advance_scope(
                &mut state,
                &request,
                result
                    .as_ref()
                    .ok()
                    .map(|schedule| schedule.runs.len() < request.limit as usize),
            );
            std::mem::take(&mut state.dirty)
        };
        if changed_while_waiting {
            continue;
        }
        let schedule = match result {
            Ok(schedule) => {
                let mut state = shared.state.lock().unwrap();
                state.wait_failures = 0;
                state.wait_eligible_at = None;
                schedule
            }
            Err(error) if error == APPLICATION_SERVICE_REQUEST_CANCELLED => continue,
            Err(error) => {
                let mut state = shared.state.lock().unwrap();
                state.wait_failures = state.wait_failures.saturating_add(1);
                state.wait_eligible_at =
                    Some(Instant::now() + backoff_delay(config, state.wait_failures));
                state.last_error = Some(error);
                continue;
            }
        };
        for work in schedule.runs {
            let execution_cancellation = {
                let mut state = shared.state.lock().unwrap();
                loop {
                    if state.lifecycle != RuntimeSupervisorLifecycle::Running {
                        state.waiting_for_capacity.clear();
                        break 'scheduling;
                    }
                    if !state.conversations.contains(&work.conversation_id)
                        || state.active_executions.contains_key(&work.conversation_id)
                        || state.blocked_workers.contains_key(&work.conversation_id)
                    {
                        break;
                    }
                    if state.active_executions.len() < config.maximum_concurrent_runs {
                        break;
                    }
                    state
                        .waiting_for_capacity
                        .insert(work.conversation_id.clone(), work.run_id.clone());
                    state = shared.changed.wait(state).unwrap();
                }
                state.waiting_for_capacity.remove(&work.conversation_id);
                if !state.conversations.contains(&work.conversation_id)
                    || state.active_executions.contains_key(&work.conversation_id)
                    || state.blocked_workers.contains_key(&work.conversation_id)
                {
                    continue;
                }
                let cancellation = RuntimeEffectCancellation::default();
                state.active_executions.insert(
                    work.conversation_id.clone(),
                    ActiveExecution {
                        run_id: work.run_id.clone(),
                        cancellation: cancellation.clone(),
                    },
                );
                cancellation
            };
            let execution_shared = Arc::clone(&shared);
            let execution_worker = Arc::clone(&worker);
            let conversation_id = work.conversation_id.clone();
            match std::thread::Builder::new().name("nola-runtime-effect-run".into()).spawn(move || {
                // A worker panic may follow an external side effect. Quarantine
                // this owner until explicit re-registration; never retry it as
                // though an ordinary unavailable Host proved nothing happened.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    execution_worker.run(&work, &execution_cancellation)
                }));
                let mut state = execution_shared.state.lock().unwrap();
                if state.active_executions.get(&work.conversation_id)
                    .is_some_and(|active| active.run_id == work.run_id)
                {
                    state.active_executions.remove(&work.conversation_id);
                }
                match outcome {
                    Ok(Ok(RuntimeEffectWorkerOutcome::Drained)) => {
                        state.backoff.remove(&work.conversation_id);
                    }
                    Ok(Ok(RuntimeEffectWorkerOutcome::RetryableUnavailable { reason }
                        | RuntimeEffectWorkerOutcome::BlockedConfiguration { reason })) => {
                        record_worker_backoff(&mut state, &work.conversation_id, reason, config);
                    }
                    Ok(Err(error)) => {
                        record_worker_backoff(&mut state, &work.conversation_id, error, config);
                    }
                    Err(_) => {
                        let reason = "Runtime effect worker panicked; execution outcome requires reconciliation".to_string();
                        state.blocked_workers.insert(work.conversation_id.clone(), reason.clone());
                        state.last_error = Some(reason);
                    }
                }
                state.dirty = true;
                cancel_wait(&state);
                execution_shared.changed.notify_all();
            }) {
                Ok(execution) => executions.push(execution),
                Err(error) => {
                    let mut state = shared.state.lock().unwrap();
                    state.active_executions.remove(&conversation_id);
                    record_worker_backoff(&mut state, &conversation_id,
                        format!("start Runtime effect worker failed: {error}"), config);
                    shared.changed.notify_all();
                }
            }
        }
    }
    // Both drain and shutdown own all physical workers until they exit.
    for execution in executions {
        let _ = execution.join();
    }
    let mut state = shared.state.lock().unwrap();
    state.waiting_for_capacity.clear();
    state.lifecycle = RuntimeSupervisorLifecycle::Stopped;
    shared.changed.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn registry_above_64_is_queried_in_rotating_bounded_batches() {
        let mut state = State::new();
        state.conversations = (0..141).map(|index| format!("owner-{index:03}")).collect();
        let mut observed = BTreeSet::new();
        for _ in 0..3 {
            let (scope, more) = next_scope(&state, Instant::now(), 64);
            assert_eq!(scope.len(), 64);
            assert!(more);
            observed.extend(scope.iter().cloned());
            let request = RuntimeEffectWorkWaitRequest {
                conversation_ids: scope,
                limit: 64,
                wait_ms: 200,
            };
            request.validate().unwrap();
            advance_scope(&mut state, &request, Some(true));
        }
        assert_eq!(observed, state.conversations);
        let removed_cursor = state.scope_cursor.clone().unwrap();
        state.conversations.remove(&removed_cursor);
        assert_eq!(next_scope(&state, Instant::now(), 64).0.len(), 64);
    }

    #[test]
    fn saturation_subdivides_without_skipping_hidden_owners() {
        let mut state = State::new();
        state.conversations = ["a", "b", "busy", "z"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut serviced = BTreeSet::new();
        for _ in 0..32 {
            let (scope, _) = next_scope(&state, Instant::now(), state.scope_window.unwrap_or(64));
            let saturated = scope.iter().any(|id| id == "busy");
            if !saturated || scope.len() == 1 {
                serviced.extend(scope.iter().cloned());
            }
            let request = RuntimeEffectWorkWaitRequest {
                conversation_ids: scope,
                limit: 64,
                wait_ms: 200,
            };
            advance_scope(&mut state, &request, Some(!saturated));
            if serviced == state.conversations {
                break;
            }
        }
        assert_eq!(
            serviced, state.conversations,
            "one owner with 64 heads must not hide the other owners forever"
        );
    }

    #[test]
    fn cancellation_and_small_schedule_limits_do_not_restart_at_the_first_owner() {
        let mut state = State::new();
        state.conversations = (0..130).map(|index| format!("owner-{index:03}")).collect();
        let mut firsts = BTreeSet::new();
        for _ in 0..130 {
            let (scope, _) = next_scope(&state, Instant::now(), 3);
            assert_eq!(scope.len(), 3);
            firsts.insert(scope[0].clone());
            let request = RuntimeEffectWorkWaitRequest {
                conversation_ids: scope,
                limit: 3,
                wait_ms: 200,
            };
            advance_scope(&mut state, &request, None);
        }
        assert_eq!(firsts, state.conversations);
    }

    #[test]
    fn rotating_scopes_keep_active_quarantined_and_backed_off_owners_excluded() {
        let now = Instant::now();
        let mut state = State::new();
        state.conversations = (0..132).map(|index| format!("owner-{index:03}")).collect();
        state.active_executions.insert(
            "owner-000".into(),
            ActiveExecution {
                run_id: "run-active".into(),
                cancellation: RuntimeEffectCancellation::default(),
            },
        );
        state
            .blocked_workers
            .insert("owner-001".into(), "unknown outcome".into());
        state.backoff.insert(
            "owner-002".into(),
            Backoff {
                failures: 1,
                eligible_at: now + Duration::from_secs(1),
            },
        );
        let mut observed = BTreeSet::new();
        for _ in 0..3 {
            let (scope, _) = next_scope(&state, now, 64);
            observed.extend(scope.iter().cloned());
            advance_scope(
                &mut state,
                &RuntimeEffectWorkWaitRequest {
                    conversation_ids: scope,
                    limit: 64,
                    wait_ms: 200,
                },
                Some(true),
            );
        }
        assert_eq!(observed.len(), 129);
        assert!(!observed.contains("owner-000"));
        assert!(!observed.contains("owner-001"));
        assert!(!observed.contains("owner-002"));
        state.scope_cursor = None;
        assert!(next_scope(&state, now + Duration::from_secs(2), 64)
            .0
            .contains(&"owner-002".to_string()));
    }

    fn serial_config() -> RuntimeSupervisorConfig {
        RuntimeSupervisorConfig {
            maximum_concurrent_runs: 1,
            ..Default::default()
        }
    }

    struct ChannelClient {
        calls: mpsc::Sender<RuntimeEffectWorkWaitRequest>,
        responses: Mutex<mpsc::Receiver<Result<RuntimeEffectWorkSchedule, String>>>,
    }

    impl RuntimeEffectWorkWaitClient for ChannelClient {
        fn wait_for_runtime_effect_work(
            &self,
            request: &RuntimeEffectWorkWaitRequest,
            cancellation: &ApplicationServiceRequestCancellation,
        ) -> Result<RuntimeEffectWorkSchedule, String> {
            self.calls.send(request.clone()).unwrap();
            let deadline = Instant::now() + Duration::from_millis(u64::from(request.wait_ms));
            loop {
                if cancellation.is_cancelled() {
                    return Err(APPLICATION_SERVICE_REQUEST_CANCELLED.into());
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(RuntimeEffectWorkSchedule {
                        observed_at_ms: 100,
                        runs: Vec::new(),
                        next_wake_at_ms: None,
                    });
                }
                match self
                    .responses
                    .lock()
                    .unwrap()
                    .recv_timeout(remaining.min(Duration::from_millis(5)))
                {
                    Ok(response) => return response,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err("fixture response channel disconnected".into())
                    }
                }
            }
        }
    }

    fn schedule(
        _request: &RuntimeEffectWorkWaitRequest,
        conversations: &[&str],
    ) -> RuntimeEffectWorkSchedule {
        let observed_at_ms = 100;
        RuntimeEffectWorkSchedule {
            observed_at_ms,
            runs: conversations
                .iter()
                .enumerate()
                .map(|(index, conversation)| RuntimeEffectWorkRunHead {
                    conversation_id: (*conversation).into(),
                    run_id: format!("run-{conversation}"),
                    effect_id: format!("effect-{conversation}"),
                    request_id: format!("request-{conversation}"),
                    attempt_id: None,
                    reason: nola_protocol::RuntimeEffectWorkReason::Pending,
                    ready_at_ms: index as i64,
                    created_at_ms: index as i64,
                })
                .collect(),
            next_wake_at_ms: None,
        }
    }

    fn fixture() -> (
        ChannelClient,
        mpsc::Receiver<RuntimeEffectWorkWaitRequest>,
        mpsc::Sender<Result<RuntimeEffectWorkSchedule, String>>,
    ) {
        let (call_tx, call_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        (
            ChannelClient {
                calls: call_tx,
                responses: Mutex::new(response_rx),
            },
            call_rx,
            response_tx,
        )
    }

    #[test]
    fn exponential_backoff_is_bounded() {
        let config = RuntimeSupervisorConfig {
            initial_backoff: Duration::from_millis(10),
            maximum_backoff: Duration::from_millis(35),
            ..serial_config()
        };
        assert_eq!(backoff_delay(config, 1), Duration::from_millis(10));
        assert_eq!(backoff_delay(config, 2), Duration::from_millis(20));
        assert_eq!(backoff_delay(config, 3), Duration::from_millis(35));
        assert_eq!(backoff_delay(config, 30), Duration::from_millis(35));
        assert_eq!(wait_milliseconds(Duration::from_nanos(1)), 1);
        assert_eq!(wait_milliseconds(Duration::from_micros(1_001)), 2);
    }

    #[test]
    fn invalid_configuration_is_rejected_before_starting_a_thread() {
        struct Client;
        impl RuntimeEffectWorkWaitClient for Client {
            fn wait_for_runtime_effect_work(
                &self,
                _: &RuntimeEffectWorkWaitRequest,
                _: &ApplicationServiceRequestCancellation,
            ) -> Result<RuntimeEffectWorkSchedule, String> {
                unreachable!()
            }
        }
        struct Worker;
        impl RuntimeEffectWorker for Worker {
            fn run(
                &self,
                _: &RuntimeEffectWorkRunHead,
                _: &RuntimeEffectCancellation,
            ) -> Result<RuntimeEffectWorkerOutcome, String> {
                unreachable!()
            }
        }
        assert!(RuntimeEffectSupervisor::spawn(
            Client,
            Worker,
            RuntimeSupervisorConfig {
                schedule_limit: 0,
                ..serial_config()
            }
        )
        .is_err());
        assert!(RuntimeEffectSupervisor::spawn(
            Client,
            Worker,
            RuntimeSupervisorConfig {
                long_wait: Duration::from_micros(999),
                ..serial_config()
            }
        )
        .is_err());
    }

    #[test]
    fn cancellation_callback_runs_exactly_once_even_when_registered_after_cancel() {
        use std::sync::atomic::AtomicUsize;
        let before = RuntimeEffectCancellation::default();
        let before_calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&before_calls);
        before
            .on_cancel(Arc::new(move || {
                observed.fetch_add(1, Ordering::SeqCst);
            }))
            .unwrap();
        before.cancel();
        before.cancel();
        assert_eq!(before_calls.load(Ordering::SeqCst), 1);
        assert!(before.on_cancel(Arc::new(|| {})).is_err());

        let after = RuntimeEffectCancellation::default();
        after.cancel();
        let after_calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&after_calls);
        after
            .on_cancel(Arc::new(move || {
                observed.fetch_add(1, Ordering::SeqCst);
            }))
            .unwrap();
        assert_eq!(after_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dirty_registration_cancels_the_old_wait_and_reissues_one_bounded_scope() {
        struct Worker;
        impl RuntimeEffectWorker for Worker {
            fn run(
                &self,
                _: &RuntimeEffectWorkRunHead,
                _: &RuntimeEffectCancellation,
            ) -> Result<RuntimeEffectWorkerOutcome, String> {
                Ok(RuntimeEffectWorkerOutcome::Drained)
            }
        }
        let (client, calls, _responses) = fixture();
        let supervisor = RuntimeEffectSupervisor::spawn(
            client,
            Worker,
            RuntimeSupervisorConfig {
                long_wait: Duration::from_secs(1),
                ..serial_config()
            },
        )
        .unwrap();
        let handle = supervisor.handle();
        handle.register_conversation("a").unwrap();
        let first = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(first.conversation_ids, ["a"]);
        handle.register_conversation("b").unwrap();
        let second = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(second.conversation_ids.iter().map(String::as_str).collect::<BTreeSet<_>>(), ["a", "b"].into_iter().collect());
        assert!(supervisor.shutdown().unwrap().conversation_ids.len() == 2);
    }

    #[test]
    fn idle_supervisor_does_not_open_a_long_wait_and_schedule_pages_are_bounded() {
        struct Worker;
        impl RuntimeEffectWorker for Worker {
            fn run(
                &self,
                _: &RuntimeEffectWorkRunHead,
                _: &RuntimeEffectCancellation,
            ) -> Result<RuntimeEffectWorkerOutcome, String> {
                Ok(RuntimeEffectWorkerOutcome::Drained)
            }
        }
        let (client, calls, _responses) = fixture();
        let supervisor = RuntimeEffectSupervisor::spawn(client, Worker, serial_config()).unwrap();
        assert!(calls.recv_timeout(Duration::from_millis(30)).is_err());
        let handle = supervisor.handle();
        for index in 0..64 {
            handle
                .register_conversation(format!("conversation-{index:02}"))
                .unwrap();
        }
        handle.register_conversation("conversation-64").unwrap();
        for invalid in ["", " conversation", "conversation ", "conversation\n"] {
            assert!(handle.register_conversation(invalid).is_err());
        }
        assert_eq!(handle.snapshot().conversation_ids.len(), 65);
        let request = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!request.conversation_ids.is_empty() && request.conversation_ids.len() <= 64);
        // This first request may precede the remaining registrations. The
        // multi-page rotation tests separately assert the 200 ms wait bound.
        supervisor.shutdown().unwrap();
    }

    #[test]
    fn one_worker_serializes_heads_and_drain_finishes_inflight_work() {
        struct Worker {
            started: mpsc::Sender<String>,
            releases: Mutex<mpsc::Receiver<()>>,
        }
        impl RuntimeEffectWorker for Worker {
            fn run(
                &self,
                work: &RuntimeEffectWorkRunHead,
                _: &RuntimeEffectCancellation,
            ) -> Result<RuntimeEffectWorkerOutcome, String> {
                self.started.send(work.conversation_id.clone()).unwrap();
                self.releases.lock().unwrap().recv().unwrap();
                Ok(RuntimeEffectWorkerOutcome::Drained)
            }
        }
        let (client, calls, responses) = fixture();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let supervisor = RuntimeEffectSupervisor::spawn(
            client,
            Worker {
                started: started_tx,
                releases: Mutex::new(release_rx),
            },
            serial_config(),
        )
        .unwrap();
        let handle = supervisor.handle();
        handle.register_conversation("a").unwrap();
        handle.register_conversation("b").unwrap();
        let request = loop {
            let request = calls.recv_timeout(Duration::from_secs(1)).unwrap();
            if request.conversation_ids.len() == 2 {
                break request;
            }
        };
        responses.send(Ok(schedule(&request, &["a", "b"]))).unwrap();
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "a"
        );
        assert!(started_rx.recv_timeout(Duration::from_millis(30)).is_err());
        handle.drain();
        release_tx.send(()).unwrap();
        let snapshot = supervisor.drain().unwrap();
        assert_eq!(snapshot.lifecycle, RuntimeSupervisorLifecycle::Stopped);
        assert!(
            started_rx.try_recv().is_err(),
            "drain started another scheduled head"
        );
    }

    #[test]
    fn unregister_cancels_only_the_matching_inflight_worker() {
        struct Worker(mpsc::Sender<bool>);
        impl RuntimeEffectWorker for Worker {
            fn run(
                &self,
                _: &RuntimeEffectWorkRunHead,
                cancellation: &RuntimeEffectCancellation,
            ) -> Result<RuntimeEffectWorkerOutcome, String> {
                while !cancellation.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(2));
                }
                self.0.send(true).unwrap();
                Ok(RuntimeEffectWorkerOutcome::Drained)
            }
        }
        let (client, calls, responses) = fixture();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let supervisor =
            RuntimeEffectSupervisor::spawn(client, Worker(cancelled_tx), serial_config()).unwrap();
        let handle = supervisor.handle();
        handle.register_conversation("target").unwrap();
        handle.register_conversation("other").unwrap();
        let request = loop {
            let request = calls.recv_timeout(Duration::from_secs(1)).unwrap();
            if request.conversation_ids.len() == 2 {
                break request;
            }
        };
        responses.send(Ok(schedule(&request, &["target"]))).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !handle
            .snapshot()
            .active_runs
            .iter()
            .any(|run| run.conversation_id == "target")
        {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(handle.unregister_conversation("target"));
        assert!(cancelled_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        assert!(handle.snapshot().conversation_ids.contains(&"other".into()));
        supervisor.shutdown().unwrap();
    }

    #[test]
    fn worker_failure_backs_off_only_its_conversation() {
        struct Worker(mpsc::Sender<String>);
        impl RuntimeEffectWorker for Worker {
            fn run(
                &self,
                work: &RuntimeEffectWorkRunHead,
                _: &RuntimeEffectCancellation,
            ) -> Result<RuntimeEffectWorkerOutcome, String> {
                self.0.send(work.conversation_id.clone()).unwrap();
                if work.conversation_id == "a" {
                    Err("a failed".into())
                } else {
                    Ok(RuntimeEffectWorkerOutcome::Drained)
                }
            }
        }
        let (client, calls, responses) = fixture();
        let (worked_tx, worked_rx) = mpsc::channel();
        let supervisor = RuntimeEffectSupervisor::spawn(
            client,
            Worker(worked_tx),
            RuntimeSupervisorConfig {
                initial_backoff: Duration::from_millis(100),
                maximum_backoff: Duration::from_millis(100),
                ..serial_config()
            },
        )
        .unwrap();
        let handle = supervisor.handle();
        handle.register_conversation("a").unwrap();
        handle.register_conversation("b").unwrap();
        let request = loop {
            let request = calls.recv_timeout(Duration::from_secs(1)).unwrap();
            if request.conversation_ids.len() == 2 {
                break request;
            }
        };
        responses.send(Ok(schedule(&request, &["a", "b"]))).unwrap();
        assert_eq!(worked_rx.recv_timeout(Duration::from_secs(1)).unwrap(), "a");
        assert_eq!(worked_rx.recv_timeout(Duration::from_secs(1)).unwrap(), "b");
        let next = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(next.conversation_ids, ["b"]);
        assert!(
            next.wait_ms <= 100,
            "backed-off conversation did not bound the wait"
        );
        let after_backoff = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(after_backoff.conversation_ids, ["a", "b"]);
        assert_eq!(handle.snapshot().last_error.as_deref(), Some("a failed"));
        supervisor.shutdown().unwrap();
    }

    #[test]
    fn retryable_unavailable_is_backed_off_without_blocking_other_conversations() {
        struct Worker(mpsc::Sender<String>);
        impl RuntimeEffectWorker for Worker {
            fn run(
                &self,
                work: &RuntimeEffectWorkRunHead,
                _: &RuntimeEffectCancellation,
            ) -> Result<RuntimeEffectWorkerOutcome, String> {
                self.0.send(work.conversation_id.clone()).unwrap();
                if work.conversation_id == "unavailable" {
                    Ok(RuntimeEffectWorkerOutcome::retryable_unavailable(
                        "warm Host is unavailable",
                    ))
                } else {
                    Ok(RuntimeEffectWorkerOutcome::Drained)
                }
            }
        }
        let (client, calls, responses) = fixture();
        let (worked_tx, worked_rx) = mpsc::channel();
        let supervisor = RuntimeEffectSupervisor::spawn(
            client,
            Worker(worked_tx),
            RuntimeSupervisorConfig {
                long_wait: Duration::from_secs(1),
                initial_backoff: Duration::from_millis(250),
                maximum_backoff: Duration::from_millis(250),
                ..serial_config()
            },
        )
        .unwrap();
        let handle = supervisor.handle();
        handle.register_conversation("unavailable").unwrap();
        handle.register_conversation("healthy").unwrap();
        let request = loop {
            let request = calls.recv_timeout(Duration::from_secs(1)).unwrap();
            if request.conversation_ids.len() == 2 {
                break request;
            }
        };
        responses
            .send(Ok(schedule(&request, &["unavailable", "healthy"])))
            .unwrap();

        assert_eq!(
            worked_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "unavailable"
        );
        assert_eq!(
            worked_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "healthy",
            "a temporarily unavailable Host must not stop another conversation"
        );
        let during_backoff = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(during_backoff.conversation_ids, ["healthy"]);
        assert!(during_backoff.wait_ms <= 250);
        assert!(
            calls.recv_timeout(Duration::from_millis(100)).is_err(),
            "the unavailable conversation caused a busy schedule loop"
        );
        let after_backoff = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(after_backoff.conversation_ids.iter().map(String::as_str).collect::<BTreeSet<_>>(), ["healthy", "unavailable"].into_iter().collect());
        assert_eq!(
            handle.snapshot().last_error.as_deref(),
            Some("warm Host is unavailable")
        );
        supervisor.shutdown().unwrap();
    }

    #[test]
    fn shutdown_cancels_and_joins_the_inflight_worker() {
        struct Worker(mpsc::Sender<bool>);
        impl RuntimeEffectWorker for Worker {
            fn run(
                &self,
                _: &RuntimeEffectWorkRunHead,
                cancellation: &RuntimeEffectCancellation,
            ) -> Result<RuntimeEffectWorkerOutcome, String> {
                while !cancellation.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(2));
                }
                self.0.send(true).unwrap();
                Ok(RuntimeEffectWorkerOutcome::Drained)
            }
        }
        let (client, calls, responses) = fixture();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let supervisor =
            RuntimeEffectSupervisor::spawn(client, Worker(cancelled_tx), serial_config()).unwrap();
        supervisor.handle().register_conversation("target").unwrap();
        let request = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        responses.send(Ok(schedule(&request, &["target"]))).unwrap();
        let handle = supervisor.handle();
        let deadline = Instant::now() + Duration::from_secs(1);
        while handle.snapshot().active_runs.is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let snapshot = supervisor.shutdown().unwrap();
        assert!(cancelled_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        assert_eq!(snapshot.lifecycle, RuntimeSupervisorLifecycle::Stopped);
        assert!(snapshot.active_runs.is_empty());
    }

    struct CancellableWorker {
        started: mpsc::Sender<String>,
        stopped: mpsc::Sender<String>,
    }

    struct TailReadyClient {
        target: String,
        calls: mpsc::Sender<RuntimeEffectWorkWaitRequest>,
    }

    impl RuntimeEffectWorkWaitClient for TailReadyClient {
        fn wait_for_runtime_effect_work(
            &self,
            request: &RuntimeEffectWorkWaitRequest,
            cancellation: &ApplicationServiceRequestCancellation,
        ) -> Result<RuntimeEffectWorkSchedule, String> {
            self.calls.send(request.clone()).unwrap();
            if request.conversation_ids.contains(&self.target) {
                return Ok(schedule(request, &[self.target.as_str()]));
            }
            let deadline = Instant::now() + Duration::from_millis(u64::from(request.wait_ms));
            while Instant::now() < deadline {
                if cancellation.is_cancelled() {
                    return Err(APPLICATION_SERVICE_REQUEST_CANCELLED.into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(schedule(request, &[]))
        }
    }

    #[test]
    fn idle_prefix_above_64_cannot_hold_ready_tail_behind_a_30_second_wait() {
        let (calls_tx, calls_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let target = "owner-139".to_string();
        let supervisor = RuntimeEffectSupervisor::spawn(
            TailReadyClient {
                target: target.clone(),
                calls: calls_tx,
            },
            CancellableWorker {
                started: started_tx,
                stopped: stopped_tx,
            },
            RuntimeSupervisorConfig {
                maximum_concurrent_runs: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let handle = supervisor.handle();
        for index in 0..140 {
            handle
                .register_conversation(format!("owner-{index:03}"))
                .unwrap();
        }
        assert_eq!(handle.snapshot().conversation_ids.len(), 140);
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            target
        );
        let calls = calls_rx.try_iter().collect::<Vec<_>>();
        assert!(calls
            .iter()
            .all(|request| request.conversation_ids.len() <= 64));
        assert!(calls
            .iter()
            .any(|request| request.conversation_ids.contains(&target) && request.wait_ms <= 200));
        assert!(handle.snapshot().active_runs.len() <= 2);
        supervisor.shutdown().unwrap();
        assert_eq!(
            stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            target
        );
    }

    impl RuntimeEffectWorker for CancellableWorker {
        fn run(
            &self,
            work: &RuntimeEffectWorkRunHead,
            cancellation: &RuntimeEffectCancellation,
        ) -> Result<RuntimeEffectWorkerOutcome, String> {
            self.started.send(work.conversation_id.clone()).unwrap();
            while !cancellation.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.stopped.send(work.conversation_id.clone()).unwrap();
            Ok(RuntimeEffectWorkerOutcome::Drained)
        }
    }

    fn register_pair(
        handle: &RuntimeEffectSupervisorHandle,
        calls: &mpsc::Receiver<RuntimeEffectWorkWaitRequest>,
    ) -> RuntimeEffectWorkWaitRequest {
        handle.register_conversation("a").unwrap();
        handle.register_conversation("b").unwrap();
        loop {
            let request = calls.recv_timeout(Duration::from_secs(1)).unwrap();
            if request.conversation_ids == ["a", "b"] {
                return request;
            }
        }
    }

    #[test]
    fn independent_run_workers_start_before_either_finishes_and_cancel_is_scoped() {
        let (client, calls, responses) = fixture();
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let supervisor = RuntimeEffectSupervisor::spawn(
            client,
            CancellableWorker {
                started: started_tx,
                stopped: stopped_tx,
            },
            RuntimeSupervisorConfig {
                maximum_concurrent_runs: 2,
                ..serial_config()
            },
        )
        .unwrap();
        let handle = supervisor.handle();
        let request = register_pair(&handle, &calls);
        responses.send(Ok(schedule(&request, &["a", "b"]))).unwrap();
        let mut started = vec![
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        ];
        started.sort();
        assert_eq!(started, ["a", "b"]);
        assert_eq!(handle.snapshot().active_runs.len(), 2);
        assert!(handle.cancel("a"));
        assert_eq!(
            stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "a"
        );
        assert!(stopped_rx.recv_timeout(Duration::from_millis(20)).is_err());
        let stopped = supervisor.shutdown().unwrap();
        assert_eq!(
            stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "b"
        );
        assert!(stopped.active_runs.is_empty());
    }

    #[test]
    fn capacity_wait_is_observable_and_releases_after_the_original_worker_stops() {
        let (client, calls, responses) = fixture();
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let supervisor = RuntimeEffectSupervisor::spawn(
            client,
            CancellableWorker {
                started: started_tx,
                stopped: stopped_tx,
            },
            serial_config(),
        )
        .unwrap();
        let handle = supervisor.handle();
        let request = register_pair(&handle, &calls);
        responses.send(Ok(schedule(&request, &["a", "b"]))).unwrap();
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "a"
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let snapshot = handle.snapshot();
            if snapshot
                .waiting_for_capacity
                .iter()
                .any(|run| run.conversation_id == "b" && run.run_id == "run-b")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "missing resource wait observation"
            );
            std::thread::yield_now();
        }
        assert!(started_rx.try_recv().is_err());
        assert!(handle.cancel("a"));
        assert_eq!(
            stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "a"
        );
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "b"
        );
        supervisor.shutdown().unwrap();
        assert_eq!(
            stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "b"
        );
    }

    #[test]
    fn shutdown_cancels_every_owned_worker_before_returning() {
        let (client, calls, responses) = fixture();
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let supervisor = RuntimeEffectSupervisor::spawn(
            client,
            CancellableWorker {
                started: started_tx,
                stopped: stopped_tx,
            },
            RuntimeSupervisorConfig {
                maximum_concurrent_runs: 2,
                ..serial_config()
            },
        )
        .unwrap();
        let handle = supervisor.handle();
        let request = register_pair(&handle, &calls);
        responses.send(Ok(schedule(&request, &["a", "b"]))).unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let snapshot = supervisor.shutdown().unwrap();
        let mut stopped = vec![
            stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        ];
        stopped.sort();
        assert_eq!(stopped, ["a", "b"]);
        assert_eq!(snapshot.lifecycle, RuntimeSupervisorLifecycle::Stopped);
        assert!(snapshot.active_runs.is_empty());
    }
}
