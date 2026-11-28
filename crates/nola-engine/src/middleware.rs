// SPDX-License-Identifier: Apache-2.0
//! Single 6-Stage `ToolExecutionMiddleware`:
//! 1. Normalize tool call & lookup `ToolSpec`
//! 2. Evaluate `EffectivePolicy` (Plan Mode + Durable Grants + Ask / Allow / Deny)
//! 3. Execute trusted `PreToolUse` hooks (RE-EVALUATING `EffectivePolicy` if `ModifyInput`!)
//! 4. Execute tool handler via `ToolRegistry`
//! 5. Execute trusted `PostToolUse` hooks
//! 6. Return normalized `MiddlewareOutcome` with full `ToolModelPayload` + `VersionTransition`s.

use std::sync::{Arc, Mutex};

use nola_store::Store;
use nola_tools::{ToolContext, ToolRegistry};
use nola_types::{
    HookDecision, HookEvent, PermissionAction, PermissionRule, RuntimeReminder,
    ToolExecutionResult, ToolSpec,
};
use serde_json::{json, Value};

use crate::hooks::HookRunner;
use crate::planning::is_plan_mode;
use crate::policy::{
    persist_permission_grant, primary_resource_pattern, DurablePermissionGrant, EffectivePolicy,
};

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
    store: Option<Store>,
    session_id: Option<String>,
    runtime_grants: Mutex<Vec<PermissionRule>>,
}

impl ToolExecutionMiddleware {
    pub fn new(
        policy: EffectivePolicy,
        hooks: HookRunner,
        approval: Option<ApprovalCallback>,
    ) -> Self {
        Self {
            policy,
            hooks,
            approval,
            store: None,
            session_id: None,
            runtime_grants: Mutex::new(Vec::new()),
        }
    }

    /// Attach `Store` and `session_id` to enable live Plan Mode checks (`kv["plan_mode:{session_id}"]`)
    /// and durable permission grant persistence (`Store::put_module_record("permission_grant", ...)`).
    pub fn with_persistence(mut self, store: Store, session_id: impl Into<String>) -> Self {
        self.store = Some(store);
        self.session_id = Some(session_id.into());
        self
    }

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

        // Stage 3: Trusted PreToolUse hooks (with mandatory policy + hook RE-CHECK on ModifyInput!)
        // Bounded to 3 rounds so mutually-modifying hooks cannot loop indefinitely.
        for round in 0..3 {
            let mut modified_this_round = false;
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
                        if input != args {
                            args = input;
                            modified_this_round = true;
                        }
                        // Security invariant: a hook-modified input MUST pass EffectivePolicy again!
                        if let Some(blocked) = self.enforce_policy(&spec, &args, &ctx.workspace_root)
                        {
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
            if !modified_this_round {
                break;
            }
            if round == 2 {
                // Final policy guard after maximum modification rounds
                if let Some(blocked) = self.enforce_policy(&spec, &args, &ctx.workspace_root) {
                    return MiddlewareOutcome {
                        result: blocked,
                        reminders,
                    };
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

    fn effective_policy_snapshot(&self) -> EffectivePolicy {
        let mut effective = self.policy.clone();
        if let (Some(store), Some(sid)) = (&self.store, &self.session_id) {
            let key = format!("plan_mode:{sid}");
            if store.get_kv(&key).ok().flatten().is_some() {
                effective.plan_mode = is_plan_mode(store, sid);
            }
        }
        if let Ok(grants) = self.runtime_grants.lock() {
            for rule in grants.iter() {
                if !effective.rules.contains(rule) {
                    effective.rules.push(rule.clone());
                }
            }
        }
        effective
    }

    fn record_approval_grant(
        &self,
        spec: &ToolSpec,
        args: &Value,
        workspace_root: &std::path::Path,
    ) {
        // Never persist permanent durable grants for bash commands touching sensitive paths
        // or Nola's own internal database/config directories.
        if spec.schema.name == "bash" {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                if crate::policy::bash_command_sensitive_reason(cmd).is_some() {
                    return;
                }
            }
        }
        let resource_pattern = primary_resource_pattern(&spec.schema.name, args, workspace_root);
        let grant = DurablePermissionGrant::for_approval(
            self.session_id.clone(),
            workspace_root,
            &spec.schema.name,
            resource_pattern,
        );
        if let Ok(mut grants) = self.runtime_grants.lock() {
            if !grants.contains(&grant.rule) {
                grants.push(grant.rule.clone());
            }
        }
        if let Some(store) = &self.store {
            let _ = persist_permission_grant(store, &grant);
        }
    }

    fn enforce_policy(
        &self,
        spec: &ToolSpec,
        args: &Value,
        workspace_root: &std::path::Path,
    ) -> Option<ToolExecutionResult> {
        let effective = self.effective_policy_snapshot();
        let decision = effective.evaluate(spec, args, workspace_root);
        match decision.action {
            PermissionAction::Allow => None,
            PermissionAction::Deny => {
                let code = if effective.plan_mode && decision.reason.starts_with("Plan Mode is active:") {
                    "plan_mode_blocked"
                } else {
                    "permission_denied"
                };
                Some(ToolExecutionResult::err_structured(
                    code,
                    decision.reason,
                    Some(json!({"domain": decision.domain})),
                ))
            }
            PermissionAction::Ask => {
                let approved = self
                    .approval
                    .as_ref()
                    .map(|cb| cb(spec, args, &decision.reason))
                    .unwrap_or(false);
                if approved {
                    self.record_approval_grant(spec, args, workspace_root);
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
