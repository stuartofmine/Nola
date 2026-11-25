// SPDX-License-Identifier: Apache-2.0
//! Bounded QuickJS (`rquickjs`) sandboxed JavaScript evaluator for fast deterministic
//! data transforms and computation (`code_exec`).

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::time::{Duration, Instant};

use nola_types::{CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use rquickjs::{Context, Runtime};
use serde_json::{json, Value};

use crate::{ToolContext, ToolHandler};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const JS_CPU_TIMEOUT_SECS: u64 = 5;

pub struct CodeExecTool;

impl ToolHandler for CodeExecTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "code_exec".into(),
                description: "Evaluate a bounded JavaScript expression/program in an isolated QuickJS sandbox (5s CPU limit, 16 MiB memory limit, no filesystem/network).".into(),
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

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let _ = args;
            return ToolExecutionResult::err_structured(
                "unsupported_platform",
                "QuickJS code_exec sandbox is only available on desktop hosts",
                None,
            );
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let Some(code) = args.get("code").and_then(|v| v.as_str()) else {
                return ToolExecutionResult::err_structured("missing_param", "missing required 'code'", None);
            };
            let rt = match Runtime::new() {
                Ok(r) => r,
                Err(e) => return ToolExecutionResult::err_structured("js_init_error", e.to_string(), None),
            };
            rt.set_memory_limit(16 * 1024 * 1024);
            rt.set_max_stack_size(512 * 1024);
            let started = Instant::now();
            rt.set_interrupt_handler(Some(Box::new(move || {
                started.elapsed() > Duration::from_secs(JS_CPU_TIMEOUT_SECS)
            })));
            let context = match Context::full(&rt) {
                Ok(c) => c,
                Err(e) => return ToolExecutionResult::err_structured("js_ctx_error", e.to_string(), None),
            };

            // Put newline before `})()` so trailing `//` comments never break wrapping.
            let wrapped = format!("JSON.stringify((function(){{\n{code}\n}})())");
            let eval_result: Result<Option<String>, String> = context.with(|ctx| {
                match ctx.eval::<Option<String>, _>(wrapped.as_bytes()) {
                    Ok(v) => Ok(v),
                    Err(rquickjs::Error::Exception) => {
                        let exc = ctx.catch();
                        let msg = exc
                            .as_exception()
                            .and_then(|e| e.message())
                            .unwrap_or_else(|| format!("{exc:?}"));
                        Err(msg)
                    }
                    Err(e) => Err(format!("{e}")),
                }
            });

            match eval_result {
                Ok(Some(json_str)) => {
                    let val: Value = serde_json::from_str(&json_str).unwrap_or(Value::String(json_str));
                    ToolExecutionResult::ok_structured(
                        json!({"result": val}),
                        "Evaluated JavaScript in QuickJS sandbox",
                    )
                }
                Ok(None) => ToolExecutionResult::ok_structured(
                    json!({"result": null}),
                    "Evaluated JavaScript (undefined)",
                ),
                Err(e) => ToolExecutionResult::err_structured("js_eval_error", e, None),
            }
        }
    }
}
