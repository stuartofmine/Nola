// SPDX-License-Identifier: MIT
// Polls authenticated messaging-channel work and executes it through nola-agent-host.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CONFIG_SCHEMA_VERSION: u32 = 1;
const PROTOCOL_SCHEMA_VERSION: u32 = 1;
const DEFAULT_POLL_SECONDS: u64 = 2;
const DEFAULT_TURN_TIMEOUT_SECONDS: u64 = 900;
const MESSAGE_LEASE_SECONDS: u64 = 180;
const MESSAGE_LEASE_RENEW_INTERVAL_SECONDS: u64 = 60;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MessagingChannelWorkerConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub enabled: bool,
    pub backend_url: String,
    pub access_token: String,
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
    #[serde(default = "default_turn_timeout_seconds")]
    pub turn_timeout_seconds: u64,
    pub execution_backend: String,
    pub project_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
struct ClaimedMessage {
    schema_version: u32,
    message_id: u64,
    lease_id: String,
    envelope: ChannelEnvelope,
}

#[derive(Clone, Debug, Deserialize)]
struct ChannelEnvelope {
    schema_version: u32,
    connection_id: String,
    external_conversation_id: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaimResponse {
    message: Option<ClaimedMessage>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ChannelSessionState {
    #[serde(default)]
    external_sessions: BTreeMap<String, String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct HostTurnResult {
    assistant_text: String,
    external_session_id: Option<String>,
    permission_requested: bool,
    completion_pending: bool,
    run_result: Option<nola_domain::run_result::RunResult>,
}

pub struct MessagingChannelWorkerHandle {
    cancelled: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for MessagingChannelWorkerHandle {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start_if_configured() -> Result<Option<MessagingChannelWorkerHandle>, String> {
    let path = default_config_path()?;
    if path.is_file() {
        validate_config(&load_config(&path)?)?;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let thread = thread::Builder::new()
        .name("nola-messaging-channel-worker".to_string())
        .spawn(move || run_loop(path, worker_cancelled))
        .map_err(|error| format!("failed to start messaging channel worker: {error}"))?;
    Ok(Some(MessagingChannelWorkerHandle {
        cancelled,
        thread: Some(thread),
    }))
}

pub fn config_projection() -> Result<Value, String> {
    let path = default_config_path()?;
    if !path.is_file() {
        return Ok(json!({
            "schema_version": CONFIG_SCHEMA_VERSION,
            "enabled": false,
            "backend_url": "",
            "has_access_token": false,
            "poll_seconds": DEFAULT_POLL_SECONDS,
            "turn_timeout_seconds": DEFAULT_TURN_TIMEOUT_SECONDS,
            "execution_backend": "nola",
            "project_root": Value::Null,
        }));
    }
    let config = load_config(&path)?;
    validate_config(&config)?;
    Ok(config_api_projection(&config))
}

pub fn update_config(input: &Value) -> Result<Value, String> {
    let object = input
        .as_object()
        .ok_or("messaging channel worker settings must be an object")?;
    let path = default_config_path()?;
    let existing = if path.is_file() {
        Some(load_config(&path)?)
    } else {
        None
    };
    let replacement = object
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let clear = match object.get("clear_access_token") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        _ => return Err("clear_access_token must be a boolean".into()),
    };
    if clear && replacement.is_some() {
        return Err("cannot replace and clear the access token together".into());
    }
    let backend_url = object
        .get("backend_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_string)
        .or_else(|| existing.as_ref().map(|value| value.backend_url.clone()))
        .unwrap_or_default();
    let identity = backend_identity(&backend_url)?;
    if let Some(previous) = existing.as_ref() {
        if backend_identity(&previous.backend_url)? != identity
            && !previous.access_token.is_empty()
            && replacement.is_none()
            && !clear
        {
            return Err(
                "changing the backend URL requires a new access token or explicit clearing".into(),
            );
        }
    }
    let access_token = if clear {
        String::new()
    } else {
        replacement
            .or_else(|| existing.as_ref().map(|value| value.access_token.clone()))
            .unwrap_or_default()
    };
    let config = MessagingChannelWorkerConfig {
        schema_version: CONFIG_SCHEMA_VERSION,
        enabled: object
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| existing.as_ref().is_some_and(|value| value.enabled)),
        backend_url,
        access_token,
        poll_seconds: object
            .get("poll_seconds")
            .and_then(Value::as_u64)
            .or_else(|| existing.as_ref().map(|value| value.poll_seconds))
            .unwrap_or(DEFAULT_POLL_SECONDS)
            .clamp(1, 300),
        turn_timeout_seconds: object
            .get("turn_timeout_seconds")
            .and_then(Value::as_u64)
            .or_else(|| existing.as_ref().map(|value| value.turn_timeout_seconds))
            .unwrap_or(DEFAULT_TURN_TIMEOUT_SECONDS)
            .clamp(30, 14_400),
        execution_backend: object
            .get("execution_backend")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                existing
                    .as_ref()
                    .map(|value| value.execution_backend.clone())
            })
            .ok_or("execution_backend is required")?,
        project_root: match object.get("project_root") {
            Some(Value::String(value)) if !value.trim().is_empty() => {
                Some(PathBuf::from(value.trim()))
            }
            Some(Value::Null) => None,
            _ => existing.and_then(|value| value.project_root),
        },
    };
    validate_config(&config)?;
    save_config(&path, &config)?;
    Ok(config_api_projection(&config))
}

pub fn list_connections() -> Result<Value, String> {
    proxy_connections_request(reqwest::Method::GET, None)
}

pub fn save_connection(input: &Value) -> Result<Value, String> {
    if !input.is_object() {
        return Err("messaging channel connection must be an object".to_string());
    }
    proxy_connections_request(reqwest::Method::PUT, Some(input))
}

fn proxy_connections_request(
    method: reqwest::Method,
    input: Option<&Value>,
) -> Result<Value, String> {
    let config = load_config(&default_config_path()?)?;
    validate_management_config(&config)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("failed to create messaging channel client: {error}"))?;
    let mut request = client
        .request(method, endpoint(&config, "/v1/channels"))
        .bearer_auth(&config.access_token);
    if let Some(input) = input {
        request = request.json(input);
    }
    let response = request
        .send()
        .map_err(|_| "messaging channel request failed".to_string())?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "messaging channel backend returned HTTP {}",
            status.as_u16()
        ));
    }
    serde_json::from_str(&body).map_err(|_| "invalid messaging channel response".to_string())
}

