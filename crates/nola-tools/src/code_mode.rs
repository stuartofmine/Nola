// SPDX-License-Identifier: Apache-2.0
//! Bounded QuickJS (`rquickjs`) sandboxed JavaScript evaluator for fast deterministic
//! data transforms and computation (`code_exec`).

use nola_types::{CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
use rquickjs::{Context, Runtime};
use serde_json::{json, Value};

use crate::{ToolContext, ToolHandler};

pub struct CodeExecTool;

impl ToolHandler for CodeExecTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "code_exec".into(),
                description: "Evaluate a bounded JavaScript expression/program in an isolated QuickJS sandbox (no filesystem or network access).".into(),
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
        let Some(code) = args.get("code").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'code'", None);
        };
        let rt = match Runtime::new() {
            Ok(r) => r,
            Err(e) => return ToolExecutionResult::err_structured("js_init_error", e.to_string(), None),
        };
        rt.set_memory_limit(16 * 1024 * 1024);
        rt.set_max_stack_size(512 * 1024);
        let context = match Context::full(&rt) {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::err_structured("js_ctx_error", e.to_string(), None),
        };

        let wrapped = format!(
            "JSON.stringify((function(){{ {code} }})())"
        );
        let eval_result: Result<Option<String>, String> = context.with(|ctx| {
            ctx.eval::<Option<String>, _>(wrapped.as_bytes())
                .map_err(|e| format!("{e}"))
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
