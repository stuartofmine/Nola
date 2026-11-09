// SPDX-License-Identifier: Apache-2.0
//! Host-side discovery and protocol metadata for installed Agent CLIs.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use nola_core::agent::host::{DelegatedAgentContextMode, DelegatedAgentRequestV1};
use nola_domain::execution_backend::{
    ExecutionBackendAvailability, ExecutionBackendCapabilities, ExecutionBackendDescriptor,
    ExecutionBackendKind, ExecutionBackendModelDescriptor, ExecutionBackendTransport,
    EXECUTION_BACKEND_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::ops::{Deref, DerefMut};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::external_execution_settings::ExternalExecutionSettings;
#[cfg(test)]
use crate::config::Config;
use crate::runtime_control::RuntimeExecutionControl;

pub(crate) trait ExternalExecutionObserver {
    fn external_text(&mut self, text: &str) -> Result<(), String>;
    fn external_reasoning(&mut self, text: &str) -> Result<(), String>;
    fn external_session_started(&mut self, backend: &str, session_id: &str) -> Result<(), String>;
    fn external_permission_requested(
        &mut self,
        request_id: &str,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        reason: &str,
    ) -> Result<Option<bool>, String>;
    fn external_permission_written(
        &mut self,
        request_id: &str,
        call_id: &str,
    ) -> Result<(), String>;
    fn external_tool_started(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: &Value,
    ) -> Result<(), String>;
    fn external_tool_finished(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: &Value,
        output: &str,
        failed: bool,
    ) -> Result<(), String>;
    fn external_context_usage(
        &mut self,
        used_tokens: usize,
        context_window_tokens: usize,
    ) -> Result<(), String>;
    fn external_accounting(&mut self, _snapshot: nola_domain::delegated_invocation::ExternalUsageSnapshot) -> Result<(), String> { Ok(()) }
    /// Called only after the owned process group has been stopped and reaped.
    fn external_execution_cancelled(&mut self, _backend: &str) -> Result<(), String> { Ok(()) }
    fn external_execution_completed(&mut self, backend: &str) -> Result<(), String>;
}

// Protocol events are observations of one admitted executor session, not
// authority to rebind it. A successful/approval-looking foreign frame is invalid.
fn validate_external_session(expected: Option<&str>, reported: Option<&str>) -> Result<(), String> {
    let reported = reported.filter(|id| !id.is_empty() && id.trim() == *id
        && id.len() <= 512 && !id.chars().any(char::is_control))
        .ok_or("external executor event has no exact session identity")?;
    if expected.is_some_and(|expected| expected != reported) {
        return Err("external executor event belongs to another session".into());
    }
    Ok(())
}

fn validate_codex_turn_frame(value: &Value, thread: Option<&str>, turn: &mut Option<String>) -> Result<(), String> {
    let method = value.get("method").and_then(Value::as_str).unwrap_or_default();
    if method.starts_with("item/") || method.starts_with("turn/")
        || method == "thread/tokenUsage/updated" {
        validate_external_session(thread, value.pointer("/params/threadId").and_then(Value::as_str))?;
        if method == "turn/started" {
            let reported = value.pointer("/params/turn/id").and_then(Value::as_str);
            validate_external_session(turn.as_deref(), reported)?;
            *turn = reported.map(str::to_owned);
        } else if method.starts_with("item/") || method == "turn/completed" {
            let expected = turn.as_deref().ok_or("Codex event arrived before the owned turn started")?;
            let reported = if method == "turn/completed" { value.pointer("/params/turn/id") }
                else { value.pointer("/params/turnId") }.and_then(Value::as_str);
            validate_external_session(Some(expected), reported)?;
        }
    }

    if value.get("id").and_then(Value::as_u64) == Some(10) && value.get("error").is_none() {
        let reported = value.pointer("/result/turn/id").and_then(Value::as_str);
        validate_external_session(turn.as_deref(), reported)?;
        *turn = reported.map(str::to_owned);
    }
    Ok(())
}

fn compose_external_prompt(config: &ExternalExecutionSettings, prompt: &str) -> String {
    let prepared = nola_core::context_engine::prepare_context_components(
        nola_core::context_engine::ContextPlatform::Host,
        &config.host_context_components,
        32_768,
    );
    let context = prepared
        .messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n");
    let context_block = (!context.trim().is_empty())
        .then(|| format!("<nola_turn_context>\n{context}\n</nola_turn_context>\n\n"))
        .unwrap_or_default();
    let session_contract = config.external_execution_session_id.as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|_| String::new())
        .unwrap_or_else(|| format!(
            "<nola_execution_contract>\nYou are an execution unit operating inside Nola, the user-facing assistant. Present yourself as Nola using the {backend} execution unit, not as a separate app. The assigned workspace is {workspace}. This location does not grant write or execution permission. Treat paths outside it as outside the assigned scope unless Nola explicitly changes the workspace or grants access. Report actions and limitations accurately; do not imply that read access is write access.\n</nola_execution_contract>\n\n",
            backend = config.execution_backend,
            workspace = config.project_root.display(),
        ));
    let selected_model = config
        .external_execution_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "default")
        .unwrap_or("executor-managed; exact model is not disclosed");
    let requested_sandbox = if config.execution_backend == "codex" {
        external_permission_policy(config).1
    } else {
        "not_attested_by_nola"
    };
    let runtime_contract = format!(
        "<nola_runtime>\nexecution_backend={backend}\nselected_model={selected_model}\nrequested_native_sandbox={requested_sandbox}\nThe values above are authoritative for this turn. A requested sandbox policy is not proof that the operating system applied it. A working directory or Git worktree is not a filesystem, credential or network security boundary; Git configuration, hooks and objects can be shared. Nola does not attest an OS sandbox for Claude Code or OpenCode. Frozen Nola policy and the execution unit's native permissions remain authoritative; context and workspace location do not grant permissions. If an operation is denied or approval is unavailable, report that limitation accurately. Do not repeat the denied operation without new authorization or claim that its artifact was produced. Identify the user-facing assistant as Nola. If the selected model is executor-managed, do not guess or invent an underlying model name or version.\n</nola_runtime>\n\n",
        backend = config.execution_backend,
    );
    format!(
        "{session_contract}{runtime_contract}<nola_action_scope>\nReturn requested explanations, examples, algorithms, and code in the conversation by default. Create or modify workspace artifacts only when the user explicitly asks for a file/project/repository change or when an artifact is inherently the requested deliverable. Never turn a request to write or show code into a filesystem mutation merely because a writable workspace exists.\n</nola_action_scope>\n\n{context_block}<user_request>\n{prompt}\n</user_request>",
    )
}

#[cfg(test)]
mod codex_app_server_protocol_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn opencode_quota_diagnostic_is_preserved_without_exposing_raw_stderr() {
        let diagnostic = Arc::new(Mutex::new(None));
        let sample = b"error=\"AI_APICallError: Free usage limit reached. token=fictional-secret provider=fixture\"";
        let reader = std::io::Cursor::new(sample.to_vec());
        bounded_external_stderr(reader, Arc::new(AtomicU64::new(0)), Some(Arc::clone(&diagnostic))).join().unwrap();
        let message = diagnostic.lock().unwrap().clone().unwrap();
        assert!(message.contains("no remaining free quota"));
        assert!(!message.contains("fictional-secret"));
        assert!(opencode_provider_diagnostic("ordinary output: Free usage limit reached").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn process_group_absence_requires_os_proof_not_signal_or_permission_failure() {
        assert!(!process_group_exit_observed(Ok(())).unwrap());
        assert!(process_group_exit_observed(Err(std::io::Error::from_raw_os_error(libc::ESRCH))).unwrap());
        assert!(process_group_exit_observed(Err(std::io::Error::from_raw_os_error(libc::EPERM))).is_err());
    }

    #[test]
    fn external_event_session_binding_rejects_foreign_and_missing_owners() {
        for reported in [None, Some(""), Some("other-session"), Some(" session ")] {
            assert!(validate_external_session(Some("session"), reported).is_err());
        }
        validate_external_session(Some("session"), Some("session")).unwrap();
        validate_external_session(None, Some("new-session")).unwrap();
    }

    #[test]
    fn codex_foreign_or_previous_turn_cannot_complete_or_request_permission() {
        let mut turn = None;
        let started = serde_json::json!({"method":"turn/started","params":{"threadId":"thread","turn":{"id":"owned-turn"}}});
        validate_codex_turn_frame(&started, Some("thread"), &mut turn).unwrap();
        validate_codex_turn_frame(&serde_json::json!({"id":10,"result":{"turn":{"id":"owned-turn"}}}), Some("thread"), &mut turn).unwrap();
        for (thread_id, turn_id) in [("foreign-thread","owned-turn"),("thread","previous-turn")] {
            let completed = serde_json::json!({"method":"turn/completed","params":{"threadId":thread_id,"turn":{"id":turn_id,"status":"completed"}}});
            assert!(matches!(parse_codex_app_server_message(&completed).as_slice(), [ExternalExecutorEvent::Completed { .. }]),
                "the parser alone cannot authorize completion");
            assert!(validate_codex_turn_frame(&completed, Some("thread"), &mut turn).is_err());
            let approval = serde_json::json!({"id":"approval","method":"item/commandExecution/requestApproval",
                "params":{"threadId":thread_id,"turnId":turn_id,"itemId":"command"}});
            assert!(validate_codex_turn_frame(&approval, Some("thread"), &mut turn).is_err());
            assert_eq!(turn.as_deref(), Some("owned-turn"));
        }
        validate_codex_turn_frame(&serde_json::json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"owned-turn","status":"completed"}}}), Some("thread"), &mut turn).unwrap();
    }

    #[test]
    fn resumed_external_prompt_keeps_authoritative_runtime_identity() {
        let mut config = ExternalExecutionSettings {
            project_root: std::path::PathBuf::from("/fictional-workspace"), execution_backend: "claude_code".into(),
            external_execution_session_id: None, external_execution_model: None, external_execution_model_provider: None,
            host_context_components: vec![], host_attachments: vec![], budget: Default::default(),
            permission_profile: crate::permissions::PermissionProfile::Cautious,
            host_input_origin: nola_core::agent::AgentInputOrigin::InteractiveUser,
        };
        config.execution_backend = "claude_code".into();
        config.external_execution_session_id = Some("session-1".into());
        config.external_execution_model = Some("default".into());
        let prompt = compose_external_prompt(&config, "which model are you using?");
        assert!(prompt.contains("execution_backend=claude_code"));
        assert!(prompt.contains("exact model is not disclosed"));
        assert!(prompt.contains("do not guess or invent an underlying model"));
        assert!(prompt.contains("native permissions remain authoritative"));
        assert!(prompt.contains("Do not repeat the denied operation without new authorization"));
        assert!(prompt.contains("requested_native_sandbox=not_attested_by_nola"));
        assert!(prompt.contains("Git worktree is not a filesystem"));
        config.external_execution_session_id = None;
        let initial = compose_external_prompt(&config, "inspect the project");
        assert!(initial.contains("assigned workspace"));
        assert!(!initial.contains("active writable workspace"));
        config.execution_backend = "codex".into();
        assert!(compose_external_prompt(&config, "inspect").contains("requested_native_sandbox=workspace-write"));
        config.permission_profile = crate::permissions::PermissionProfile::AllowAll;
        assert!(compose_external_prompt(&config, "inspect").contains("requested_native_sandbox=danger-full-access"));
    }

    #[test]
    fn app_server_streams_agent_and_reasoning_deltas_without_snapshot_duplication() {
        assert_eq!(
            parse_codex_app_server_message(&serde_json::json!({
                "method":"item/agentMessage/delta",
                "params":{"threadId":"thread-1","turnId":"turn-1","itemId":"item-1","delta":"hello"}
            })),
            vec![ExternalExecutorEvent::TextDelta {
                text: "hello".into()
            }]
        );
        assert_eq!(
            parse_codex_app_server_message(&serde_json::json!({
                "method":"item/reasoning/summaryTextDelta",
                "params":{"threadId":"thread-1","turnId":"turn-1","itemId":"item-2","summaryIndex":0,"delta":"inspect"}
            })),
            vec![ExternalExecutorEvent::ReasoningDelta {
                text: "inspect".into()
            }]
        );
        assert!(parse_codex_app_server_message(&serde_json::json!({
            "method":"item/completed",
            "params":{"item":{"id":"item-1","type":"agentMessage","text":"hello"}}
        }))
        .is_empty());
    }

    #[test]
    fn app_server_projects_command_lifecycle_with_arguments_and_result() {
        let started = parse_codex_app_server_message(&serde_json::json!({
            "method":"item/started",
            "params":{"item":{"id":"cmd-1","type":"commandExecution","command":"pwd","cwd":"/tmp","status":"inProgress","commandActions":[]}}
        }));
        assert_eq!(
            started,
            vec![ExternalExecutorEvent::ToolStarted {
                call_id: "cmd-1".into(),
                name: "shell".into(),
                arguments: serde_json::json!({"command":"pwd","cwd":"/tmp"}),
            }]
        );
        let finished = parse_codex_app_server_message(&serde_json::json!({
            "method":"item/completed",
            "params":{"item":{"id":"cmd-1","type":"commandExecution","command":"pwd","cwd":"/tmp","status":"completed","commandActions":[],"aggregatedOutput":"/tmp\n","exitCode":0}}
        }));
        assert_eq!(
            finished,
            vec![ExternalExecutorEvent::ToolFinished {
                call_id: "cmd-1".into(),
                name: "shell".into(),
                output: "/tmp\n".into(),
                failed: false,
            }]
        );
    }

    #[test]
    fn app_server_projects_web_search_lifecycle_with_query_and_results() {
        let started = parse_codex_app_server_message(&serde_json::json!({
            "method":"item/started",
            "params":{"item":{
                "id":"search-1",
                "type":"webSearch",
                "query":"2026年8月16日 今日新闻",
                "action":{"type":"search","queries":["2026年8月16日 今日新闻"]},
                "results":null
            }}
        }));
        assert_eq!(
            started,
            vec![ExternalExecutorEvent::ToolStarted {
                call_id: "search-1".into(),
                name: "web_search".into(),
                arguments: serde_json::json!({
                    "query":"2026年8月16日 今日新闻",
                    "action":{"type":"search","queries":["2026年8月16日 今日新闻"]}
                }),
            }]
        );

        let results = serde_json::json!([{
            "title":"Example headline",
            "url":"https://example.com/news",
            "snippet":"Example summary"
        }]);
        let finished = parse_codex_app_server_message(&serde_json::json!({
            "method":"item/completed",
            "params":{"item":{
                "id":"search-1",
                "type":"webSearch",
                "query":"2026年8月16日 今日新闻",
                "action":{"type":"search","queries":["2026年8月16日 今日新闻"]},
                "results":results
            }}
        }));
        assert_eq!(
            finished,
            vec![ExternalExecutorEvent::ToolFinished {
                call_id: "search-1".into(),
                name: "web_search".into(),
                output: results.to_string(),
                failed: false,
            }]
        );
    }

    #[test]
    fn app_server_turn_failure_is_terminal_and_preserves_error() {
        assert_eq!(
            parse_codex_app_server_message(&serde_json::json!({
                "method":"turn/completed",
                "params":{"turn":{"id":"turn-1","status":"failed","items":[],"error":{"message":"model unavailable"}}}
            })),
            vec![ExternalExecutorEvent::Failed {
                message: "model unavailable".into()
            }]
        );
    }

    #[test]
    fn app_server_projects_authoritative_context_usage() {
        assert_eq!(
            parse_codex_app_server_message(&serde_json::json!({
                "method":"thread/tokenUsage/updated",
                "params":{
                    "tokenUsage":{
                        "total":{"inputTokens":169902,"outputTokens":4200},
                        "last":{"inputTokens":33084,"outputTokens":900},
                        "modelContextWindow":258400
                    }
                }
            })),
            vec![ExternalExecutorEvent::Usage {
                used_tokens: 33084,
                context_window_tokens: 258400,
            }]
        );
    }

    #[test]
    fn app_server_permission_policy_matches_two_explicit_invocation_modes() {
        let mut config = Config::for_test().unwrap().external_execution_settings();
        config.permission_profile = crate::permissions::PermissionProfile::Cautious;
        // Defaults have already been resolved by composition.
        assert_eq!(
            external_permission_policy(&config),
            ("on-request", "workspace-write")
        );

        config.permission_profile = crate::permissions::PermissionProfile::Cautious;
        assert_eq!(
            external_permission_policy(&config),
            ("on-request", "workspace-write")
        );

        config.permission_profile = crate::permissions::PermissionProfile::AllowAll;
        assert_eq!(
            external_permission_policy(&config),
            ("never", "danger-full-access")
        );

        for origin in [
            nola_core::agent::AgentInputOrigin::ScheduledAction,
            nola_core::agent::AgentInputOrigin::Replay,
            nola_core::agent::AgentInputOrigin::SystemRecovery,
            nola_core::agent::AgentInputOrigin::GoalContinuation,
            nola_core::agent::AgentInputOrigin::DelegatedAgent,
        ] {
            config.host_input_origin = origin;
            assert_eq!(
                external_permission_policy(&config),
                ("never", "danger-full-access"),
                "{origin:?} uses its own frozen allow-all, not an interactive grant"
            );
        }

        config.host_input_origin = nola_core::agent::AgentInputOrigin::GoalInteractiveUser;
        assert_eq!(
            external_permission_policy(&config),
            ("never", "danger-full-access")
        );
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExternalExecutorEvent {
    Started {
        session_id: String,
    },
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolStarted {
        call_id: String,
        name: String,
        arguments: Value,
    },
    ToolFinished {
        call_id: String,
        name: String,
        output: String,
        failed: bool,
    },
    Usage {
        used_tokens: u64,
        context_window_tokens: u64,
    },
    Completed {
        session_id: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalExecutorCommand {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub prompt_via_stdin: bool,
}

const DEFAULT_EXTERNAL_WALL_TIME_SECONDS: u64 = 24 * 60 * 60;
const DEFAULT_EXTERNAL_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_EXTERNAL_CPU_TIME_SECONDS: u64 = 12 * 60 * 60;
const DEFAULT_EXTERNAL_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const DEFAULT_EXTERNAL_MAX_PROCESSES: u32 = 128;
const DEFAULT_EXTERNAL_LINE_BYTES: usize = 8 * 1024 * 1024;
const EXTERNAL_STDERR_PREVIEW_BYTES: usize = 64 * 1024;
const EXTERNAL_RESOURCE_SAMPLE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
struct ExternalExecutionBudget {
    max_wall_time: Duration,
    max_output_bytes: u64,
    max_cpu_time_ms: u64,
    max_memory_bytes: u64,
    max_processes: u32,
    max_line_bytes: usize,
}

impl ExternalExecutionBudget {
    fn from_config(config: &ExternalExecutionSettings) -> Self {
        let runtime = &config.budget;
        let max_output_bytes = runtime
            .max_output_bytes
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(DEFAULT_EXTERNAL_OUTPUT_BYTES);
        Self {
            max_wall_time: Duration::from_secs(
                runtime
                    .max_wall_time_seconds
                    .and_then(|value| u64::try_from(value).ok())
                    .unwrap_or(DEFAULT_EXTERNAL_WALL_TIME_SECONDS),
            ),
            max_output_bytes,
            max_cpu_time_ms: runtime
                .max_cpu_time_seconds
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(DEFAULT_EXTERNAL_CPU_TIME_SECONDS)
                .saturating_mul(1_000),
            max_memory_bytes: runtime
                .max_memory_bytes
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(DEFAULT_EXTERNAL_MEMORY_BYTES),
            max_processes: runtime
                .max_processes
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(DEFAULT_EXTERNAL_MAX_PROCESSES),
            max_line_bytes: runtime
                .max_line_bytes
                .unwrap_or(DEFAULT_EXTERNAL_LINE_BYTES)
                .min(usize::try_from(max_output_bytes).unwrap_or(usize::MAX)),
        }
    }
}

struct ExternalExecutionBudgetMonitor {
    budget: ExternalExecutionBudget,
    started: Instant,
    output_bytes: Arc<AtomicU64>,
    last_sample: Instant,
}

impl ExternalExecutionBudgetMonitor {
    fn new(budget: ExternalExecutionBudget, output_bytes: Arc<AtomicU64>) -> Self {
        Self {
            budget,
            started: Instant::now(),
            output_bytes,
            last_sample: Instant::now() - EXTERNAL_RESOURCE_SAMPLE_INTERVAL,
        }
    }

    fn check(&mut self, child: &mut std::process::Child) -> Result<(), String> {
        let output_bytes = self.output_bytes.load(Ordering::Relaxed);
        if output_bytes > self.budget.max_output_bytes {
            return Err(format!(
                "External executor exceeded its output budget of {} bytes (observed {output_bytes} bytes)",
                self.budget.max_output_bytes
            ));
        }
        if self.started.elapsed() > self.budget.max_wall_time {
            return Err(format!(
                "External executor exceeded its wall-clock budget of {} seconds",
                self.budget.max_wall_time.as_secs()
            ));
        }
        if self.last_sample.elapsed() < EXTERNAL_RESOURCE_SAMPLE_INTERVAL {
            return Ok(());
        }
        self.last_sample = Instant::now();
        #[cfg(any(unix, windows))]
        {
            if child.try_wait().map_err(|error| format!("Observe external executor exit failed: {error}"))?.is_some() {
                return Ok(());
            }
            let root_pid = child.id();
            let sample = crate::task_runtime::process_tree_usage_if_present(root_pid, None)
                .map_err(|error| format!("External executor resource sampling failed: {error}"))?;
            let Some(usage) = sample else {
                // The child may exit between try_wait and the OS snapshot.
                // Retain its real cached exit status and continue draining the
                // protocol, never replace absence with fabricated zero usage.
                if child.try_wait().map_err(|error| format!("Observe external executor exit failed: {error}"))?.is_some() {
                    return Ok(());
                }
                return Err(format!("External executor resource sampling found no live root for pid {root_pid} without confirmed exit"));
            };
            if usage.cpu_time_ms > self.budget.max_cpu_time_ms {
                return Err(format!(
                    "External executor exceeded its CPU-time budget of {} ms (observed {} ms)",
                    self.budget.max_cpu_time_ms, usage.cpu_time_ms
                ));
            }
            if usage.memory_bytes > self.budget.max_memory_bytes {
                return Err(format!(
                    "External executor exceeded its memory budget of {} bytes (observed {} bytes)",
                    self.budget.max_memory_bytes, usage.memory_bytes
                ));
            }
            if usage.processes > self.budget.max_processes {
                return Err(format!(
                    "External executor exceeded its process-count budget of {} (observed {})",
                    self.budget.max_processes, usage.processes
                ));
            }
        }
        Ok(())
    }
}

fn bounded_external_line_receiver<R: Read + Send + 'static>(
    mut reader: R,
    max_line_bytes: usize,
    output_bytes: Arc<AtomicU64>,
) -> mpsc::Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let mut pending = Vec::<u8>::new();
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            let count = match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) => {
                    let _ = tx.send(Err(format!(
                        "read external executor output failed: {error}"
                    )));
                    return;
                }
            };
            output_bytes.fetch_add(count as u64, Ordering::Relaxed);
            for byte in &chunk[..count] {
                if *byte == b'\n' {
                    if pending.last() == Some(&b'\r') {
                        pending.pop();
                    }
                    let line = String::from_utf8_lossy(&pending).into_owned();
                    pending.clear();
                    if tx.send(Ok(line)).is_err() {
                        return;
                    }
                } else {
                    pending.push(*byte);
                    if pending.len() > max_line_bytes {
                        let _ = tx.send(Err(format!(
                            "External executor exceeded its protocol-line budget of {max_line_bytes} bytes"
                        )));
                        return;
                    }
                }
            }
        }
        if !pending.is_empty() {
            let _ = tx.send(Ok(String::from_utf8_lossy(&pending).into_owned()));
        }
    });
    rx
}

