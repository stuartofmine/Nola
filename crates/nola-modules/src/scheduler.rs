// SPDX-License-Identifier: Apache-2.0
//! 5-field Cron Expression Evaluator, Unified Web+Core Automation Parser,
//! Execution Stats Persistence, and Background Automation Scheduler.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Timelike, Utc};
use nola_engine::{
    AgentEngine, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner, ModelCaller,
};
use nola_llm::{LlmClient, LlmError};
use nola_store::Store;
use nola_tools::{path_within_workspace, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    now_ms, AutomationRecord, FrozenSystemBlock, HostCapabilities, ModelRouteConfig,
    ModelStreamDelta, NormalizedModelResponse, StructuredModelRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::automations_messaging::KIND_AUTOMATION;
use crate::ModulesSuite;

pub const KIND_AUTOMATION_STATS: &str = "automation_stats";

/// Alias so callers using either `ModelRoute` or `ModelRouteConfig` compile cleanly.
pub type ModelRoute = ModelRouteConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationRunStats {
    pub id: String,
    pub last_run_ms: Option<i64>,
    pub last_status: String,
    pub success_count: u64,
    pub failed_count: u64,
    pub next_scheduled_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub updated_at_ms: i64,
}

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

/// Derive a standard 5-field cron expression from Web UI `frequency`, `time` (`HH:MM`), and optional `weekday`.
pub fn derive_cron_from_frequency(
    frequency: &str,
    time_str: Option<&str>,
    weekday: Option<&Value>,
) -> String {
    let (hour, minute) = parse_hh_mm(time_str.unwrap_or("09:00"));
    match frequency.trim().to_ascii_lowercase().as_str() {
        "every_5m" | "5m" => "*/5 * * * *".into(),
        "every_15m" | "15m" => "*/15 * * * *".into(),
        "every_30m" | "30m" => "*/30 * * * *".into(),
        "hourly" => format!("{minute} * * * *"),
        "weekdays" => format!("{minute} {hour} * * 1-5"),
        "weekly" => {
            let dow = parse_weekday(weekday).unwrap_or(1);
            format!("{minute} {hour} * * {dow}")
        }
        "monthly" => format!("{minute} {hour} 1 * *"),
        _ => format!("{minute} {hour} * * *"), // default daily
    }
}

fn parse_hh_mm(raw: &str) -> (u32, u32) {
    let trimmed = raw.trim();
    // Accept both "HH:MM" and ISO strings containing "THH:MM"
    let time_part = trimmed
        .split('T')
        .nth(1)
        .unwrap_or(trimmed);
    let mut parts = time_part.split(':');
    let hour = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|&h| h <= 23)
        .unwrap_or(9);
    let minute = parts
        .next()
        .and_then(|s| s.chars().take(2).collect::<String>().parse::<u32>().ok())
        .filter(|&m| m <= 59)
        .unwrap_or(0);
    (hour, minute)
}

fn parse_weekday(raw: Option<&Value>) -> Option<u32> {
    let v = raw?;
    if let Some(n) = v.as_u64() {
        return (n <= 7).then_some(if n == 7 { 0 } else { n as u32 });
    }
    let s = v.as_str()?.trim().to_ascii_lowercase();
    match s.as_str() {
        "0" | "7" | "sun" | "sunday" => Some(0),
        "1" | "mon" | "monday" => Some(1),
        "2" | "tue" | "tuesday" => Some(2),
        "3" | "wed" | "wednesday" => Some(3),
        "4" | "thu" | "thursday" => Some(4),
        "5" | "fri" | "friday" => Some(5),
        "6" | "sat" | "saturday" => Some(6),
        _ => None,
    }
}

