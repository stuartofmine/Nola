// SPDX-License-Identifier: Apache-2.0
//! Execution identity and cooperative cancellation, independent of terminal UI,
//! permission handling, provider transport and durable input queues.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

static EXECUTION_SCOPE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// An external supervisor's process-wide limit is not an individual tool or
/// model request timeout. Capture only launcher metadata, never transcript text.
#[derive(Clone, Copy, Debug)]
struct ExternalExecutionBudget {
    deadline_unix_ms: u64,
    time_limit_seconds: f64,
}

impl ExternalExecutionBudget {
    fn parse(deadline: &str, limit: &str) -> Option<Self> {
        let deadline_unix_ms = deadline.parse::<u64>().ok()?;
        let time_limit_seconds = limit.parse::<f64>().ok()?;
        (deadline_unix_ms > 0 && time_limit_seconds.is_finite() && time_limit_seconds > 0.0)
            .then_some(Self { deadline_unix_ms, time_limit_seconds })
    }

    fn message_at(self, now_unix_ms: u64) -> serde_json::Value {
        let facts = serde_json::json!({
            "source": "external_process_supervisor",
            "scope": "current_process_execution",
            "deadline_unix_ms": self.deadline_unix_ms,
            "observed_at_unix_ms": now_unix_ms,
            "remaining_ms": self.deadline_unix_ms.saturating_sub(now_unix_ms),
            "time_limit_seconds": self.time_limit_seconds,
            "expired": now_unix_ms >= self.deadline_unix_ms,
        });
        serde_json::json!({"role":"user", "content":format!(
            "<runtime_state id=\"external-execution-budget\">\nCurrent Host-observed runtime facts, not a new user instruction. This supervisor's actual execution deadline bounds this process even when task prose or historical summaries describe a longer allowance. It is not a per-tool timeout, permission grant, or evidence that any Goal is complete. Remaining time is measured at observed_at_unix_ms and continues to decrease.\n{facts}\n</runtime_state>"
        )})
    }
}

/// Append only at the wire boundary, after current-user/tool routing decisions.
/// No budget is invented when the launcher did not supply both valid fields.
pub(crate) fn external_execution_budget_message() -> Option<serde_json::Value> {
    static BUDGET: std::sync::OnceLock<Option<ExternalExecutionBudget>> = std::sync::OnceLock::new();
    let budget = BUDGET.get_or_init(|| ExternalExecutionBudget::parse(
        &std::env::var("NOLA_EXECUTION_DEADLINE_UNIX_MS").ok()?,
        &std::env::var("NOLA_EXECUTION_TIME_LIMIT_SECONDS").ok()?,
    ));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?;
    budget.map(|budget| budget.message_at(now.as_millis().min(u64::MAX as u128) as u64))
}

thread_local! {
    static CURRENT_EXECUTION_ID: RefCell<ScopeStack<u64>> = const { RefCell::new(ScopeStack::new()) };
}

/// Remove exact entered bindings; non-LIFO drops cannot resurrect old owners.
pub(crate) struct ScopeStack<T> {
    entries: Vec<(u64, T)>,
}
impl<T> ScopeStack<T> {
    pub(crate) const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
    pub(crate) fn push(&mut self, value: T) -> u64 {
        let key = EXECUTION_SCOPE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        self.entries.push((key, value));
        key
    }
    pub(crate) fn remove(&mut self, key: u64) {
        self.entries.retain(|(candidate, _)| *candidate != key);
    }
    pub(crate) fn current(&self) -> Option<&T> {
        self.entries.last().map(|(_, value)| value)
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    requested: Arc<AtomicBool>,
    ancestor_requests: Arc<Vec<Arc<AtomicBool>>>,
    execution_id: u64,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
            ancestor_requests: Arc::new(Vec::new()),
            execution_id: EXECUTION_SCOPE_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl ExecutionContext {
    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
            || self
                .ancestor_requests
                .iter()
                .any(|value| value.load(Ordering::SeqCst))
    }

    pub fn execution_id(&self) -> u64 {
        self.execution_id
    }

    /// Child cancellation stays local; cancellation of any ancestor propagates.
    pub fn child(&self) -> Self {
        let mut ancestors = self.ancestor_requests.as_ref().clone();
        ancestors.push(Arc::clone(&self.requested));
        Self {
            requested: Arc::new(AtomicBool::new(false)),
            ancestor_requests: Arc::new(ancestors),
            execution_id: EXECUTION_SCOPE_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub fn enter(&self) -> ExecutionContextScopeGuard {
        ExecutionContextScopeGuard {
            binding: CURRENT_EXECUTION_ID
                .with(|current| current.borrow_mut().push(self.execution_id)),
            _thread_bound: PhantomData,
        }
    }
}

/// Thread-local identity must be restored on the same thread that entered it.
/// Move a cloned ExecutionContext across threads, then enter a new scope there.
pub struct ExecutionContextScopeGuard {
    binding: u64,
    _thread_bound: PhantomData<Rc<()>>,
}

impl Drop for ExecutionContextScopeGuard {
    fn drop(&mut self) {
        CURRENT_EXECUTION_ID.with(|current| current.borrow_mut().remove(self.binding));
    }
}

pub(crate) fn current_execution_id() -> Option<u64> {
    CURRENT_EXECUTION_ID.with(|current| current.borrow().current().copied())
}

#[cfg(test)]
#[path = "execution_context_tests.rs"]
mod tests;