fn opencode_provider_diagnostic(stderr: &str) -> Option<&'static str> {
    // Only classify the executor's API error. Never surface raw log bodies,
    // headers, credentials or embedded user/tool content as a UI error.
    (stderr.contains("AI_APICallError:") && stderr.contains("Free usage limit reached"))
        .then_some("OpenCode provider usage limit reached: the selected model has no remaining free quota. Choose an already configured model or replenish its quota. Nola did not switch providers.")
}

fn bounded_external_stderr<R: Read + Send + 'static>(
    mut reader: R,
    output_bytes: Arc<AtomicU64>,
    provider_diagnostic: Option<Arc<Mutex<Option<String>>>>,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut preview = Vec::with_capacity(EXTERNAL_STDERR_PREVIEW_BYTES);
        let mut diagnostic_tail = Vec::new();
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            let count = match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => count,
                Err(_) => break,
            };
            output_bytes.fetch_add(count as u64, Ordering::Relaxed);
            if let Some(diagnostic) = provider_diagnostic.as_ref() {
                diagnostic_tail.extend_from_slice(&chunk[..count]);
                if diagnostic_tail.len() > 16 * 1024 {
                    diagnostic_tail.drain(..diagnostic_tail.len() - 16 * 1024);
                }
                if let Some(message) = opencode_provider_diagnostic(&String::from_utf8_lossy(&diagnostic_tail)) {
                    if let Ok(mut value) = diagnostic.lock() { *value = Some(message.into()); }
                }
            }
            let remaining = EXTERNAL_STDERR_PREVIEW_BYTES.saturating_sub(preview.len());
            preview.extend_from_slice(&chunk[..count.min(remaining)]);
        }
        String::from_utf8_lossy(&preview).into_owned()
    })
}

pub(crate) fn run_delegated(
    config: &ExternalExecutionSettings,
    request: &DelegatedAgentRequestV1,
    authorized_roots: &[PathBuf],
    observer: &mut dyn ExternalExecutionObserver,
    cancellation: Option<RuntimeExecutionControl>,
) -> Result<(), String> {
    request.validate().map_err(str::to_string)?;
    let mut delegated_config = config.clone();
    if let Some(primary)=request.settings.get("primary_worktree") {
        let primary=serde_json::from_value(primary.clone()).map_err(|_|"Invalid Core primary workspace binding")?;
        delegated_config.project_root=crate::tools::primary_execution_root(&primary,&config.project_root,authorized_roots)?;
    }
    delegated_config.execution_backend = request.backend_id.clone();
    delegated_config.external_execution_model = request.model_id.clone();
    delegated_config.external_execution_session_id = match request.context.mode {
        DelegatedAgentContextMode::Bootstrap => None,
        DelegatedAgentContextMode::Resume => request.backend_session_id.clone(),
    };
    delegated_config.host_context_components.clear();
    let messages = serde_json::to_string(&request.context.messages)
        .map_err(|error| format!("encode delegated Agent messages failed: {error}"))?;
    let prompt = format!(
        "<nola_delegated_messages contract_version=\"{}\" mode=\"{}\" base_anchor=\"{}\" target_anchor=\"{}\">\n{}\n</nola_delegated_messages>",
        request.contract_version,
        match request.context.mode {
            DelegatedAgentContextMode::Bootstrap => "bootstrap",
            DelegatedAgentContextMode::Resume => "resume",
        },
        request.context.base_anchor_item_id.as_deref().unwrap_or_default(),
        request.context.target_anchor_item_id,
        messages,
    );
    run_inner(&delegated_config, &prompt, true, observer, cancellation)
}

fn run_inner(
    config: &ExternalExecutionSettings,
    prompt: &str,
    prompt_is_prepared: bool,
    emitter: &mut dyn ExternalExecutionObserver,
    cancellation: Option<RuntimeExecutionControl>,
) -> Result<(), String> {
    let backend_kind = match config.execution_backend.as_str() {
        "codex" => ExecutionBackendKind::Codex,
        "claude_code" => ExecutionBackendKind::ClaudeCode,
        "opencode" => ExecutionBackendKind::OpenCode,
        other => return Err(format!("unknown execution backend: {other}")),
    };
    // Model discovery is a catalog concern. In particular, Codex model/list
    // requires an App Server handshake and used to add a complete redundant
    // cold start before every actual turn.
    let descriptor = discover(backend_kind, false);
    if descriptor.kind == ExecutionBackendKind::Codex {
        return run_codex_app_server(
            config,
            prompt,
            prompt_is_prepared,
            emitter,
            cancellation,
            &descriptor,
        );
    }
    if descriptor.kind == ExecutionBackendKind::OpenCode {
        return run_opencode_acp(
            config,
            prompt,
            prompt_is_prepared,
            emitter,
            cancellation,
            &descriptor,
        );
    }
    run_stream_json_process(
        config,
        prompt,
        prompt_is_prepared,
        emitter,
        cancellation,
        &descriptor,
    )
}

pub(crate) fn run_stream_json_process(
    config: &ExternalExecutionSettings,
    prompt: &str,
    prompt_is_prepared: bool,
    emitter: &mut dyn ExternalExecutionObserver,
    cancellation: Option<RuntimeExecutionControl>,
    descriptor: &ExecutionBackendDescriptor,
) -> Result<(), String> {
    let mut command = command_for(
        descriptor,
        &config.project_root,
        config.external_execution_session_id.as_deref(),
        config.external_execution_model.as_deref(),
        config.external_execution_model_provider.as_deref(),
    )?;
    let composed_prompt = if prompt_is_prepared {
        prompt.to_string()
    } else {
        compose_external_prompt(config, prompt)
    };
    if !command.prompt_via_stdin {
        command.arguments.push(composed_prompt.clone());
    }
    let mut child_command = external_command(&command.executable);
    let child = child_command
        .args(&command.arguments)
        .current_dir(&config.project_root)
        .stdin(if command.prompt_via_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start {} failed: {error}", descriptor.display_name))?;
    let mut child = ManagedExternalChild::new(child);
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "external executor stdout is unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "external executor stderr is unavailable".to_string())?;
    let budget = ExternalExecutionBudget::from_config(config);
    let output_bytes = Arc::new(AtomicU64::new(0));
    let rx =
        bounded_external_line_receiver(stdout, budget.max_line_bytes, Arc::clone(&output_bytes));
    let stderr_handle = bounded_external_stderr(stderr, Arc::clone(&output_bytes), None);
    let mut budget_monitor = ExternalExecutionBudgetMonitor::new(budget, output_bytes);
    let is_claude = descriptor.kind == ExecutionBackendKind::ClaudeCode;
    // Claude's permission replies use the same stdin as its prompt. Closing it
    // after the initial user frame makes later native approvals impossible.
    let mut stdin = if command.prompt_via_stdin { Some(child.stdin.take()
        .ok_or("external executor stdin is unavailable")?) } else { None };
    let mut claude = ClaudeControl::default();
    let setup_started = Instant::now();
    let setup_timeout = duration_from_environment("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS", 30, 1, 300);
    if is_claude {
        write_external_json(&mut stdin, &claude.initialize(), &mut child, cancellation.as_ref(),
            &mut budget_monitor, emitter, &descriptor.id)?;
    } else if command.prompt_via_stdin {
        write_external_input(&mut stdin, composed_prompt.clone().into_bytes(), &mut child, cancellation.as_ref(),
            &mut budget_monitor, emitter, &descriptor.id)?;
        drop(stdin.take());
    }
    let mut terminal = false;
    let mut announced_session_id: Option<String> = None;
    let mut tool_calls = HashMap::<String, (String, Value)>::new();
    let mut exit_status = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(line)) => {
                if let Ok(frame) = serde_json::from_str::<Value>(&line) {
                    if is_claude && frame["type"] == "control_response" {
                        claude.accept_initialize(&frame)?;
                        let input = claude_stream_input(config, &composed_prompt)?;
                        write_external_input(&mut stdin, input.into_bytes(), &mut child, cancellation.as_ref(),
                            &mut budget_monitor, emitter, &descriptor.id)?;
                        continue;
                    }
                    if is_claude && frame["type"] == "control_request" {
                        let permission = claude.permission(&frame, announced_session_id.as_deref(), &tool_calls)?;
                        let decision = emitter.external_permission_requested(&permission.request_id,
                            &permission.call_id, &permission.tool_name, &permission.input,
                            "Claude Code requests permission for this operation");
                        // Cancellation of the Core permission wait still needs
                        // the same real process-stop observation as other exits.
                        if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) {
                            return confirm_external_cancellation(&mut child, emitter, &descriptor.id);
                        }
                        let response = permission.response(decision?)?;
                        write_external_json(&mut stdin, &response, &mut child, cancellation.as_ref(),
                            &mut budget_monitor, emitter, &descriptor.id)?;
                        emitter.external_permission_written(&permission.request_id, &permission.call_id)?;
                        continue;
                    }
                    if is_claude && !claude.initialized {
                        return Err("Claude Code emitted execution before protocol initialization".into());
                    }
                    if frame.get("session_id").is_some()
                        || matches!(frame.get("type").and_then(Value::as_str), Some("result" | "stream_event")) {
                        validate_external_session(announced_session_id.as_deref()
                            .or(config.external_execution_session_id.as_deref()),
                            frame.get("session_id").and_then(Value::as_str))?;
                    }
                    if is_claude { claude.observe_tasks(&frame); }
                    if let Some(snapshot) = external_accounting_snapshot(&descriptor.id, &frame) {
                        emitter.external_accounting(snapshot)?;
                    }
                }
                for event in parse_event_line(descriptor.kind, &line) {
                    match event {
                        ExternalExecutorEvent::Started { session_id } => {
                            if announced_session_id.as_deref() != Some(session_id.as_str()) {
                                emitter.external_session_started(&descriptor.id, &session_id)?;
                                if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) {
                                    return confirm_external_cancellation(&mut child, emitter, &descriptor.id);
                                }
                                announced_session_id = Some(session_id);
                            }
                        }
                        ExternalExecutorEvent::TextDelta { text } => {
                            emitter.external_text(&text)?
                        }
                        ExternalExecutorEvent::ReasoningDelta { text } => {
                            emitter.external_reasoning(&text)?
                        }
                        ExternalExecutorEvent::ToolStarted {
                            call_id,
                            name,
                            arguments,
                        } => {
                            if is_claude { claude.validate_tool(&call_id, &name, &arguments)?; }
                            tool_calls.insert(call_id.clone(), (name.clone(), arguments.clone()));
                            emitter.external_tool_started(&call_id, &name, &arguments)?;
                        }
                        ExternalExecutorEvent::ToolFinished {
                            call_id,
                            name,
                            output,
                            failed,
                        } => {
                            let (resolved_name, arguments) = tool_calls
                                .remove(&call_id)
                                .map(|(stored_name, arguments)| {
                                    let resolved = if stored_name.trim().is_empty() {
                                        name.clone()
                                    } else {
                                        stored_name
                                    };
                                    (resolved, arguments)
                                })
                                .unwrap_or((name, Value::Null));
                            emitter.external_tool_finished(
                                &call_id,
                                &resolved_name,
                                &arguments,
                                &output,
                                failed,
                            )?;
                        }
                        ExternalExecutorEvent::Usage {
                            used_tokens,
                            context_window_tokens,
                        } => {
                            emitter.external_context_usage(
                                used_tokens as usize,
                                context_window_tokens as usize,
                            )?;
                        }
                        ExternalExecutorEvent::Completed { session_id, .. } => {
                            if let Some(id) = session_id {
                                if announced_session_id.as_deref() != Some(id.as_str()) {
                                    emitter.external_session_started(&descriptor.id, &id)?;
                                    announced_session_id = Some(id);
                                }
                            }
                            // A turn result with native children still in flight
                            // is not the final result of this owned execution.
                            terminal = !is_claude || claude.tasks.is_empty();
                            if is_claude && terminal { drop(stdin.take()); }
                        }
                        ExternalExecutorEvent::Failed { message } => return Err(message),
                    }
                }
            }
            Ok(Err(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if is_claude && !claude.initialized && setup_started.elapsed() >= setup_timeout {
            return Err("Claude Code initialize handshake timed out".into());
        }
        budget_monitor.check(&mut child)?;
        if cancellation
            .as_ref()
            .is_some_and(RuntimeExecutionControl::is_requested)
        {
            return confirm_external_cancellation(&mut child, emitter, &descriptor.id);
        }
        if exit_status.is_none() {
            exit_status = child
                .try_wait()
                .map_err(|error| format!("wait for external executor failed: {error}"))?;
        }
    }
    let status = match exit_status {
        Some(status) => status,
        None => child
            .wait()
            .map_err(|error| format!("wait for external executor failed: {error}"))?,
    };
    let stderr = stderr_handle.join().unwrap_or_default();
    if !status.success() {
        return Err(if stderr.is_empty() {
            format!("{} exited with {status}", descriptor.display_name)
        } else {
            stderr
        });
    }
    if is_claude && !claude.tasks.is_empty() {
        return Err("Claude Code exited with native child tasks still unconfirmed".into());
    }
    if !terminal {
        return Err(format!(
            "{} ended without a terminal event",
            descriptor.display_name
        ));
    }
    emitter.external_execution_completed(&descriptor.id)?;
    child.disarm();
    Ok(())
}

