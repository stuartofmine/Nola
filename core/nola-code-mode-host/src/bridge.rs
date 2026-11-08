// SPDX-License-Identifier: Apache-2.0
//! Per-isolate callback rendezvous. No execution or authorization lives here.

use crate::{CodeModeBridge, ToolFuture};
use nola_domain::code_mode::{CodeCellIdentity, CodeCellLimits, CodeNestedCall, CodeToolResult};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::{Arc, Mutex}};
use tokio::sync::oneshot;

/// The supervisor enqueues requests in order on the admitted parent effect's
/// response stream. It must not call a tool executor from this interface.
pub trait CellEventSink: Send + Sync + 'static {
    fn request_tool(&self, call: CodeNestedCall) -> Result<(), String>;
    fn output(&self, owner: &CodeCellIdentity, text: String) -> Result<(), String>;
}

#[derive(Default)]
struct Pending {
    last_sequence: u32,
    closed: Option<String>,
    waiters: BTreeMap<u32, oneshot::Sender<Result<Value, String>>>,
    /// Bounded by max_calls. Retain digests, not every large tool result, to
    /// distinguish an exact redelivery from conflicting receipt content.
    delivered: BTreeMap<u32, [u8; 32]>,
}

pub struct CellBridge {
    owner: CodeCellIdentity,
    limits: CodeCellLimits,
    sink: Arc<dyn CellEventSink>,
    pending: Mutex<Pending>,
    submission: Mutex<()>,
}

impl CellBridge {
    pub fn new(owner: CodeCellIdentity, limits: CodeCellLimits, sink: Arc<dyn CellEventSink>)
        -> Result<Self, String>
    {
        owner.validate()?;
        limits.validate()?;
        Ok(Self { owner, limits, sink, pending: Mutex::new(Pending::default()), submission: Mutex::new(()) })
    }

    /// Called ONLY for a fenced ResolveCodeTool request from Core's committed
    /// outbox. Repeated identical receipts acknowledge without waking twice.
    pub fn resolve(&self, result: CodeToolResult) -> Result<Value, String> {
        if result.cell != self.owner { return Err("code_cell_owner_mismatch".into()); }
        if result.sequence == 0 || result.sequence > self.limits.max_calls {
            return Err("code_cell_result_sequence_invalid".into());
        }
        let encoded = serde_json::to_vec(&result).map_err(|error| error.to_string())?;
        let digest: [u8; 32] = Sha256::digest(&encoded).into();
        let mut pending = self.pending.lock().map_err(|_| "code_cell_bridge_poisoned")?;
        if let Some(existing) = pending.delivered.get(&result.sequence) {
            return if existing == &digest { Ok(result.acknowledgement()) }
                else { Err("code_cell_conflicting_result".into()) };
        }
        let waiter = pending.waiters.remove(&result.sequence).ok_or("code_cell_result_has_no_waiter")?;
        pending.delivered.insert(result.sequence, digest);
        let value = if serde_json::to_vec(&result.output).map_err(|error| error.to_string())?.len()
            > self.limits.result_bytes as usize {
            Err("code_cell_result_budget_exceeded".into())
        } else if result.is_error {
            Err(result.output.to_string())
        } else {
            Ok(result.output.clone())
        };
        // A dropped JS receiver is not permission to retry an already finished
        // operation. The receipt still acknowledges the committed result.
        let _ = waiter.send(value);
        Ok(result.acknowledgement())
    }

    /// Stops waiting locally. This is NOT proof that committed child effects
    /// were cancelled; the supervisor must reconcile those through Core.
    pub fn close(&self, reason: &str) -> Result<(), String> {
        let mut pending = self.pending.lock().map_err(|_| "code_cell_bridge_poisoned")?;
        if pending.closed.is_none() { pending.closed = Some(reason.to_owned()); }
        let reason = pending.closed.clone().expect("closed reason set");
        for (_, waiter) in std::mem::take(&mut pending.waiters) {
            let _ = waiter.send(Err(reason.clone()));
        }
        Ok(())
    }
}