/// Parse BOTH canonical `AutomationRecord` AND Web-normalized automation JSON into an `AutomationRecord`.
pub fn parse_automation_value(raw: &Value, default_workspace: &Path) -> Option<AutomationRecord> {
    if let Ok(mut canonical) = serde_json::from_value::<AutomationRecord>(raw.clone()) {
        if canonical.workspace_root.trim().is_empty() {
            canonical.workspace_root = default_workspace.display().to_string();
        }
        if canonical.schedule.split_whitespace().count() != 5 {
            if let Some(freq) = raw.get("frequency").and_then(|v| v.as_str()) {
                canonical.schedule = derive_cron_from_frequency(
                    freq,
                    raw.get("time").and_then(|v| v.as_str()),
                    raw.get("weekday"),
                );
            }
        }
        return Some(canonical);
    }

    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())?
        .to_string();
    let name = raw
        .get("name")
        .or_else(|| raw.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("Automation")
        .to_string();
    let prompt = raw
        .get("prompt")
        .or_else(|| raw.get("userPrompt"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let explicit_schedule = raw
        .get("schedule")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| s.split_whitespace().count() == 5);
    let schedule = match explicit_schedule {
        Some(s) => s.to_string(),
        None => {
            let frequency = raw
                .get("frequency")
                .and_then(|v| v.as_str())
                .unwrap_or("daily");
            derive_cron_from_frequency(
                frequency,
                raw.get("time").and_then(|v| v.as_str()),
                raw.get("weekday"),
            )
        }
    };
    let workspace_root = raw
        .get("workspace_root")
        .or_else(|| raw.get("workspaceRoot"))
        .or_else(|| raw.get("scriptCwd"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| default_workspace.display().to_string());
    let enabled = raw.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    let last_run_ms = raw
        .get("last_run_ms")
        .or_else(|| raw.get("lastRunMs"))
        .and_then(|v| v.as_i64());
    let updated_at_ms = raw
        .get("updated_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);

    Some(AutomationRecord {
        id,
        name,
        schedule,
        prompt,
        workspace_root,
        enabled,
        last_run_ms,
        updated_at_ms,
    })
}

/// Compute the next UTC timestamp (in ms) matching a 5-field cron expression after `after`
/// (scanning up to 366 days using day/hour stepping when month/day/hour do not match).
pub fn next_scheduled_at_ms(cron_expr: &str, after: DateTime<Utc>) -> Option<i64> {
    let fields: Vec<&str> = cron_expr.split_whitespace().collect();
    if fields.len() != 5 {
        return None;
    }
    let start = after
        .with_second(0)
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(after)
        + ChronoDuration::minutes(1);
    let end = start + ChronoDuration::days(366);
    let mut candidate = start;
    while candidate < end {
        if !field_matches(fields[3], candidate.month(), 1, 12, false)
            || !field_matches(fields[2], candidate.day(), 1, 31, false)
            || !field_matches(
                fields[4],
                candidate.weekday().num_days_from_sunday(),
                0,
                7,
                true,
            )
        {
            candidate = (candidate + ChronoDuration::days(1))
                .with_hour(0)
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap_or(candidate + ChronoDuration::days(1));
            continue;
        }
        if !field_matches(fields[1], candidate.hour(), 0, 23, false) {
            candidate = (candidate + ChronoDuration::hours(1))
                .with_minute(0)
                .unwrap_or(candidate + ChronoDuration::hours(1));
            continue;
        }
        if field_matches(fields[0], candidate.minute(), 0, 59, false) {
            return Some(candidate.timestamp_millis());
        }
        candidate += ChronoDuration::minutes(1);
    }
    None
}

/// Persist run outcome stats into `module_records(kind = "automation_stats")`.
pub fn record_automation_run_stats(
    store: &Store,
    id: &str,
    schedule: &str,
    run_at: DateTime<Utc>,
    success: bool,
    error: Option<&str>,
) -> Result<AutomationRunStats, String> {
    let mut stats = store
        .get_module_record::<AutomationRunStats>(KIND_AUTOMATION_STATS, id)
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| AutomationRunStats {
            id: id.to_string(),
            last_run_ms: None,
            last_status: "idle".into(),
            success_count: 0,
            failed_count: 0,
            next_scheduled_at_ms: None,
            last_error: None,
            updated_at_ms: now_ms(),
        });

    stats.last_run_ms = Some(run_at.timestamp_millis());
    if success {
        stats.last_status = "success".into();
        stats.success_count = stats.success_count.saturating_add(1);
        stats.last_error = None;
    } else {
        stats.last_status = "failed".into();
        stats.failed_count = stats.failed_count.saturating_add(1);
        stats.last_error = error.map(|s| s.chars().take(500).collect());
    }
    stats.next_scheduled_at_ms = next_scheduled_at_ms(schedule, run_at);
    stats.updated_at_ms = now_ms();

    store
        .put_module_record(KIND_AUTOMATION_STATS, id, &stats)
        .map_err(|e| e.to_string())?;
    Ok(stats)
}

/// Aggregate `(total_success_count, total_failed_count, earliest_next_scheduled_at_ms)`
/// across all persisted `automation_stats` records for `build_dashboard_snapshot`.
pub fn automation_stats_summary(store: &Store) -> (u64, u64, Option<i64>) {
    let stats: Vec<AutomationRunStats> = store
        .list_module_records(KIND_AUTOMATION_STATS)
        .unwrap_or_default();
    let mut success_total = 0u64;
    let mut failed_total = 0u64;
    let mut next_earliest: Option<i64> = None;
    for s in stats {
        success_total = success_total.saturating_add(s.success_count);
        failed_total = failed_total.saturating_add(s.failed_count);
        if let Some(next_ms) = s.next_scheduled_at_ms {
            next_earliest = Some(match next_earliest {
                Some(cur) => cur.min(next_ms),
                None => next_ms,
            });
        }
    }
    (success_total, failed_total, next_earliest)
}

struct LlmClientCaller {
    client: Arc<LlmClient>,
}

impl ModelCaller for LlmClientCaller {
    fn call(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        self.client.complete(route, req, stream, on_delta)
    }
}

/// Spawn a clean background daemon thread ticking every 30s until the returned `stop` flag is set.
pub fn spawn_background_scheduler(
    store: Store,
    workspace_root: PathBuf,
    llm: Option<Arc<LlmClient>>,
    route: Option<ModelRoute>,
) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_worker = Arc::clone(&stop);
    let _ = std::thread::Builder::new()
        .name("nola-automation-scheduler".into())
        .spawn(move || {
            let suite = Arc::new(ModulesSuite::new(store, workspace_root, None));
            while !stop_worker.load(Ordering::Relaxed) {
                if let (Some(ref client), Some(ref model_route)) = (&llm, &route) {
                    let caller: Arc<dyn ModelCaller> = Arc::new(LlmClientCaller {
                        client: Arc::clone(client),
                    });
                    let policy = EffectivePolicy::default();
                    let _ = AutomationScheduler::run_due(
                        &suite,
                        caller,
                        model_route.clone(),
                        policy,
                        Utc::now(),
                    );
                }
                // Sleep 30 seconds in 1-second slices so shutdown is responsive.
                for _ in 0..30 {
                    if stop_worker.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        });
    stop
}

/// Evaluates all enabled `AutomationRecord`s (both canonical and Web-normalized)
/// and executes those due at `at`.
pub struct AutomationScheduler;

impl AutomationScheduler {
    pub fn find_due(suite: &ModulesSuite, at: DateTime<Utc>) -> Vec<AutomationRecord> {
        let now = at.timestamp_millis();
        suite
            .list_records::<Value>(KIND_AUTOMATION)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|raw| parse_automation_value(&raw, &suite.workspace_root))
            .filter(|rec| {
                if !rec.enabled || !cron_matches(&rec.schedule, at) {
                    return false;
                }
                // Check both record.last_run_ms and persisted automation_stats.last_run_ms
                let stats_last = suite
                    .store
                    .get_module_record::<AutomationRunStats>(KIND_AUTOMATION_STATS, &rec.id)
                    .ok()
                    .flatten()
                    .and_then(|s| s.last_run_ms);
                let effective_last = match (rec.last_run_ms, stats_last) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                };
                // Guard against firing twice in the same UTC minute bucket
                effective_last
                    .map(|last| last.div_euclid(60_000) < now.div_euclid(60_000))
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
        for rec in due {
            let rec_ws = Path::new(&rec.workspace_root);
            if !path_within_workspace(&suite.workspace_root, rec_ws) {
                let err = format!(
                    "automation workspace '{}' escapes active workspace root",
                    rec.workspace_root
                );
                let _ = record_automation_run_stats(
                    &suite.store,
                    &rec.id,
                    &rec.schedule,
                    at,
                    false,
                    Some(&err),
                );
                results.push((rec.id.clone(), Err(err)));
                continue;
            }
            // Stamp last_run_ms BEFORE execution by patching only timestamp fields on the raw
            // stored JSON so Web-normalized fields (`userPrompt`, `frequency`, `time`, etc.) are preserved!
            patch_automation_timestamps(suite, &rec, Some(at.timestamp_millis()), now_ms());

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

            match &outcome {
                Ok(_) => {
                    let _ = record_automation_run_stats(
                        &suite.store,
                        &rec.id,
                        &rec.schedule,
                        at,
                        true,
                        None,
                    );
                }
                Err(err) => {
                    let _ = record_automation_run_stats(
                        &suite.store,
                        &rec.id,
                        &rec.schedule,
                        at,
                        false,
                        Some(err),
                    );
                }
            }

            patch_automation_timestamps(suite, &rec, Some(at.timestamp_millis()), now_ms());
            results.push((rec.id, outcome));
        }
        results
    }
}

fn patch_automation_timestamps(
    suite: &ModulesSuite,
    rec: &AutomationRecord,
    last_run_ms: Option<i64>,
    updated_at_ms: i64,
) {
    if let Ok(Some(mut raw)) = suite.get_record::<Value>(KIND_AUTOMATION, &rec.id) {
        if let Some(obj) = raw.as_object_mut() {
            if let Some(ts) = last_run_ms {
                obj.insert("last_run_ms".into(), Value::Number(ts.into()));
                if obj.contains_key("lastRunMs") {
                    obj.insert("lastRunMs".into(), Value::Number(ts.into()));
                }
            }
            obj.insert("updated_at_ms".into(), Value::Number(updated_at_ms.into()));
            let _ = suite.upsert_record(KIND_AUTOMATION, &rec.id, &raw);
            return;
        }
    }
    let mut fallback = rec.clone();
    if let Some(ts) = last_run_ms {
        fallback.last_run_ms = Some(ts);
    }
    fallback.updated_at_ms = updated_at_ms;
    let _ = suite.upsert_record(KIND_AUTOMATION, &rec.id, &fallback);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scheduler_parses_web_normalized_automations_and_tracks_stats() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

        // Save a Web-normalized automation record (no `schedule`, uses `userPrompt`, `frequency`, `time`)
        store
            .put_module_record(
                KIND_AUTOMATION,
                "web-auto-1",
                &json!({
                    "id": "web-auto-1",
                    "name": "Morning Digest",
                    "userPrompt": "Summarize overnight CI status",
                    "frequency": "daily",
                    "time": "09:30",
                    "timezone": "UTC",
                    "enabled": true
                }),
            )
            .unwrap();

        let at_due = DateTime::parse_from_rfc3339("2026-11-25T09:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let at_other = DateTime::parse_from_rfc3339("2026-11-25T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let due = AutomationScheduler::find_due(&suite, at_due);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "web-auto-1");
        assert_eq!(due[0].schedule, "30 9 * * *");
        assert_eq!(due[0].prompt, "Summarize overnight CI status");

        let not_due = AutomationScheduler::find_due(&suite, at_other);
        assert!(not_due.is_empty());

        // Record stats and verify aggregation + duplicate-fire prevention
        record_automation_run_stats(&store, "web-auto-1", "30 9 * * *", at_due, true, None)
            .unwrap();
        record_automation_run_stats(
            &store,
            "web-auto-2",
            "0 * * * *",
            at_due,
            false,
            Some("timeout"),
        )
        .unwrap();

        let (succ, fail, next_ms) = automation_stats_summary(&store);
        assert_eq!(succ, 1);
        assert_eq!(fail, 1);
        assert!(next_ms.is_some());

        // Same minute must not double-fire now that stats recorded last_run_ms
        let due_again = AutomationScheduler::find_due(&suite, at_due);
        assert!(due_again.is_empty());

        // Background scheduler stops cleanly when signaled
        let stop = spawn_background_scheduler(store, dir.path().to_path_buf(), None, None);
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn scheduler_minute_bucket_366d_next_and_raw_patch_preserve_web_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

        store
            .put_module_record(
                KIND_AUTOMATION,
                "web-preserve",
                &json!({
                    "id": "web-preserve",
                    "name": "Yearly Report",
                    "userPrompt": "Run yearly summary",
                    "frequency": "monthly",
                    "time": "09:00",
                    "customWebField": "keep_me",
                    "enabled": true
                }),
            )
            .unwrap();

        let rec = parse_automation_value(
            &store
                .get_module_record::<Value>(KIND_AUTOMATION, "web-preserve")
                .unwrap()
                .unwrap(),
            dir.path(),
        )
        .unwrap();
        patch_automation_timestamps(&suite, &rec, Some(1_700_000_000_000), 1_700_000_001_000);
        let patched: Value = store
            .get_module_record(KIND_AUTOMATION, "web-preserve")
            .unwrap()
            .unwrap();
        assert_eq!(patched["customWebField"], "keep_me");
        assert_eq!(patched["userPrompt"], "Run yearly summary");
        assert_eq!(patched["last_run_ms"], 1_700_000_000_000i64);

        // Yearly cron (>8 days away) is found within 366 days!
        let jan2 = DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let next_jan1 = next_scheduled_at_ms("0 9 1 1 *", jan2);
        assert!(next_jan1.is_some());
        let dt = DateTime::from_timestamp_millis(next_jan1.unwrap()).unwrap();
        assert_eq!((dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute()), (2027, 1, 1, 9, 0));
    }
}
