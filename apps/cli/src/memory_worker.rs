// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Executes a Core-authored background memory plan through an LLM provider.
// - Boundary: Core owns prompt, bounded input, schema, materialization, and validation; this Host owns provider I/O and an atomic result inbox.

use std::fs;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nola_core::agent::MemoryWorkerPlan;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::execution_context::ExecutionContext;
use crate::llm::{stream_chat, LlmConfig};

const WORKER_ARGUMENT: &str = "--memory-worker-request";
const TRANSIENT_LLM_RETRY_DELAYS_MS: [u64; 3] = [250, 750, 2_000];
const RESULT_SUFFIX: &str = ".result.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemoryWorkerLlmConfig {
    #[serde(skip)]
    pub siwc: Option<crate::model_siwc::SiwcExecutionContext>,
    pub native_model_route: Option<nola_host_protocol::HostNativeModelRouteBinding>,
    pub provider: Option<String>,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub temperature: f64,
    pub top_p: f64,
    pub max_output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemoryWorkerRequest {
    pub plan: MemoryWorkerPlan,
    pub llm: MemoryWorkerLlmConfig,
    pub inbox_directory: PathBuf,
    /// Absolute wall-clock boundary inherited from the Core-owned worker
    /// lease. Detached workers may outlive their foreground launcher, but
    /// must never outlive the claim that authorizes their provider request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_epoch_ms: Option<i64>,
    /// Monotonic fallback for wall-clock adjustments after the request was
    /// authored. The effective budget is the smaller of this and deadline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemoryWorkerResult {
    pub job_id: String,
    pub ownership_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn run_if_requested() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some(WORKER_ARGUMENT) {
        return None;
    }
    let result = std::env::args()
        .nth(2)
        .ok_or_else(|| "memory worker request path is missing".to_string())
        .and_then(|path| read_request(Path::new(&path)))
        .and_then(run_request);
    Some(match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("memory worker: {error}");
            1
        }
    })
}

pub(crate) fn spawn(request: &MemoryWorkerRequest) -> Result<(), String> {
    fs::create_dir_all(&request.inbox_directory)
        .map_err(|error| format!("create memory worker inbox failed: {error}"))?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("locate memory worker executable failed: {error}"))?;
    let payload = serde_json::to_vec(request)
        .map_err(|error| format!("encode memory worker request failed: {error}"))?;
    let request_path = request.inbox_directory.join(format!(
        "{}.{}.request.json",
        request.plan.job_id, request.plan.ownership_token
    ));
    fs::write(&request_path, payload)
        .map_err(|error| format!("write memory worker request failed: {error}"))?;
    let error_log_path = request.inbox_directory.join(format!(
        "{}.{}.worker.log",
        request.plan.job_id, request.plan.ownership_token
    ));
    let error_log = fs::File::create(&error_log_path)
        .map_err(|error| format!("create memory worker log failed: {error}"))?;
    let mut command = Command::new(executable);
    command
        .arg(WORKER_ARGUMENT)
        .arg(&request_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(error_log));
    #[cfg(unix)]
    if should_detach_from_launcher(std::env::var_os("NOLA_MEMORY_WORKER_DETACH")) {
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("start memory worker failed: {error}"))?;
    // Dropping Child without waiting leaves a zombie while the foreground CLI
    // remains alive. A tiny reaper thread preserves detached product semantics
    // while collecting workers that finish before their launcher exits.
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub(crate) fn load_results(directory: &Path) -> Result<Vec<(PathBuf, MemoryWorkerResult)>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("read memory result inbox failed: {error}")),
    };
    let mut results = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !file_name.ends_with(RESULT_SUFFIX) {
            continue;
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("read memory worker result failed: {error}"))?;
        let result = serde_json::from_slice::<MemoryWorkerResult>(&bytes)
            .map_err(|error| format!("decode memory worker result failed: {error}"))?;
        results.push((path, result));
    }
    results.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(results)
}

fn read_request(path: &Path) -> Result<MemoryWorkerRequest, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("read memory worker request failed: {error}"))?;
    let _ = fs::remove_file(path);
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("decode memory worker request failed: {error}"))
}

fn run_request(request: MemoryWorkerRequest) -> Result<(), String> {
    let worker_result = execute_plan_before_deadline(&request);
    let result = match worker_result {
        Ok(output) => MemoryWorkerResult {
            job_id: request.plan.job_id.clone(),
            ownership_token: request.plan.ownership_token.clone(),
            output: Some(output),
            error: None,
        },
        Err(error) => MemoryWorkerResult {
            job_id: request.plan.job_id.clone(),
            ownership_token: request.plan.ownership_token.clone(),
            output: None,
            error: Some(error),
        },
    };
    write_result(&request.inbox_directory, &result)
}