/// Bind tokens to the full normalized URL, including path/query, not merely origin.
/// URL parsing normalizes host case/default ports; terminal slashes match endpoint().
fn backend_identity(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Ok(String::new());
    }
    let url = reqwest::Url::parse(value.trim()).map_err(|_| "invalid messaging backend URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "messaging backend URL must use HTTP(S) without credentials or a fragment".into(),
        );
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn config_api_projection(config: &MessagingChannelWorkerConfig) -> Value {
    json!({
        "schema_version": config.schema_version,
        "enabled": config.enabled,
        "backend_url": config.backend_url,
        "has_access_token": !config.access_token.trim().is_empty(),
        "poll_seconds": config.poll_seconds,
        "turn_timeout_seconds": config.turn_timeout_seconds,
        "execution_backend": config.execution_backend,
        "project_root": config.project_root,
    })
}

fn run_loop(config_path: PathBuf, cancelled: Arc<AtomicBool>) {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            eprintln!("messaging channel worker could not create HTTP client: {error}");
            return;
        }
    };
    let state_path = default_session_state_path().ok();
    let mut sessions = state_path
        .as_deref()
        .and_then(|path| load_session_state(path).ok())
        .unwrap_or_default();

    while !cancelled.load(Ordering::Relaxed) {
        let config = match load_config(&config_path).and_then(|config| {
            validate_config(&config)?;
            Ok(config)
        }) {
            Ok(config) if config.enabled => config,
            Ok(_) => {
                sleep_interruptibly(&cancelled, DEFAULT_POLL_SECONDS);
                continue;
            }
            Err(_error) if !config_path.is_file() => {
                sleep_interruptibly(&cancelled, DEFAULT_POLL_SECONDS);
                continue;
            }
            Err(error) => {
                eprintln!("messaging channel worker configuration is invalid: {error}");
                sleep_interruptibly(&cancelled, 5);
                continue;
            }
        };
        match claim_message(&client, &config) {
            Ok(Some(message)) => {
                let key = channel_conversation_key(&message.envelope);
                let nola_conversation_id = stable_nola_conversation_id(&message.envelope);
                let previous_external_session = sessions.external_sessions.get(&key).cloned();
                match execute_host_turn(
                    &client,
                    &config,
                    &message,
                    &nola_conversation_id,
                    previous_external_session.as_deref(),
                ) {
                    Ok(result) if result.permission_requested => {
                        let _ = fail_message(
                            &client,
                            &config,
                            &message,
                            "interactive permission is required; open Nola to continue",
                            false,
                        );
                    }
                    Ok(result) if result.completion_pending => {
                        let _ = fail_message(&client, &config, &message,
                            "the original Nola Run has no committed terminal outcome; open Nola to reconcile it", false);
                    }
                    Ok(result) if !result.assistant_text.trim().is_empty() => {
                        match complete_message(
                            &client,
                            &config,
                            &message,
                            &nola_conversation_id,
                            result.assistant_text.trim(),
                        ) {
                            Ok(()) => {
                                if let Some(external_session_id) = result.external_session_id {
                                    sessions.external_sessions.insert(key, external_session_id);
                                    if let Some(path) = state_path.as_deref() {
                                        let _ = save_session_state(path, &sessions);
                                    }
                                }
                            }
                            Err(error) => eprintln!("messaging completion failed: {error}"),
                        }
                    }
                    Ok(result) => {
                        let _ = fail_message(
                            &client,
                            &config,
                            &message,
                            "agent execution ended without a deliverable assistant response",
                            result.run_result.is_none(),
                        );
                    }
                    Err(error) => {
                        let _ = fail_message(&client, &config, &message, &error, true);
                    }
                }
            }
            Ok(None) => sleep_interruptibly(&cancelled, config.poll_seconds.max(1)),
            Err(error) => {
                eprintln!("messaging claim failed: {error}");
                sleep_interruptibly(&cancelled, config.poll_seconds.max(1).saturating_mul(2));
            }
        }
    }
}

