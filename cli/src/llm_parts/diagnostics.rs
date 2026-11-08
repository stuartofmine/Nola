// SPDX-License-Identifier: Apache-2.0
//! Debug sinks and bounded diagnostic projections. Provider labels are inputs,
//! never resolved here, so adapters can report without depending on orchestration.

mod request_audit {
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{fs::OpenOptions, io::Write, path::Path};

    fn sensitive_field(key: &str) -> bool {
        matches!(key.to_ascii_lowercase().as_str(), "api_key" | "apikey" | "authorization" | "password" | "access_token" | "refresh_token" | "client_secret")
    }

    fn sanitized(request: &reqwest::blocking::Request, configured_secrets: &[&str]) -> Result<Value, String> {
        let bytes = request.body().and_then(|body| body.as_bytes()).ok_or("non-buffered request")?;
        let mut body: Value = serde_json::from_slice(bytes).map_err(|_| "non-JSON request")?;
        let mut secrets: Vec<String> = configured_secrets.iter().map(|value| (*value).to_owned()).collect();
        for name in ["authorization", "x-api-key", "api-key"] {
            if let Some(value) = request.headers().get(name).and_then(|value| value.to_str().ok()) {
                secrets.push(value.to_owned());
                if let Some((scheme, token)) = value.split_once(' ') {
                    secrets.push(token.to_owned());
                    if scheme.eq_ignore_ascii_case("basic") {
                        use base64::Engine;
                        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(token) {
                            if let Ok(credentials) = String::from_utf8(decoded) {
                                if let Some((_, password)) = credentials.split_once(':') {
                                    secrets.push(password.to_owned());
                                }
                                secrets.push(credentials);
                            }
                        }
                    }
                }
            }
        }
        // URL values may also carry authentication; never export the query or
        // userinfo, and redact their values if repeated in the JSON body.
        secrets.extend(request.url().query_pairs().map(|(_, value)| value.into_owned()));
        if let Some(password) = request.url().password() { secrets.push(password.into()); }
        fn collect_credentials(value: &Value, secrets: &mut Vec<String>) {
            match value {
                Value::Array(items) => for item in items { collect_credentials(item, secrets); },
                Value::Object(fields) => for (key, value) in fields {
                    if sensitive_field(key) {
                        if let Some(secret) = value.as_str() { secrets.push(secret.to_owned()); }
                    }
                    collect_credentials(value, secrets);
                },
                _ => (),
            }
        }
        collect_credentials(&body, &mut secrets);
        secrets.retain(|secret| !secret.is_empty());
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        fn scrub(value: &mut Value, secrets: &[String]) {
            match value {
                Value::String(text) => for secret in secrets { *text = text.replace(secret, "[REDACTED]"); },
                Value::Array(items) => for item in items { scrub(item, secrets); },
                Value::Object(fields) => for (key, value) in fields {
                    if sensitive_field(key) {
                        *value = json!("[REDACTED]");
                    } else { scrub(value, secrets); }
                },
                _ => (),
            }
        }
        scrub(&mut body, &secrets);
        let mut endpoint = request.url().clone();
        let _ = endpoint.set_username("");
        let _ = endpoint.set_password(None);
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        let sanitized_bytes = serde_json::to_vec(&body).map_err(|_| "JSON serialization failed")?;
        Ok(json!({"schema":"nola.model_request_audit.v1", "phase":"prepared_for_dispatch",
            "provider_receipt_proven":false, "method":request.method().as_str(), "endpoint":endpoint.as_str(),
            "body_sha256":format!("{:x}", Sha256::digest(bytes)),
            "redacted_body_sha256":format!("{:x}", Sha256::digest(sanitized_bytes)),
            "redaction":"configured_credentials_request_auth_url_values_and_sensitive_body_fields", "body":body}))
    }

