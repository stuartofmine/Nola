// SPDX-License-Identifier: Apache-2.0
//! Restricted JavaScript computation. The bridge MUST round-trip through Core
//! admission; this crate deliberately has no tool executor or filesystem API.

use nola_domain::code_mode::{CodeCellIdentity, CodeCellLimits, CodeNestedCall};
use rquickjs::{AsyncContext, AsyncRuntime, Function, Promise, prelude::Async};
use serde_json::Value;
use std::{future::Future, pin::Pin, sync::{Arc, atomic::{AtomicBool, AtomicU32, Ordering}}, time::{Duration, Instant}};

#[cfg(test)]
mod tests;

pub mod bridge;
pub mod worker;

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;

fn script_error(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error) -> String {
    // Retain the actual JS exception so the model can correct arguments or
    // parsing. Bound diagnostics independently of successful output budgets.
    rquickjs::CaughtError::from_error(ctx, error).to_string().chars().take(4096).collect()
}

pub trait CodeModeBridge: Send + Sync + 'static {
    /// Must complete only from the ordinary committed tool result, including
    /// permission waits. A script callback is never authority to execute a tool.
    fn invoke(&self, call: CodeNestedCall) -> ToolFuture;
    /// Persist/sequence output through the parent runtime; do not print to stdio.
    fn output(&self, owner: &CodeCellIdentity, text: String) -> Result<(), String>;
}

/// No module loader, Node globals, libc bindings, or custom allocator is installed.
/// Completion still requires Core to settle pending calls; engine return alone
/// is not an execution-success receipt.
pub async fn evaluate(
    owner: CodeCellIdentity,
    limits: CodeCellLimits,
    source: String,
    tools: Vec<nola_domain::code_mode::CodeToolDescription>,
    bridge: Arc<dyn CodeModeBridge>,
    cancelled: Arc<AtomicBool>,
) -> Result<(), String> {
    owner.validate()?;
    limits.validate_source(&source)?;
    let catalog = serde_json::to_string(&tools).map_err(|error| error.to_string())?;
    if catalog.len() > limits.result_bytes as usize { return Err("code_cell_catalog_budget_exceeded".into()); }
    let duration = Duration::from_millis(limits.wall_time_ms);
    let deadline = Instant::now().checked_add(duration).ok_or("invalid_code_cell_deadline")?;
    let runtime = AsyncRuntime::new().map_err(|e| e.to_string())?;
    runtime.set_memory_limit(usize::try_from(limits.heap_bytes).map_err(|_| "invalid_code_cell_heap")?).await;
    runtime.set_max_stack_size(256 * 1024).await;
    let interrupt = cancelled.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || interrupt.load(Ordering::Acquire) || Instant::now() >= deadline))).await;
    let context = AsyncContext::full(&runtime).await.map_err(|e| e.to_string())?;
    let sequence = Arc::new(AtomicU32::new(0));
    let output_bytes = Arc::new(AtomicU32::new(0));
    let permits = Arc::new(tokio::sync::Semaphore::new(limits.max_in_flight as usize));
    let abort = cancelled.clone();
    let computation = context.async_with(async move |ctx| {
        let callback_owner = owner.clone();
        let callback_limits = limits.clone();
        let callback_bridge = bridge.clone();
        let callback_cancelled = cancelled.clone();
        let invoke = Function::new(ctx.clone(), Async(move |name: String, arguments: String| {
            let owner = callback_owner.clone();
            let limits = callback_limits.clone();
            let bridge = callback_bridge.clone();
            let cancelled = callback_cancelled.clone();
            let sequence = sequence.clone();
            let permits = permits.clone();
            async move {
                let result: Result<Value, String> = async {
                    if cancelled.load(Ordering::Acquire) { return Err("code_cell_cancelled".into()); }
                    if arguments.len() > limits.argument_bytes as usize { return Err("code_cell_argument_budget_exceeded".into()); }
                    // Invalid local requests must not consume Core's contiguous
                    // callback sequence. Permission failures, by contrast, are
                    // real admitted calls and settle through the bridge.
                    let mut call = CodeNestedCall { cell: owner.clone(), sequence: 1,
                        tool_name: name, arguments: serde_json::from_str(&arguments).map_err(|e| e.to_string())? };
                    call.validate(&owner, &limits)?;
                    let _permit = permits.acquire().await.map_err(|_| "code_cell_closed".to_string())?;
                    if cancelled.load(Ordering::Acquire) { return Err("code_cell_cancelled".into()); }
                    let previous = sequence.try_update(Ordering::AcqRel, Ordering::Acquire,
                        |value| (value < limits.max_calls).then(|| value + 1))
                        .map_err(|_| "code_cell_call_budget_exceeded".to_string())?;
                    call.sequence = previous + 1;
                    let value = bridge.invoke(call).await?;
                    let encoded = serde_json::to_string(&value).map_err(|e| e.to_string())?;
                    if encoded.len() > limits.result_bytes as usize { return Err("code_cell_result_budget_exceeded".into()); }
                    Ok(value)
                }.await;
                let envelope = match result {
                    Ok(value) => serde_json::json!({"ok":true,"value":value}),
                    Err(error) => serde_json::json!({"ok":false,"error":error}),
                };
                Ok::<String, rquickjs::Error>(envelope.to_string())
            }
        })).map_err(|e| e.to_string())?;
        let output = Function::new(ctx.clone(), move |text: String| -> bool {
            let Ok(size) = u32::try_from(text.len()) else { return false; };
            // Empty output still consumes a queue entry. Charge one byte so
            // text('') loops cannot allocate an unbounded event backlog.
            let size = size.max(1);
            if cancelled.load(Ordering::Acquire) || output_bytes.try_update(Ordering::AcqRel, Ordering::Acquire,
                |used| used.checked_add(size).filter(|total| *total <= limits.output_bytes)).is_err() { return false; }
            bridge.output(&owner, text).is_ok()
        }).map_err(|e| e.to_string())?;
        // Capture native callbacks in lexical scope and remove their globals
        // before evaluating user code. Tools return asynchronous JS promises.
        ctx.globals().set("__nola_invoke", invoke).map_err(|e| e.to_string())?;
        ctx.globals().set("__nola_output", output).map_err(|e| e.to_string())?;
        ctx.globals().set("__nola_catalog", catalog).map_err(|e| e.to_string())?;
        let run: Function = ctx.eval(include_str!("bootstrap.js")).map_err(|e| script_error(&ctx, e))?;
        let script = format!("(async () => {{\n{source}\n}})");
        let program: Function = ctx.eval(script).map_err(|e| script_error(&ctx, e))?;
        let promise: Promise = run.call((program,)).map_err(|e| script_error(&ctx, e))?;
        promise.into_future::<()>().await.map_err(|e| script_error(&ctx, e))
    });
    let result = tokio::time::timeout(duration, computation).await
        .unwrap_or_else(|_| Err("code_cell_deadline_exceeded".to_string()));
    if result.is_err() {
        // The supervisor must reconcile/cancel already committed children; this
        // flag only prevents queued callbacks from starting new work.
        abort.store(true, Ordering::Release);
    }
    result
}