fn execute_plan_before_deadline(request: &MemoryWorkerRequest) -> Result<Value, String> {
    let wall_budget = request.deadline_epoch_ms.and_then(remaining_until_epoch_ms);
    let monotonic_budget = request.execution_timeout_ms.map(Duration::from_millis);
    let remaining = match (wall_budget, monotonic_budget) {
        (Some(wall), Some(monotonic)) => wall.min(monotonic),
        (Some(wall), None) => wall,
        (None, Some(monotonic)) if request.deadline_epoch_ms.is_none() => monotonic,
        (None, None) if request.deadline_epoch_ms.is_none() => {
            return execute_plan_with_retry(request);
        }
        _ => return Err("memory worker deadline expired before execution".to_string()),
    };
    if remaining.is_zero() {
        return Err("memory worker deadline expired before execution".to_string());
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let owned_request = request.clone();
    thread::spawn(move || {
        let _ = sender.send(execute_plan_with_retry(&owned_request));
    });
    receiver
        .recv_timeout(remaining)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => {
                "memory worker exceeded its Core-owned lease deadline".to_string()
            }
            mpsc::RecvTimeoutError::Disconnected => {
                "memory worker execution ended without a result".to_string()
            }
        })?
}

fn remaining_until_epoch_ms(deadline_epoch_ms: i64) -> Option<Duration> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    let remaining_ms = deadline_epoch_ms.saturating_sub(now);
    (remaining_ms > 0).then(|| Duration::from_millis(remaining_ms as u64))
}

#[cfg(unix)]
fn should_detach_from_launcher(value: Option<std::ffi::OsString>) -> bool {
    !value.is_some_and(|value| {
        let value = value.to_string_lossy();
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        )
    })
}

