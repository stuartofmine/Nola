// SPDX-License-Identifier: Apache-2.0
//! Unified loop & error-streak guard with shared `semantic_observation_arguments`
//! and automatic reset on `VersionTransition::changed()` or 3 clean batches.

use nola_types::{RuntimeReminder, ToolExecutionResult};
use serde_json::{json, Value};

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
        _ => args.clone(),
    };
    format!(
        "{tool_name}:{}",
        serde_json::to_string(&normalized).unwrap_or_default()
    )
}

#[derive(Debug, Default)]
pub struct LoopGuard {
    last_signature: Option<String>,
    identical_streak: usize,
    error_streak: usize,
    clean_batches: usize,
}

impl LoopGuard {
    /// Observe a completed batch of `(tool_name, args, result)` and optionally return
    /// an ephemeral tail `RuntimeReminder` nudging the model to change strategy.
    pub fn observe_batch(
        &mut self,
        batch: &[(String, Value, ToolExecutionResult)],
    ) -> Option<RuntimeReminder> {
        let any_changed = batch.iter().any(|(_, _, r)| r.has_changed_version());
        let all_ok = batch.iter().all(|(_, _, r)| !r.is_error);

        if any_changed {
            self.reset();
            return None;
        }

        let sig = batch
            .iter()
            .map(|(n, a, _)| semantic_observation_arguments(n, a))
            .collect::<Vec<_>>()
            .join("|");
        let is_repeat = self.last_signature.as_deref() == Some(sig.as_str());

        if is_repeat {
            self.identical_streak += 1;
            self.clean_batches = 0;
        } else {
            self.last_signature = Some(sig);
            self.identical_streak = 1;
        }

        if all_ok {
            self.error_streak = 0;
            if !is_repeat {
                self.clean_batches += 1;
                if self.clean_batches >= 3 {
                    self.identical_streak = 1;
                }
            }
        } else {
            self.error_streak += 1;
            self.clean_batches = 0;
        }

        if self.identical_streak >= 3 {
            return Some(RuntimeReminder {
                tag: "loop_guard".into(),
                text: "You repeated the exact same tool call 3+ times without modifying workspace state. Stop repeating it, synthesize what you already observed, and try a different approach or conclude.".into(),
            });
        }
        if self.error_streak >= 4 {
            return Some(RuntimeReminder {
                tag: "error_recovery".into(),
                text: "The last 4 tool batches returned errors without changing workspace state. Inspect the exact error messages, verify paths/arguments, and adjust strategy before retrying.".into(),
            });
        }
        None
    }

    pub fn reset(&mut self) {
        self.last_signature = None;
        self.identical_streak = 0;
        self.error_streak = 0;
        self.clean_batches = 0;
    }
}