pub(crate) fn run_opencode_acp(
    config: &ExternalExecutionSettings,
    prompt: &str,
    prompt_is_prepared: bool,
    emitter: &mut dyn ExternalExecutionObserver,
    cancellation: Option<RuntimeExecutionControl>,
    descriptor: &ExecutionBackendDescriptor,
) -> Result<(), String> {
    let command = command_for(descriptor, &config.project_root,
        config.external_execution_session_id.as_deref(),
        config.external_execution_model.as_deref(),
        config.external_execution_model_provider.as_deref())?;
    let cwd = config.project_root.to_string_lossy().into_owned();
    let mut child_command = external_command(&command.executable);
    let child = child_command
        .args(&command.arguments)
        .current_dir(&config.project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start OpenCode ACP failed: {error}"))?;
    let mut child = ManagedExternalChild::new(child);
    let mut stdin = Some(child.stdin.take().ok_or("OpenCode ACP stdin is unavailable")?);
    let stdout = child
        .stdout
        .take()
        .ok_or("OpenCode ACP stdout is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("OpenCode ACP stderr is unavailable")?;
    let budget = ExternalExecutionBudget::from_config(config);
    let output_bytes = Arc::new(AtomicU64::new(0));
    let rx = bounded_external_line_receiver(stdout, budget.max_line_bytes, Arc::clone(&output_bytes));
    let provider_diagnostic = Arc::new(Mutex::new(None));
    let stderr_handle = bounded_external_stderr(stderr, Arc::clone(&output_bytes), Some(Arc::clone(&provider_diagnostic)));
    let mut budget_monitor = ExternalExecutionBudgetMonitor::new(budget, output_bytes);

    let mut cleanup_session: Option<String> = None;
    let mut cancel_notified = false;
    let protocol_result = (|| -> Result<bool, String> {
    write_external_json(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize",
            "params":{
                "protocolVersion":1,
                "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},
                "clientInfo":{"name":"Nola","version":env!("CARGO_PKG_VERSION")}
            }
        }), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
    let initialized = read_acp_response(&rx, 1, &mut child, cancellation.as_ref(), &mut budget_monitor, emitter)?;
    if initialized
        .pointer("/protocolVersion")
        .and_then(Value::as_u64)
        != Some(1)
    {
        return Err("OpenCode ACP did not negotiate protocol version 1".to_string());
    }

    let requested_session = config
        .external_execution_session_id
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let mut active_session = None;
    if let Some(session_id) = requested_session {
        let supports_resume = initialized
            .pointer("/agentCapabilities/sessionCapabilities/resume")
            .is_some_and(|value| value.as_bool().unwrap_or(true));
        let method = if supports_resume {
            "session/resume"
        } else {
            "session/load"
        };
        write_external_json(
            &mut stdin,
            &serde_json::json!({
                "jsonrpc":"2.0", "id":2, "method":method,
                "params":{"sessionId":session_id,"cwd":cwd,"mcpServers":[],"additionalDirectories":[]}
            }), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
        let resumed = read_acp_response(&rx, 2, &mut child, cancellation.as_ref(), &mut budget_monitor, emitter)?;
        if resumed.get("sessionId").is_some() {
            validate_external_session(Some(session_id), resumed.get("sessionId").and_then(Value::as_str))?;
        }
        active_session = Some(session_id.to_string());
    }
    if active_session.is_none() {
        write_external_json(
            &mut stdin,
            &serde_json::json!({
                "jsonrpc":"2.0", "id":3, "method":"session/new",
                "params":{"cwd":cwd,"mcpServers":[],"additionalDirectories":[]}
            }), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
        let created = read_acp_response(&rx, 3, &mut child, cancellation.as_ref(), &mut budget_monitor, emitter)?;
        active_session = created
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    let session_id = active_session.ok_or("OpenCode ACP session response did not include an id")?;
    cleanup_session = Some(session_id.clone());
    emitter.external_session_started(&descriptor.id, &session_id)?;
    if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) {
        return Err("external execution cancelled".into());
    }

    // ACP selects the model on the session; `opencode acp` has no --model flag.
    // Wait for acceptance before dispatch so a frozen selection cannot silently
    // run on the executor's default provider/model.
    if let Some(model) = config.external_execution_model.as_deref() {
        write_external_json(&mut stdin, &serde_json::json!({
            "jsonrpc":"2.0", "id":5, "method":"session/set_config_option",
            "params":{"sessionId":session_id,"configId":"model","value":model}
        }), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
        read_acp_response(&rx, 5, &mut child, cancellation.as_ref(), &mut budget_monitor, emitter)?;
    }

    let prompt_content = external_image_content_blocks(
        &config.host_attachments,
        if prompt_is_prepared {
            prompt.to_string()
        } else {
            compose_external_prompt(config, prompt)
        },
        ExternalImageBlockStyle::Acp,
    )?;
    write_external_json(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc":"2.0", "id":4, "method":"session/prompt",
            "params":{"sessionId":session_id,"prompt":prompt_content}
        }), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
    let mut terminal = false;
    let mut cancel_sent: Option<Instant> = None;
    let mut tool_calls = HashMap::<String, (String, Value)>::new();
    let mut tool_inputs = HashMap::<String, Value>::new();
    loop {
        if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) && cancel_sent.is_none() {
            write_external_json(&mut stdin, &serde_json::json!({"jsonrpc":"2.0","method":"session/cancel",
                "params":{"sessionId":session_id}}), &mut child, None, &mut budget_monitor, emitter, &descriptor.id)?;
            cancel_notified = true;
            cancel_sent = Some(Instant::now());
        }
        if cancel_sent.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)) { break; }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(line)) => {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if matches!(value.get("method").and_then(Value::as_str),
                    Some("session/update" | "session/request_permission")) {
                    validate_external_session(Some(&session_id), value.pointer("/params/sessionId").and_then(Value::as_str))?;
                }
                if cancel_sent.is_some() {
                    if value.get("id").and_then(Value::as_u64) == Some(4) { terminal = true; break; }
                    continue;
                }
                if value.get("method").and_then(Value::as_str) == Some("session/request_permission")
                {
                    let id = value
                        .get("id")
                        .filter(|id| id.is_string() || id.is_number())
                        .ok_or("OpenCode permission request is missing its RPC identity")?;
                    let mut params = value
                        .get("params")
                        .cloned()
                        .ok_or("OpenCode permission request missing parameters")?;
                    let call_id = params
                        .pointer("/toolCall/toolCallId")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .ok_or("OpenCode permission request missing tool call identity")?
                        .to_string();
                    if params
                        .pointer("/toolCall/rawInput")
                        .is_none_or(|input| input.is_null())
                    {
                        let input = tool_inputs
                            .get(&call_id)
                            .cloned()
                            .ok_or("OpenCode permission request missing raw input")?;
                        params["toolCall"]["rawInput"] = input;
                    }
                    // Preserve the protocol request/call identity and complete resource facts.
                    // ACP kind/title are display metadata, never permission-policy selectors.
                    let request_id = id
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| id.to_string());
                    let decision = emitter.external_permission_requested(
                        &request_id,
                        &call_id,
                        "opencode.tool",
                        &params,
                        "External tool requests permission",
                    );
                    if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) {
                        return Err("external execution cancelled".into());
                    }
                    let outcome = opencode_acp_permission_outcome(&params, decision?)?;
                    write_external_json(
                        &mut stdin,
                        &serde_json::json!({
                            "jsonrpc":"2.0", "id":id, "result":{"outcome":outcome}
                        }), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
                    emitter.external_permission_written(&request_id, &call_id)?;
                    continue;
                }
                if value.get("method").and_then(Value::as_str) == Some("session/update") {
                    let update = value.pointer("/params/update").unwrap_or(&Value::Null);
                    if matches!(
                        update.get("sessionUpdate").and_then(Value::as_str),
                        Some("tool_call" | "tool_call_update")
                    ) {
                        let call_id = string_at(update, &["toolCallId"]);
                        if let Some(input) = update.get("rawInput").filter(|input| {
                            input.as_object().is_none_or(|object| !object.is_empty())
                        }) {
                            tool_inputs.insert(call_id, input.clone());
                        }
                    }
                    for event in parse_opencode_acp_update(update) {
                        match event {
                            ExternalExecutorEvent::TextDelta { text } => {
                                emitter.external_text(&text)?
                            }
                            ExternalExecutorEvent::ReasoningDelta { text } => {
                                emitter.external_reasoning(&text)?
                            }
                            ExternalExecutorEvent::ToolStarted {
                                call_id,
                                name,
                                arguments,
                            } => {
                                if !tool_calls.contains_key(&call_id) {
                                    tool_calls.insert(call_id.clone(), (name.clone(), arguments.clone()));
                                    emitter.external_tool_started(&call_id, &name, &arguments)?;
                                }
                            }
                            ExternalExecutorEvent::ToolFinished {
                                call_id,
                                name,
                                output,
                                failed,
                            } => {
                                let (name, arguments) = match tool_calls.remove(&call_id) {
                                    Some(call) => call,
                                    None => {
                                        let arguments = tool_inputs.remove(&call_id).ok_or("ACP tool completion has no concrete input")?;
                                        emitter.external_tool_started(&call_id, &name, &arguments)?;
                                        (name, arguments)
                                    }
                                };
                                emitter.external_tool_finished(
                                    &call_id, &name, &arguments, &output, failed,
                                )?;
                            }
                            ExternalExecutorEvent::Usage {
                                used_tokens,
                                context_window_tokens,
                            } => {
                                emitter.external_context_usage(
                                    used_tokens as usize,
                                    context_window_tokens as usize,
                                )?;
                            }
                            _ => {}
                        }
                    }
                } else if value.get("id").and_then(Value::as_u64) == Some(4) {
                    if let Some(snapshot)=external_accounting_snapshot("opencode",&value) {
                        emitter.external_accounting(snapshot)?;
                    }
                    if let Some(error) = value.get("error") {
                        return Err(format!(
                            "OpenCode ACP prompt failed: {}",
                            bounded_json(error, 2000)
                        ));
                    }
                    terminal = value.get("result").is_some();
                }
            }
            Ok(Err(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(message) = provider_diagnostic.lock().map_err(|_| "OpenCode diagnostic unavailable")?.clone() {
            return Err(message);
        }
        if let Err(sampling_error) = budget_monitor.check(&mut child) {
            // A provider rejection already observed by the stderr reader is
            // the actionable cause; preserve it over a later sampling error.
            return Err(provider_diagnostic.lock().ok().and_then(|value| value.clone()).unwrap_or(sampling_error));
        }
        if terminal {
            break;
        }
        if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) { continue; }
        // Do not race child.try_wait() against the stdout reader. An exited
        // process may still have terminal tool/update frames buffered in the
        // channel. Normal completion is controlled by the ACP response above;
        // abnormal exit is observed when the reader disconnects after drain.
    }
    if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) { return Ok(true); }
    if !terminal { return Err("OpenCode ACP ended before prompt completion".into()); }
    Ok(false)
    })();
    // A rejected output/permission commit can leave the loop before its next
    // cancellation poll. Still send the exact session control before EOF.
    if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) && !cancel_notified {
        if let Some(session_id) = cleanup_session.as_deref() {
            let _ = write_external_json(&mut stdin, &serde_json::json!({"jsonrpc":"2.0","method":"session/cancel",
                "params":{"sessionId":session_id}}), &mut child, None, &mut budget_monitor, emitter, &descriptor.id);
        }
    }
    // ACP is a persistent server. A completed prompt is not process exit:
    // close its input and observe normal shutdown instead of signalling it.
    drop(stdin);
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(5);
    let shutdown = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Ok(None) => break Err(format!("OpenCode EOF shutdown unconfirmed for pid {pid}; no signal sent")),
            Err(error) => break Err(format!("OpenCode exit observation failed: {error}")),
        }
    };
    // Never retry the previous OS-denied signal path, including from Drop.
    child.disarm();
    let stopped = shutdown.and_then(|status| {
        observe_owned_group_exit(pid, Duration::from_secs(5)).map_err(|error| format!("OpenCode process group shutdown unconfirmed: {error}"))?;
        Ok(status)
    });
    let status = match stopped {
        Ok(status) => status,
        Err(stop) => {
            eprintln!("Nola OpenCode shutdown observation unconfirmed pid={pid}: {stop}");
            return Err(match protocol_result { Err(cause) => format!("{cause}; {stop}"), _ => stop });
        }
    };
    while !stderr_handle.is_finished() {
        if Instant::now() >= deadline { return Err("OpenCode stderr shutdown unconfirmed".into()); }
        std::thread::sleep(Duration::from_millis(25));
    }
    let stderr = stderr_handle.join().unwrap_or_default();
    if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) {
        emitter.external_execution_cancelled(&descriptor.id)?;
        return Err("external execution cancelled".into());
    }
    protocol_result?;
    if !status.success() {
        return Err(opencode_provider_diagnostic(&stderr).map(str::to_string).unwrap_or_else(|| format!("OpenCode ACP exited with {status}")));
    }
    emitter.external_execution_completed(&descriptor.id)?;
    Ok(())
}

fn opencode_acp_permission_outcome(
    params: &Value,
    decision: Option<bool>,
) -> Result<Value, String> {
    let approved =
        decision.ok_or("OpenCode permission transport unavailable; no decision received")?;
    let kind = if approved {
        "allow_once"
    } else {
        "reject_once"
    };
    let option_id = params
        .get("options")
        .and_then(Value::as_array)
        .and_then(|options| {
            options
                .iter()
                .find(|option| option.get("kind").and_then(Value::as_str) == Some(kind))
        })
        .and_then(|option| option.get("optionId").and_then(Value::as_str))
        .ok_or_else(|| format!("OpenCode permission response has no {kind} option"))?;
    Ok(serde_json::json!({"outcome":"selected","optionId":option_id}))
}

fn read_acp_response(
    rx: &mpsc::Receiver<Result<String, String>>, expected_id: u64,
    child: &mut Child, cancellation: Option<&RuntimeExecutionControl>,
    monitor: &mut ExternalExecutionBudgetMonitor, _observer: &mut dyn ExternalExecutionObserver,
) -> Result<Value, String> {
    let deadline = Instant::now() + duration_from_environment("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS", 30, 1, 300);
    loop {
        if cancellation.is_some_and(RuntimeExecutionControl::is_requested) {
            return Err("external execution cancelled".into());
        }
        monitor.check(child)?;
        if Instant::now() >= deadline { return Err("OpenCode ACP initialize/session handshake timed out".into()); }
        let line = match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => line?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err("OpenCode ACP ended during setup".into()),
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue; };
        if value.get("id").and_then(Value::as_u64) != Some(expected_id) { continue; }
        if let Some(error) = value.get("error") {
            return Err(format!("OpenCode ACP request failed: {}", bounded_json(error, 1600)));
        }
        return value.get("result").cloned().ok_or_else(|| "OpenCode ACP response has no result".into());
    }
}