fn claim_message(
    client: &reqwest::blocking::Client,
    config: &MessagingChannelWorkerConfig,
) -> Result<Option<ClaimedMessage>, String> {
    let response = client
        .post(endpoint(config, "/v1/channels/inbox/claim"))
        .bearer_auth(&config.access_token)
        .json(&json!({"schema_version": PROTOCOL_SCHEMA_VERSION, "lease_seconds": MESSAGE_LEASE_SECONDS}))
        .send()
        .map_err(|error| format!("claim request failed: {error}"))?;
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("claim returned {status}: {}", bounded(&body, 1000)));
    }
    let body: ClaimResponse = response
        .json()
        .map_err(|error| format!("invalid claim response: {error}"))?;
    if let Some(message) = body.message.as_ref() {
        validate_claim(message)?;
    }
    Ok(body.message)
}

fn complete_message(
    client: &reqwest::blocking::Client,
    config: &MessagingChannelWorkerConfig,
    message: &ClaimedMessage,
    nola_conversation_id: &str,
    reply_text: &str,
) -> Result<(), String> {
    post_lease_action(
        client,
        config,
        message,
        "complete",
        json!({
            "schema_version": PROTOCOL_SCHEMA_VERSION,
            "lease_id": message.lease_id,
            "nola_conversation_id": nola_conversation_id,
            "reply_text": reply_text,
        }),
    )
}

fn fail_message(
    client: &reqwest::blocking::Client,
    config: &MessagingChannelWorkerConfig,
    message: &ClaimedMessage,
    error: &str,
    retryable: bool,
) -> Result<(), String> {
    post_lease_action(
        client,
        config,
        message,
        "fail",
        json!({
            "schema_version": PROTOCOL_SCHEMA_VERSION,
            "lease_id": message.lease_id,
            "error": bounded(error, 2000),
            "retryable": retryable,
        }),
    )
}