    fn archive(root: &Path, request: &reqwest::blocking::Request, configured_secrets: &[&str]) -> Result<(), String> {
        let mut record = sanitized(request, configured_secrets)?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "invalid audit clock")?.as_millis();
        record["recorded_at_ms"] = json!(now);
        let id = uuid::Uuid::new_v4().to_string();
        record["dispatch_id"] = json!(id);
        std::fs::create_dir_all(root).map_err(|_| "audit directory unavailable")?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(root.join(format!("{now}-{id}.json")))
            .map_err(|_| "audit file unavailable")?;
        file.write_all(&serde_json::to_vec(&record).map_err(|_| "audit serialization failed")?)
            .map_err(|_| "audit write failed".to_string())
    }

    pub(in crate::llm) fn record(request: &reqwest::blocking::RequestBuilder, configured_secrets: &[&str]) {
        let Some(root) = std::env::var_os("NOLA_EVAL_REQUEST_AUDIT_DIR") else { return; };
        let outcome = request.try_clone().ok_or_else(|| "request is not cloneable".to_string())
            .and_then(|builder| builder.build().map_err(|_| "request cannot be built".to_string()))
            .and_then(|request| archive(Path::new(&root), &request, configured_secrets));
        if let Err(error) = outcome {
            // Never log the URL/body/credentials, including library error strings.
            eprintln!("nola_request_audit_unavailable: {error}");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn exact_body_is_archived_without_authentication_or_prompt_credential_echoes() {
            let root = tempfile::tempdir().unwrap();
            let request = reqwest::blocking::Client::new()
                .post("http://user:password-value@localhost/v1/responses?key=query-value")
                .bearer_auth("secret-fixture-token")
                .json(&json!({"model":"fixture","input":[{"role":"tool","content":"stdout: fail; secret-fixture-token query-value password-value another-secret"}],
                    "tools":[{"name":"read"}],"api_key":"another-secret"}))
                .build().unwrap();
            archive(root.path(), &request, &["secret-fixture-token"]).unwrap();
            let path = std::fs::read_dir(root.path()).unwrap().next().unwrap().unwrap().path();
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            }
            let text = std::fs::read_to_string(path).unwrap();
            for secret in ["secret-fixture-token", "query-value", "password-value", "another-secret"] {
                assert!(!text.contains(secret), "fixture credential was not redacted: {secret}");
            }
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["body"]["tools"][0]["name"], "read");
            assert!(value["body"]["input"][0]["content"].as_str().unwrap().contains("stdout: fail"));
            assert_eq!(value["provider_receipt_proven"], false);
            assert_eq!(value["endpoint"], "http://localhost/v1/responses");
        }
    }
}

pub(super) use request_audit::record as record_prepared_request;

use super::contracts::{LlmConfig, StreamedChoice};
use crate::execution_context::{current_execution_id, ExecutionContext};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn redacted_endpoint(endpoint: &str) -> String {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return "invalid-endpoint".into();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return "invalid-endpoint".into();
    }
    url.host_str().unwrap_or("invalid-endpoint").to_owned()
}

#[cfg(test)]
mod endpoint_redaction_tests {
    #[test]
    fn endpoint_diagnostics_never_copy_paths_query_fragment_or_user_info() {
        assert_eq!(
            super::redacted_endpoint("https://user:secret@example.invalid/v1?key=secret#secret"),
            "example.invalid"
        );
        assert_eq!(
            super::redacted_endpoint("not a URL secret"),
            "invalid-endpoint"
        );
        assert_eq!(
            super::redacted_endpoint("file:///private/secret"),
            "invalid-endpoint"
        );
    }
}

#[derive(Debug, Clone)]
struct LlmDebugLog {
    enabled: bool,
    path: Option<PathBuf>,
}

impl Default for LlmDebugLog {
    fn default() -> Self {
        Self {
            enabled: false,
            path: None,
        }
    }
}

static LLM_DEBUG_LOG: OnceLock<Mutex<LlmDebugLog>> = OnceLock::new();
static EXECUTION_DEBUG_LOGS: OnceLock<Mutex<HashMap<u64, Option<PathBuf>>>> = OnceLock::new();
static DEBUG_RUN_ID: OnceLock<String> = OnceLock::new();
static DEBUG_LOG_WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn configure_local_debug_log(enabled: bool, project_root: &Path) {
    let state = LLM_DEBUG_LOG.get_or_init(|| Mutex::new(LlmDebugLog::default()));
    let mut logger = state.lock().unwrap_or_else(|err| err.into_inner());
    logger.enabled = enabled;
    logger.path = if enabled {
        resolve_local_debug_log_path(project_root)
    } else {
        None
    };
}

