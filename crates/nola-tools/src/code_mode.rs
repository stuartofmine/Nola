// SPDX-License-Identifier: Apache-2.0
//! Bounded QuickJS (`rquickjs`) sandboxed JavaScript evaluator (`code_exec`):
//! - 5s CPU interrupt timeout, 16 MiB RAM cap, 512 KiB stack cap
//! - Exposes frozen global `ALL_TOOLS` array and `tools.<name>(args)` Proxy backed by
//!   `ToolContext::code_mode_dispatcher` (bounded to at most 24 host tool calls per script)
//! - Pure JavaScript evaluation works seamlessly when `code_mode_dispatcher` is `None`.

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::sync::Arc;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::time::{Duration, Instant};

use nola_types::{CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use rquickjs::{Context, Function, Runtime};
use serde_json::{json, Value};

use crate::{ToolContext, ToolHandler};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const JS_CPU_TIMEOUT_SECS: u64 = 5;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_CODE_MODE_TOOL_CALLS: usize = 24;

pub struct CodeExecTool;

impl ToolHandler for CodeExecTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "code_exec".into(),
                description: "Evaluate a bounded JavaScript expression/program in an isolated QuickJS sandbox (5s CPU limit, 16 MiB memory limit). When host tool dispatch is enabled, exposes frozen global `ALL_TOOLS` and `tools.<name>(args)` (max 24 calls).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "code": { "type": "string", "description": "JavaScript source returning a JSON-serializable value" }
                    },
                    "required": ["code"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let _ = (ctx, args);
            return ToolExecutionResult::err_structured(
                "unsupported_platform",
                "QuickJS code_exec sandbox is only available on desktop hosts",
                None,
            );
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let Some(code) = args.get("code").and_then(|v| v.as_str()) else {
                return ToolExecutionResult::err_structured(
                    "missing_param",
                    "missing required 'code'",
                    None,
                );
            };
            let rt = match Runtime::new() {
                Ok(r) => r,
                Err(e) => {
                    return ToolExecutionResult::err_structured(
                        "js_init_error",
                        e.to_string(),
                        None,
                    )
                }
            };
            rt.set_memory_limit(16 * 1024 * 1024);
            rt.set_max_stack_size(512 * 1024);
            let started = Instant::now();
            rt.set_interrupt_handler(Some(Box::new(move || {
                started.elapsed() > Duration::from_secs(JS_CPU_TIMEOUT_SECS)
            })));
            let context = match Context::full(&rt) {
                Ok(c) => c,
                Err(e) => {
                    return ToolExecutionResult::err_structured("js_ctx_error", e.to_string(), None)
                }
            };

            let dispatcher = ctx.code_mode_dispatcher.clone();
            let tool_names_json = serde_json::to_string(&ctx.available_tool_names)
                .unwrap_or_else(|_| "[]".to_string());
            let call_count = Arc::new(AtomicUsize::new(0));
            let call_count_cb = Arc::clone(&call_count);

            let bootstrap = format!(
                r#"
globalThis.ALL_TOOLS = Object.freeze({tool_names_json});
Object.defineProperty(globalThis, 'tools', {{
    value: new Proxy(Object.create(null), {{
        get(_target, prop) {{
            if (typeof prop !== 'string') return undefined;
            return function(args) {{
                const raw = globalThis.__nola_call_tool(prop, JSON.stringify(args === undefined ? {{}} : args));
                const parsed = JSON.parse(raw);
                if (parsed && Object.prototype.hasOwnProperty.call(parsed, '__nola_err')) {{
                    throw new Error(String(parsed.__nola_err));
                }}
                return parsed ? parsed.__nola_ok : null;
            }};
        }},
        set() {{ return false; }},
        deleteProperty() {{ return false; }},
        ownKeys() {{ return [...globalThis.ALL_TOOLS]; }},
        getOwnPropertyDescriptor(_target, prop) {{
            if (typeof prop === 'string' && globalThis.ALL_TOOLS.includes(prop)) {{
                return {{ enumerable: true, configurable: true }};
            }}
            return undefined;
        }}
    }}),
    writable: false,
    configurable: false,
    enumerable: true
}});
"#
            );

            // Put newline before `})()` so trailing `//` comments never break wrapping.
            let wrapped = format!("JSON.stringify((function(){{\n{code}\n}})())");

            let eval_result: Result<Option<String>, String> = context.with(|js_ctx| {
                let host_fn = Function::new(
                    js_ctx.clone(),
                    move |name: String, args_json: String| -> String {
                        if name == "code_exec" {
                            return json!({"__nola_err": "recursive code_exec calls are not allowed"})
                                .to_string();
                        }
                        let Some(ref dispatch) = dispatcher else {
                            return json!({"__nola_err": "host tool dispatch is not enabled in this context"})
                                .to_string();
                        };
                        let prev = call_count_cb.fetch_add(1, Ordering::SeqCst);
                        if prev >= MAX_CODE_MODE_TOOL_CALLS {
                            return json!({
                                "__nola_err": format!(
                                    "code_exec exceeded maximum host tool calls ({MAX_CODE_MODE_TOOL_CALLS})"
                                )
                            })
                            .to_string();
                        }
                        let parsed_args: Value = if args_json.trim().is_empty() {
                            json!({})
                        } else {
                            match serde_json::from_str(&args_json) {
                                Ok(v) => v,
                                Err(e) => {
                                    return json!({"__nola_err": format!("invalid tool args JSON: {e}")})
                                        .to_string();
                                }
                            }
                        };
                        match dispatch(&name, parsed_args) {
                            Ok(val) => json!({"__nola_ok": val}).to_string(),
                            Err(e) => json!({"__nola_err": e}).to_string(),
                        }
                    },
                )
                .map_err(|e| format!("failed to register host tool bridge: {e}"))?;

                js_ctx
                    .globals()
                    .set("__nola_call_tool", host_fn)
                    .map_err(|e| format!("failed to expose host tool bridge: {e}"))?;

                if let Err(e) = js_ctx.eval::<(), _>(bootstrap.as_bytes()) {
                    return Err(format_js_error(&js_ctx, e));
                }

                match js_ctx.eval::<Option<String>, _>(wrapped.as_bytes()) {
                    Ok(v) => Ok(v),
                    Err(e) => Err(format_js_error(&js_ctx, e)),
                }
            });

            let calls_used = call_count.load(Ordering::SeqCst);
            match eval_result {
                Ok(Some(json_str)) => {
                    let val: Value =
                        serde_json::from_str(&json_str).unwrap_or(Value::String(json_str));
                    let mut out = json!({"result": val});
                    if calls_used > 0 {
                        out["tool_calls_used"] = json!(calls_used);
                    }
                    ToolExecutionResult::ok_structured(out, "Evaluated JavaScript in QuickJS sandbox")
                }
                Ok(None) => {
                    let mut out = json!({"result": null});
                    if calls_used > 0 {
                        out["tool_calls_used"] = json!(calls_used);
                    }
                    ToolExecutionResult::ok_structured(out, "Evaluated JavaScript (undefined)")
                }
                Err(e) => ToolExecutionResult::err_structured("js_eval_error", e, None),
            }
        }
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn format_js_error(js_ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> String {
    match err {
        rquickjs::Error::Exception => {
            let exc = js_ctx.catch();
            exc.as_exception()
                .and_then(|e| e.message())
                .unwrap_or_else(|| format!("{exc:?}"))
        }
        other => format!("{other}"),
    }
}