fn renew_message(
    client: &reqwest::blocking::Client,
    config: &MessagingChannelWorkerConfig,
    message: &ClaimedMessage,
) -> Result<(), String> {
    post_lease_action(
        client,
        config,
        message,
        "renew",
        json!({
            "schema_version": PROTOCOL_SCHEMA_VERSION,
            "lease_id": message.lease_id,
            "lease_seconds": MESSAGE_LEASE_SECONDS,
        }),
    )
}

fn report_progress(
    client: &reqwest::blocking::Client,
    config: &MessagingChannelWorkerConfig,
    message: &ClaimedMessage,
    text: &str,
) -> Result<(), String> {
    post_lease_action(
        client,
        config,
        message,
        "progress",
        json!({
            "schema_version": PROTOCOL_SCHEMA_VERSION,
            "lease_id": message.lease_id,
            "text": text.chars().take(4096).collect::<String>(),
        }),
    )
}

fn post_lease_action(
    client: &reqwest::blocking::Client,
    config: &MessagingChannelWorkerConfig,
    message: &ClaimedMessage,
    action: &str,
    body: Value,
) -> Result<(), String> {
    let path = format!("/v1/channels/inbox/{}/{action}", message.message_id);
    let response = client
        .post(endpoint(config, &path))
        .bearer_auth(&config.access_token)
        .json(&body)
        .send()
        .map_err(|error| format!("{action} request failed: {error}"))?;
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let body = response.text().unwrap_or_default();
    Err(format!(
        "{action} returned {status}: {}",
        bounded(&body, 1000)
    ))
}