pub struct ExecutionDebugLogGuard {
    execution_id: u64,
    previous: Option<Option<PathBuf>>,
}

impl Drop for ExecutionDebugLogGuard {
    fn drop(&mut self) {
        let logs = EXECUTION_DEBUG_LOGS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut logs = logs.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(previous) = self.previous.take() {
            logs.insert(self.execution_id, previous);
        } else {
            logs.remove(&self.execution_id);
        }
    }
}

pub fn configure_execution_debug_log(
    cancellation: &ExecutionContext,
    enabled: bool,
    project_root: &Path,
) -> ExecutionDebugLogGuard {
    let path = enabled
        .then(|| resolve_local_debug_log_path(project_root))
        .flatten();
    let logs = EXECUTION_DEBUG_LOGS.get_or_init(|| Mutex::new(HashMap::new()));
    let previous = logs
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(cancellation.execution_id(), path);
    ExecutionDebugLogGuard {
        execution_id: cancellation.execution_id(),
        previous,
    }
}

pub(super) fn log_llm_debug_event(
    cfg: &LlmConfig,
    provider: &str,
    phase: &str,
    endpoint: &str,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    detail: &str,
) {
    if !is_local_llm_debug_enabled() {
        return;
    }
    let payload = model_request_debug_payload(cfg, endpoint, messages, tools, tool_choice, detail);
    let line = format!(
        "nola_llm_debug ts={} run_id={} phase={} execution_id={} provider={} payload={}",
        local_debug_timestamp(),
        local_debug_run_id(),
        debug_log_field(phase, 80),
        current_execution_id()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".into()),
        debug_log_field(provider, 80),
        payload,
    );
    append_debug_log_line(&line);
}

// Project at the writer, not in a log viewer: truncation is not redaction.
// Model identifiers, tool names and provider error details can contain local
// paths or user-controlled text, so none are copied into the diagnostic record.
pub(super) fn model_request_debug_payload(
    cfg: &LlmConfig,
    endpoint: &str,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    detail: &str,
) -> Value {
    let message_bytes = json_len(messages);
    let tool_bytes = json_len(tools);
    json!({
        "model_identifier_bytes": cfg.model.len(),
        "endpoint_host": redacted_endpoint(endpoint),
        "tool_choice": match tool_choice { "auto" | "none" | "required" => tool_choice, _ => "other" },
        "tool_count": tools.len(),
        "message_count": messages.len(),
        "message_json_bytes": message_bytes,
        "tools_json_bytes": tool_bytes,
        "approx_prompt_tokens": message_bytes.saturating_add(tool_bytes).saturating_add(3) / 4,
        "detail_bytes": detail.len(),
    })
}

// Debug-only structural evidence: never request text, commands or headers.
pub(super) fn cache_request_projection(body: &Value, tail: Option<usize>) -> Value {
    use sha2::{Digest, Sha256};
    let digest = |v: &Value| format!("{:x}", Sha256::digest(v.to_string().as_bytes()));
    let items = body["input"].as_array().cloned().unwrap_or_default();
    let leading = items.iter().take_while(|m| matches!(m["role"].as_str(), Some("system" | "developer"))).count();
    let mut breakpoints = Vec::new();
    let messages = items.iter().enumerate().map(|(index,m)| {
        if m.get("prompt_cache_breakpoint").is_some() || ["content", "output"].iter().any(|key| m[*key].as_array().is_some_and(|blocks| blocks.iter().any(|b| b.get("prompt_cache_breakpoint").is_some()))) { breakpoints.push(index); }
        let content = m.get("content").unwrap_or(m);
        json!({"index":index,"role":m.get("role"),"item_type":m.get("type"),
            "section":if index < leading {"leading"} else if tail.is_some_and(|t| index >= t) {"runtime"} else {"history"},
            "content_sha256":digest(content),"content_bytes":content.to_string().len()})
    }).collect::<Vec<_>>();
    let permission = tail.and_then(|i| items.get(i)).and_then(|m| m["content"].as_str())
        .and_then(|text| text.split_once('\n')).and_then(|(_,data)| serde_json::from_str::<Value>(data).ok());
    json!({"type":"model_cache_request","provider":"responses","model":body["model"],
        "request_sha256":digest(body),"leading_sha256":digest(&json!(&items[..leading])),
        "tools_sha256":digest(&body["tools"]),"cache_mode":body.pointer("/prompt_cache_options/mode").and_then(Value::as_str).unwrap_or("implicit"),
        "history_boundary":tail,"breakpoint_items":breakpoints,"messages":messages,
        "permission_projection_bytes":permission.as_ref().map(|v| v.to_string().len()),
        "permission_receipts":permission.as_ref().and_then(|v| v.pointer("/facts/recent_permission_receipts")).and_then(Value::as_array).map(Vec::len)})
}

