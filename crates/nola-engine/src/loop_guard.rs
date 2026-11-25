// SPDX-License-Identifier: Apache-2.0
//! Unified loop, resource-family investigation stall & error-streak guard with:
//! - Canonical sorted parallel tool-call signatures (order-independent deduplication)
//! - `InvestigationIdentity` resource-family stall detection (catches A->B->A->B alternation
//!   and drifting line-range scans on the same target)
//! - Hard circuit breaker (`LoopGuardVerdict::TerminateNoProgress`) after 6 repeated
//!   identical/error/stalled batches without recovery
//! - Automatic reset on `VersionTransition::changed()`.

use std::collections::VecDeque;

use nola_types::{RuntimeReminder, ToolExecutionResult};
use serde_json::{json, Value};

const IDENTICAL_REMIND_THRESHOLD: usize = 3;
const ERROR_REMIND_THRESHOLD: usize = 4;
const INVESTIGATION_STALL_REMIND_THRESHOLD: usize = 3;
const HARD_CIRCUIT_BREAKER_THRESHOLD: usize = 6;
const INVESTIGATION_WINDOW_CAPACITY: usize = 8;

/// Resource-family identity normalizing read/search tool calls to `(tool_name, target_path_or_scope)`
/// so alternating A->B->A->B loops or drifting line-range scans on the same target are detected.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InvestigationIdentity {
    pub tool_name: String,
    pub target: String,
}

/// Verdict produced by `LoopGuard::observe_batch_verdict`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopGuardVerdict {
    Continue,
    Remind(RuntimeReminder),
    TerminateNoProgress(String),
}

/// Canonicalize tool arguments for duplicate-call detection so identical reads/searches
/// don't slip past due to JSON key order or whitespace differences.
pub fn semantic_observation_arguments(tool_name: &str, args: &Value) -> String {
    let normalized = match tool_name {
        "read_file" => json!({
            "path": args.get("path").and_then(|v| v.as_str()).unwrap_or("").trim(),
            "start_line": args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1),
            "end_line": args.get("end_line").and_then(|v| v.as_u64()).unwrap_or(0),
            "page": args.get("page").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "grep" => json!({
            "pattern": args.get("pattern").and_then(|v| v.as_str()).unwrap_or("").trim(),
            "path": args.get("path").and_then(|v| v.as_str()).unwrap_or("").trim(),
            "case_insensitive": args.get("case_insensitive").and_then(|v| v.as_bool()).unwrap_or(false),
        }),
        "bash" => json!({
            "command": args.get("command").and_then(|v| v.as_str()).unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" "),
        }),
        _ => canonicalize_json(args),
    };
    format!(
        "{tool_name}:{}",
        serde_json::to_string(&normalized).unwrap_or_default()
    )
}

/// Normalize read/search/inspection tool calls to `(tool_name, target_path_or_scope)`.
/// Returns `None` for mutating or non-investigation tools.
pub fn investigation_identity(tool_name: &str, args: &Value) -> Option<InvestigationIdentity> {
    let target = match tool_name {
        "read_file" | "file_outline" | "list_dir" | "lsp_diagnostics" => {
            normalize_scope_path(args.get("path").and_then(|v| v.as_str()).unwrap_or("."))
        }
        "read_many_files" => {
            let mut paths: Vec<String> = args
                .get("paths")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(normalize_scope_path)
                        .collect()
                })
                .unwrap_or_default();
            paths.sort();
            paths.dedup();
            if paths.is_empty() {
                ".".to_string()
            } else {
                paths.join(",")
            }
        }
        "grep" | "glob" => {
            let path = normalize_scope_path(args.get("path").and_then(|v| v.as_str()).unwrap_or("."));
            path
        }
        "repo_map" | "workspace_status" => {
            normalize_scope_path(args.get("path").and_then(|v| v.as_str()).unwrap_or("."))
        }
        "tool_output" => args
            .get("output_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        "git" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
            if !matches!(
                action,
                "status" | "diff" | "log" | "show" | "blame" | "worktree_list"
            ) {
                return None;
            }
            let scope = args
                .get("path")
                .or_else(|| args.get("target"))
                .and_then(|v| v.as_str())
                .map(normalize_scope_path)
                .unwrap_or_else(|| ".".to_string());
            format!("{action}:{scope}")
        }
        _ => return None,
    };

    Some(InvestigationIdentity {
        tool_name: tool_name.to_string(),
        target,
    })
}

fn normalize_scope_path(raw: &str) -> String {
    let trimmed = raw.trim().replace('\\', "/");
    let stripped = trimmed
        .strip_prefix("./")
        .unwrap_or(&trimmed)
        .trim_end_matches('/');
    if stripped.is_empty() {
        ".".to_string()
    } else {
        stripped.to_string()
    }
}