fn parse_opencode_acp_update(update: &Value) -> Vec<ExternalExecutorEvent> {
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "agent_message_chunk" => acp_content_text(update.get("content").unwrap_or(&Value::Null))
            .map(|text| ExternalExecutorEvent::TextDelta { text }).into_iter().collect(),
        "agent_thought_chunk" => acp_content_text(update.get("content").unwrap_or(&Value::Null))
            .map(|text| ExternalExecutorEvent::ReasoningDelta { text }).into_iter().collect(),
        "tool_call" | "tool_call_update" => {
            let mut events = Vec::new();
            let status = update.get("status").and_then(Value::as_str);
            let input = update.get("rawInput").filter(|input| !input.is_null()
                && input.as_object().is_none_or(|object| !object.is_empty()));
            // Pending ACP frames may contain only an empty placeholder. They
            // are not proof of the actual arguments the executor will use.
            if let Some(arguments) = input.filter(|_| status != Some("pending")) {
                events.push(ExternalExecutorEvent::ToolStarted {
                    call_id: string_at(update, &["toolCallId"]), name: string_at(update, &["title"]), arguments: arguments.clone(),
                });
            }
            if matches!(status, Some("completed" | "failed")) {
                events.push(ExternalExecutorEvent::ToolFinished {
                    call_id: string_at(update, &["toolCallId"]), name: string_at(update, &["title"]),
                    output: update.get("rawOutput").map(json_text).or_else(||update.get("content").map(json_text)).unwrap_or_default(),
                    failed: status == Some("failed"),
                });
            }
            events
        },
        "usage_update" => match (
            update.get("used").and_then(Value::as_u64),
            update.get("size").and_then(Value::as_u64),
        ) {
            (Some(used_tokens), Some(context_window_tokens)) if context_window_tokens > 0 => {
                vec![ExternalExecutorEvent::Usage {
                    used_tokens,
                    context_window_tokens,
                }]
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn acp_content_text(content: &Value) -> Option<String> {
    (content.get("type").and_then(Value::as_str) == Some("text"))
        .then(|| string_at(content, &["text"]))
        .filter(|value| !value.is_empty())
}

pub(crate) fn run_codex_app_server(
    config: &ExternalExecutionSettings,
    prompt: &str,
    prompt_is_prepared: bool,
    emitter: &mut dyn ExternalExecutionObserver,
    cancellation: Option<RuntimeExecutionControl>,
    descriptor: &ExecutionBackendDescriptor,
) -> Result<(), String> {
    let (approval_policy, sandbox) = external_permission_policy(config);
    let executable = descriptor
        .executable_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "Codex is not installed".to_string())?;
    let mut child_command = external_command(&executable);
    // Own this transport for the Run. A user's shared daemon may be occupied,
    // stale, or a different binary version; never start/restart it for Nola.
    // Codex's persisted thread ID remains the resume authority across children.
    let arguments = codex_app_server_arguments();
    let child = child_command
        .args(&arguments)
        .current_dir(&config.project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start Codex app-server failed: {error}"))?;
    let mut child = ManagedExternalChild::new(child);
    let mut stdin = Some(child.stdin.take().ok_or("Codex app-server stdin is unavailable")?);
    let stdout = child
        .stdout
        .take()
        .ok_or("Codex app-server stdout is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Codex app-server stderr is unavailable")?;
    let budget = ExternalExecutionBudget::from_config(config);
    let output_bytes = Arc::new(AtomicU64::new(0));
    let rx =
        bounded_external_line_receiver(stdout, budget.max_line_bytes, Arc::clone(&output_bytes));
    let stderr_handle = bounded_external_stderr(stderr, Arc::clone(&output_bytes), None);
    let mut budget_monitor = ExternalExecutionBudgetMonitor::new(budget, output_bytes);
    write_external_json(
        &mut stdin,
        &serde_json::json!({
            "id": 1,
            "method": "initialize",
            "params": {"clientInfo": {"name": "nola", "title": "Nola", "version": env!("CARGO_PKG_VERSION")}}
        }), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;

    let composed_prompt = if prompt_is_prepared {
        prompt.to_string()
    } else {
        compose_external_prompt(config, prompt)
    };
    let cwd = config.project_root.to_string_lossy().into_owned();
    let requested_session = config
        .external_execution_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let thread_request_id = 2_u64;
    let mut initialized = false;
    let mut thread_requested = false;
    let mut turn_requested = false;
    let mut terminal = false;
    let mut active_thread: Option<String> = None;
    let mut active_turn: Option<String> = None;
    let mut interrupt_requested: Option<Instant> = None;
    let mut cancelled_terminal = false;
    let mut tool_calls = HashMap::<String, (String, Value)>::new();
    let mut agent_message_boundary = CodexAgentMessageBoundary::default();
    let maximum_runtime = duration_from_environment(
        "NOLA_EXTERNAL_EXECUTION_MAX_SECONDS",
        12 * 60 * 60,
        60,
        7 * 24 * 60 * 60,
    );
    let deadline = Instant::now() + maximum_runtime;
    let mut last_protocol_progress = Instant::now();
    let setup_timeout = duration_from_environment(
        "NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS", 30, 1, 300,
    );
    let mut setup_started = Instant::now();
    let idle_timeout =
        duration_from_environment("NOLA_EXTERNAL_EXECUTION_IDLE_SECONDS", 15 * 60, 30, 60 * 60);

    'codex_turn: while Instant::now() < deadline {
        if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) {
            if let (Some(thread), Some(turn)) = (active_thread.as_deref(), active_turn.as_deref()) {
                if interrupt_requested.is_none() {
                    // Native protocol interruption owns the exact active turn.
                    // This control write must not be cancelled by its own token.
                    write_external_json(&mut stdin, &serde_json::json!({"id":11,"method":"turn/interrupt",
                        "params":{"threadId":thread,"turnId":turn}}), &mut child, None,
                        &mut budget_monitor, emitter, &descriptor.id)?;
                    interrupt_requested = Some(Instant::now());
                }
                if interrupt_requested.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)) {
                    return confirm_external_cancellation(&mut child, emitter, &descriptor.id);
                }
            }
        }
        if !turn_requested && setup_started.elapsed() >= setup_timeout {
            return Err(format!("Codex {} handshake timed out after {} seconds",
                if initialized { "thread" } else { "initialize" }, setup_timeout.as_secs()));
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(line)) => {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let response_id = value.get("id").and_then(Value::as_u64);
                let method = value
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                // Long-running commands may report output, terminal activity,
                // MCP progress, or newer protocol notifications. Any valid
                // response/notification proves the app-server is alive.
                if response_id.is_some() || !method.is_empty() {
                    last_protocol_progress = Instant::now();
                }
                if value.get("id").and_then(Value::as_u64) == Some(1) {
                    if let Some(error) = value.get("error") {
                        return Err(format!(
                            "Codex initialize failed: {}",
                            bounded_json(error, 1200)
                        ));
                    }
                    write_external_json(
                        &mut stdin,
                        &serde_json::json!({"method":"initialized","params":{}}), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
                    initialized = true;
                    setup_started = Instant::now();
                }
                if initialized && !thread_requested {
                    let mut params = serde_json::json!({
                        "cwd":cwd,
                        "approvalPolicy":approval_policy,
                        "sandbox":sandbox
                    });
                    if let Some(model) = config.external_execution_model.as_deref() {
                        params["model"] = Value::String(model.to_string());
                    }
                    if let Some(provider) = config.external_execution_model_provider.as_deref() {
                        params["modelProvider"] = Value::String(provider.to_string());
                    }
                    let request = if let Some(session_id) = requested_session.as_deref() {
                        params["threadId"] = Value::String(session_id.to_string());
                        serde_json::json!({"id":thread_request_id,"method":"thread/resume","params":params})
                    } else {
                        serde_json::json!({"id":thread_request_id,"method":"thread/start","params":params})
                    };
                    write_external_json(&mut stdin, &request, &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
                    thread_requested = true;
                }
                if value.get("id").and_then(Value::as_u64) == Some(thread_request_id) {
                    if let Some(error) = value.get("error") {
                        return Err(format!(
                            "Codex thread setup failed: {}",
                            bounded_json(error, 1200)
                        ));
                    }
                    active_thread = value
                        .pointer("/result/thread/id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let thread_id = active_thread
                        .as_deref()
                        .ok_or("Codex thread response did not include an id")?;
                    if requested_session
                        .as_deref()
                        .is_some_and(|expected| expected != thread_id)
                    {
                        return Err("Codex resume returned a different thread identity".into());
                    }
                    emitter.external_session_started(&descriptor.id, thread_id)?;
                    if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) {
                        return confirm_external_cancellation(&mut child, emitter, &descriptor.id);
                    }
                    let mut turn_input = vec![serde_json::json!({
                        "type": "text",
                        "text": composed_prompt
                    })];
                    turn_input.extend(
                        config
                            .host_attachments
                            .iter()
                            .filter(|attachment| attachment.mime_type.starts_with("image/"))
                            .map(|attachment| {
                                serde_json::json!({
                                    "type": "localImage",
                                    "path": attachment.path
                                })
                            }),
                    );
                    write_external_json(
                        &mut stdin,
                        &serde_json::json!({"id":10,"method":"turn/start","params":{
                            "threadId":thread_id,
                            "input":turn_input
                        }}), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
                    turn_requested = true;
                }
                if turn_requested {
                    validate_codex_turn_frame(&value, active_thread.as_deref(), &mut active_turn)?;
                    if interrupt_requested.is_some() {
                        if method == "turn/completed" && matches!(value.pointer("/params/turn/status").and_then(Value::as_str), Some("interrupted" | "completed")) {
                            terminal = true; cancelled_terminal = true; break 'codex_turn;
                        }
                        if response_id == Some(11) && value.get("error").is_some() {
                            return confirm_external_cancellation(&mut child, emitter, &descriptor.id);
                        }
                        // Keep consuming the protocol to its stop boundary, but
                        // do not commit new model output after user interruption.
                        continue;
                    }

                    if matches!(
                        method,
                        "item/commandExecution/requestApproval"
                            | "item/fileChange/requestApproval"
                            | "item/permissions/requestApproval"
                    ) {
                        let rpc_id = value
                            .get("id")
                            .filter(|id| id.is_string() || id.is_number())
                            .cloned()
                            .ok_or("Codex permission request missing RPC identity")?;
                        let params = value.get("params").cloned().unwrap_or(Value::Null);
                        let call_id = params
                            .get("itemId")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .ok_or("Codex permission request missing item identity")?;
                        let request_id = rpc_id
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| rpc_id.to_string());
                        let tool_name = if method == "item/commandExecution/requestApproval" {
                            "bash"
                        } else {
                            "write_file"
                        };
                        let reason = params
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("This operation needs your approval.");
                        let decision = emitter.external_permission_requested(
                            &request_id,
                            call_id,
                            tool_name,
                            &params,
                            reason,
                        );
                        if cancellation.as_ref().is_some_and(RuntimeExecutionControl::is_requested) {
                            return confirm_external_cancellation(&mut child, emitter, &descriptor.id);
                        }
                        let codex_decision =
                            match decision? {
                                Some(true) => "accept",
                                Some(false) => "decline",
                                None => return Err(
                                    "Codex permission transport unavailable; no decision received"
                                        .into(),
                                ),
                            };
                        write_external_json(
                            &mut stdin,
                            &serde_json::json!({
                                "id": rpc_id,
                                "result": {"decision": codex_decision}
                            }), &mut child, cancellation.as_ref(), &mut budget_monitor, emitter, &descriptor.id)?;
                        emitter.external_permission_written(&request_id, call_id)?;
                        continue;
                    }
                    if value.get("id").and_then(Value::as_u64) == Some(10) {
                        if let Some(error) = value.get("error") {
                            return Err(format!(
                                "Codex turn start failed: {}",
                                bounded_json(error, 1200)
                            ));
                        }
                    }
                    if let Some(snapshot) = external_accounting_snapshot(&descriptor.id, &value) {
                        emitter.external_accounting(snapshot)?;
                    }
                    for event in parse_codex_app_server_message(&value) {
                        match event {
                            ExternalExecutorEvent::TextDelta { text } => {
                                if let Some(separator) =
                                    agent_message_boundary.separator_before(&value, &text)
                                {
                                    emitter.external_text(separator)?;
                                }
                                emitter.external_text(&text)?
                            }
                            ExternalExecutorEvent::ReasoningDelta { text } => {
                                emitter.external_reasoning(&text)?
                            }
                            ExternalExecutorEvent::ToolStarted {
                                call_id,
                                name,
                                arguments,
                            } => {
                                tool_calls
                                    .insert(call_id.clone(), (name.clone(), arguments.clone()));
                                emitter.external_tool_started(&call_id, &name, &arguments)?;
                            }
                            ExternalExecutorEvent::ToolFinished {
                                call_id,
                                name,
                                output,
                                failed,
                            } => {
                                let (name, arguments) =
                                    tool_calls.remove(&call_id).unwrap_or((name, Value::Null));
                                emitter.external_tool_finished(
                                    &call_id, &name, &arguments, &output, failed,
                                )?;
                            }
                            ExternalExecutorEvent::Usage {
                                used_tokens,
                                context_window_tokens,
                            } => {
                                emitter.external_context_usage(
                                    used_tokens as usize,
                                    context_window_tokens as usize,
                                )?;
                            }
                            ExternalExecutorEvent::Completed { .. } => terminal = true,
                            ExternalExecutorEvent::Failed { message } => return Err(message),
                            ExternalExecutorEvent::Started { .. } => {}
                        }
                    }
                }
            }
            Ok(Err(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_protocol_progress.elapsed() >= idle_timeout {
                    let _ = terminate_external_child(&mut child);
                    return Err(format!(
                        "Codex app-server produced no protocol progress for {} seconds",
                        idle_timeout.as_secs()
                    ));
                }
            }
        }
        budget_monitor.check(&mut child)?;
        if terminal {
            break;
        }
        if cancellation
            .as_ref()
            .is_some_and(RuntimeExecutionControl::is_requested)
        {
            if active_turn.is_none() {
                return confirm_external_cancellation(&mut child, emitter, &descriptor.id);
            }
        }
        // Do not race child.try_wait() against the stdout reader. A fast
        // app-server exit can leave ToolFinished or turn/completed frames in
        // the channel. Protocol terminal controls the normal path; channel
        // disconnect controls the abnormal path after all buffered frames.
    }
    // A completed turn still has state that the app-server must flush before a
    // later process can resume the thread. Closing stdin gives it an orderly
    // EOF; killing it here can leave the thread locked or only partly saved.
    drop(stdin);
    let graceful_deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("wait for Codex app-server failed: {error}"))?
        {
            break status;
        }
        if Instant::now() >= graceful_deadline {
            break terminate_external_child(&mut child)
                .map_err(|error| format!("wait for Codex app-server failed: {error}"))?;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    // Helpers can retain stderr after the leader exits. Keep diagnostics inside
    // the same shutdown deadline; a completed turn is not an EOF/cleanup proof.
    while !stderr_handle.is_finished() {
        if Instant::now() >= graceful_deadline {
            return Err("Codex app-server shutdown is unconfirmed: stderr did not close before the deadline".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let stderr = stderr_handle.join().unwrap_or_default();
    if !terminal {
        return Err(if stderr.trim().is_empty() {
            format!("Codex app-server ended before turn completion ({status})")
        } else {
            bounded_json(&Value::String(stderr), 3000)
        });
    }
    if cancelled_terminal {
        observe_owned_group_exit(child.id(), Duration::from_secs(5)).map_err(|error| {
            eprintln!("Nola Codex interrupted turn shutdown unconfirmed pid={}: {error}", child.id());
            format!("Codex interrupted turn has unconfirmed process shutdown: {error}")
        })?;
        emitter.external_execution_cancelled(&descriptor.id)?;
        child.disarm();
        return Err("external execution cancelled".into());
    }
    emitter.external_execution_completed(&descriptor.id)?;
    child.disarm();
    Ok(())
}

pub(crate) fn external_permission_policy(config: &ExternalExecutionSettings) -> (&'static str, &'static str) {
    // This is the admitted invocation's own explicit policy, not the current
    // window's selection. Input origin is not a third permission mode.
    match config.permission_profile {
        crate::permissions::PermissionProfile::Cautious => ("on-request", "workspace-write"),
        crate::permissions::PermissionProfile::AllowAll => ("never", "danger-full-access"),
    }
}

fn codex_app_server_arguments() -> Vec<String> {
    ["app-server", "--listen", "stdio://"].into_iter().map(str::to_owned).collect()
}

fn duration_from_environment(
    key: &str,
    default_seconds: u64,
    minimum: u64,
    maximum: u64,
) -> Duration {
    let seconds = std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default_seconds)
        .clamp(minimum, maximum);
    Duration::from_secs(seconds)
}

/// A pipe can block even before the first protocol frame arrives. Keep only
/// this I/O job off-thread; Core/Host lifecycle decisions remain with the caller.
fn write_external_input(
    stdin: &mut Option<ChildStdin>, bytes: Vec<u8>, child: &mut Child,
    cancellation: Option<&RuntimeExecutionControl>, monitor: &mut ExternalExecutionBudgetMonitor,
    observer: &mut dyn ExternalExecutionObserver, backend: &str,
) -> Result<(), String> {
    if cancellation.is_some_and(RuntimeExecutionControl::is_requested) {
        if backend == "opencode" { return Err("external execution cancelled".into()); }
        return confirm_external_cancellation(child, observer, backend);
    }
    let mut writer = stdin.take().ok_or("external stdin is unavailable")?;
    let (returned, receive) = mpsc::sync_channel(1);
    let job = std::thread::spawn(move || {
        let result = writer.write_all(&bytes).and_then(|_| writer.flush());
        let _ = returned.send((writer, result));
    });
    loop {
        if cancellation.is_some_and(RuntimeExecutionControl::is_requested) {
            if backend == "opencode" { return Err("external execution cancelled".into()); }
            // If stopping fails, do not join a writer which can still be blocked.
            // The caller preserves the unconfirmed delivery for reconciliation.
            terminate_external_child(child).map_err(|error| format!("external process stop is unconfirmed: {error}"))?;
            job.join().map_err(|_| "external input writer panicked")?;
            observer.external_execution_cancelled(backend)?;
            return Err("external execution cancelled".into());
        }
        if let Err(error) = monitor.check(child) {
            if backend != "opencode" {
                terminate_external_child(child).map_err(|stop| format!("{error}; external process stop is unconfirmed: {stop}"))?;
                let _ = job.join();
            }
            return Err(error);
        }
        match receive.recv_timeout(Duration::from_millis(50)) {
            Ok((writer, result)) => {
                *stdin = Some(writer);
                job.join().map_err(|_| "external input writer panicked")?;
                return result.map_err(|error| format!("write external input failed: {error}"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {},
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err("external input writer ended without a result".into()),
        }
    }
}

fn write_external_json(
    stdin: &mut Option<ChildStdin>, value: &Value, child: &mut Child,
    cancellation: Option<&RuntimeExecutionControl>, monitor: &mut ExternalExecutionBudgetMonitor,
    observer: &mut dyn ExternalExecutionObserver, backend: &str,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| format!("encode external input failed: {error}"))?;
    bytes.push(b'\n');
    write_external_input(stdin, bytes, child, cancellation, monitor, observer, backend)
}

fn write_json_line(writer: &mut impl Write, value: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|error| format!("encode app-server request failed: {error}"))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("write app-server request failed: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("flush app-server request failed: {error}"))
}

fn bounded_json(value: &Value, limit: usize) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    };
    text.chars().take(limit).collect()
}

#[derive(Default)]
struct CodexAgentMessageBoundary {
    active_item_id: Option<String>,
    has_visible_text: bool,
    ends_with_line_break: bool,
}

impl CodexAgentMessageBoundary {
    /// Codex exposes progress updates and the final answer as distinct
    /// `agentMessage` items. Their delta strings do not include a separator,
    /// so forwarding the deltas verbatim collapses independent paragraphs.
    /// Preserve the provider's text exactly within an item and restore only
    /// the item boundary that the JSON-RPC envelope otherwise carries.
    fn separator_before(&mut self, notification: &Value, text: &str) -> Option<&'static str> {
        let item_id = notification
            .pointer("/params/itemId")
            .and_then(Value::as_str)
            .or_else(|| {
                notification
                    .pointer("/params/item/id")
                    .and_then(Value::as_str)
            });
        let starts_with_line_break = text.starts_with('\n') || text.starts_with('\r');
        let changed_item = match (self.active_item_id.as_deref(), item_id) {
            (Some(previous), Some(current)) => previous != current,
            _ => false,
        };
        let separator = changed_item
            .then_some(())
            .filter(|_| {
                self.has_visible_text && !self.ends_with_line_break && !starts_with_line_break
            })
            .map(|_| "\n\n");

        if let Some(item_id) = item_id {
            self.active_item_id = Some(item_id.to_string());
        }
        if !text.is_empty() {
            self.has_visible_text = true;
            self.ends_with_line_break = text.ends_with('\n') || text.ends_with('\r');
        }
        separator
    }
}

fn parse_codex_app_server_message(value: &Value) -> Vec<ExternalExecutorEvent> {
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = value.get("params").unwrap_or(&Value::Null);
    match method {
        "item/agentMessage/delta" => nonempty(string_at(params, &["delta"]))
            .map(|text| ExternalExecutorEvent::TextDelta { text })
            .into_iter()
            .collect(),
        "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => {
            nonempty(string_at(params, &["delta"]))
                .map(|text| ExternalExecutorEvent::ReasoningDelta { text })
                .into_iter()
                .collect()
        }
        "item/started" => {
            codex_app_server_tool_event(params.get("item").unwrap_or(&Value::Null), false)
        }
        "item/completed" => {
            codex_app_server_tool_event(params.get("item").unwrap_or(&Value::Null), true)
        }
        "thread/tokenUsage/updated" => {
            let usage = params.get("tokenUsage").unwrap_or(&Value::Null);
            let last = usage.get("last").unwrap_or(&Value::Null);
            let total = usage.get("total").unwrap_or(&Value::Null);
            let used_tokens = last
                .get("inputTokens")
                .and_then(Value::as_u64)
                .or_else(|| total.get("inputTokens").and_then(Value::as_u64));
            let context_window_tokens = usage.get("modelContextWindow").and_then(Value::as_u64);
            match (used_tokens, context_window_tokens) {
                (Some(used_tokens), Some(context_window_tokens)) if context_window_tokens > 0 => {
                    vec![ExternalExecutorEvent::Usage {
                        used_tokens,
                        context_window_tokens,
                    }]
                }
                _ => Vec::new(),
            }
        }
        "turn/completed" => {
            let turn = params.get("turn").unwrap_or(&Value::Null);
            match turn.get("status").and_then(Value::as_str) {
                Some("completed") => vec![ExternalExecutorEvent::Completed {
                    session_id: None,
                    input_tokens: None,
                    output_tokens: None,
                }],
                Some("failed") => vec![ExternalExecutorEvent::Failed {
                    message: turn
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex turn failed")
                        .to_string(),
                }],
                Some("interrupted") => vec![ExternalExecutorEvent::Failed {
                    message: "external execution cancelled: Codex turn interrupted".into(),
                }],
                _ => vec![ExternalExecutorEvent::Failed {
                    message: "Codex terminal notification has an invalid completion status".into(),
                }],
            }
        }
        "error" => vec![ExternalExecutorEvent::Failed {
            message: string_at(params, &["message"]),
        }],
        _ => Vec::new(),
    }
}

fn codex_app_server_tool_event(item: &Value, completed: bool) -> Vec<ExternalExecutorEvent> {
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
    let call_id = string_at(item, &["id"]);
    let (name, arguments, output, failed) = match item_type {
        "commandExecution" => (
            "shell".to_string(),
            serde_json::json!({"command":string_at(item, &["command"]),"cwd":string_at(item, &["cwd"])}),
            string_at(item, &["aggregatedOutput"]),
            item.get("exitCode")
                .and_then(Value::as_i64)
                .is_some_and(|code| code != 0),
        ),
        "fileChange" => (
            "file_change".to_string(),
            item.get("changes").cloned().unwrap_or(Value::Array(vec![])),
            item.get("changes")
                .cloned()
                .unwrap_or(Value::Array(vec![]))
                .to_string(),
            item.get("status").and_then(Value::as_str) == Some("failed"),
        ),
        "webSearch" => (
            "web_search".to_string(),
            serde_json::json!({
                "query": string_at(item, &["query"]),
                "action": item.get("action").cloned().unwrap_or(Value::Null),
            }),
            item.get("results")
                .cloned()
                .unwrap_or(Value::Null)
                .to_string(),
            false,
        ),
        "mcpToolCall" => (
            format!(
                "{}.{}",
                string_at(item, &["server"]),
                string_at(item, &["tool"])
            ),
            item.get("arguments").cloned().unwrap_or(Value::Null),
            item.get("result")
                .or_else(|| item.get("error"))
                .cloned()
                .unwrap_or(Value::Null)
                .to_string(),
            item.get("status").and_then(Value::as_str) == Some("failed"),
        ),
        "dynamicToolCall" => (
            string_at(item, &["tool"]),
            item.get("arguments").cloned().unwrap_or(Value::Null),
            item.get("contentItems")
                .cloned()
                .unwrap_or(Value::Null)
                .to_string(),
            item.get("success").and_then(Value::as_bool) == Some(false),
        ),
        _ => return Vec::new(),
    };
    if completed {
        vec![ExternalExecutorEvent::ToolFinished {
            call_id,
            name,
            output,
            failed,
        }]
    } else {
        vec![ExternalExecutorEvent::ToolStarted {
            call_id,
            name,
            arguments,
        }]
    }
}

pub fn command_for(
    descriptor: &ExecutionBackendDescriptor,
    workspace: &Path,
    session_id: Option<&str>,
    model: Option<&str>,
    model_provider: Option<&str>,
) -> Result<ExternalExecutorCommand, String> {
    let executable = descriptor
        .executable_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{} is not installed", descriptor.display_name))?;
    if descriptor.availability != ExecutionBackendAvailability::Available {
        return Err(format!("{} is not available", descriptor.display_name));
    }
    let cwd = workspace.to_string_lossy().into_owned();
    let arguments = match descriptor.kind {
        ExecutionBackendKind::Codex => {
            let mut args = Vec::new();
            if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
                args.extend(["--model".into(), model.into()]);
            }
            if let Some(provider) = model_provider.filter(|value| !value.trim().is_empty()) {
                args.extend(["-c".into(), format!("model_provider={provider}")]);
            }
            args.extend([
                "--ask-for-approval".into(),
                "never".into(),
                "--sandbox".into(),
                "workspace-write".into(),
                "-C".into(),
                cwd,
                "exec".into(),
            ]);
            if let Some(id) = session_id.filter(|value| !value.trim().is_empty()) {
                args.extend([
                    "resume".into(),
                    "--json".into(),
                    "--skip-git-repo-check".into(),
                    id.into(),
                    "-".into(),
                ]);
            } else {
                args.extend(["--json".into(), "--skip-git-repo-check".into(), "-".into()]);
            }
            args
        }
        ExecutionBackendKind::ClaudeCode => {
            let mut args = vec![
                "-p".into(),
                "--verbose".into(),
                "--input-format".into(),
                "stream-json".into(),
                "--output-format".into(),
                "stream-json".into(),
                "--include-partial-messages".into(),
                "--permission-mode".into(),
                "default".into(),
                "--permission-prompt-tool".into(),
                "stdio".into(),
            ];
            if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
                args.extend(["--model".into(), model.into()]);
            }
            if let Some(id) = session_id.filter(|value| !value.trim().is_empty()) {
                args.extend(["--resume".into(), id.into()]);
            }
            args
        }
        ExecutionBackendKind::OpenCode => {
            vec!["acp".into(), "--cwd".into(), cwd, "--print-logs".into(), "--log-level".into(), "ERROR".into()]
        }
        ExecutionBackendKind::Nola => return Err("Nola executes in-process".into()),
    };
    let prompt_via_stdin = true;
    Ok(ExternalExecutorCommand {
        executable,
        arguments,
        prompt_via_stdin,
    })
}

#[derive(Clone, Copy)]
enum ExternalImageBlockStyle {
    Claude,
    Acp,
}

fn external_image_content_blocks(
    attachments: &[crate::external_execution_settings::HostAttachment],
    text: String,
    style: ExternalImageBlockStyle,
) -> Result<Vec<Value>, String> {
    let mut blocks = vec![serde_json::json!({"type":"text", "text":text})];
    for attachment in attachments
        .iter()
        .filter(|attachment| attachment.mime_type.starts_with("image/"))
    {
        let data = std::fs::read(&attachment.path).map_err(|error| {
            format!("read image attachment {} failed: {error}", attachment.name)
        })?;
        if data.is_empty() {
            return Err(format!("image attachment {} is empty", attachment.name));
        }
        let encoded = BASE64_STANDARD.encode(data);
        blocks.push(match style {
            ExternalImageBlockStyle::Claude => serde_json::json!({
                "type":"image",
                "source":{
                    "type":"base64",
                    "media_type":attachment.mime_type,
                    "data":encoded
                }
            }),
            ExternalImageBlockStyle::Acp => serde_json::json!({
                "type":"image",
                "mimeType":attachment.mime_type,
                "data":encoded
            }),
        });
    }
    Ok(blocks)
}


#[derive(Default)]
struct ClaudeControl {
    initialized: bool,
    requests: std::collections::HashSet<String>,
    calls: HashMap<String, (String, Value)>,
    tasks: std::collections::HashSet<String>,
}

impl ClaudeControl {
    fn initialize(&self) -> Value {
        serde_json::json!({"type":"control_request", "request_id":"nola-initialize",
            "request":{"subtype":"initialize", "hooks":null}})
    }

    fn accept_initialize(&mut self, frame: &Value) -> Result<(), String> {
        if self.initialized || frame.pointer("/response/request_id").and_then(Value::as_str) != Some("nola-initialize")
            || frame.pointer("/response/subtype").and_then(Value::as_str) != Some("success") {
            return Err("Claude Code initialization response does not match the owned request".into());
        }
        self.initialized = true;
        Ok(())
    }

    fn permission(&mut self, frame: &Value, session: Option<&str>,
        tools: &HashMap<String, (String, Value)>) -> Result<ClaudePermission, String> {
        if !self.initialized || session.is_none() {
            return Err("Claude Code permission request has no admitted session".into());
        }
        if let Some(reported) = frame.get("session_id").and_then(Value::as_str) {
            validate_external_session(session, Some(reported))?;
        }
        let request = frame.get("request").ok_or("Claude Code permission request is missing")?;
        if request["subtype"] != "can_use_tool" {
            return Err("Unsupported Claude Code control request".into());
        }
        let exact = |v: Option<&Value>| v.and_then(Value::as_str)
            .filter(|v| !v.is_empty() && v.trim() == *v && !v.chars().any(char::is_control))
            .map(str::to_owned).ok_or_else(|| "Claude Code permission identity is missing or invalid".to_string());
        let request_id = exact(frame.get("request_id"))?;
        let call_id = exact(request.get("tool_use_id"))?;
        let tool_name = exact(request.get("tool_name"))?;
        let input = request.get("input").filter(|v| v.is_object()).cloned()
            .ok_or("Claude Code permission input must be an object")?;
        let binding = (tool_name.clone(), input.clone());
        if tools.get(&call_id).is_some_and(|known| known != &binding)
            || self.calls.get(&call_id).is_some_and(|known| known != &binding) {
            return Err("Claude Code permission does not match the owned tool call".into());
        }
        if !self.requests.insert(request_id.clone()) {
            return Err("Claude Code repeated a permission request identity".into());
        }
        // The native control request can precede the assistant snapshot. Bind
        // it here and check any later snapshot, rather than assuming frame order.
        self.calls.insert(call_id.clone(), binding);
        Ok(ClaudePermission { request_id, call_id, tool_name, input })
    }

    fn validate_tool(&self, call_id: &str, name: &str, input: &Value) -> Result<(), String> {
        if self.calls.get(call_id).is_some_and(|known| known != &(name.to_owned(), input.clone())) {
            return Err("Claude Code tool changed after its permission request".into());
        }
        Ok(())
    }

    fn observe_tasks(&mut self, frame: &Value) {
        if frame["type"] != "system" { return; }
        if let Some(id) = frame.get("task_id").and_then(Value::as_str) {
            match frame.get("subtype").and_then(Value::as_str) {
                Some("task_started") => { self.tasks.insert(id.into()); }
                Some("task_notification") => { self.tasks.remove(id); }
                _ => {}
            }
        }
    }
}