pub(super) fn log_cache_dispatch(body: &Value) {
    if !is_local_llm_debug_enabled() { return; }
    // Diagnostic classification only, never an authority or cache-hint decision.
    let tail = body["input"].as_array().and_then(|items| items.iter().position(|m| {
        if m["role"] != "user" { return false; }
        m["content"].as_str().and_then(|s|s.split_once('\n'))
            .and_then(|(_,s)|serde_json::from_str::<Value>(s).ok())
            .is_some_and(|v| v["schema"] == "nola.runtime_facts.v1" && v["kind"] == "current-permission-state")
    }));
    let mut value=cache_request_projection(body,tail);
    value["phase"]=json!("dispatch_attempt");
    log_local_debug_payload("model_cache_request",&value);
}

pub(super) fn log_cache_server_diagnostics(value: Option<&Value>) {
    if !is_local_llm_debug_enabled() { return; }
    use sha2::{Digest, Sha256};
    // Only documented enum values and unsigned counters may leave this boundary.
    let kind=value.and_then(|v|v["type"].as_str()).filter(|v|matches!(*v,"cache_hit"|"cache_miss"|"comparison_response_not_found"|"unavailable"));
    let reason=value.and_then(|v|v["reason"].as_str()).filter(|v|matches!(*v,"model_changed"|"prompt_cache_key_changed"|"service_tier_changed"|"tools_changed"|"text_format_changed"|"reasoning_effort_changed"|"verbosity_changed"|"context_compacted"|"input_changed"));
    log_local_debug_payload("model_cache_server", &json!({"present":value.is_some(),"kind":kind,"reason":reason,
        "comparison_reusable_tokens":value.and_then(|v|v["comparison_reusable_tokens"].as_u64()),
        "cache_missed_tokens":value.and_then(|v|v["cache_missed_tokens"].as_u64()),
        "diagnostics_sha256":value.map(|v| format!("{:x}",Sha256::digest(v.to_string().as_bytes()))),
        "diagnostics_bytes":value.map(|v|v.to_string().len())}));
}

fn is_local_llm_debug_enabled() -> bool {
    current_debug_log_path().is_some()
}

pub fn log_local_debug_payload(phase: &str, payload: &Value) {
    if !is_local_llm_debug_enabled() {
        return;
    }
    let enriched = enrich_debug_payload(payload.clone());
    let payload_json = serde_json::to_string(&enriched).unwrap_or_else(|_| "{}".to_string());
    let line = format!(
        "nola_agent_debug ts={} phase={} payload={}",
        local_debug_timestamp(),
        debug_log_field(phase, 80),
        payload_json
    );
    append_debug_log_line(&line);
}

fn enrich_debug_payload(mut payload: Value) -> Value {
    if let Some(object) = payload.as_object_mut() {
        object
            .entry("run_id".to_string())
            .or_insert_with(|| Value::String(local_debug_run_id().to_string()));
        if let Some(execution_id) = current_execution_id() {
            object
                .entry("execution_id".to_string())
                .or_insert_with(|| Value::from(execution_id));
        }
    }
    payload
}

