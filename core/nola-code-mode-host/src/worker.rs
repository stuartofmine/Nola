// SPDX-License-Identifier: Apache-2.0
//! Isolate worker lifecycle. The Host drains events, yields on nested requests,
//! and resumes draining after dispatching committed Core result receipts.

use crate::bridge::{CellBridge, CellEventSink};
use nola_domain::code_mode::{CodeCellIdentity, CodeCellLimits, CodeNestedCall, CodeToolResult};
use serde_json::Value;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc};
use std::thread::JoinHandle;

pub enum CellEvent {
    ToolRequested(CodeNestedCall),
    Output(String),
    Finished(Result<(), String>),
}

struct EventSink(mpsc::Sender<CellEvent>);
impl CellEventSink for EventSink {
    fn request_tool(&self, call: CodeNestedCall) -> Result<(), String> {
        self.0.send(CellEvent::ToolRequested(call)).map_err(|_| "code_cell_event_stream_closed".into())
    }
    fn output(&self, _: &CodeCellIdentity, text: String) -> Result<(), String> {
        self.0.send(CellEvent::Output(text)).map_err(|_| "code_cell_event_stream_closed".into())
    }
}

/// One worker per accepted cell; the owning supervisor must retain completed
/// identities and reject duplicate StartCodeCell deliveries, never respawn them.
pub struct CellWorker {
    owner: CodeCellIdentity,
    bridge: Arc<CellBridge>,
    cancelled: Arc<AtomicBool>,
    events: mpsc::Receiver<CellEvent>,
    thread: Option<JoinHandle<()>>,
}

impl CellWorker {
    pub fn start(owner: CodeCellIdentity, limits: CodeCellLimits, source: String,
        tools: Vec<nola_domain::code_mode::CodeToolDescription>) -> Result<Self, String> {
        owner.validate()?;
        limits.validate_source(&source)?;
        let (sender, events) = mpsc::channel();
        let bridge = Arc::new(CellBridge::new(owner.clone(), limits.clone(), Arc::new(EventSink(sender.clone())))?);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_owner = owner.clone();
        let worker_bridge = bridge.clone();
        let worker_cancelled = cancelled.clone();
        let thread = std::thread::Builder::new().name("nola-code-cell".into()).spawn(move || {
            // QuickJS is created and dropped on its owning thread. No JS values
            // or engine handles cross the boundary; only typed JSON messages do.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build()
                    .map_err(|error| error.to_string())?;
                runtime.block_on(crate::evaluate(worker_owner, limits, source, tools,
                    worker_bridge.clone(), worker_cancelled))
            })).unwrap_or_else(|_| Err("code_cell_worker_panicked".into()));
            let _ = worker_bridge.close("code_cell_worker_finished");
            let _ = sender.send(CellEvent::Finished(outcome));
        }).map_err(|error| error.to_string())?;
        Ok(Self { owner, bridge, cancelled, events, thread: Some(thread) })
    }

    pub fn owner(&self) -> &CodeCellIdentity { &self.owner }

    /// Bounded wait only. A timeout means the worker is still running, not a
    /// restart signal. The supervisor remains responsible for lease renewal.
    pub fn next_event(&self, timeout: std::time::Duration) -> Result<Option<CellEvent>, String> {
        if timeout > std::time::Duration::from_secs(60) {
            return Err("code_cell_poll_wait_exceeded".into());
        }
        match self.events.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("code_cell_event_stream_closed".into()),
        }
    }

    pub fn resolve(&self, result: CodeToolResult) -> Result<Value, String> { self.bridge.resolve(result) }

    /// Request local isolate stop; Core must separately cancel/reconcile tools.
    pub fn request_stop(&self) -> Result<(), String> {
        self.cancelled.store(true, Ordering::Release);
        self.bridge.close("code_cell_cancelled")
    }

    pub fn join_if_finished(&mut self) -> Result<bool, String> {
        if self.thread.is_none() { return Ok(true); }
        if !self.thread.as_ref().is_some_and(|thread| thread.is_finished()) { return Ok(false); }
        self.thread.take().expect("finished thread exists").join()
            .map_err(|_| "code_cell_worker_panicked".to_string())?;
        Ok(true)
    }
}

impl Drop for CellWorker {
    fn drop(&mut self) {
        // No unbounded UI-thread join. The engine's interrupt/deadline and
        // closing its waiters stop detached work; effects remain Core-owned.
        let _ = self.request_stop();
    }
}