struct ClaudePermission { request_id: String, call_id: String, tool_name: String, input: Value }
impl ClaudePermission {
    fn response(&self, decision: Option<bool>) -> Result<Value, String> {
        let decision = decision.ok_or("Claude Code permission transport unavailable; no decision received")?;
        let body = if decision { serde_json::json!({"behavior":"allow", "updatedInput":self.input}) }
            else { serde_json::json!({"behavior":"deny", "message":"Nola denied this operation"}) };
        // Deliberately no updatedPermissions: this authorizes one exact input,
        // never a persistent native permission rule.
        Ok(serde_json::json!({"type":"control_response", "response":{
            "subtype":"success", "request_id":self.request_id, "response":body}}))
    }
}

fn claude_stream_input(config: &ExternalExecutionSettings, prompt: &str) -> Result<String, String> {
    let content = external_image_content_blocks(
        &config.host_attachments,
        prompt.to_string(),
        ExternalImageBlockStyle::Claude,
    )?;
    let message = serde_json::json!({
        "type":"user",
        "message":{"role":"user", "content":content}
    });
    Ok(format!("{message}\n"))
}

pub fn parse_event_line(kind: ExecutionBackendKind, line: &str) -> Vec<ExternalExecutorEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match kind {
        ExecutionBackendKind::Codex => parse_codex_event(&value),
        ExecutionBackendKind::ClaudeCode => parse_claude_event(&value),
        ExecutionBackendKind::OpenCode => parse_opencode_event(&value),
        ExecutionBackendKind::Nola => Vec::new(),
    }
}

fn parse_codex_event(value: &Value) -> Vec<ExternalExecutorEvent> {
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "thread.started" => vec![ExternalExecutorEvent::Started {
            session_id: string_at(value, &["thread_id"]),
        }],
        "item.completed" => {
            let item = value.get("item").unwrap_or(&Value::Null);
            match item.get("type").and_then(Value::as_str).unwrap_or_default() {
                "agent_message" => nonempty(string_at(item, &["text"]))
                    .map(|text| ExternalExecutorEvent::TextDelta { text })
                    .into_iter()
                    .collect(),
                "command_execution" => vec![ExternalExecutorEvent::ToolFinished {
                    call_id: string_at(item, &["id"]),
                    name: "shell".into(),
                    output: string_at(item, &["aggregated_output"]),
                    failed: item.get("exit_code").and_then(Value::as_i64).unwrap_or(0) != 0,
                }],
                _ => Vec::new(),
            }
        }
        "item.started" => {
            let item = value.get("item").unwrap_or(&Value::Null);
            (item.get("type").and_then(Value::as_str) == Some("command_execution"))
                .then(|| ExternalExecutorEvent::ToolStarted {
                    call_id: string_at(item, &["id"]),
                    name: "shell".into(),
                    arguments: serde_json::json!({"command": string_at(item, &["command"])}),
                })
                .into_iter()
                .collect()
        }
        "turn.completed" => vec![ExternalExecutorEvent::Completed {
            session_id: None,
            input_tokens: value.pointer("/usage/input_tokens").and_then(Value::as_u64),
            output_tokens: value
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64),
        }],
        "error" | "turn.failed" => vec![ExternalExecutorEvent::Failed {
            message: string_at(value, &["message"]),
        }],
        _ => Vec::new(),
    }
}