pub(super) fn log_model_response_debug(cfg: &LlmConfig, choice: &StreamedChoice) {
    if !is_local_llm_debug_enabled() {
        return;
    }
    let payload = enrich_debug_payload(model_response_debug_payload(cfg, choice));
    let payload_json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    let line = format!(
        "nola_llm_debug ts={} phase=provider_response payload={}",
        local_debug_timestamp(),
        payload_json,
    );
    append_debug_log_line(&line);
}

pub(super) fn model_response_debug_payload(cfg: &LlmConfig, choice: &StreamedChoice) -> Value {
    let content_chars = choice.content.chars().count();
    let reasoning_chars = choice.reasoning.chars().count();
    let return_kind = if !choice.tool_calls.is_empty() {
        "tool_calls"
    } else if !choice.content.is_empty() {
        "text"
    } else if !choice.reasoning.is_empty() {
        "reasoning"
    } else {
        "empty"
    };
    let tool_calls = choice
        .tool_calls
        .iter()
        .take(32)
        .map(|call| {
            let arguments_chars = call.arguments.chars().count();
            json!({
                "arguments_chars": arguments_chars,
                "arguments_json_bytes": call.arguments.len(),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "type": "provider_response",
        "provider":cfg.provider,"model":cfg.model,
        "request_sha256":choice.request_snapshot.as_ref().map(|s| {
            use sha2::{Digest,Sha256};format!("{:x}",Sha256::digest(s.body.to_string().as_bytes()))
        }),
        "cache_usage":{"input_tokens":choice.usage.input_tokens,"cached_tokens":choice.usage.cached_input_tokens,
            "output_tokens":choice.usage.output_tokens,"cached_reported":choice.usage.cached_input_tokens.is_some(),
            "input_reported":choice.usage.input_tokens.is_some(),"output_reported":choice.usage.output_tokens.is_some(),
            "is_compaction":cfg.compaction_output_budget.is_some()},
        "scope": "provider",
        "model_identifier_bytes": cfg.model.len(),
        "endpoint_host": redacted_endpoint(&cfg.endpoint),
        "return_kind": return_kind,
        "finish_reason": match choice.finish_reason.as_str() {
            "stop" | "length" | "tool_calls" | "content_filter" | "end_turn" | "max_tokens" | "tool_use" => choice.finish_reason.as_str(),
            "" => "unspecified",
            _ => "other",
        },
        "content_chars": content_chars,
        "content_json_bytes": choice.content.len(),
        "reasoning_chars": reasoning_chars,
        "reasoning_json_bytes": choice.reasoning.len(),
        "tool_call_count": choice.tool_calls.len(),
        "tool_calls_truncated": choice.tool_calls.len() > 32,
        "tool_calls": tool_calls,
    })
}

fn append_debug_log_line(line: &str) {
    let Some(path) = current_debug_log_path() else {
        return;
    };

    let _write_guard = DEBUG_LOG_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    rotate_debug_log_if_needed(&path, line.len().saturating_add(1));
    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => file,
        Err(_) => return,
    };
    let _ = writeln!(file, "{}", line);
}

fn rotate_debug_log_if_needed(path: &Path, incoming_bytes: usize) {
    let max_bytes = std::env::var("NOLA_CLI_DEBUG_LOG_MAX_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(16 * 1024 * 1024)
        .clamp(1024 * 1024, 1024 * 1024 * 1024);
    let current_bytes = std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if current_bytes.saturating_add(incoming_bytes as u64) <= max_bytes {
        return;
    }
    let backups = std::env::var("NOLA_CLI_DEBUG_LOG_BACKUPS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(3)
        .clamp(1, 10);
    rotate_debug_log(path, backups);
}

fn rotate_debug_log(path: &Path, backups: usize) {
    for index in (1..=backups).rev() {
        let source = if index == 1 {
            path.to_path_buf()
        } else {
            debug_log_backup_path(path, index - 1)
        };
        let destination = debug_log_backup_path(path, index);
        if !source.exists() {
            continue;
        }
        let _ = std::fs::remove_file(&destination);
        let _ = std::fs::rename(source, destination);
    }
}

fn debug_log_backup_path(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

#[cfg(test)]
mod debug_log_rotation_regression_tests {
    use super::{debug_log_backup_path, rotate_debug_log};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn debug_log_rotation_keeps_newest_backups_in_order() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nola-debug-log-rotation-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create debug-log test directory");
        let log_path = root.join("cli-debug.log");

        fs::write(&log_path, "oldest\n").expect("write oldest log");
        rotate_debug_log(&log_path, 2);
        assert_eq!(
            fs::read_to_string(debug_log_backup_path(&log_path, 1)).expect("read first backup"),
            "oldest\n"
        );

        fs::write(&log_path, "newer\n").expect("write newer log");
        rotate_debug_log(&log_path, 2);
        assert_eq!(
            fs::read_to_string(debug_log_backup_path(&log_path, 1)).expect("read newest backup"),
            "newer\n"
        );
        assert_eq!(
            fs::read_to_string(debug_log_backup_path(&log_path, 2)).expect("read older backup"),
            "oldest\n"
        );

        fs::write(&log_path, "newest\n").expect("write newest log");
        rotate_debug_log(&log_path, 2);
        assert_eq!(
            fs::read_to_string(debug_log_backup_path(&log_path, 1))
                .expect("read newest retained backup"),
            "newest\n"
        );
        assert_eq!(
            fs::read_to_string(debug_log_backup_path(&log_path, 2))
                .expect("read second retained backup"),
            "newer\n"
        );

        let _ = fs::remove_dir_all(root);
    }
}

fn current_debug_log_path() -> Option<PathBuf> {
    if let Some(execution_id) = current_execution_id() {
        let logs = EXECUTION_DEBUG_LOGS.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(path) = logs
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(&execution_id)
            .cloned()
        {
            return path;
        }
    }
    let state = LLM_DEBUG_LOG
        .get_or_init(|| Mutex::new(LlmDebugLog::default()))
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    state.enabled.then(|| state.path.clone()).flatten()
}

fn local_debug_timestamp() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(now) => format_local_debug_timestamp(now.as_secs(), now.subsec_millis()),
        Err(_) => "0.000".to_string(),
    }
}

fn local_debug_run_id() -> &'static str {
    DEBUG_RUN_ID.get_or_init(|| {
        let (seconds, milliseconds) = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|now| (now.as_secs(), now.subsec_millis()))
            .unwrap_or((0, 0));
        format!("{}-{seconds}-{milliseconds:03}", std::process::id())
    })
}

