// SPDX-License-Identifier: Apache-2.0
//! Single 6-Stage `ToolExecutionMiddleware`:
//! 1. Normalize tool call & lookup `ToolSpec`
//! 2. Evaluate `EffectivePolicy` (Ask / Allow / Deny)
//! 3. Execute trusted `PreToolUse` hooks (RE-EVALUATING `EffectivePolicy` if `ModifyInput`!)
//! 4. Execute tool handler via `ToolRegistry`
//! 5. Execute trusted `PostToolUse` hooks
//! 6. Return normalized `MiddlewareOutcome` with full `ToolModelPayload` + `VersionTransition`s.

use std::sync::Arc;

use nola_tools::{ToolContext, ToolRegistry};
use nola_types::{
    HookDecision, HookEvent, PermissionAction, RuntimeReminder, ToolExecutionResult, ToolSpec,
};
use serde_json::{json, Value};

use crate::hooks::HookRunner;
use crate::policy::EffectivePolicy;

/// Optional interactive user approval callback when policy evaluates to `PermissionAction::Ask`.
pub type ApprovalCallback = Arc<dyn Fn(&ToolSpec, &Value, &str) -> bool + Send + Sync>;

#[derive(Debug, Clone)]
pub struct MiddlewareOutcome {
    pub result: ToolExecutionResult,
    pub reminders: Vec<RuntimeReminder>,
}

pub struct ToolExecutionMiddleware {
    pub policy: EffectivePolicy,
    pub hooks: HookRunner,
    pub approval: Option<ApprovalCallback>,
}

impl ToolExecutionMiddleware {
    pub fn execute(
        &self,
        registry: &ToolRegistry,
        ctx: &ToolContext,
        tool_name: &str,
        raw_args: Value,
    ) -> MiddlewareOutcome {
        let mut reminders = Vec::new();

        // Stage 1: Normalize & resolve spec
        let Some(handler) = registry.get(tool_name) else {
            return MiddlewareOutcome {
                result: ToolExecutionResult::err_structured(
                    "unknown_tool",
                    format!("tool '{tool_name}' is not registered"),
                    None,
                ),
                reminders,
            };
        };
        let spec = handler.spec();
        let mut args = match raw_args {
            Value::Object(_) => raw_args,
            Value::Null => json!({}),
            other => json!({ "input": other }),
        };

        // Stage 2: EffectivePolicy evaluation
        if let Some(blocked) = self.enforce_policy(&spec, &args, &ctx.workspace_root) {
            return MiddlewareOutcome {
                result: blocked,
                reminders,
            };
        }

        // Stage 3: Trusted PreToolUse hooks (with mandatory policy RE-CHECK on ModifyInput!)
        let pre_decisions = self.hooks.run_event(HookEvent::PreToolUse, tool_name, &args);
        for dec in pre_decisions {
            match dec {
                HookDecision::Allow => {}
                HookDecision::Block { reason } => {
                    return MiddlewareOutcome {
                        result: ToolExecutionResult::err_structured(
                            "hook_blocked",
                            format!("PreToolUse hook blocked '{tool_name}': {reason}"),
                            None,
                        ),
                        reminders,
                    };
                }
                HookDecision::ModifyInput { input } => {
                    args = input;
                    // Security invariant: a hook-modified input MUST pass EffectivePolicy again!
                    if let Some(blocked) = self.enforce_policy(&spec, &args, &ctx.workspace_root) {
                        return MiddlewareOutcome {
                            result: blocked,
                            reminders,
                        };
                    }
                }
                HookDecision::InjectContext { text } => {
                    reminders.push(RuntimeReminder {
                        tag: "pre_hook".into(),
                        text,
                    });
                }
            }
        }

        // Stage 4: Execute tool handler
        let result = registry.execute(tool_name, ctx, args.clone());

        // Stage 5: Trusted PostToolUse hooks
        let post_payload = json!({
            "arguments": args,
            "is_error": result.is_error,
            "summary": result.payload.ui_summary(),
            "effects": result.effects,
        });
        for dec in self
            .hooks
            .run_event(HookEvent::PostToolUse, tool_name, &post_payload)
        {
            if let HookDecision::InjectContext { text } = dec {
                reminders.push(RuntimeReminder {
                    tag: "post_hook".into(),
                    text,
                });
            }
        }

        // Stage 6: Return intact result + tail reminders
        MiddlewareOutcome { result, reminders }
    }

    fn enforce_policy(
        &self,
        spec: &ToolSpec,
        args: &Value,
        workspace_root: &std::path::Path,
    ) -> Option<ToolExecutionResult> {
        let decision = self.policy.evaluate(spec, args, workspace_root);
        match decision.action {
            PermissionAction::Allow => None,
            PermissionAction::Deny => Some(ToolExecutionResult::err_structured(
                "permission_denied",
                decision.reason,
                Some(json!({"domain": decision.domain})),
            )),
            PermissionAction::Ask => {
                let approved = self
                    .approval
                    .as_ref()
                    .map(|cb| cb(spec, args, &decision.reason))
                    .unwrap_or(false);
                if approved {
                    None
                } else {
                    Some(ToolExecutionResult::err_structured(
                        "approval_required",
                        format!("user approval denied or unavailable: {}", decision.reason),
                        Some(json!({"domain": decision.domain})),
                    ))
                }
            }
        }
    }
}