fn execute_plan_with_retry(request: &MemoryWorkerRequest) -> Result<Value, String> {
    let mut attempt = 0;
    loop {
        match execute_plan(request) {
            Ok(output) => return Ok(output),
            Err(error)
                if is_transient_llm_transport_error(&error)
                    && attempt < TRANSIENT_LLM_RETRY_DELAYS_MS.len() =>
            {
                let delay_ms = TRANSIENT_LLM_RETRY_DELAYS_MS[attempt];
                eprintln!(
                    "memory worker: transient LLM transport failure; retrying in {delay_ms}ms: {error}"
                );
                attempt += 1;
                thread::sleep(Duration::from_millis(delay_ms));
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_transient_llm_transport_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    [
        "connection refused",
        "connection reset",
        "connection was rejected",
        "connection was closed",
        "error sending request",
        "tcp connect error",
        "timed out",
        "timeout",
        "temporarily unavailable",
        "broken pipe",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn execute_plan(request: &MemoryWorkerRequest) -> Result<Value, String> {
    if request.llm.provider.as_deref() == Some("siwc_responses") {
        return Err("SIWC memory-worker tool contract is not supported".into());
    }
    let first = execute_plan_attempt(request, None)?;
    match validate_memory_worker_output(&request.plan, &first) {
        Ok(()) => Ok(first),
        Err(first_error) => {
            let correction = format!(
                "Your previous submit_memory_worker_output call violated the required schema: {first_error}. Submit the tool exactly once again. The arguments must be one JSON object matching the supplied schema, not a JSON-encoded string and not a top-level array."
            );
            let repaired = execute_plan_attempt(request, Some(&correction))?;
            validate_memory_worker_output(&request.plan, &repaired).map_err(|second_error| {
                format!(
                    "memory worker returned invalid structured output after one repair attempt: {second_error}"
                )
            })?;
            Ok(repaired)
        }
    }
}

fn execute_plan_attempt(
    request: &MemoryWorkerRequest,
    correction: Option<&str>,
) -> Result<Value, String> {
    let llm = LlmConfig {
        siwc: request.llm.siwc.clone(), native_model_route: request.llm.native_model_route.clone(),
        compaction_output_budget: None,
        chat_output_token_field: Default::default(),
        provider: request.llm.provider.clone(),
        fallbacks: Vec::new(),
        endpoint: request.llm.endpoint.clone(),
        api_key: request.llm.api_key.clone(),
        model: request.llm.model.clone(),
        temperature: request.llm.temperature.min(0.3),
        top_p: request.llm.top_p,
        max_output_tokens: request.llm.max_output_tokens.min(6_144),
        max_retries: crate::llm::configured_llm_max_retries(),
        thinking_enabled: false, reasoning_summary_supported: None,
        reasoning_selection: None,
        sampling_override: false,
        generation_options: Default::default(),
        cancellation: ExecutionContext::default(),
        openai_prompt_cache: false,
        openai_prompt_cache_key: None,
        openai_prompt_cache_key_prefix: None,
        openai_prompt_cache_retention: None,
        openai_prompt_cache_history_breakpoint: false,
        openai_prompt_cache_history_breakpoint_endpoints: None,
        parallel_tool_calls: None,
    };
    let model_input = serde_json::to_string_pretty(&request.plan.model_input)
        .map_err(|error| format!("encode Core memory worker input failed: {error}"))?;
    let mut messages = vec![
        json!({"role": "system", "content": request.plan.system_instructions}),
        json!({"role": "user", "content": model_input}),
    ];
    if let Some(correction) = correction {
        messages.push(json!({"role": "system", "content": correction}));
    }
    let tools = vec![json!({
        "type": "function",
        "function": {
            "name": "submit_memory_worker_output",
            "description": "Submit semantic memory candidates for deterministic Core materialization and validation.",
            "parameters": request.plan.response_schema
        }
    })];
    let choice = stream_chat(&llm, &messages, &tools, "required", |_| {})?;
    let call = choice
        .tool_calls
        .iter()
        .find(|call| call.name == "submit_memory_worker_output")
        .ok_or_else(|| "memory worker did not submit the required structured output".to_string())?;
    decode_memory_worker_arguments(&call.arguments)
}

fn decode_memory_worker_arguments(arguments: &str) -> Result<Value, String> {
    let mut value = serde_json::from_str::<Value>(arguments)
        .map_err(|error| format!("decode semantic memory worker output failed: {error}"))?;
    for _ in 0..2 {
        let Value::String(encoded) = value else { break };
        value = serde_json::from_str::<Value>(&encoded).map_err(|error| {
            format!("decode nested semantic memory worker output failed: {error}")
        })?;
    }
    Ok(value)
}

fn validate_memory_worker_output(plan: &MemoryWorkerPlan, output: &Value) -> Result<(), String> {
    let object = output
        .as_object()
        .ok_or_else(|| "top-level value must be an object".to_string())?;
    let expected_type = match plan.kind {
        nola_core::agent::MemoryJobKind::Extraction => "extraction",
        nola_core::agent::MemoryJobKind::Consolidation => "consolidation",
    };
    if object.get("type").and_then(Value::as_str) != Some(expected_type) {
        return Err(format!("type must be {expected_type}"));
    }
    if !object
        .get("summary")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Err("summary must be a non-empty string".to_string());
    }
    if !object.get("candidates").is_some_and(Value::is_array) {
        return Err("candidates must be an array".to_string());
    }
    Ok(())
}

fn write_result(directory: &Path, result: &MemoryWorkerResult) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("create memory result inbox failed: {error}"))?;
    let destination = directory.join(format!(
        "{}.{}.result.json",
        result.job_id, result.ownership_token
    ));
    let temporary = directory.join(format!(
        ".{}.{}.{}.tmp",
        result.job_id,
        result.ownership_token,
        std::process::id()
    ));
    let bytes = serde_json::to_vec(result)
        .map_err(|error| format!("encode memory worker result failed: {error}"))?;
    fs::write(&temporary, bytes)
        .map_err(|error| format!("write memory worker result failed: {error}"))?;
    fs::rename(&temporary, &destination)
        .map_err(|error| format!("commit memory worker result failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        decode_memory_worker_arguments, is_transient_llm_transport_error, load_results,
        remaining_until_epoch_ms, write_result, MemoryWorkerResult,
    };
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn classifies_local_loopback_lifecycle_failures_as_transient() {
        assert!(is_transient_llm_transport_error(
            "LLM request failed: tcp connect error: Connection refused (os error 61)"
        ));
        assert!(is_transient_llm_transport_error(
            "error sending request: connection was closed by remote"
        ));
    }

    #[test]
    fn does_not_retry_authentication_or_protocol_failures() {
        assert!(!is_transient_llm_transport_error("401 Unauthorized"));
        assert!(!is_transient_llm_transport_error(
            "memory worker did not submit the required structured output"
        ));
    }

    #[test]
    fn unwraps_double_encoded_structured_tool_arguments() {
        let encoded = serde_json::to_string(
            &serde_json::json!({
                "type": "extraction",
                "summary": "No durable memory",
                "candidates": []
            })
            .to_string(),
        )
        .unwrap();
        let decoded = decode_memory_worker_arguments(&encoded).unwrap();
        assert_eq!(decoded["type"], "extraction");
        assert!(decoded["candidates"].as_array().unwrap().is_empty());
    }

    #[test]
    fn result_inbox_ignores_request_payloads_and_keeps_attempts_distinct() {
        let directory = std::env::temp_dir().join(format!(
            "nola-memory-worker-results-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("job.token.request.json"), b"not a result").unwrap();
        fs::write(directory.join("job.json"), b"not a current result filename").unwrap();
        for token in ["old-token", "new-token"] {
            write_result(
                &directory,
                &MemoryWorkerResult {
                    job_id: "job".to_string(),
                    ownership_token: token.to_string(),
                    output: None,
                    error: Some("failed".to_string()),
                },
            )
            .unwrap();
        }
        let results = load_results(&directory).unwrap();
        assert_eq!(results.len(), 2);
        assert_ne!(results[0].0, results[1].0);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn expired_worker_deadline_has_no_remaining_budget() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!(remaining_until_epoch_ms(now.saturating_sub(1)).is_none());
        assert!(remaining_until_epoch_ms(now.saturating_add(2_000)).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn detach_policy_defaults_on_and_accepts_explicit_false() {
        assert!(super::should_detach_from_launcher(None));
        assert!(!super::should_detach_from_launcher(Some("false".into())));
        assert!(!super::should_detach_from_launcher(Some("0".into())));
        assert!(super::should_detach_from_launcher(Some("true".into())));
    }
}