pub(super) fn format_local_debug_timestamp(seconds: u64, milliseconds: u32) -> String {
    format!("{seconds}.{milliseconds:03}")
}

fn resolve_local_debug_log_path(project_root: &Path) -> Option<PathBuf> {
    if let Ok(raw) = env::var("NOLA_CLI_DEBUG_LOG_PATH") {
        let expanded = expand_path_prefix(&raw);
        if let Some(path) = ensure_log_path_dir(expanded) {
            return Some(path);
        }
    }

    if let Some(home) = crate::platform_paths::home_dir() {
        let home = home.as_path();
        let cache_candidates = [
            home.join(".cache").join("nola").join("cli-debug.log"),
            home.join(".nola").join("cli-debug.log"),
            home.join(".nola").join("logs").join("cli-debug.log"),
        ];
        for candidate in cache_candidates {
            if let Some(path) = ensure_log_path_dir(candidate) {
                return Some(path);
            }
        }
    }

    let project_default = project_root.join(".nola").join("cli-debug.log");
    ensure_log_path_dir(project_default)
}

fn ensure_log_path_dir(path: PathBuf) -> Option<PathBuf> {
    let parent = path.parent()?;
    if create_dir_all(parent).is_ok() {
        return Some(path);
    }
    None
}

fn expand_path_prefix(raw: &str) -> PathBuf {
    let text = raw.trim();
    if text == "~" {
        return crate::platform_paths::home_dir().unwrap_or_else(|| PathBuf::from("."));
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = crate::platform_paths::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(text)
}

pub(super) fn trim_debug_detail(text: &str) -> String {
    let trimmed = text.trim();
    let preview = trimmed.chars().take(1_000).collect::<String>();
    if trimmed.chars().count() > 1_000 {
        format!("{preview}...")
    } else {
        preview
    }
}

fn debug_log_field(text: &str, max_chars: usize) -> String {
    let truncated = text.chars().take(max_chars).collect::<String>();
    let mut out = String::with_capacity(truncated.len());
    for ch in truncated.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push(' '),
            ch => out.push(ch),
        }
    }
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