fn parse_claude_event(value: &Value) -> Vec<ExternalExecutorEvent> {
    let session_id = string_at(value, &["session_id"]);
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "system" if value.get("subtype").and_then(Value::as_str) == Some("init") => {
            vec![ExternalExecutorEvent::Started { session_id }]
        }
        "stream_event" => {
            let event = value.get("event").unwrap_or(&Value::Null);
            if event.get("type").and_then(Value::as_str) == Some("content_block_delta") {
                let delta = event.get("delta").unwrap_or(&Value::Null);
                return match delta
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                {
                    "text_delta" => nonempty(string_at(delta, &["text"]))
                        .map(|text| ExternalExecutorEvent::TextDelta { text })
                        .into_iter()
                        .collect(),
                    "thinking_delta" => nonempty(string_at(delta, &["thinking"]))
                        .map(|text| ExternalExecutorEvent::ReasoningDelta { text })
                        .into_iter()
                        .collect(),
                    _ => Vec::new(),
                };
            }
            Vec::new()
        }
        // Text and thinking blocks in assistant snapshots repeat already
        // streamed deltas. Tool-use blocks are different: Claude Code exposes
        // their complete identity and arguments only in this snapshot.
        "assistant" => value
            .pointer("/message/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
            .map(|block| ExternalExecutorEvent::ToolStarted {
                call_id: string_at(block, &["id"]),
                name: string_at(block, &["name"]),
                arguments: block
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
            })
            .collect(),
        "user" => value
            .pointer("/message/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
            .map(|block| ExternalExecutorEvent::ToolFinished {
                call_id: string_at(block, &["tool_use_id"]),
                name: "Claude Code tool".to_string(),
                output: claude_tool_result_text(block.get("content").unwrap_or(&Value::Null)),
                failed: block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
            .collect(),
        "result" => {
            if value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let errors = value.get("errors").and_then(Value::as_array)
                    .into_iter().flatten().filter_map(Value::as_str)
                    .filter(|message| !message.trim().is_empty())
                    .collect::<Vec<_>>().join("\n");
                vec![ExternalExecutorEvent::Failed {
                    message: nonempty(errors)
                        .or_else(|| nonempty(string_at(value, &["result"])))
                        .or_else(|| nonempty(string_at(value, &["subtype"])))
                        .unwrap_or_else(|| "Claude Code execution failed".into()),
                }]
            } else {
                let input_tokens = claude_input_tokens(value);
                let mut events = claude_context_usage(value)
                    .map(
                        |(used_tokens, context_window_tokens)| ExternalExecutorEvent::Usage {
                            used_tokens,
                            context_window_tokens,
                        },
                    )
                    .into_iter()
                    .collect::<Vec<_>>();
                events.push(ExternalExecutorEvent::Completed {
                    session_id: nonempty(session_id),
                    input_tokens,
                    output_tokens: value
                        .pointer("/usage/output_tokens")
                        .and_then(Value::as_u64),
                });
                events
            }
        }
        _ => Vec::new(),
    }
}

/// Billing counters are separate from the context occupancy gauge. Native
/// cumulative frames replace their prior snapshot; they are never added twice.
fn external_accounting_snapshot(backend: &str, value: &Value) -> Option<nola_domain::delegated_invocation::ExternalUsageSnapshot> {
    use nola_domain::delegated_invocation::{ExternalUsageSnapshot, ExternalUsageBasis};
    let (scope_id, basis, usage, input, output, read, write, model) = match backend {
        "codex" if value["method"] == "thread/tokenUsage/updated" => {
            let params = value.get("params")?;
            let usage = params.pointer("/tokenUsage/total")?;
            (params["threadId"].as_str()?.to_string(), ExternalUsageBasis::SessionCumulative, usage,
             "inputTokens", "outputTokens", "cachedInputTokens", "cacheWriteInputTokens", None)
        }
        "claude_code" if value["type"] == "result" => {
            let usage = value.get("usage")?;
            (value["session_id"].as_str()?.to_string(), ExternalUsageBasis::InvocationCumulative, usage,
             "input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens",
             value.get("modelUsage").and_then(Value::as_object).and_then(|models| (models.len()==1).then(||models.keys().next().unwrap().clone())))
        }
        "claude_code" if value["type"] == "assistant" => {
            let usage = value.pointer("/message/usage")?;
            (value["session_id"].as_str()?.to_string(), ExternalUsageBasis::LastModelResponse, usage,
                "input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens",
                value.pointer("/message/model").and_then(Value::as_str).map(str::to_owned))
        }
        "opencode" => {
            let usage=value.pointer("/result/usage")?;
            ("last-model-response".into(),ExternalUsageBasis::LastModelResponse,usage,
                "inputTokens","outputTokens","cachedReadTokens","cachedWriteTokens",None)
        }
        _ => return None,
    };
    let read = usage.get(read).and_then(Value::as_u64);
    let write = usage.get(write).and_then(Value::as_u64);
    let input = usage.get(input).and_then(Value::as_u64).map(|input| {
        if backend == "claude_code" || backend == "opencode" { input.saturating_add(read.unwrap_or(0)).saturating_add(write.unwrap_or(0)) } else { input }
    });
    Some(ExternalUsageSnapshot { backend_id: backend.into(), scope_id, basis,
        // Do not infer the actual provider from the executable or selected profile.
        provider_id: None, model_id: model, input_tokens: input,
        output_tokens: usage.get(output).and_then(Value::as_u64), cache_read_tokens: read, cache_write_tokens: write })
}

fn claude_input_tokens(value: &Value) -> Option<u64> {
    let usage = value.get("usage")?;
    let input = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let cache_write = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Some(input.saturating_add(cache_read).saturating_add(cache_write))
}

fn claude_context_usage(value: &Value) -> Option<(u64, u64)> {
    let model_usage = value.get("modelUsage")?.as_object()?;
    let usage = model_usage.values().next()?;
    let input = usage
        .get("inputTokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let cache_read = usage
        .get("cacheReadInputTokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let cache_write = usage
        .get("cacheCreationInputTokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let context_window = usage.get("contextWindow").and_then(Value::as_u64)?;
    (context_window > 0).then_some((
        input.saturating_add(cache_read).saturating_add(cache_write),
        context_window,
    ))
}

fn claude_tool_result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn parse_opencode_event(value: &Value) -> Vec<ExternalExecutorEvent> {
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let part = value
        .get("part")
        .or_else(|| value.pointer("/properties/part"))
        .unwrap_or(&Value::Null);
    let session_id = string_at(value, &["sessionID", "session_id"]);
    match event_type {
        "text" | "text.delta" => nonempty(string_at(part, &["text"]))
            .map(|text| ExternalExecutorEvent::TextDelta { text })
            .into_iter()
            .collect(),
        "reasoning" | "reasoning.delta" => nonempty(string_at(part, &["text"]))
            .map(|text| ExternalExecutorEvent::ReasoningDelta { text })
            .into_iter()
            .collect(),
        "step_start" | "session.created" | "session.started" => nonempty(session_id)
            .map(|session_id| ExternalExecutorEvent::Started { session_id })
            .into_iter()
            .collect(),
        "tool_use" | "tool" => parse_opencode_tool_event(part),
        "step_finish" | "session.idle" | "session.completed" => {
            vec![ExternalExecutorEvent::Completed {
                session_id: nonempty(session_id),
                input_tokens: value
                    .pointer("/part/tokens/input")
                    .and_then(Value::as_u64)
                    .or_else(|| value.pointer("/usage/input_tokens").and_then(Value::as_u64)),
                output_tokens: value
                    .pointer("/part/tokens/output")
                    .and_then(Value::as_u64)
                    .or_else(|| {
                        value
                            .pointer("/usage/output_tokens")
                            .and_then(Value::as_u64)
                    }),
            }]
        }
        "error" => vec![ExternalExecutorEvent::Failed {
            message: opencode_error_message(value),
        }],
        _ => Vec::new(),
    }
}

fn parse_opencode_tool_event(part: &Value) -> Vec<ExternalExecutorEvent> {
    let call_id = string_at(part, &["callID", "call_id", "id"]);
    let name = string_at(part, &["tool", "name"]);
    let state = part.get("state").unwrap_or(&Value::Null);
    let status = string_at(state, &["status"]);
    let arguments = state
        .get("input")
        .or_else(|| part.get("input"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let output = state
        .get("output")
        .or_else(|| state.get("error"))
        .or_else(|| part.get("output"))
        .map(json_text)
        .unwrap_or_default();
    match status.as_str() {
        "completed" => vec![
            ExternalExecutorEvent::ToolStarted {
                call_id: call_id.clone(),
                name: name.clone(),
                arguments,
            },
            ExternalExecutorEvent::ToolFinished {
                call_id,
                name,
                output,
                failed: false,
            },
        ],
        "error" | "failed" => vec![
            ExternalExecutorEvent::ToolStarted {
                call_id: call_id.clone(),
                name: name.clone(),
                arguments,
            },
            ExternalExecutorEvent::ToolFinished {
                call_id,
                name,
                output,
                failed: true,
            },
        ],
        _ => vec![ExternalExecutorEvent::ToolStarted {
            call_id,
            name,
            arguments,
        }],
    }
}

fn opencode_error_message(value: &Value) -> String {
    nonempty(string_at(value, &["message"]))
        .or_else(|| {
            value
                .pointer("/error/data/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            value
                .get("error")
                .map(json_text)
                .filter(|text| !text.is_empty())
        })
        .unwrap_or_else(|| "OpenCode failed without an error message".to_string())
}

fn json_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default())
}

fn string_at(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

pub fn discover_execution_backends() -> Vec<ExecutionBackendDescriptor> {
    let mut result = vec![ExecutionBackendDescriptor::built_in_nola()];
    result.extend([
        discover(ExecutionBackendKind::Codex, true),
        discover(ExecutionBackendKind::ClaudeCode, true),
        discover(ExecutionBackendKind::OpenCode, true),
    ]);
    result
}

/// Resolves one structured backend selection without probing unrelated
/// executors. Turn startup calls this path after the native Host has already
/// selected a backend ID; full catalog and model discovery belongs only to
/// the explicit model/backend picker.
pub fn discover_execution_backend(
    id: &str,
    include_models: bool,
) -> Option<ExecutionBackendDescriptor> {
    match id {
        "nola" => Some(ExecutionBackendDescriptor::built_in_nola()),
        "codex" => Some(discover(ExecutionBackendKind::Codex, include_models)),
        "claude_code" => Some(discover(ExecutionBackendKind::ClaudeCode, include_models)),
        "opencode" => Some(discover(ExecutionBackendKind::OpenCode, include_models)),
        _ => None,
    }
}

/// Pure display metadata for hot UI paths. Availability and model discovery
/// are intentionally excluded because they may execute external processes.
pub fn execution_backend_display_name(id: &str) -> &str {
    match id {
        "nola" => "Nola",
        "codex" => "Codex",
        "claude_code" => "Claude Code",
        "opencode" => "OpenCode",
        _ => id,
    }
}

/// Read-only machine contract used by native clients to render the exact
/// execution-unit catalog that this Agent Host will enforce at run time.
/// It intentionally runs before ordinary CLI configuration loading, so
/// discovery never requires an LLM endpoint or credentials.
pub fn run_catalog_command_if_requested() -> Option<i32> {
    if !std::env::args().any(|argument| argument == "--list-execution-backends") {
        return None;
    }
    match serde_json::to_string(&discover_execution_backends()) {
        Ok(value) => {
            println!("{value}");
            Some(0)
        }
        Err(error) => {
            eprintln!("failed to encode execution backend catalog: {error}");
            Some(1)
        }
    }
}

fn discover(kind: ExecutionBackendKind, include_models: bool) -> ExecutionBackendDescriptor {
    let executable_name = kind.executable_name().expect("external backend executable");
    let path = find_executable(executable_name);
    let version = path.as_ref().and_then(|path| read_version(path));
    let authentication = path
        .as_ref()
        .and_then(|path| probe_authentication(kind, path));
    let availability = if path.is_none() {
        ExecutionBackendAvailability::NotInstalled
    } else if version.is_none() {
        ExecutionBackendAvailability::Incompatible
    } else if matches!(
        authentication,
        Some(AuthenticationProbe::Unauthenticated(_))
    ) {
        ExecutionBackendAvailability::Unauthenticated
    } else if matches!(authentication, Some(AuthenticationProbe::Unreachable(_))) {
        ExecutionBackendAvailability::Unreachable
    } else {
        ExecutionBackendAvailability::Available
    };
    let (display_name, transport, supports_permission_bridge) = match kind {
        ExecutionBackendKind::Codex => ("Codex", ExecutionBackendTransport::JsonRpcStdio, true),
        ExecutionBackendKind::ClaudeCode => (
            "Claude Code",
            ExecutionBackendTransport::StreamJsonProcess,
            true,
        ),
        ExecutionBackendKind::OpenCode => {
            ("OpenCode", ExecutionBackendTransport::JsonRpcStdio, true)
        }
        ExecutionBackendKind::Nola => unreachable!(),
    };
    let (models, default_model_id) = if include_models {
        discover_models(kind, path.as_deref())
    } else {
        (Vec::new(), None)
    };
    ExecutionBackendDescriptor {
        schema_version: EXECUTION_BACKEND_SCHEMA_VERSION,
        id: kind.id().to_string(),
        kind,
        display_name: display_name.to_string(),
        availability,
        transport,
        executable_path: path
            .as_ref()
            .map(|value| value.to_string_lossy().into_owned()),
        version,
        capabilities: ExecutionBackendCapabilities {
            streams_text: true,
            streams_reasoning: true,
            streams_tools: true,
            supports_resume: true,
            supports_cancel: true,
            // Codex app-server and OpenCode ACP carry request identities and
            // checked approval writes. Claude's one-shot stream-json does not.
            supports_permission_bridge,
        },
        diagnostic: authentication.and_then(AuthenticationProbe::diagnostic),
        models,
        default_model_id,
    }
}

fn discover_models(
    kind: ExecutionBackendKind,
    executable: Option<&Path>,
) -> (Vec<ExecutionBackendModelDescriptor>, Option<String>) {
    let mut models = Vec::<ExecutionBackendModelDescriptor>::new();
    let mut default_model = None;
    match kind {
        ExecutionBackendKind::Codex => {
            if let Some(path) = executable {
                if let Some((catalog_models, catalog_default)) = discover_codex_models(path) {
                    models = catalog_models;
                    default_model = catalog_default;
                }
            }
            if let Some(home) = external_execution_home() {
                let contents = fs::read_to_string(PathBuf::from(home).join(".codex/config.toml"))
                    .unwrap_or_default();
                let mut section_model: Option<String> = None;
                let mut section_provider: Option<String> = None;
                let mut selections = Vec::<(String, Option<String>)>::new();
                for line in contents.lines().map(str::trim) {
                    if line.starts_with('[') {
                        if let Some(model) = section_model.take() {
                            selections.push((model, section_provider.take()));
                        }
                        section_provider = None;
                    }
                    if let Some(value) = quoted_setting(line, "model_provider") {
                        section_provider = Some(value);
                    }
                    if let Some(model) = quoted_setting(line, "model") {
                        section_model = Some(model);
                    }
                }
                if let Some(model) = section_model {
                    selections.push((model, section_provider));
                }
                for (model, provider) in selections {
                    if default_model.is_none() {
                        default_model = Some(model.clone());
                    }
                    push_model(&mut models, model, provider);
                }
            }
        }
        ExecutionBackendKind::ClaudeCode => {
            for alias in ["default", "sonnet", "opus", "haiku", "opusplan"] {
                push_model(&mut models, alias.to_string(), None);
            }
            if let Some(home) = env::var_os("HOME") {
                let settings =
                    fs::read_to_string(PathBuf::from(home).join(".claude/settings.json"))
                        .ok()
                        .and_then(|value| serde_json::from_str::<Value>(&value).ok());
                if let Some(model) = settings
                    .as_ref()
                    .and_then(|value| value.get("model"))
                    .and_then(Value::as_str)
                {
                    default_model = Some(model.to_string());
                    push_model(&mut models, model.to_string(), None);
                }
            }
        }
        ExecutionBackendKind::OpenCode => {
            if let Some(path) = executable {
                if let Ok((status, stdout, _)) =
                    command_output_with_timeout(path, &["models"], Duration::from_secs(4))
                {
                    if status.success() {
                        for model in stdout.lines().map(str::trim).filter(|line| {
                            line.contains('/') && !line.chars().any(char::is_whitespace)
                        }) {
                            let provider = model.split('/').next().map(str::to_string);
                            push_model(&mut models, model.to_string(), provider);
                        }
                    }
                }
            }
        }
        ExecutionBackendKind::Nola => {}
    }
    (models, default_model)
}

fn discover_codex_models(
    executable: &Path,
) -> Option<(Vec<ExecutionBackendModelDescriptor>, Option<String>)> {
    let mut command = external_command(executable);
    let mut child = command
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    if write_json_line(&mut stdin, &serde_json::json!({
        "id": 1,
        "method": "initialize",
        "params": {"clientInfo": {"name": "nola-model-catalog", "title": "Nola", "version": env!("CARGO_PKG_VERSION")}}
    })).is_err() {
        let _ = terminate_external_child(&mut child);
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut requested_models = false;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("id").and_then(Value::as_u64) == Some(1) && !requested_models {
                    if value.get("error").is_some() {
                        break;
                    }
                    if write_json_line(
                        &mut stdin,
                        &serde_json::json!({"method":"initialized","params":{}}),
                    )
                    .is_err()
                        || write_json_line(
                            &mut stdin,
                            &serde_json::json!({
                                "id": 2,
                                "method": "model/list",
                                "params": {"includeHidden": false, "limit": 100}
                            }),
                        )
                        .is_err()
                    {
                        break;
                    }
                    requested_models = true;
                } else if value.get("id").and_then(Value::as_u64) == Some(2) {
                    let result = parse_codex_model_list_response(&value);
                    let _ = terminate_external_child(&mut child);
                    return result;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
    }
    let _ = terminate_external_child(&mut child);
    None
}

fn parse_codex_model_list_response(
    value: &Value,
) -> Option<(Vec<ExecutionBackendModelDescriptor>, Option<String>)> {
    if value.get("error").is_some() {
        return None;
    }
    let data = value.pointer("/result/data")?.as_array()?;
    let mut models = Vec::new();
    let mut default_model = None;
    for item in data {
        if item.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let id = item
            .get("model")
            .and_then(Value::as_str)
            .or_else(|| item.get("id").and_then(Value::as_str))?
            .trim();
        if id.is_empty() {
            continue;
        }
        let display_name = item
            .get("displayName")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(id)
            .to_string();
        if item
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            default_model = Some(id.to_string());
        }
        if !models
            .iter()
            .any(|model: &ExecutionBackendModelDescriptor| model.id == id)
        {
            models.push(ExecutionBackendModelDescriptor {
                id: id.to_string(),
                display_name,
                provider_id: None,
            });
        }
    }
    (!models.is_empty()).then_some((models, default_model))
}

fn external_execution_home() -> Option<PathBuf> {
    env::var_os("NOLA_EXTERNAL_EXECUTION_HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
}

fn quoted_setting(line: &str, key: &str) -> Option<String> {
    let (candidate, value) = line.split_once('=')?;
    if candidate.trim() != key {
        return None;
    }
    let value = value.trim().trim_matches(['\'', '"']);
    (!value.is_empty()).then(|| value.to_string())
}

fn push_model(
    models: &mut Vec<ExecutionBackendModelDescriptor>,
    id: String,
    provider_id: Option<String>,
) {
    if models
        .iter()
        .any(|model| model.id == id && model.provider_id == provider_id)
    {
        return;
    }
    models.push(ExecutionBackendModelDescriptor {
        display_name: id.clone(),
        id,
        provider_id,
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AuthenticationProbe {
    Authenticated,
    Unauthenticated(String),
    Unreachable(String),
}

impl AuthenticationProbe {
    fn diagnostic(self) -> Option<String> {
        match self {
            Self::Authenticated => None,
            Self::Unauthenticated(value) | Self::Unreachable(value) => Some(value),
        }
    }
}

fn probe_authentication(kind: ExecutionBackendKind, path: &Path) -> Option<AuthenticationProbe> {
    type AuthenticationCache =
        HashMap<(ExecutionBackendKind, PathBuf), (Instant, AuthenticationProbe)>;
    static CACHE: OnceLock<Mutex<AuthenticationCache>> = OnceLock::new();
    let key = (kind, path.to_path_buf());
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(values) = cache.lock() {
        if let Some((checked_at, value)) = values.get(&key) {
            if checked_at.elapsed() < Duration::from_secs(15) {
                return Some(value.clone());
            }
        }
    }
    let arguments: &[&str] = match kind {
        ExecutionBackendKind::Codex => &["login", "status"],
        ExecutionBackendKind::ClaudeCode => &["auth", "status"],
        // OpenCode authentication is provider-specific. A successful version
        // probe is the only portable local readiness signal it exposes.
        ExecutionBackendKind::OpenCode | ExecutionBackendKind::Nola => return None,
    };
    let output = command_output_with_timeout(path, arguments, Duration::from_secs(2));
    let result = match output {
        Ok((status, stdout, stderr)) if status.success() => {
            let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
            if combined.contains("not logged")
                || combined.contains("loggedin\": false")
                || combined.contains("not authenticated")
            {
                Some(AuthenticationProbe::Unauthenticated(
                    "Installed, but authentication is required.".to_string(),
                ))
            } else {
                Some(AuthenticationProbe::Authenticated)
            }
        }
        Ok((_status, stdout, stderr)) => {
            let detail = bounded_nonempty(&format!("{stdout}\n{stderr}"), 240)
                .unwrap_or_else(|| "Installed, but authentication is required.".to_string());
            Some(AuthenticationProbe::Unauthenticated(detail))
        }
        Err(error) => Some(AuthenticationProbe::Unreachable(error)),
    };
    if let (Some(value), Ok(mut values)) = (result.as_ref(), cache.lock()) {
        values.insert(key, (Instant::now(), value.clone()));
    }
    result
}

fn command_output_with_timeout(
    path: &Path,
    arguments: &[&str],
    timeout: Duration,
) -> Result<(ExitStatus, String, String), String> {
    let mut command = external_command(path);
    let mut child = command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("authentication check could not start: {error}"))?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|error| format!("authentication check failed: {error}"))?;
                return Ok((
                    status,
                    String::from_utf8_lossy(&output.stdout).trim().to_string(),
                    String::from_utf8_lossy(&output.stderr).trim().to_string(),
                ));
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = terminate_external_child(&mut child);
                return Err("Authentication check timed out.".to_string());
            }
            Err(error) => {
                let _ = terminate_external_child(&mut child);
                return Err(format!("authentication check failed: {error}"));
            }
        }
    }
}

fn external_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    // Execution units may create helpers. Isolating the root process lets
    // cancellation and protocol failures terminate the whole tree instead of
    // leaving descendants alive with inherited pipes.
    #[cfg(unix)]
    command.process_group(0);
    if let Some(home) =
        env::var_os("NOLA_EXTERNAL_EXECUTION_HOME").filter(|value| !value.is_empty())
    {
        command.env("HOME", home);
    }
    command
}

struct ManagedExternalChild {
    child: Option<Child>,
}

impl ManagedExternalChild {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn disarm(&mut self) {
        self.child.take();
    }
}

impl Deref for ManagedExternalChild {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        self.child.as_ref().expect("managed external child")
    }
}

impl DerefMut for ManagedExternalChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.child.as_mut().expect("managed external child")
    }
}

impl Drop for ManagedExternalChild {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        // The leader may already have exited while descendants still hold the
        // stream open. Every armed exit (including observer refusal) owns cleanup.
        let _ = terminate_external_child(child);
    }
}

fn confirm_external_cancellation<T>(
    child: &mut Child, observer: &mut dyn ExternalExecutionObserver, backend: &str,
) -> Result<T, String> {
    let pid = child.id();
    terminate_external_child(child).map_err(|error| {
        // OS-only facts; never log prompts, protocol bodies or credentials.
        eprintln!("Nola external stop unconfirmed backend={backend} pid={pid}: {error}");
        format!("external process stop is unconfirmed: {error}")
    })?;
    observer.external_execution_cancelled(backend)?;
    Err("external execution cancelled".into())
}

#[cfg(unix)]
fn process_group_exit_observed(observation: std::io::Result<()>) -> std::io::Result<bool> {
    match observation {
        Ok(()) => Ok(false),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn observe_owned_group_exit(pid: u32, timeout: Duration) -> std::io::Result<()> {
    let group = i32::try_from(pid).map_err(std::io::Error::other)?;
    let deadline = Instant::now() + timeout;
    loop {
        let observation = if unsafe { libc::kill(-group, 0) } == 0 { Ok(()) }
            else { Err(std::io::Error::last_os_error()) };
        if process_group_exit_observed(observation)? { return Ok(()); }
        if Instant::now() >= deadline {
            return Err(std::io::Error::other(format!("owned process group {group} exit remains unconfirmed")));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
#[cfg(not(unix))]
fn observe_owned_group_exit(_pid: u32, _timeout: Duration) -> std::io::Result<()> {
    Err(std::io::Error::other("owned process group observation unavailable"))
}

fn terminate_external_child(child: &mut Child) -> std::io::Result<ExitStatus> {
    #[cfg(unix)]
    {
        if let Ok(process_group) = i32::try_from(child.id()) {
            unsafe {
                libc::kill(-process_group, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(50));
            if unsafe { libc::kill(-process_group, libc::SIGKILL) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    // Reap an already-exited leader without claiming its whole group stopped.
                    let _ = child.try_wait();
                    return Err(std::io::Error::new(error.kind(), format!("SIGKILL owned process group {process_group}: {error}")));
                }
            }
        }
    }
    #[cfg(windows)]
    {
        crate::task_runtime::terminate_external_process_tree(child.id());
    }
    let _ = child.kill();
    let pid = child.id();
    let status = child.wait().map_err(|error| std::io::Error::new(error.kind(), format!("wait owned process {pid}: {error}")))?;
    #[cfg(unix)]
    {
        // Signal delivery and reaping the leader do not establish that the
        // entire owned group has exited. Unknown/EPERM remains unconfirmed.
        observe_owned_group_exit(pid, Duration::from_secs(1))?;
    }
    // The Windows tree helper reports no outcome; a reaped leader alone is
    // insufficient evidence that its descendants have stopped.
    #[cfg(windows)]
    return Err(std::io::Error::other(format!(
        "Windows process leader exited with {status}; process tree stop is unconfirmed"
    )));
    #[cfg(not(windows))]
    Ok(status)
}

fn bounded_nonempty(value: &str, limit: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.chars().take(limit).collect())
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 && is_executable(candidate) {
        return fs::canonicalize(candidate).ok();
    }
    executable_search_candidates(name)
        .into_iter()
        .find(|candidate| is_executable(candidate))
        .and_then(|candidate| fs::canonicalize(candidate).ok())
}

/// Desktop applications do not inherit the user's interactive shell PATH.
/// Search PATH first, then the documented user-level install locations used
/// by the supported execution units and the standard package-manager bins.
/// The returned order is stable so discovery and execution resolve the same
/// binary on macOS, Linux, and Windows.
fn executable_search_candidates(name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let executable_names = executable_file_names(name);
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            for executable_name in &executable_names {
                candidates.push(directory.join(executable_name));
            }
        }
    }

    let home = env::var_os("NOLA_EXTERNAL_EXECUTION_HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| env::var_os("HOME"))
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if let Some(home) = home {
        let mut relative_directories = vec![
            PathBuf::from(".local/bin"),
            PathBuf::from("bin"),
            PathBuf::from(".npm-global/bin"),
            PathBuf::from(".bun/bin"),
            PathBuf::from(".volta/bin"),
            PathBuf::from("Library/pnpm"),
            PathBuf::from("AppData/Roaming/npm"),
        ];
        match name {
            "opencode" => relative_directories.insert(0, PathBuf::from(".opencode/bin")),
            "claude" => {
                relative_directories.insert(0, PathBuf::from(".claude/local"));
                relative_directories.insert(1, PathBuf::from(".claude/bin"));
            }
            _ => {}
        }
        for directory in relative_directories {
            for executable_name in &executable_names {
                candidates.push(home.join(&directory).join(executable_name));
            }
        }
    }

    for directory in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        for executable_name in &executable_names {
            candidates.push(Path::new(directory).join(executable_name));
        }
    }
    candidates
}

fn executable_file_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        return vec![
            name.to_string(),
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
        ];
    }
    #[cfg(not(windows))]
    {
        vec![name.to_string()]
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn read_version(path: &Path) -> Option<String> {
    let (status, stdout, _) =
        command_output_with_timeout(path, &["--version"], Duration::from_secs(2)).ok()?;
    if !status.success() {
        return None;
    }
    let value = stdout.trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    #[test]
    fn codex_transport_is_owned_stdio_not_shared_daemon_proxy() {
        assert_eq!(super::codex_app_server_arguments(), ["app-server", "--listen", "stdio://"]);
    }
    #[test]
    fn codex_terminal_requires_explicit_success_status() {
        for status in [Value::Null, serde_json::json!("interrupted"), serde_json::json!("inProgress"), serde_json::json!("failed")] {
            let events = parse_codex_app_server_message(&serde_json::json!({
                "method":"turn/completed", "params":{"turn":{"status":status}}
            }));
            assert!(matches!(events.as_slice(), [ExternalExecutorEvent::Failed { .. }]));
        }
        let events = parse_codex_app_server_message(&serde_json::json!({
            "method":"turn/completed", "params":{"turn":{"status":"completed"}}
        }));
        assert!(matches!(events.as_slice(), [ExternalExecutorEvent::Completed { .. }]));
    }
    #[cfg(unix)]
    #[test]
    fn codex_initialize_deadline_reaps_owned_process_tree() {
        let _environment = crate::store::test_env_lock();
        let previous = std::env::var_os("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS");
        std::env::set_var("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS", "1");
        let (config, descriptor) = protocol_fixture(ExecutionBackendKind::Codex,
            "while IFS= read -r line; do :; done\nwait");
        let mut observer = TestExternalObserver::default();
        let result = run_codex_app_server(&config, "fixture", true, &mut observer, None, &descriptor);
        match previous {
            Some(value) => std::env::set_var("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS", value),
            None => std::env::remove_var("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS"),
        }
        assert!(result.unwrap_err().contains("initialize handshake timed out"));
        assert_protocol_fixture_reaped(&config);
        fs::remove_dir_all(config.project_root).unwrap();
    }
    use super::*;

    #[derive(Default)]
    struct TestExternalObserver {
        cancel_on_rejected_text: Option<RuntimeExecutionControl>,
        reject_session: bool,
        cancel_on_session: Option<RuntimeExecutionControl>,
        stop_confirmed: bool,
        approve_permission: bool,
        reject_permission: bool,
        reject_permission_write: bool,
        permission_writes: Vec<(String, String)>,
        permissions: Vec<(String, Value)>,
    }

    impl ExternalExecutionObserver for TestExternalObserver {
        fn external_text(&mut self, _: &str) -> Result<(), String> {
            if let Some(control) = self.cancel_on_rejected_text.as_ref() {
                control.request(); return Err("fixture output commit rejected".into());
            }
            Ok(())
        }
        fn external_reasoning(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn external_session_started(&mut self, _: &str, _: &str) -> Result<(), String> {
            if let Some(control) = &self.cancel_on_session { control.request(); }
            if self.reject_session {
                Err("fixture admission rejected".into())
            } else {
                Ok(())
            }
        }
        fn external_permission_requested(
            &mut self,
            request_id: &str,
            call_id: &str,
            _: &str,
            arguments: &Value,
            _: &str,
        ) -> Result<Option<bool>, String> {
            self.permissions
                .push((request_id.into(), arguments.clone()));
            if let Some(expected) = arguments
                .pointer("/toolCall/toolCallId")
                .and_then(Value::as_str)
            {
                assert_eq!(call_id, expected);
            }
            if self.reject_permission {
                return Err("fixture claim admission rejected".into());
            }
            Ok(self.approve_permission.then_some(true))
        }
        fn external_permission_written(
            &mut self,
            request_id: &str,
            call_id: &str,
        ) -> Result<(), String> {
            self.permission_writes
                .push((request_id.into(), call_id.into()));
            if self.reject_permission_write {
                Err("fixture written admission rejected".into())
            } else {
                Ok(())
            }
        }
        fn external_tool_started(&mut self, _: &str, _: &str, _: &Value) -> Result<(), String> {
            Ok(())
        }
        fn external_tool_finished(
            &mut self,
            _: &str,
            _: &str,
            _: &Value,
            _: &str,
            _: bool,
        ) -> Result<(), String> {
            Ok(())
        }
        fn external_context_usage(&mut self, _: usize, _: usize) -> Result<(), String> {
            Ok(())
        }
        fn external_execution_cancelled(&mut self, _: &str) -> Result<(), String> {
            self.stop_confirmed = true;
            Ok(())
        }
        fn external_execution_completed(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[cfg(unix)]
    fn protocol_fixture(
        kind: ExecutionBackendKind,
        body: &str,
    ) -> (ExternalExecutionSettings, ExecutionBackendDescriptor) {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "nola-delegated-protocol-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("executor");
        fs::write(&executable, format!("#!/bin/sh\nif [ \"$2\" = daemon ]; then exit 0; fi\necho $$ > process.pid\nsleep 30 &\necho $! > descendant.pid\n{body}\n")).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let mut descriptor = ExecutionBackendDescriptor::built_in_nola();
        descriptor.id = match kind {
            ExecutionBackendKind::Codex => "codex",
            ExecutionBackendKind::OpenCode => "opencode",
            _ => "claude_code",
        }
        .into();
        descriptor.kind = kind;
        descriptor.executable_path = Some(executable.to_string_lossy().into_owned());
        // A protocol fixture has no reason to load the developer's daily
        // configuration or credentials, even when its executable is local.
        let config = ExternalExecutionSettings {
            project_root: directory, execution_backend: descriptor.id.clone(),
            external_execution_session_id: Some("original-session".into()),
            external_execution_model: None, external_execution_model_provider: None,
            host_context_components: Vec::new(), host_attachments: Vec::new(),
            budget: Default::default(), permission_profile: crate::permissions::PermissionProfile::Cautious,
            host_input_origin: nola_core::agent::AgentInputOrigin::InteractiveUser,
        };
        (config, descriptor)
    }

    #[cfg(unix)]
    fn assert_protocol_fixture_reaped(config: &ExternalExecutionSettings) {
        for file in ["process.pid", "descendant.pid"] {
            let pid: i32 = fs::read_to_string(config.project_root.join(file))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "{file} remained alive");
        }
    }

    #[cfg(unix)]
    #[test]
    fn delegated_cancel_after_session_stops_owned_processes_before_ack() {
        for kind in [ExecutionBackendKind::ClaudeCode, ExecutionBackendKind::OpenCode, ExecutionBackendKind::Codex] {
          for during_setup in [true, false] {
            let body = if during_setup { "while IFS= read -r line; do :; done\nwait" } else if kind == ExecutionBackendKind::ClaudeCode {
                r#"while IFS= read -r line; do
  case "$line" in
    *'"subtype":"initialize"'*) echo '{"type":"control_response","response":{"subtype":"success","request_id":"nola-initialize","response":{}}}' ;;
    *'"type":"user"'*) echo '{"type":"system","subtype":"init","session_id":"original-session"}' ;;
  esac
done
wait"#
            } else {
                r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"id":2,"result":{}}' ;;
    *'"method":"thread/resume"'*) echo '{"id":2,"result":{"thread":{"id":"original-session"}}}' ;;
    *'"method":"session/prompt"'*|*'"method":"turn/start"'*) echo dispatched_after_stop > forbidden ;;
  esac
done
wait
"#
            };
            let (config, descriptor) = protocol_fixture(kind, body);
            let control = RuntimeExecutionControl::default();
            let setup_cancel = during_setup.then(|| {
                let control = control.clone();
                let marker = config.project_root.join("descendant.pid");
                std::thread::spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while !marker.exists() && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(5)); }
                    control.request();
                })
            });
            let mut observer = TestExternalObserver { cancel_on_session: (!during_setup).then(|| control.clone()), ..Default::default() };
            let result = match kind {
                ExecutionBackendKind::ClaudeCode => run_stream_json_process(&config, "fixture", true, &mut observer, Some(control), &descriptor),
                ExecutionBackendKind::OpenCode => run_opencode_acp(&config, "fixture", true, &mut observer, Some(control), &descriptor),
                _ => run_codex_app_server(&config, "fixture", true, &mut observer, Some(control), &descriptor),
            };
            if let Some(cancel) = setup_cancel { cancel.join().unwrap(); }
            assert_eq!(result.unwrap_err(), "external execution cancelled");
            assert!(observer.stop_confirmed);
            assert_protocol_fixture_reaped(&config);
            assert!(!config.project_root.join("forbidden").exists(), "cancelled session dispatched a new turn");
            fs::remove_dir_all(config.project_root).unwrap();
          }
        }
    }

    #[cfg(unix)]
    #[test]
    fn blocked_external_stdin_write_is_cancellable_and_reaps_its_processes() {
        let (config, descriptor) = protocol_fixture(ExecutionBackendKind::ClaudeCode,
            "ps -o pgid= -p $$ > process.pgid\ntouch ready\nwait");
        let control = RuntimeExecutionControl::default();
        let worker_control = control.clone();
        let root = config.project_root.clone();
        let (started, ready) = mpsc::sync_channel(1);
        let (finished, result) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let child = external_command(Path::new(descriptor.executable_path.as_ref().unwrap()))
                .current_dir(&config.project_root).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
            let mut child = ManagedExternalChild::new(child);
            let mut stdin = child.stdin.take();
            let mut monitor = ExternalExecutionBudgetMonitor::new(ExternalExecutionBudget::from_config(&config), Arc::new(AtomicU64::new(0)));
            let mut observer = TestExternalObserver::default();
            let deadline = Instant::now() + Duration::from_secs(3);
            while !config.project_root.join("ready").exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            let pid = child.id();
            let pgid = unsafe { libc::getpgid(pid as i32) };
            assert!(config.project_root.join("ready").exists(), "fixture did not become ready: child_pid={pid} pgid={pgid} status={:?}", child.try_wait());
            let reported_pid = fs::read_to_string(config.project_root.join("process.pid")).unwrap();
            let reported_pgid = fs::read_to_string(config.project_root.join("process.pgid")).unwrap();
            eprintln!("blocked-input fixture ready: child_pid={pid} actual_pgid={pgid} script_pid={} script_pgid={}", reported_pid.trim(), reported_pgid.trim());
            assert_eq!(reported_pid.trim().parse::<u32>().unwrap(), pid);
            assert_eq!(reported_pgid.trim().parse::<i32>().unwrap(), pgid);
            assert_eq!(pgid, pid as i32, "fixture must own its process group before testing a group stop");
            started.send(pid).unwrap();
            let outcome = write_external_input(&mut stdin, vec![b'x'; 2 * 1024 * 1024], &mut child,
                Some(&worker_control), &mut monitor, &mut observer, "claude_code");
            finished.send((outcome, observer.stop_confirmed)).unwrap();
        });
        let pid = ready.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(result.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)), "non-consuming child unexpectedly accepted a 2MiB write");
        control.request();
        let (outcome, confirmed) = result.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|error| {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
            panic!("blocked input did not respond to cancellation: {error}");
        });
        worker.join().unwrap();
        assert_eq!(outcome.unwrap_err(), "external execution cancelled");
        assert!(confirmed);
        // Both PIDs were recorded before the non-consuming shell entered wait.
        for file in ["process.pid", "descendant.pid"] {
            let pid: i32 = fs::read_to_string(root.join(file)).unwrap().trim().parse().unwrap();
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "blocked writer left {file} alive");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn delegated_strict_resume_protocol_failure_never_starts_a_new_session() {
        for kind in [ExecutionBackendKind::OpenCode, ExecutionBackendKind::Codex] {
            let (config, descriptor) = protocol_fixture(
                kind.clone(),
                r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*|*'"method":"session/resume"'*|*'"method":"thread/resume"'*) echo '{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"unknown original session"}}' ;;
    *'"method":"session/new"'*|*'"method":"thread/start"'*) echo fallback > forbidden ;;
  esac
done
wait
"#,
            );
            let mut observer = TestExternalObserver::default();
            let error = match kind {
                ExecutionBackendKind::OpenCode => {
                    run_opencode_acp(&config, "fixture", true, &mut observer, None, &descriptor)
                }
                _ => {
                    run_codex_app_server(&config, "fixture", true, &mut observer, None, &descriptor)
                }
            }
            .unwrap_err();
            assert!(error.contains("unknown original session"), "{error}");
            let requests = fs::read_to_string(config.project_root.join("requests.jsonl")).unwrap();
            assert!(requests.contains("original-session"));
            assert!(
                !requests.contains("session/new") && !requests.contains("thread/start"),
                "{requests}"
            );
            assert!(!config.project_root.join("forbidden").exists());
            assert_protocol_fixture_reaped(&config);
            fs::remove_dir_all(config.project_root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn delegated_observer_rejection_stops_all_real_protocol_processes() {
        for kind in [
            ExecutionBackendKind::ClaudeCode,
            ExecutionBackendKind::OpenCode,
            ExecutionBackendKind::Codex,
        ] {
            let body = if kind == ExecutionBackendKind::ClaudeCode {
                r#"while IFS= read -r line; do
  case "$line" in
    *'"subtype":"initialize"'*) echo '{"type":"control_response","response":{"subtype":"success","request_id":"nola-initialize","response":{}}}' ;;
    *'"type":"user"'*) echo '{"type":"system","subtype":"init","session_id":"original-session"}' ;;
  esac
done
wait"#
            } else {
                r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*|*'"method":"session/resume"'*) echo '{"jsonrpc":"2.0","id":2,"result":{}}' ;;
    *'"method":"thread/resume"'*) echo '{"id":2,"result":{"thread":{"id":"original-session"}}}' ;;
    *'"method":"session/prompt"'*|*'"method":"turn/start"'*) echo forbidden > forbidden ;;
  esac
