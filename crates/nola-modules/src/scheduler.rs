// SPDX-License-Identifier: Apache-2.0
//! 5-field Cron Expression Evaluator & Background Automation Scheduler.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::{DateTime, Datelike, Timelike, Utc};
use nola_engine::{
    AgentEngine, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner, ModelCaller,
};
use nola_tools::{path_within_workspace, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    now_ms, AutomationRecord, FrozenSystemBlock, HostCapabilities, ModelRouteConfig,
};

use crate::automations_messaging::KIND_AUTOMATION;
use crate::ModulesSuite;

/// Evaluate a standard 5-field cron expression (`min hour dom month dow`) against `dt`.
pub fn cron_matches(expr: &str, dt: DateTime<Utc>) -> bool {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }
    field_matches(fields[0], dt.minute(), 0, 59, false)
        && field_matches(fields[1], dt.hour(), 0, 23, false)
        && field_matches(fields[2], dt.day(), 1, 31, false)
        && field_matches(fields[3], dt.month(), 1, 12, false)
        && field_matches(fields[4], dt.weekday().num_days_from_sunday(), 0, 7, true)
}

fn field_matches(spec: &str, value: u32, min: u32, max: u32, is_dow: bool) -> bool {
    for raw_part in spec.split(',') {
        let raw_part = raw_part.trim();
        let (base, step) = match raw_part.split_once('/') {
            Some((lhs, rhs)) => {
                let Ok(s) = rhs.parse::<u32>() else {
                    continue;
                };
                if s == 0 {
                    continue;
                }
                (lhs.trim(), Some(s))
            }
            None => (raw_part, None),
        };
        if base == "*" {
            let s = step.unwrap_or(1);
            if (value.saturating_sub(min)) % s == 0 {
                return true;
            }
            continue;
        }
        if let Some((a, b)) = base.split_once('-') {
            if let (Ok(lo), Ok(hi)) = (a.parse::<u32>(), b.parse::<u32>()) {
                if lo <= hi && lo >= min && hi <= max {
                    let s = step.unwrap_or(1);
                    let mut cur = lo;
                    while cur <= hi {
                        let norm = if is_dow && cur == 7 { 0 } else { cur };
                        if norm == value {
                            return true;
                        }
                        cur = cur.saturating_add(s);
                    }
                }
            }
            continue;
        }
        if let Ok(exact) = base.parse::<u32>() {
            if exact >= min && exact <= max {
                let norm = if is_dow && exact == 7 { 0 } else { exact };
                if norm == value {
                    return true;
                }
            }
        }
    }
    false
}

/// Evaluates all enabled `AutomationRecord`s and executes those due at `at`.
pub struct AutomationScheduler;

impl AutomationScheduler {
    pub fn find_due(suite: &ModulesSuite, at: DateTime<Utc>) -> Vec<AutomationRecord> {
        let now = at.timestamp_millis();
        suite
            .list_records::<AutomationRecord>(KIND_AUTOMATION)
            .unwrap_or_default()
            .into_iter()
            .filter(|rec| {
                if !rec.enabled || !cron_matches(&rec.schedule, at) {
                    return false;
                }
                // Guard against firing twice in the same minute
                rec.last_run_ms
                    .map(|last| now.saturating_sub(last) >= 55_000)
                    .unwrap_or(true)
            })
            .collect()
    }

    pub fn run_due(
        suite: &Arc<ModulesSuite>,
        caller: Arc<dyn ModelCaller>,
        route: ModelRouteConfig,
        policy: EffectivePolicy,
        at: DateTime<Utc>,
    ) -> Vec<(String, Result<String, String>)> {
        let due = Self::find_due(suite, at);
        let mut results = Vec::new();
        for mut rec in due {
            let rec_ws = Path::new(&rec.workspace_root);
            if !path_within_workspace(&suite.workspace_root, rec_ws) {
                results.push((
                    rec.id.clone(),
                    Err(format!(
                        "automation workspace '{}' escapes active workspace root",
                        rec.workspace_root
                    )),
                ));
                continue;
            }
            // Stamp last_run_ms BEFORE execution so concurrent scheduler ticks cannot double-fire!
            rec.last_run_ms = Some(at.timestamp_millis());
            rec.updated_at_ms = now_ms();
            let _ = suite.upsert_record(KIND_AUTOMATION, &rec.id, &rec);

            let mut registry = ToolRegistry::with_builtin_tools();
            suite.register_tools(&mut registry);
            let engine =
                AgentEngine::new(suite.store.clone(), Arc::new(registry), Arc::clone(&caller));
            let mut ctx = ToolContext::new(rec_ws);
            ctx.permission_profile = policy.profile;
            ctx.network_allowed = policy.network_allowed;
            ctx.sandbox_mode = SandboxMode::from_env();

            let hooks = HookRunner::load(
                rec_ws,
                suite.user_home.as_deref(),
                policy.workspace_trusted,
            );
            let cfg = AgentRunConfig {
                session_id: Some(format!("auto_{}", rec.id)),
                resume: false,
                route: route.clone(),
                frozen_system_blocks: vec![FrozenSystemBlock {
                    id: "automation".into(),
                    text: format!("You are executing scheduled automation '{}'.", rec.name),
                }],
                host_capabilities: HostCapabilities {
                    interactive_user_input: false,
                    network_allowed: policy.network_allowed,
                },
                policy: policy.clone(),
                hooks,
                approval: None,
                max_steps: 24,
                force_tool_set: ForceToolSet::Auto,
                stream: false,
                json_schema: None,
                cancel: Arc::new(AtomicBool::new(false)),
                auto_recall_memory: true,
                inbox: None,
            };
            let outcome = engine
                .run(&ctx, &rec.prompt, cfg, |_| {})
                .map(|r| r.final_text)
                .map_err(|e| e.to_string());
            rec.updated_at_ms = now_ms();
            let _ = suite.upsert_record(KIND_AUTOMATION, &rec.id, &rec);
            results.push((rec.id, outcome));
        }
        results
    }
}