pub(super) fn debug_context_metrics(
    provider: &str,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    body: &Value,
) {
    if !env_flag("NOLA_LLM_DEBUG_CONTEXT") {
        return;
    }
    let message_bytes = json_len(messages);
    let tool_bytes = json_len(tools);
    let body_bytes = json_len(body);
    let system_content_chars = messages
        .iter()
        .filter(|message| {
            message
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| role == "system")
        })
        .filter_map(|message| message.get("content"))
        .map(json_text_len)
        .sum::<usize>();
    let user_content_chars = messages
        .iter()
        .filter(|message| {
            message
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| role == "user")
        })
        .filter_map(|message| message.get("content"))
        .map(json_text_len)
        .sum::<usize>();
    let assistant_content_chars = messages
        .iter()
        .filter(|message| {
            message
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| role == "assistant")
        })
        .filter_map(|message| message.get("content"))
        .map(json_text_len)
        .sum::<usize>();
    let tool_content_chars = messages
        .iter()
        .filter(|message| {
            message
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| role == "tool")
        })
        .filter_map(|message| message.get("content"))
        .map(json_text_len)
        .sum::<usize>();
    let nola_md_chars = messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .filter(|text| text.contains("NOLA.md") || text.contains("Project Wiki"))
        .map(|text| text.chars().count())
        .sum::<usize>();
    let message_content_chars = messages
        .iter()
        .filter_map(|message| message.get("content"))
        .map(json_text_len)
        .sum::<usize>();
    let tool_names = tools
        .iter()
        .filter_map(|tool| {
            tool.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();
    eprintln!(
        "nola_llm_context provider={provider} tool_choice={tool_choice} messages={} tools={} system_chars={} user_chars={} assistant_chars={} tool_result_chars={} nola_md_chars={} message_content_chars={} message_json_bytes={} tools_json_bytes={} body_json_bytes={} approx_prompt_tokens_by_bytes={} approx_messages_tokens={} approx_tools_tokens={} approx_body_tokens={} tool_names={}",
        messages.len(),
        tools.len(),
        system_content_chars,
        user_content_chars,
        assistant_content_chars,
        tool_content_chars,
        nola_md_chars,
        message_content_chars,
        message_bytes,
        tool_bytes,
        body_bytes,
        (message_bytes + tool_bytes + 3) / 4,
        (message_bytes + 3) / 4,
        (tool_bytes + 3) / 4,
        (body_bytes + 3) / 4,
        tool_names.join(",")
    );
}

pub(super) fn debug_response_usage(parsed: &Value) {
    if !env_flag("NOLA_LLM_DEBUG_CONTEXT") {
        return;
    }
    if let Some(usage) = parsed.get("usage") {
        eprintln!("nola_llm_response_usage {usage}");
    }
}

fn json_len<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn json_text_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        Value::Array(items) => items.iter().map(json_text_len).sum(),
        Value::Object(map) => map.values().map(json_text_len).sum(),
        _ => 0,
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod cache_projection_tests {
    use super::*;
    #[test]
    fn cache_diagnostics_are_hashes_and_sizes_without_sensitive_body_text() {
        let facts=json!({"facts":{"recent_permission_receipts":[{"resource":"secret command"}]}});
        let body=json!({"model":"fixture","input":[{"role":"user","content":"private input sk-private-key"},
            {"role":"user","content":format!("Current Core runtime facts\n{facts}")}],"tools":[]});
        let value=cache_request_projection(&body,Some(1));let encoded=value.to_string();
        for secret in ["private input","sk-private-key","secret command"] {assert!(!encoded.contains(secret));}
        assert_eq!(value["permission_receipts"],1);assert_eq!(value["messages"][1]["section"],"runtime");
        assert_eq!(value["messages"][0]["content_sha256"].as_str().unwrap().len(),64);
    }
}