done
wait
"#
            };
            let (config, descriptor) = protocol_fixture(kind.clone(), body);
            let mut observer = TestExternalObserver {
                reject_session: true,
                ..Default::default()
            };
            let result = match kind {
                ExecutionBackendKind::ClaudeCode => run_stream_json_process(
                    &config,
                    "fixture",
                    true,
                    &mut observer,
                    None,
                    &descriptor,
                ),
                ExecutionBackendKind::OpenCode => {
                    run_opencode_acp(&config, "fixture", true, &mut observer, None, &descriptor)
                }
                _ => {
                    run_codex_app_server(&config, "fixture", true, &mut observer, None, &descriptor)
                }
            };
            assert_eq!(result.unwrap_err(), "fixture admission rejected");
            assert!(!config.project_root.join("forbidden").exists());
            assert_protocol_fixture_reaped(&config);
            fs::remove_dir_all(config.project_root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn opencode_output_commit_rejection_still_sends_native_cancel_before_eof() {
        let body = r#"
trap 'kill "$(cat descendant.pid)" 2>/dev/null; wait' EXIT
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"id":2,"result":{}}' ;;
    *'"method":"session/prompt"'*) echo '{"method":"session/update","params":{"sessionId":"original-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"fictional output"}}}}' ;;
    *'"method":"session/cancel"'*) echo '{"id":4,"result":{"stopReason":"cancelled"}}'; exit 0 ;;
  esac
done
"#;
        let (config, descriptor) = protocol_fixture(ExecutionBackendKind::OpenCode, body);
        let token = RuntimeExecutionControl::default();
        let mut observer = TestExternalObserver {cancel_on_rejected_text:Some(token.clone()),..Default::default()};
        let error = run_opencode_acp(&config,"fictional output",true,&mut observer,Some(token),&descriptor).unwrap_err();
        assert_eq!(error,"external execution cancelled");
        assert!(observer.stop_confirmed);
        let wire=fs::read_to_string(config.project_root.join("requests.jsonl")).unwrap();
        assert!(wire.contains("session/cancel"));
        assert_protocol_fixture_reaped(&config);
        fs::remove_dir_all(config.project_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn opencode_provider_quota_stops_prompt_without_waiting_for_resource_timeout() {
        let body = r#"
trap 'kill "$(cat descendant.pid)" 2>/dev/null; wait' EXIT
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"jsonrpc":"2.0","id":2,"result":{}}' ;;
    *'"method":"session/prompt"'*) printf '%s\n' 'error="AI_APICallError: Free usage limit reached. fictional-secret"' >&2 ;;
  esac
done
"#;
        let (config, descriptor) = protocol_fixture(ExecutionBackendKind::OpenCode, body);
        let mut observer = TestExternalObserver::default();
        let started = Instant::now();
        let message = run_opencode_acp(&config, "fictional quota fixture", true, &mut observer, None, &descriptor).unwrap_err();
        assert!(message.contains("no remaining free quota"), "{message}");
        assert!(!message.contains("fictional-secret"));
        assert!(!message.contains("sampler"));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_protocol_fixture_reaped(&config);
        fs::remove_dir_all(config.project_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn delegated_opencode_permission_keeps_rpc_call_and_input_without_autoapproval() {
        let (config, descriptor) = protocol_fixture(
            ExecutionBackendKind::OpenCode,
            r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"id":2,"result":{}}' ;;
    *'"method":"session/prompt"'*) echo '{"id":"permission-37","method":"session/request_permission","params":{"sessionId":"original-session","toolCall":{"toolCallId":"call-9","kind":"read","rawInput":{"path":"/private/file"},"locations":[{"path":"/private/file"}]},"options":[{"optionId":"yes","kind":"allow_once"},{"optionId":"no","kind":"reject_once"}]}}' ;;
    *'"id":"permission-37"'*) echo unauthorized > forbidden ;;
  esac
done
wait
"#,
        );
        let mut observer = TestExternalObserver::default();
        let error = run_opencode_acp(&config, "fixture", true, &mut observer, None, &descriptor)
            .unwrap_err();
        assert!(error.contains("no decision received"), "{error}");
        assert_eq!(observer.permissions.len(), 1);
        let (id, params) = &observer.permissions[0];
        assert_eq!(id, "permission-37");
        assert_eq!(params.pointer("/toolCall/toolCallId").unwrap(), "call-9");
        assert_eq!(
            params.pointer("/toolCall/rawInput/path").unwrap(),
            "/private/file"
        );
        assert_eq!(
            params.pointer("/toolCall/locations/0/path").unwrap(),
            "/private/file"
        );
        assert!(!config.project_root.join("forbidden").exists());
        assert_protocol_fixture_reaped(&config);
        fs::remove_dir_all(config.project_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn delegated_permission_observer_rejection_gates_protocol_write_and_reaps_processes() {
        for (kind, reject_claim) in [
            (ExecutionBackendKind::OpenCode, true),
            (ExecutionBackendKind::OpenCode, false),
            (ExecutionBackendKind::Codex, true),
            (ExecutionBackendKind::Codex, false),
        ] {
            let (config, descriptor) = protocol_fixture(
                kind.clone(),
                r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"id":2,"result":{}}' ;;
    *'"method":"thread/resume"'*) echo '{"id":2,"result":{"thread":{"id":"original-session"}}}' ;;
    *'"method":"session/prompt"'*) echo '{"id":"permission-37","method":"session/request_permission","params":{"sessionId":"original-session","toolCall":{"toolCallId":"call-9","rawInput":{"path":"/private/file"}},"options":[{"optionId":"yes","kind":"allow_once"}]}}' ;;
    *'"method":"turn/start"'*) echo '{"id":10,"result":{"turn":{"id":"fixture-turn"}}}'; echo '{"id":"permission-37","method":"item/commandExecution/requestApproval","params":{"threadId":"original-session","turnId":"fixture-turn","itemId":"call-9","command":"fixture"}}' ;;
  esac