fn execute_host_turn(
    client: &reqwest::blocking::Client,
    config: &MessagingChannelWorkerConfig,
    message: &ClaimedMessage,
    nola_conversation_id: &str,
    external_session_id: Option<&str>,
) -> Result<HostTurnResult, String> {
    let prompt = message
        .envelope
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "channel message does not contain supported text".to_string())?;
    let executable = agent_host_executable()?;
    let project_root = config
        .project_root
        .clone()
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?);
    let submission_id = uuid::Uuid::new_v4().to_string();
    let assistant_message_id = format!("{submission_id}:assistant");
    let mut source = crate::source_journal::SourceRunJournal::new(
        nola_conversation_id.to_owned(),
        submission_id.clone(),
        assistant_message_id.clone(),
    );
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let mut child = Command::new(executable)
        .arg("run")
        .arg("--host-request-stdin")
        .arg("--json")
        .arg("--session")
        .arg(nola_conversation_id)
        .arg("--project")
        .arg(&project_root)
        .env("NOLA_DESKTOP_HOST", "1")
        .env("NOLA_CLIENT_SURFACE", "messaging")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to launch nola-agent-host: {error}"))?;

    let request = json!({
        "schema_version": PROTOCOL_SCHEMA_VERSION,
        "session_id": nola_conversation_id,
        "prompt": prompt,
        "execution_backend": config.execution_backend,
        "external_session_id": external_session_id,
        "client_surface": "messaging",
        "submission_id": submission_id,
        "user_message_id": submission_id,
        "assistant_message_id": assistant_message_id,
    });
    let mut stdin = child
        .stdin
        .take()
        .ok_or("agent host stdin is unavailable")?;
    serde_json::to_writer(&mut stdin, &request)
        .map_err(|error| format!("failed to encode host request: {error}"))?;
    stdin
        .write_all(b"\n")
        .map_err(|error| format!("failed to write host request: {error}"))?;
    drop(stdin);

    let stdout = child
        .stdout
        .take()
        .ok_or("agent host stdout is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("agent host stderr is unavailable")?;
    let (line_tx, line_rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let _ = line_tx.send(line);
        }
    });
    let stderr_thread = thread::spawn(move || {
        let mut text = String::new();
        let _ = BufReader::new(stderr)
            .take(64 * 1024)
            .read_to_string(&mut text);
        text
    });

    let deadline = Instant::now() + Duration::from_secs(config.turn_timeout_seconds.max(30));
    let mut next_lease_renewal =
        Instant::now() + Duration::from_secs(MESSAGE_LEASE_RENEW_INTERVAL_SECONDS);
    let mut next_progress = Instant::now();
    let mut last_progress = String::new();
    let mut result = HostTurnResult::default();
    // Telegram renders an empty draft as its native Thinking placeholder.
    // Other channels acknowledge this as unsupported without producing noise.
    let _ = report_progress(client, config, message, "");
    loop {
        while let Ok(line) = line_rx.try_recv() {
            let line = line.map_err(|error| format!("agent host output failed: {error}"))?;
            if let Ok(event) = serde_json::from_str::<Value>(&line) {
                if let Err(error) =
                    apply_source_host_event(&mut result, &event, &mut source, &mut application)
                {
                    result.completion_pending = true;
                    eprintln!(
                        "channel source journal read is unresolved: {}",
                        bounded(&error, 1000)
                    );
                }
            }
        }
        if result.permission_requested {
            let _ = child.kill();
        }
        if Instant::now() >= next_lease_renewal {
            if let Err(error) = renew_message(client, config, message) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_thread.join();
                return Err(format!("message lease renewal failed: {error}"));
            }
            next_lease_renewal =
                Instant::now() + Duration::from_secs(MESSAGE_LEASE_RENEW_INTERVAL_SECONDS);
        }
        if Instant::now() >= next_progress && result.assistant_text != last_progress {
            if report_progress(client, config, message, &result.assistant_text).is_ok() {
                last_progress.clone_from(&result.assistant_text);
            }
            next_progress = Instant::now() + Duration::from_millis(750);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed waiting for agent host: {error}"))?
        {
            while let Ok(line) = line_rx.try_recv() {
                let line = line.map_err(|error| format!("agent host output failed: {error}"))?;
                if let Ok(event) = serde_json::from_str::<Value>(&line) {
                    if let Err(error) =
                        apply_source_host_event(&mut result, &event, &mut source, &mut application)
                    {
                        result.completion_pending = true;
                        eprintln!(
                            "channel source journal read is unresolved: {}",
                            bounded(&error, 1000)
                        );
                    }
                }
            }
            let stderr = stderr_thread.join().unwrap_or_default();
            if result.permission_requested {
                return Ok(result);
            }
            if source.catch_up(&mut application).is_err() || source.seal.is_none() {
                // EOF/process exit does not prove failure or permit executing
                // this inbox message again with a replacement Run identity.
                result.completion_pending = true;
                return Ok(result);
            }
            // The admitted seal is authoritative even if the transport process
            // subsequently exits unsuccessfully. Re-running this message could
            // duplicate already committed external effects.
            let _ = (status, stderr);
            result.completion_pending = false;
            result.assistant_text = source.public_text();
            result.run_result = source.seal.as_ref().map(|seal| seal.result.clone());
            // Completing an inbox message acknowledges reply delivery, not task
            // acceptance. Incomplete/blocked/failed/cancelled replies retain
            // their original result in the canonical Run and must not retry it.
            return Ok(result);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "agent host exceeded the {} second channel turn timeout",
                config.turn_timeout_seconds.max(30)
            ));
        }
        thread::sleep(Duration::from_millis(40));
    }
}

fn apply_host_event(result: &mut HostTurnResult, value: &Value) {
    if value.get("type").and_then(Value::as_str) == Some("core_permission_projection_updated") {
        result.permission_requested = value
            .pointer("/projection/pending")
            .and_then(Value::as_array)
            .is_some_and(|pending| !pending.is_empty());
        return;
    }
    if let Some(trace) = value.get("trace") {
        let kind = trace
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload = trace.get("payload").unwrap_or(&Value::Null);
        if let Some(session_id) = payload
            .get("external_session_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            result.external_session_id = Some(session_id.to_string());
        }
        match kind {
            "tool.permission.requested" | "permission.requested" => {
                result.permission_requested = true
            }
            _ => {}
        }
    }
}