fn canonicalize_json(val: &Value) -> Value {
    match val {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::with_capacity(map.len());
            for k in keys {
                sorted.insert(k.clone(), canonicalize_json(&map[k]));
            }
            Value::Object(sorted)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

#[derive(Debug, Default)]
pub struct LoopGuard {
    last_signature: Option<String>,
    identical_streak: usize,
    error_streak: usize,
    clean_batches: usize,
    recent_investigations: VecDeque<InvestigationIdentity>,
    investigation_stall_streak: usize,
}

impl LoopGuard {
    /// Observe a completed batch of `(tool_name, args, result)` and return a rich
    /// `LoopGuardVerdict` (`Continue`, `Remind`, or hard circuit breaker `TerminateNoProgress`).
    pub fn observe_batch_verdict(
        &mut self,
        batch: &[(String, Value, ToolExecutionResult)],
    ) -> LoopGuardVerdict {
        if batch.is_empty() {
            return LoopGuardVerdict::Continue;
        }
        let any_changed = batch.iter().any(|(_, _, r)| r.has_changed_version());
        let all_ok = batch.iter().all(|(_, _, r)| !r.is_error);

        if any_changed {
            self.reset();
            return LoopGuardVerdict::Continue;
        }

        // Canonical sorted batch signature so permuting parallel tool calls cannot evade detection.
        let mut call_sigs: Vec<String> = batch
            .iter()
            .map(|(n, a, _)| semantic_observation_arguments(n, a))
            .collect();
        call_sigs.sort();
        let sig = call_sigs.join("|");
        let is_repeat = self.last_signature.as_deref() == Some(sig.as_str());

        if is_repeat {
            self.identical_streak += 1;
            self.clean_batches = 0;
        } else {
            self.last_signature = Some(sig);
            self.identical_streak = 1;
        }

        // Investigation resource-family stall tracking (catches A->B->A->B alternation & drifting line ranges).
        let inv_ids: Vec<Option<InvestigationIdentity>> = batch
            .iter()
            .map(|(n, a, _)| investigation_identity(n, a))
            .collect();
        if inv_ids.iter().all(|id| id.is_some()) {
            let mut batch_ids: Vec<InvestigationIdentity> =
                inv_ids.into_iter().flatten().collect();
            batch_ids.sort();
            batch_ids.dedup();
            let all_seen = !batch_ids.is_empty()
                && batch_ids
                    .iter()
                    .all(|id| self.recent_investigations.contains(id));
            if all_seen {
                self.investigation_stall_streak += 1;
                self.clean_batches = 0;
            } else {
                for id in batch_ids {
                    if !self.recent_investigations.contains(&id) {
                        if self.recent_investigations.len() >= INVESTIGATION_WINDOW_CAPACITY {
                            self.recent_investigations.pop_front();
                        }
                        self.recent_investigations.push_back(id);
                    }
                }
                self.investigation_stall_streak = 0;
            }
        } else {
            self.investigation_stall_streak = 0;
        }

        if all_ok {
            self.error_streak = 0;
            if !is_repeat && self.investigation_stall_streak == 0 {
                self.clean_batches += 1;
                if self.clean_batches >= 3 {
                    self.identical_streak = 1;
                }
            }
        } else {
            self.error_streak += 1;
            self.clean_batches = 0;
        }

        // Hard circuit breaker: terminate cleanly without burning unlimited steps!
        if self.identical_streak >= HARD_CIRCUIT_BREAKER_THRESHOLD {
            let summary = format!(
                "No-progress circuit breaker triggered: repeated the same tool batch {} consecutive times without modifying workspace state.",
                self.identical_streak
            );
            return LoopGuardVerdict::TerminateNoProgress(summary);
        }
        if self.error_streak >= HARD_CIRCUIT_BREAKER_THRESHOLD {
            let summary = format!(
                "No-progress circuit breaker triggered: {} consecutive tool batches returned errors without modifying workspace state.",
                self.error_streak
            );
            return LoopGuardVerdict::TerminateNoProgress(summary);
        }
        if self.investigation_stall_streak >= HARD_CIRCUIT_BREAKER_THRESHOLD {
            let targets: Vec<String> = self
                .recent_investigations
                .iter()
                .map(|id| format!("{}({})", id.tool_name, id.target))
                .collect();
            let summary = format!(
                "No-progress circuit breaker triggered: {} consecutive non-advancing investigation batches revisiting the same target family [{}] without making workspace progress.",
                self.investigation_stall_streak,
                targets.join(", ")
            );
            return LoopGuardVerdict::TerminateNoProgress(summary);
        }

        // Soft nudge reminders prior to circuit breaker
        if self.identical_streak >= IDENTICAL_REMIND_THRESHOLD {
            return LoopGuardVerdict::Remind(RuntimeReminder {
                tag: "loop_guard".into(),
                text: "You repeated the exact same tool call 3+ times without modifying workspace state. Stop repeating it, synthesize what you already observed, and try a different approach or conclude.".into(),
            });
        }
        if self.error_streak >= ERROR_REMIND_THRESHOLD {
            return LoopGuardVerdict::Remind(RuntimeReminder {
                tag: "error_recovery".into(),
                text: "The last 4 tool batches returned errors without changing workspace state. Inspect the exact error messages, verify paths/arguments, and adjust strategy before retrying.".into(),
            });
        }
        if self.investigation_stall_streak >= INVESTIGATION_STALL_REMIND_THRESHOLD {
            return LoopGuardVerdict::Remind(RuntimeReminder {
                tag: "investigation_stall".into(),
                text: "You have repeatedly inspected the same file/search targets without modifying workspace state. Synthesize what you already know, move to concrete edits/verification, or conclude.".into(),
            });
        }
        LoopGuardVerdict::Continue
    }

    /// Observe a completed batch of `(tool_name, args, result)` and optionally return
    /// an ephemeral tail `RuntimeReminder` nudging the model to change strategy.
    pub fn observe_batch(
        &mut self,
        batch: &[(String, Value, ToolExecutionResult)],
    ) -> Option<RuntimeReminder> {
        match self.observe_batch_verdict(batch) {
            LoopGuardVerdict::Continue => None,
            LoopGuardVerdict::Remind(reminder) => Some(reminder),
            LoopGuardVerdict::TerminateNoProgress(summary) => Some(RuntimeReminder {
                tag: "loop_guard_circuit_breaker".into(),
                text: summary,
            }),
        }
    }

    pub fn reset(&mut self) {
        self.last_signature = None;
        self.identical_streak = 0;
        self.error_streak = 0;
        self.clean_batches = 0;
        self.recent_investigations.clear();
        self.investigation_stall_streak = 0;
    }
}