done
wait
"#,
            );
            let mut observer = TestExternalObserver {
                approve_permission: true,
                reject_permission: reject_claim,
                reject_permission_write: true,
                ..Default::default()
            };
            let result = match kind {
                ExecutionBackendKind::OpenCode => {
                    run_opencode_acp(&config, "fixture", true, &mut observer, None, &descriptor)
                }
                _ => {
                    run_codex_app_server(&config, "fixture", true, &mut observer, None, &descriptor)
                }
            };
            if reject_claim {
                assert_eq!(result.unwrap_err(), "fixture claim admission rejected");
                assert!(observer.permission_writes.is_empty());
                let requests =
                    fs::read_to_string(config.project_root.join("requests.jsonl")).unwrap();
                assert!(
                    !requests.contains("permission-37"),
                    "rejected claim wrote a protocol decision: {requests}"
                );
            } else {
                assert_eq!(result.unwrap_err(), "fixture written admission rejected");
                assert_eq!(
                    observer.permission_writes,
                    vec![("permission-37".into(), "call-9".into())]
                );
            }
            assert_protocol_fixture_reaped(&config);
            fs::remove_dir_all(config.project_root).unwrap();
        }
    }

    #[test]
    fn catalog_is_stable_even_when_optional_clis_are_absent() {
        let catalog = discover_execution_backends();
        assert_eq!(catalog.len(), 4);
        assert_eq!(catalog[0].id, "nola");
        assert!(catalog.iter().any(|item| item.id == "codex"));
        assert!(catalog.iter().any(|item| item.id == "claude_code"));
        assert!(catalog.iter().any(|item| item.id == "opencode"));
    }

    #[test]
    fn built_in_backend_resolution_is_local_and_does_not_probe_external_clis() {
        let started = Instant::now();
        let descriptor = discover_execution_backend("nola", false).expect("built-in backend");
        assert_eq!(descriptor, ExecutionBackendDescriptor::built_in_nola());
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "built-in backend resolution unexpectedly performed external discovery: {:?}",
            started.elapsed()
        );
        assert!(discover_execution_backend("unknown", false).is_none());
    }

    #[test]
    fn executable_discovery_includes_official_user_install_locations() {
        let home = crate::platform_paths::user_home_dir().expect("effective execution home");
        let opencode = executable_search_candidates("opencode");
        assert!(opencode.contains(&home.join(".opencode/bin/opencode")));
        let claude = executable_search_candidates("claude");
        assert!(claude.contains(&home.join(".local/bin/claude")));
        let codex = executable_search_candidates("codex");
        assert!(codex.contains(&home.join(".npm-global/bin/codex")));
    }

    #[test]
    fn catalog_machine_contract_serializes_diagnostics_and_capabilities() {
        let encoded = serde_json::to_value(discover_execution_backends()).unwrap();
        let entries = encoded.as_array().unwrap();
        assert_eq!(entries.len(), 4);
        for entry in entries {
            assert_eq!(entry["schema_version"], EXECUTION_BACKEND_SCHEMA_VERSION);
            assert!(entry["id"].is_string());
            assert!(entry["display_name"].is_string());
            assert!(entry["availability"].is_string());
            assert!(entry["capabilities"]["streams_text"].is_boolean());
            assert!(entry["capabilities"]["supports_resume"].is_boolean());
        }
        let opencode = entries
            .iter()
            .find(|entry| entry["id"] == "opencode")
            .unwrap();
        assert_eq!(opencode["transport"], "json_rpc_stdio");
    }

    #[test]
    fn opencode_acp_updates_preserve_stream_and_tool_lifecycle() {
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"hello"}
            })),
            vec![ExternalExecutorEvent::TextDelta {
                text: "hello".into()
            }]
        );
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"agent_thought_chunk",
                "content":{"type":"text","text":"inspect"}
            })),
            vec![ExternalExecutorEvent::ReasoningDelta {
                text: "inspect".into()
            }]
        );
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"tool_call",
                "toolCallId":"call-1",
                "title":"Read file",
                "rawInput":{"path":"/tmp/a.txt"}
            })),
            vec![ExternalExecutorEvent::ToolStarted {
                call_id: "call-1".into(),
                name: "Read file".into(),
                arguments: serde_json::json!({"path":"/tmp/a.txt"}),
            }]
        );
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"usage_update",
                "used":53000,
                "size":200000
            })),
            vec![ExternalExecutorEvent::Usage {
                used_tokens: 53000,
                context_window_tokens: 200000,
            }]
        );
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"tool_call_update",
                "toolCallId":"call-1",
                "title":"Read file",
                "status":"completed",
                "rawOutput":"body"
            })),
            vec![ExternalExecutorEvent::ToolFinished {
                call_id: "call-1".into(),
                name: "Read file".into(),
                output: "body".into(),
                failed: false,
            }]
        );
    }

    #[test]
    fn opencode_acp_permission_requires_an_explicit_decision() {
        let params = serde_json::json!({"toolCall":{"kind":"read","rawInput":{"path":"/tmp/file"}},
            "options":[{"optionId":"yes","kind":"allow_once"},{"optionId":"no","kind":"reject_once"}]});
        assert!(opencode_acp_permission_outcome(&params, None).is_err());
        assert_eq!(
            opencode_acp_permission_outcome(&params, Some(true)).unwrap(),
            serde_json::json!({"outcome":"selected","optionId":"yes"})
        );
        assert_eq!(
            opencode_acp_permission_outcome(&params, Some(false)).unwrap(),
            serde_json::json!({"outcome":"selected","optionId":"no"})
        );
        assert!(opencode_acp_permission_outcome(&serde_json::json!({}), Some(true)).is_err());
    }

    #[test]
    fn installed_entries_have_canonical_absolute_paths() {
        for entry in discover_execution_backends().into_iter().skip(1) {
            if entry.availability == ExecutionBackendAvailability::Available {
                let path = PathBuf::from(entry.executable_path.expect("available path"));
                assert!(path.is_absolute());
                assert!(is_executable(&path));
                assert!(entry.version.is_some());
            }
        }
    }

    #[test]
    fn codex_fixture_preserves_ordered_lifecycle() {
        let lines = [
            r#"{"type":"thread.started","thread_id":"thread-1"}"#,
            r#"{"type":"item.completed","item":{"id":"item-0","type":"agent_message","text":"hello"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":12,"output_tokens":3}}"#,
        ];
        let events = lines
            .iter()
            .flat_map(|line| parse_event_line(ExecutionBackendKind::Codex, line))
            .collect::<Vec<_>>();
        assert_eq!(
            events[0],
            ExternalExecutorEvent::Started {
                session_id: "thread-1".into()
            }
        );
        assert_eq!(
            events[1],
            ExternalExecutorEvent::TextDelta {
                text: "hello".into()
            }
        );
        assert!(matches!(
            events[2],
            ExternalExecutorEvent::Completed {
                input_tokens: Some(12),
                output_tokens: Some(3),
                ..
            }
        ));
    }

    #[test]
    fn codex_agent_message_items_preserve_paragraph_boundaries() {
        let notifications = [
            serde_json::json!({
                "method":"item/agentMessage/delta",
                "params":{"itemId":"progress-1","delta":"Inspecting"}
            }),
            serde_json::json!({
                "method":"item/agentMessage/delta",
                "params":{"itemId":"progress-1","delta":" the project."}
            }),
            serde_json::json!({
                "method":"item/agentMessage/delta",
                "params":{"itemId":"progress-2","delta":"Reviewing changes."}
            }),
            serde_json::json!({
                "method":"item/agentMessage/delta",
                "params":{"itemId":"final","delta":"\n\nSummary:\n- one\n- two"}
            }),
        ];
        let mut boundary = CodexAgentMessageBoundary::default();
        let mut output = String::new();
        for notification in notifications {
            let events = parse_codex_app_server_message(&notification);
            let ExternalExecutorEvent::TextDelta { text } = &events[0] else {
                panic!("expected text delta");
            };
            if let Some(separator) = boundary.separator_before(&notification, text) {
                output.push_str(separator);
            }
            output.push_str(text);
        }
        assert_eq!(
            output,
            "Inspecting the project.\n\nReviewing changes.\n\nSummary:\n- one\n- two"
        );
    }

    #[test]
    fn codex_agent_message_boundary_requires_item_ids_for_new_blocks() {
        let notification = serde_json::json!({
            "method":"item/agentMessage/delta",
            "params":{"delta":"continued"}
        });
        let mut boundary = CodexAgentMessageBoundary::default();
        assert_eq!(boundary.separator_before(&notification, "first"), None);
        assert_eq!(boundary.separator_before(&notification, "continued"), None);

        assert_eq!(boundary.separator_before(&notification, "next block"), None);
        assert_eq!(boundary.separator_before(&notification, " continued"), None);
    }

    #[test]
    fn claude_snapshot_does_not_duplicate_streamed_text() {
        let delta = r#"{"type":"stream_event","session_id":"s1","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}}"#;
        let snapshot = r#"{"type":"assistant","session_id":"s1","message":{"content":[{"type":"text","text":"hello"}]}}"#;
        assert_eq!(
            parse_event_line(ExecutionBackendKind::ClaudeCode, delta),
            vec![ExternalExecutorEvent::TextDelta {
                text: "hello".into()
            }]
        );
        assert!(parse_event_line(ExecutionBackendKind::ClaudeCode, snapshot).is_empty());
    }

    #[test]
    fn opencode_acp_model_selection_is_not_a_cli_flag() {
        let mut descriptor = ExecutionBackendDescriptor::built_in_nola();
        descriptor.kind = ExecutionBackendKind::OpenCode;
        descriptor.executable_path = Some("/fixture/opencode".into());
        let command = command_for(&descriptor, Path::new("/fixture/workspace"),
            Some("original-child"), Some("openai/selected-model"), None).unwrap();
        assert_eq!(command.arguments, ["acp", "--cwd", "/fixture/workspace"]);
    }

    #[test]
    fn claude_failed_result_preserves_protocol_errors() {
        for (extra, expected) in [
            (serde_json::json!({"errors":["model unavailable", "login required"]}), "model unavailable\nlogin required"),
            (serde_json::json!({"errors":[]}), "error_during_execution"),
        ] {
            let mut result = serde_json::json!({"type":"result", "subtype":"error_during_execution", "is_error":true});
            result.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            assert_eq!(parse_claude_event(&result), vec![ExternalExecutorEvent::Failed {message:expected.into()}]);
        }
    }

    #[test]
    fn claude_snapshots_preserve_tool_lifecycle_without_repeating_text() {
        let started = r#"{"type":"assistant","session_id":"s1","message":{"content":[{"type":"thinking","thinking":"inspect"},{"type":"tool_use","id":"call-1","name":"Read","input":{"file_path":"/tmp/input.txt"}},{"type":"text","text":"duplicate"}]}}"#;
        assert_eq!(
            parse_event_line(ExecutionBackendKind::ClaudeCode, started),
            vec![ExternalExecutorEvent::ToolStarted {
                call_id: "call-1".into(),
                name: "Read".into(),
                arguments: serde_json::json!({"file_path":"/tmp/input.txt"}),
            }]
        );
        let finished = r#"{"type":"user","session_id":"s1","message":{"content":[{"type":"tool_result","tool_use_id":"call-1","content":"file body"}]}}"#;
        assert_eq!(
            parse_event_line(ExecutionBackendKind::ClaudeCode, finished),
            vec![ExternalExecutorEvent::ToolFinished {
                call_id: "call-1".into(),
                name: "Claude Code tool".into(),
                output: "file body".into(),
                failed: false,
            }]
        );
    }

    #[test]
    fn native_accounting_keeps_cumulative_scope_and_unknown_counters() {
        use nola_domain::delegated_invocation::ExternalUsageBasis;
        let frame=serde_json::json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-1",
            "tokenUsage":{"total":{"inputTokens":1200,"cachedInputTokens":800,"outputTokens":50},"last":{"inputTokens":200}}}});
        let first=external_accounting_snapshot("codex",&frame).unwrap();
        assert_eq!(first.basis,ExternalUsageBasis::SessionCumulative);
        assert_eq!(first.scope_id,"thread-1"); assert_eq!(first.input_tokens,Some(1200));
        assert_eq!(first.cache_read_tokens,Some(800)); assert_eq!(first.cache_write_tokens,None);
        assert_eq!(external_accounting_snapshot("codex",&frame),Some(first));
        let claude=external_accounting_snapshot("claude_code",&serde_json::json!({"type":"result","session_id":"session",
            "usage":{"input_tokens":10,"cache_read_input_tokens":90,"cache_creation_input_tokens":20,"output_tokens":5},
            "modelUsage":{"actual-model":{}}})).unwrap();
        assert_eq!(claude.input_tokens,Some(120)); assert_eq!(claude.output_tokens,Some(5));
        assert_eq!(claude.model_id.as_deref(),Some("actual-model")); assert_eq!(claude.provider_id,None);
        assert!(external_accounting_snapshot("opencode",&serde_json::json!({"sessionUpdate":"usage_update","used":100,"size":10000})).is_none());
        let opencode=external_accounting_snapshot("opencode",&serde_json::json!({"result":{"usage":{"inputTokens":3,"outputTokens":8,"cachedReadTokens":90,"cachedWriteTokens":7}}})).unwrap();
        assert_eq!(opencode.basis,ExternalUsageBasis::LastModelResponse);
        assert_eq!(opencode.input_tokens,Some(100)); assert_eq!(opencode.cache_read_tokens,Some(90));
    }

    #[test]
    fn claude_result_projects_cache_aware_authoritative_context_usage() {
        let result = serde_json::json!({
            "type":"result",
            "session_id":"s1",
            "is_error":false,
            "usage":{
                "input_tokens":120,
                "cache_read_input_tokens":18000,
                "cache_creation_input_tokens":880,
                "output_tokens":240
            },
            "modelUsage":{
                "claude-opus-4-7":{
                    "inputTokens":120,
                    "cacheReadInputTokens":18000,
                    "cacheCreationInputTokens":880,
                    "outputTokens":240,
                    "contextWindow":200000
                }
            }
        });
        assert_eq!(
            parse_claude_event(&result),
            vec![
                ExternalExecutorEvent::Usage {
                    used_tokens: 19000,
                    context_window_tokens: 200000,
                },
                ExternalExecutorEvent::Completed {
                    session_id: Some("s1".into()),
                    input_tokens: Some(19000),
                    output_tokens: Some(240),
                },
            ]
        );
    }

    #[test]
    fn command_builders_keep_global_codex_flags_before_exec() {
        let descriptor = discover_execution_backends()
            .into_iter()
            .find(|item| item.id == "codex")
            .unwrap();
        if descriptor.availability != ExecutionBackendAvailability::Available {
            return;
        }
        let command = command_for(
            &descriptor,
            Path::new("/tmp/project"),
            None,
            Some("gpt-test"),
            Some("test_provider"),
        )
        .unwrap();
        assert!(command
            .arguments
            .windows(2)
            .any(|pair| pair == ["--model", "gpt-test"]));
        assert!(command
            .arguments
            .windows(2)
            .any(|pair| pair == ["-c", "model_provider=test_provider"]));
        assert!(command
            .arguments
            .windows(2)
            .any(|pair| pair == ["--sandbox", "workspace-write"]));
        let exec = command
            .arguments
            .iter()
            .position(|arg| arg == "exec")
            .unwrap();
        let sandbox = command
            .arguments
            .iter()
            .position(|arg| arg == "--sandbox")
            .unwrap();
        assert!(sandbox < exec);
        assert_eq!(command.arguments.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn codex_resume_keeps_resume_only_flags_after_subcommand() {
        let descriptor = discover_execution_backends()
            .into_iter()
            .find(|item| item.id == "codex")
            .unwrap();
        if descriptor.availability != ExecutionBackendAvailability::Available {
            return;
        }
        let command = command_for(
            &descriptor,
            Path::new("/tmp/project"),
            Some("thread-1"),
            None,
            None,
        )
        .unwrap();
        let resume = command
            .arguments
            .iter()
            .position(|arg| arg == "resume")
            .unwrap();
        assert_eq!(
            &command.arguments[resume..],
            &["resume", "--json", "--skip-git-repo-check", "thread-1", "-"]
        );
    }

    #[test]
    fn claude_permission_bridge_requires_owned_session_call_and_explicit_decision() {
        let mut descriptor = ExecutionBackendDescriptor::built_in_nola();
        descriptor.kind = ExecutionBackendKind::ClaudeCode;
        descriptor.executable_path = Some("/fictional/claude".into());
        let command = command_for(&descriptor, Path::new("/fictional"), None, None, None).unwrap();
        assert!(command.arguments.windows(2).any(|v| v == ["--permission-mode", "default"]));
        assert!(command.arguments.windows(2).any(|v| v == ["--permission-prompt-tool", "stdio"]));
        assert!(!command.arguments.iter().any(|v| v == "dontAsk" || v.contains("bypass")));
        let mut control = ClaudeControl::default();
        let tools = HashMap::from([("call-1".into(), ("Write".into(), serde_json::json!({"file_path":"/isolated/result.txt","content":"fictional"}))) ]);
        let frame = serde_json::json!({"type":"control_request", "request_id":"permission-1", "request":{
            "subtype":"can_use_tool", "tool_use_id":"call-1", "tool_name":"Write", "input":tools["call-1"].1}});
        assert_eq!(control.initialize()["request"]["subtype"], "initialize");
        assert!(control.permission(&frame, Some("owned-session"), &tools).is_err());
        let initialized = serde_json::json!({"type":"control_response", "response":{"request_id":"nola-initialize","subtype":"success","response":{}}});
        control.accept_initialize(&initialized).unwrap();
        assert!(control.accept_initialize(&initialized).is_err());
        assert!(control.permission(&frame, None, &tools).is_err());
        let mut foreign = frame.clone(); foreign["session_id"] = "foreign-session".into();
        assert!(control.permission(&foreign, Some("owned-session"), &tools).is_err());
        let mut changed = frame.clone(); changed["request"]["input"]["content"] = "changed after admission".into();
        assert!(control.permission(&changed, Some("owned-session"), &tools).is_err());
        let permission = control.permission(&frame, Some("owned-session"), &tools).unwrap();
        assert!(permission.response(None).is_err());
        let denied = permission.response(Some(false)).unwrap();
        assert_eq!(denied["response"]["request_id"], "permission-1");
        assert_eq!(denied["response"]["response"]["behavior"], "deny");
        let allowed = permission.response(Some(true)).unwrap();
        assert_eq!(allowed["response"]["response"]["updatedInput"], tools["call-1"].1);
        assert!(allowed["response"]["response"].get("updatedPermissions").is_none());
        assert!(control.permission(&frame, Some("owned-session"), &tools).is_err());
        let mut early = ClaudeControl::default(); early.accept_initialize(&initialized).unwrap();
        early.permission(&frame, Some("owned-session"), &HashMap::new()).unwrap();
        early.validate_tool("call-1", "Write", &tools["call-1"].1).unwrap();
        assert!(early.validate_tool("call-1", "Write", &serde_json::json!({"content":"changed"})).is_err());
        control.observe_tasks(&serde_json::json!({"type":"system","subtype":"task_started","task_id":"child"}));
        assert!(!control.tasks.is_empty(), "Keep permission input open for a live native child");
        control.observe_tasks(&serde_json::json!({"type":"system","subtype":"task_notification","task_id":"child"}));
        assert!(control.tasks.is_empty());
    }

    #[test]
    fn claude_uses_native_stream_input_for_new_and_resumed_sessions() {
        let descriptor = discover_execution_backends()
            .into_iter()
            .find(|item| item.id == "claude_code")
            .unwrap();
        if descriptor.availability != ExecutionBackendAvailability::Available {
            return;
        }
        let fresh = command_for(
            &descriptor,
            Path::new("/tmp/project"),
            None,
            Some("sonnet"),
            None,
        )
        .unwrap();
        assert!(fresh.prompt_via_stdin);
        assert!(fresh
            .arguments
            .windows(2)
            .any(|pair| pair == ["--input-format", "stream-json"]));
        assert!(!fresh
            .arguments
            .iter()
            .any(|argument| argument == "--resume"));
        assert!(fresh
            .arguments
            .windows(2)
            .any(|pair| pair == ["--model", "sonnet"]));

        let resumed = command_for(
            &descriptor,
            Path::new("/tmp/project"),
            Some("session-1"),
            Some("sonnet"),
            None,
        )
        .unwrap();
        assert!(resumed.prompt_via_stdin);
        assert!(resumed
            .arguments
            .windows(2)
            .any(|pair| pair == ["--resume", "session-1"]));
    }

    #[test]
    fn external_image_content_is_native_for_claude_and_opencode() {
        let root =
            std::env::temp_dir().join(format!("nola-external-image-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("sample.png");
        std::fs::write(&image, b"not-empty-image-fixture").unwrap();
        let attachments = vec![crate::external_execution_settings::HostAttachment {
            path: image.clone(),
            name: "sample.png".to_string(),
            mime_type: "image/png".to_string(),
            byte_count: std::fs::metadata(&image).unwrap().len(),
        }];

        let content = external_image_content_blocks(
            &attachments,
            "describe it".to_string(),
            ExternalImageBlockStyle::Claude,
        )
        .unwrap();
        let claude = serde_json::json!({
            "type":"user",
            "message":{"role":"user", "content":content}
        })
        .to_string();
        let claude: Value = serde_json::from_str(claude.trim()).unwrap();
        assert_eq!(claude["message"]["content"][1]["type"], "image");
        assert_eq!(
            claude["message"]["content"][1]["source"]["media_type"],
            "image/png"
        );

        let acp = external_image_content_blocks(
            &attachments,
            "describe it".to_string(),
            ExternalImageBlockStyle::Acp,
        )
        .unwrap();
        assert_eq!(acp[1]["type"], "image");
        assert_eq!(acp[1]["mimeType"], "image/png");
        assert!(!acp[1]["data"].as_str().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_native_image_at_thirty_megabyte_product_limit_is_preserved() {
        let root = std::env::temp_dir().join(format!(
            "nola-external-large-image-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("large.png");
        let file = std::fs::File::create(&image).unwrap();
        file.set_len(30 * 1024 * 1024).unwrap();
        let attachments = vec![crate::external_execution_settings::HostAttachment {
            path: image.clone(),
            name: "large.png".to_string(),
            mime_type: "image/png".to_string(),
            byte_count: 30 * 1024 * 1024,
        }];

        let content = external_image_content_blocks(
            &attachments,
            "inspect".to_string(),
            ExternalImageBlockStyle::Claude,
        )
        .unwrap();
        assert!(content[1]["source"]["data"].as_str().unwrap().len() > 40_000_000);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opencode_current_json_contract_preserves_session_tools_and_terminal_event() {
        let lines = [
            r#"{"type":"step_start","timestamp":1,"sessionID":"ses-1","part":{"type":"step-start"}}"#,
            r#"{"type":"tool_use","timestamp":2,"sessionID":"ses-1","part":{"id":"part-1","callID":"call-1","tool":"read","state":{"status":"completed","input":{"filePath":"README.md"},"output":"contents"}}}"#,
            r#"{"type":"text","timestamp":3,"sessionID":"ses-1","part":{"type":"text","text":"answer"}}"#,
            r#"{"type":"step_finish","timestamp":4,"sessionID":"ses-1","part":{"type":"step-finish","tokens":{"input":12,"output":4}}}"#,
        ];
        let events = lines
            .iter()
            .flat_map(|line| parse_event_line(ExecutionBackendKind::OpenCode, line))
            .collect::<Vec<_>>();
        assert_eq!(
            events[0],
            ExternalExecutorEvent::Started {
                session_id: "ses-1".into()
            }
        );
        assert!(
            matches!(&events[1], ExternalExecutorEvent::ToolStarted { call_id, name, .. } if call_id == "call-1" && name == "read")
        );
        assert!(
            matches!(&events[2], ExternalExecutorEvent::ToolFinished { call_id, output, failed: false, .. } if call_id == "call-1" && output == "contents")
        );
        assert_eq!(
            events[3],
            ExternalExecutorEvent::TextDelta {
                text: "answer".into()
            }
        );
        assert!(
            matches!(events[4], ExternalExecutorEvent::Completed { session_id: Some(ref id), input_tokens: Some(12), output_tokens: Some(4) } if id == "ses-1")
        );
    }

    #[test]
    fn opencode_nested_api_error_remains_actionable() {
        let line = r#"{"type":"error","sessionID":"ses-2","error":{"name":"APIError","data":{"message":"No endpoints found that support tool use","statusCode":404}}}"#;
        assert_eq!(
            parse_event_line(ExecutionBackendKind::OpenCode, line),
            vec![ExternalExecutorEvent::Failed {
                message: "No endpoints found that support tool use".into(),
            }]
        );
    }

    #[test]
    fn external_prompt_uses_typed_nola_context_without_replaying_chat_history() {
        let mut config = Config::for_test().unwrap().external_execution_settings();
        config.host_context_components = vec![nola_core::context_engine::ContextComponent {
            kind: nola_core::context_engine::ContextComponentKind::Document,
            id: "document-1".to_string(),
            content: Value::String("trusted reference".to_string()),
            source: None,
        }];
        let prompt = compose_external_prompt(&config, "current request");
        assert!(prompt.contains("<nola_turn_context>"));
        assert!(prompt.contains("<nola_execution_contract>"));
        assert!(prompt.contains("<nola_action_scope>"));
        assert!(prompt.contains("code in the conversation by default"));
        assert!(prompt.contains("user-facing assistant"));
        assert!(prompt.contains(&config.project_root.display().to_string()));
        assert!(prompt.contains("trusted reference"));
        assert!(prompt.contains("<user_request>\ncurrent request"));
        assert!(!prompt.contains("previous assistant answer"));
    }

    #[test]
    fn external_prompt_keeps_identity_and_workspace_without_optional_context() {
        let config = Config::for_test().unwrap().external_execution_settings();
        let prompt = compose_external_prompt(&config, "who are you?");
        assert!(prompt.contains("<nola_execution_contract>"));
        assert!(prompt.contains("inside Nola"));
        assert!(prompt.contains(&config.project_root.display().to_string()));
        assert!(prompt.contains("<user_request>\nwho are you?"));
        assert!(!prompt.contains("<nola_turn_context>"));
    }

    #[test]
    fn resumed_external_prompt_does_not_repeat_the_stable_execution_contract() {
        let mut config = Config::for_test().unwrap().external_execution_settings();
        config.external_execution_session_id = Some("existing-session".to_string());
        let prompt = compose_external_prompt(&config, "continue");
        assert!(!prompt.contains("<nola_execution_contract>"));
        assert!(prompt.contains("<nola_action_scope>"));
        assert!(prompt.contains("<user_request>\ncontinue"));
    }

    #[test]
    fn codex_model_list_uses_protocol_display_names_and_default() {
        let response = serde_json::json!({
            "id": 2,
            "result": {
                "data": [
                    {"id":"catalog-a","model":"gpt-a","displayName":"GPT A","hidden":false,"isDefault":true},
                    {"id":"catalog-hidden","model":"gpt-hidden","displayName":"Hidden","hidden":true,"isDefault":false},
                    {"id":"catalog-b","model":"gpt-b","displayName":"GPT B","hidden":false,"isDefault":false}
                ]
            }
        });
        let (models, default_model) = parse_codex_model_list_response(&response).unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["gpt-a", "gpt-b"]
        );
        assert_eq!(
            models
                .iter()
                .map(|model| model.display_name.as_str())
                .collect::<Vec<_>>(),
            vec!["GPT A", "GPT B"]
        );
        assert_eq!(default_model.as_deref(), Some("gpt-a"));
    }

    #[cfg(unix)]
    #[test]
    fn external_executor_termination_kills_descendants_and_leaves_no_orphan() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "nola-external-process-tree-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create process-tree fixture directory");
        let descendant_pid_path = directory.join("descendant.pid");
        let script = format!(
            "sleep 30 & echo $! > '{}'; wait",
            descendant_pid_path.display()
        );
        let mut command = external_command(Path::new("/bin/sh"));
        let mut child = command
            .args(["-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn external process tree");

        let deadline = Instant::now() + Duration::from_secs(2);
        while !descendant_pid_path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let descendant_pid = std::fs::read_to_string(&descendant_pid_path)
            .expect("external descendant pid")
            .trim()
            .parse::<i32>()
            .expect("numeric descendant pid");

        terminate_external_child(&mut child).expect("terminate external process tree");
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(descendant_pid, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(
            unsafe { libc::kill(descendant_pid, 0) },
            0,
            "external executor descendant remained alive after cancellation"
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn dropping_external_executor_after_protocol_error_kills_its_process_tree() {
        let mut command = external_command(Path::new("/bin/sh"));
        let child = ManagedExternalChild::new(
            command
                .args(["-c", "sleep 30 & wait"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn guarded external process tree"),
        );
        let process_group = i32::try_from(child.id()).expect("external process group id");
        std::thread::sleep(Duration::from_millis(50));
        drop(child);

        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(-process_group, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(
            unsafe { libc::kill(-process_group, 0) },
            0,
            "protocol-error cleanup left the external process group alive"
        );
    }

    #[test]
    fn oversized_external_protocol_line_is_rejected_without_unbounded_buffering() {
        let output_bytes = Arc::new(AtomicU64::new(0));
        let receiver = bounded_external_line_receiver(
            std::io::Cursor::new(vec![b'x'; 1_025]),
            1_024,
            Arc::clone(&output_bytes),
        );
        let error = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("bounded protocol reader result")
            .expect_err("oversized protocol line must fail");
        assert!(
            error.contains("protocol-line budget of 1024 bytes"),
            "{error}"
        );
        assert_eq!(output_bytes.load(Ordering::Relaxed), 1_025);
    }

    #[cfg(unix)]
    #[test]
    fn exited_external_child_preserves_real_status_without_inventing_a_resource_sample() {
        let config = Config::for_test().unwrap().external_execution_settings();
        let mut monitor = ExternalExecutionBudgetMonitor::new(
            ExternalExecutionBudget::from_config(&config), Arc::new(AtomicU64::new(0)));
        let mut child = std::process::Command::new("/bin/sh").args(["-c", "exit 23"]).spawn().unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(23));
        monitor.check(&mut child).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(23));
    }

    #[cfg(unix)]
    #[test]
    fn external_process_tree_exceeding_process_budget_is_detected_and_reaped() {
        let mut config = Config::for_test().unwrap().external_execution_settings();
        config.budget.max_processes = Some(1);
        let budget = ExternalExecutionBudget::from_config(&config);
        let output_bytes = Arc::new(AtomicU64::new(0));
        let mut monitor = ExternalExecutionBudgetMonitor::new(budget, output_bytes);
        let mut command = external_command(Path::new("/bin/sh"));
        let child = command
            .args(["-c", "sleep 30 & wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn budgeted external process tree");
        let mut child = ManagedExternalChild::new(child);
        let process_group = i32::try_from(child.id()).expect("external process group id");

        let deadline = Instant::now() + Duration::from_secs(2);
        let error = loop {
            match monitor.check(&mut child) {
                Ok(()) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(110));
                }
                Ok(()) => panic!("external process-count budget was not enforced"),
                Err(error) => break error,
            }
        };
        assert!(error.contains("process-count budget of 1"), "{error}");
        drop(child);
        assert_ne!(
            unsafe { libc::kill(-process_group, 0) },
            0,
            "budget failure left the external process group alive"
        );
    }

    #[cfg(unix)]
    #[test]
    fn real_stream_json_executor_budget_failure_propagates_and_reaps_process_tree() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "nola-external-budget-e2e-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create external budget fixture");
        let executable = directory.join("fake-claude");
        let pid_path = directory.join("process-group.pid");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\necho $$ > '{}'\nsleep 30 &\necho '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"budget-session\"}}'\nwait\n",
                pid_path.display()
            ),
        )
        .expect("write fake Claude executable");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fake Claude metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).expect("make fake Claude executable");

        let mut config = Config::for_test().unwrap().external_execution_settings();
        config.project_root = directory.clone();
        config.budget.max_processes = Some(1);
        let descriptor = ExecutionBackendDescriptor {
            schema_version: EXECUTION_BACKEND_SCHEMA_VERSION,
            id: "claude_code".to_string(),
            kind: ExecutionBackendKind::ClaudeCode,
            display_name: "Fake Claude Code".to_string(),
            availability: ExecutionBackendAvailability::Available,
            transport: ExecutionBackendTransport::StreamJsonProcess,
            executable_path: Some(executable.to_string_lossy().into_owned()),
            version: Some("test".to_string()),
            capabilities: ExecutionBackendCapabilities {
                streams_text: true,
                streams_reasoning: true,
                streams_tools: true,
                supports_resume: true,
                supports_cancel: true,
                supports_permission_bridge: false,
            },
            diagnostic: None,
            models: Vec::new(),
            default_model_id: None,
        };
        let mut observer = TestExternalObserver::default();
        let error = run_stream_json_process(
            &config,
            "budget test",
            true,
            &mut observer,
            None,
            &descriptor,
        )
        .expect_err("external process-count budget must fail the real protocol run");
        assert!(error.contains("process-count budget of 1"), "{error}");

        let process_group = std::fs::read_to_string(&pid_path)
            .expect("fake Claude process group")
            .trim()
            .parse::<i32>()
            .expect("numeric process group");
        assert_ne!(
            unsafe { libc::kill(-process_group, 0) },
            0,
            "real protocol budget failure left the process tree alive"
        );
        let _ = std::fs::remove_dir_all(directory);
    }
}