fn apply_source_host_event(
    result: &mut HostTurnResult,
    value: &Value,
    source: &mut crate::source_journal::SourceRunJournal,
    application: &mut crate::application_port::CliApplicationPort,
) -> Result<(), String> {
    apply_host_event(result, value);
    let kind = value.get("type").and_then(Value::as_str);
    if kind == Some("assistant_journal_committed_v1") {
        let through: nola_domain::assistant_journal::AssistantJournalCursor =
            serde_json::from_value(
                value
                    .get("through")
                    .cloned()
                    .ok_or("Missing journal notification cursor")?,
            )
            .map_err(|error| error.to_string())?;
        if through.identity.conversation_id != source.conversation_id
            || through.identity.run_id != source.run_id
        {
            return Ok(());
        }
    } else if kind == Some("assistant_run_terminal_committed_v1") {
        if value.get("session_id").and_then(Value::as_str) != Some(source.conversation_id.as_str())
            || value.get("run_id").and_then(Value::as_str) != Some(source.run_id.as_str())
        {
            return Ok(());
        }
    } else {
        return Ok(());
    }
    source.catch_up(application)?;
    result.assistant_text = source.public_text();
    Ok(())
}

fn validate_config(config: &MessagingChannelWorkerConfig) -> Result<(), String> {
    if config.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(format!(
            "unsupported messaging channel worker config schema {}",
            config.schema_version
        ));
    }
    if config.execution_backend.trim().is_empty() {
        return Err("messaging execution backend is empty".to_string());
    }
    if config.enabled {
        let url = reqwest::Url::parse(config.backend_url.trim())
            .map_err(|error| format!("invalid messaging backend URL: {error}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("messaging backend URL must use http or https".to_string());
        }
        if config.access_token.trim().is_empty() {
            return Err("messaging channel worker access token is empty".to_string());
        }
    }
    Ok(())
}

fn validate_management_config(config: &MessagingChannelWorkerConfig) -> Result<(), String> {
    let url = reqwest::Url::parse(config.backend_url.trim())
        .map_err(|error| format!("invalid messaging backend URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("messaging backend URL must use http or https".to_string());
    }
    if config.access_token.trim().is_empty() {
        return Err("messaging channel worker access token is empty".to_string());
    }
    Ok(())
}

fn validate_claim(message: &ClaimedMessage) -> Result<(), String> {
    if message.schema_version != PROTOCOL_SCHEMA_VERSION
        || message.envelope.schema_version != PROTOCOL_SCHEMA_VERSION
    {
        return Err("unsupported messaging channel protocol schema".to_string());
    }
    if message.message_id == 0
        || message.lease_id.trim().is_empty()
        || message.envelope.connection_id.trim().is_empty()
        || message.envelope.external_conversation_id.trim().is_empty()
    {
        return Err("messaging claim is missing an identity field".to_string());
    }
    Ok(())
}

fn load_config(path: &Path) -> Result<MessagingChannelWorkerConfig, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("failed reading {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid {}: {error}", path.display()))
}

fn save_config(path: &Path, config: &MessagingChannelWorkerConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed creating {}: {error}", parent.display()))?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed writing {}: {error}", temporary.display()))?;
    set_private_permissions(&temporary)?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("failed replacing {}: {error}", path.display()))?;
    set_private_permissions(path)?;
    Ok(())
}

fn load_session_state(path: &Path) -> Result<ChannelSessionState, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("failed reading {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid {}: {error}", path.display()))
}

fn save_session_state(path: &Path, state: &ChannelSessionState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed creating {}: {error}", parent.display()))?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed writing {}: {error}", temporary.display()))?;
    set_private_permissions(&temporary)?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("failed replacing {}: {error}", path.display()))?;
    Ok(())
}

fn default_config_path() -> Result<PathBuf, String> {
    Ok(private_state_root()?.join("messaging-channel-worker.json"))
}

fn default_session_state_path() -> Result<PathBuf, String> {
    Ok(private_state_root()?.join("messaging-channel-sessions.json"))
}