impl CodeModeBridge for CellBridge {
    fn invoke(&self, call: CodeNestedCall) -> ToolFuture {
        let registered = (|| {
            let _submission = self.submission.lock().map_err(|_| "code_cell_bridge_poisoned")?;
            call.validate(&self.owner, &self.limits)?;
            let mut pending = self.pending.lock().map_err(|_| "code_cell_bridge_poisoned")?;
            if let Some(reason) = &pending.closed { return Err(reason.clone()); }
            if pending.last_sequence.checked_add(1) != Some(call.sequence) {
                return Err("code_cell_call_sequence_mismatch".into());
            }
            if pending.waiters.len() >= self.limits.max_in_flight as usize {
                return Err("code_cell_concurrency_exceeded".into());
            }
            let (sender, receiver) = oneshot::channel();
            pending.last_sequence = call.sequence;
            pending.waiters.insert(call.sequence, sender);
            drop(pending);
            if let Err(error) = self.sink.request_tool(call) {
                // A transport enqueue failure leaves sequence delivery uncertain;
                // never continue with the next number or silently retry source.
                self.close(&error)?;
                return Err(error);
            }
            Ok(receiver)
        })();
        Box::pin(async move {
            registered?.await.map_err(|_| "code_cell_result_channel_closed".to_owned())?
        })
    }

    fn output(&self, owner: &CodeCellIdentity, text: String) -> Result<(), String> {
        if owner != &self.owner { return Err("code_cell_owner_mismatch".into()); }
        if let Some(reason) = &self.pending.lock().map_err(|_| "code_cell_bridge_poisoned")?.closed {
            return Err(reason.clone());
        }
        self.sink.output(owner, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Sink(Mutex<Vec<CodeNestedCall>>);
    impl CellEventSink for Sink {
        fn request_tool(&self, call: CodeNestedCall) -> Result<(), String> {
            self.0.lock().unwrap().push(call);
            Ok(())
        }
        fn output(&self, _: &CodeCellIdentity, _: String) -> Result<(), String> { Ok(()) }
    }

    fn bridge() -> CellBridge {
        CellBridge::new(CodeCellIdentity { host_id: "h".into(), conversation_id: "s".into(),
            turn_id: "t".into(), parent_call_id: "p".into(), cell_id: "c".into(), generation: "g".into() },
            CodeCellLimits::default(), Arc::new(Sink::default())).unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn result_requires_exact_waiter_and_redelivery_does_not_reexecute() {
        let bridge = bridge();
        let future = bridge.invoke(CodeNestedCall { cell: bridge.owner.clone(), sequence: 1,
            tool_name: "read_file".into(), arguments: serde_json::json!({}) });
        let result = CodeToolResult { cell: bridge.owner.clone(), sequence: 1,
            output: serde_json::json!({"text":"ok"}), is_error: false };
        let mut wrong = result.clone();
        wrong.cell.generation = "other".into();
        assert!(bridge.resolve(wrong).is_err());
        assert_eq!(bridge.resolve(result.clone()).unwrap(), result.acknowledgement());
        assert_eq!(future.await.unwrap(), result.output);
        assert!(bridge.resolve(result.clone()).is_ok());
        let mut conflict = result;
        conflict.output = Value::Null;
        assert!(bridge.resolve(conflict).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_rejects_waiters_and_future_invocations() {
        let bridge = bridge();
        let call = CodeNestedCall { cell: bridge.owner.clone(), sequence: 1,
            tool_name: "read_file".into(), arguments: serde_json::json!({}) };
        let future = bridge.invoke(call.clone());
        bridge.close("worker_stopped").unwrap();
        assert_eq!(future.await.unwrap_err(), "worker_stopped");
        let mut next = call;
        next.sequence = 2;
        assert_eq!(bridge.invoke(next).await.unwrap_err(), "worker_stopped");
    }
}