fn private_state_root() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("NOLA_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(root).join("cli"));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is unavailable; cannot locate Nola private state")?;
    Ok(home.join(".nola").join("cli"))
}

fn set_private_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed securing {}: {error}", path.display()))?;
    }
    Ok(())
}

fn agent_host_executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe()
        .map_err(|error| format!("cannot locate current Nola executable: {error}"))?;
    let sibling = current.with_file_name("nola-agent-host");
    if sibling.is_file() {
        return Ok(sibling);
    }
    Err(format!(
        "nola-agent-host is not installed beside {}",
        current.display()
    ))
}

fn channel_conversation_key(envelope: &ChannelEnvelope) -> String {
    format!(
        "{}\u{1f}{}",
        envelope.connection_id, envelope.external_conversation_id
    )
}

fn stable_nola_conversation_id(envelope: &ChannelEnvelope) -> String {
    let mut digest = Sha256::new();
    digest.update(b"nola-messaging-conversation-v1\0");
    digest.update(channel_conversation_key(envelope).as_bytes());
    let hex = format!("{:x}", digest.finalize());
    format!("channel-{}", &hex[..32])
}

fn endpoint(config: &MessagingChannelWorkerConfig, path: &str) -> String {
    format!("{}{}", config.backend_url.trim_end_matches('/'), path)
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn sleep_interruptibly(cancelled: &AtomicBool, seconds: u64) {
    for _ in 0..seconds.saturating_mul(10) {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

const fn default_poll_seconds() -> u64 {
    DEFAULT_POLL_SECONDS
}

const fn default_turn_timeout_seconds() -> u64 {
    DEFAULT_TURN_TIMEOUT_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_identity_normalizes_only_url_equivalent_spelling() {
        assert_eq!(
            backend_identity(" HTTPS://EXAMPLE.COM:443/api/ ").unwrap(),
            backend_identity("https://example.com/api").unwrap()
        );
        assert_ne!(
            backend_identity("https://example.com/a").unwrap(),
            backend_identity("https://example.com/b").unwrap()
        );
        for invalid in [
            "file:///tmp/local",
            "https://user:secret@example.com",
            "https://example.com/#fragment",
            "not a URL",
        ] {
            assert!(backend_identity(invalid).is_err());
        }
    }

    fn envelope() -> ChannelEnvelope {
        ChannelEnvelope {
            schema_version: 1,
            connection_id: "telegram-primary".to_string(),
            external_conversation_id: "chat-42".to_string(),
            text: Some("hello".to_string()),
        }
    }

    #[test]
    fn stable_conversation_id_is_scoped_to_connection_and_chat() {
        let first = stable_nola_conversation_id(&envelope());
        let second = stable_nola_conversation_id(&envelope());
        assert_eq!(first, second);
        assert!(first.starts_with("channel-"));

        let mut other = envelope();
        other.external_conversation_id = "chat-43".to_string();
        assert_ne!(first, stable_nola_conversation_id(&other));
    }

    #[test]
    fn committed_source_journal_is_the_only_visible_answer_authority() {
        let (mut port, stored) =
            crate::source_journal::source_tests::completed_source(&[], "once🙂");
        let mut source = crate::source_journal::SourceRunJournal::new(
            stored.conversation_id.clone(),
            stored.run_id.clone(),
            stored.assistant_message_id.clone(),
        );
        let heads: Vec<nola_domain::assistant_journal::AssistantJournalCursor> = port
            .execute(
                nola_application::ApplicationCommand::LocalRunAssistantJournalHeads {
                    conversation_id: stored.conversation_id,
                    run_id: stored.run_id,
                    after_message_id: None,
                    limit: 32,
                },
            )
            .unwrap();
        let mut result = HostTurnResult::default();
        apply_host_event(
            &mut result,
            &json!({"trace":{"kind":"assistant.delta","payload":{"delta":"duplicate"}}}),
        );
        assert!(result.assistant_text.is_empty());
        let notification = json!({"type":"assistant_journal_committed_v1","through":heads[0]});
        apply_source_host_event(&mut result, &notification, &mut source, &mut port).unwrap();
        assert_eq!(result.assistant_text, "once🙂");
        apply_source_host_event(&mut result, &notification, &mut source, &mut port).unwrap();
        apply_host_event(
            &mut result,
            &json!({"trace":{"kind":"assistant.reset","payload":{}}}),
        );
        assert_eq!(result.assistant_text, "once🙂");
        assert!(source.seal.is_some());
    }

    #[test]
    fn foreign_run_and_trace_terminal_cannot_supply_channel_body_or_completion() {
        let (mut port, stored) =
            crate::source_journal::source_tests::completed_source(&[], "source answer");
        let mut source = crate::source_journal::SourceRunJournal::new(
            stored.conversation_id.clone(),
            stored.run_id.clone(),
            stored.assistant_message_id.clone(),
        );
        let mut result = HostTurnResult::default();
        apply_host_event(
            &mut result,
            &json!({"trace":{"kind":"run.completed","payload":{"text":"forged"}}}),
        );
        assert!(result.assistant_text.is_empty());
        assert!(source.seal.is_none());
        apply_source_host_event(
            &mut result,
            &json!({"type":"assistant_run_terminal_committed_v1",
            "session_id":source.conversation_id,"run_id":"other-run","seal_digest":"a".repeat(64)}),
            &mut source,
            &mut port,
        )
        .unwrap();
        assert!(result.assistant_text.is_empty());
        assert!(source.seal.is_none());
        let seal = stored.seal.unwrap();
        apply_source_host_event(&mut result, &json!({"type":"assistant_run_terminal_committed_v1",
            "session_id":source.conversation_id,"run_id":source.run_id,"seal_digest":seal.seal_digest}),
            &mut source, &mut port).unwrap();
        assert_eq!(result.assistant_text, "source answer");
    }

    #[test]
    fn permission_and_external_session_metadata_remain_trace_owned() {
        let mut result = HostTurnResult::default();
        apply_host_event(
            &mut result,
            &json!({"trace": {"kind": "assistant.delta", "payload": {"delta": "ignored"}}}),
        );
        apply_host_event(
            &mut result,
            &json!({"trace": {"kind": "phase.changed", "payload": {
                "phase": "external_session_started",
                "execution_backend": "codex",
                "external_session_id": "thread-9"
            }}}),
        );
        apply_host_event(
            &mut result,
            &json!({"trace": {"kind": "tool.permission.requested", "payload": {}}}),
        );
        assert!(result.assistant_text.is_empty());
        assert_eq!(result.external_session_id.as_deref(), Some("thread-9"));
        assert!(result.permission_requested);
    }

    #[test]
    fn worker_config_rejects_missing_credentials_and_non_http_urls() {
        let mut config = MessagingChannelWorkerConfig {
            schema_version: 1,
            enabled: true,
            backend_url: "file:///tmp/backend".to_string(),
            access_token: String::new(),
            poll_seconds: 2,
            turn_timeout_seconds: 60,
            execution_backend: "nola".to_string(),
            project_root: None,
        };
        assert!(validate_config(&config).is_err());
        config.backend_url = "https://nola.example".to_string();
        config.access_token = "secret".to_string();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn disabled_worker_config_can_preserve_an_incomplete_draft() {
        let config = MessagingChannelWorkerConfig {
            schema_version: 1,
            enabled: false,
            backend_url: String::new(),
            access_token: String::new(),
            poll_seconds: 2,
            turn_timeout_seconds: 900,
            execution_backend: "nola".to_string(),
            project_root: None,
        };
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn api_projection_masks_the_channel_access_token() {
        let config = MessagingChannelWorkerConfig {
            schema_version: 1,
            enabled: true,
            backend_url: "https://nola.example".to_string(),
            access_token: "do-not-return-this-token".to_string(),
            poll_seconds: 2,
            turn_timeout_seconds: 900,
            execution_backend: "codex".to_string(),
            project_root: None,
        };
        let projection = config_api_projection(&config);
        assert_eq!(projection["has_access_token"], true);
        assert!(projection.get("access_token").is_none());
        assert!(!projection.to_string().contains("do-not-return-this-token"));
    }
}
