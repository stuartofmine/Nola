// SPDX-License-Identifier: Apache-2.0
// Shared lifecycle executor; owns no model or general tool driver.
#[derive(Clone, Default)]
struct LifecycleHookExecutor {
    lifecycle_hook_rules: Vec<nola_core::lifecycle_hooks::LifecycleHookRule>,
    lifecycle_hook_registry_revision_sha256: Option<String>,
    lifecycle_hook_handler: Option<std::sync::Arc<std::sync::Mutex<LifecycleHookHandler>>>,
    lifecycle_hook_capabilities: Vec<nola_core::lifecycle_hooks::LifecycleHookRequiredCapability>,
    current_turn_lifecycle_event_ids: HashSet<String>,
    lifecycle_session_event_ids: HashSet<String>,
}
impl LifecycleHookExecutor {
    fn run_lifecycle_hooks(
        &mut self,
        request: &HostRequestEnvelope,
        sequence: &mut u64,
        event_name: nola_core::lifecycle_hooks::LifecycleHookEventName,
        event_id: Option<&str>,
        tool_name: Option<&str>,
        tool_call_id: Option<&str>,
        once_per_turn: bool,
        require_success: bool,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.lifecycle_hook_rules.is_empty() {
            return Ok(());
        }
        let event_name_id = match event_name {
            nola_core::lifecycle_hooks::LifecycleHookEventName::SessionStart => "session_start",
            nola_core::lifecycle_hooks::LifecycleHookEventName::UserPromptSubmit => {
                "user_prompt_submit"
            }
            nola_core::lifecycle_hooks::LifecycleHookEventName::PreToolUse => "pre_tool_use",
            nola_core::lifecycle_hooks::LifecycleHookEventName::PostToolUse => "post_tool_use",
            nola_core::lifecycle_hooks::LifecycleHookEventName::ToolError => "tool_error",
            nola_core::lifecycle_hooks::LifecycleHookEventName::TurnEnd => "turn_end",
            nola_core::lifecycle_hooks::LifecycleHookEventName::SessionEnd => "session_end",
        };
        if once_per_turn {
            let inserted = match event_name {
                nola_core::lifecycle_hooks::LifecycleHookEventName::SessionStart
                | nola_core::lifecycle_hooks::LifecycleHookEventName::SessionEnd => self
                    .lifecycle_session_event_ids
                    .insert(format!("{}:{event_name_id}", request.session_id)),
                _ => self.current_turn_lifecycle_event_ids.insert(format!(
                    "{}:{event_name_id}",
                    request.turn_id.as_deref().unwrap_or("session")
                )),
            };
            if !inserted {
                if require_success {
                    return Err("session-end hook already attempted; durable recovery is required".into());
                }
                return Ok(());
            }
        }
        let event = nola_core::lifecycle_hooks::LifecycleHookEvent {
            id: event_id
                .map(str::to_string)
                .unwrap_or_else(|| format!("event.{}.{}", request.effect_id.0, event_name_id)),
            name: event_name,
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            tool_name: tool_name.map(str::to_string),
            tool_call_id: tool_call_id.map(str::to_string),
        };
        let Ok(plan) = nola_core::lifecycle_hooks::plan_lifecycle_hooks(
            &nola_core::lifecycle_hooks::LifecycleHookPlanInput {
                schema_version: nola_core::lifecycle_hooks::LIFECYCLE_HOOK_SCHEMA_VERSION,
                event: Some(event.clone()),
                rules: self.lifecycle_hook_rules.clone(),
            },
        ) else {
            if require_success {
                return Err("session-end hook plan is invalid".into());
            }
            return Ok(());
        };
        if require_success && plan.rejected_rules.iter().any(|rule| matches!(rule.reason,
            nola_core::lifecycle_hooks::LifecycleHookRejectionReason::InvalidRule
                | nola_core::lifecycle_hooks::LifecycleHookRejectionReason::DuplicateRule)) {
            return Err("session-end hook registry contains invalid or duplicate rules".into());
        }
        for invocation in plan.invocations {
            let running = nola_core::lifecycle_hooks::project_lifecycle_hook_execution(
                &nola_core::lifecycle_hooks::LifecycleHookExecutionInput {
                    schema_version: nola_core::lifecycle_hooks::LIFECYCLE_HOOK_SCHEMA_VERSION,
                    registry_revision_sha256: self
                        .lifecycle_hook_registry_revision_sha256
                        .clone()
                        .unwrap_or_default(),
                    invocation: invocation.clone(),
                    status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Running,
                    duration_ms: None,
                    output_preview: None,
                    output_truncated: false,
                    error_code: None,
                },
            );
            if require_success && running.is_err() {
                return Err("session-end hook start projection is invalid".into());
            }
            if let Ok(projection) = running {
                CliAgentHostDriver::emit(
                    request,
                    sequence,
                    HostResponse::LifecycleHookUpdated { projection },
                    emit,
                )?;
            }

            let capability_available = invocation.required_capability
                == nola_core::lifecycle_hooks::LifecycleHookRequiredCapability::None
                || self
                    .lifecycle_hook_capabilities
                    .contains(&invocation.required_capability);
            let started = std::time::Instant::now();
            let result = if !capability_available {
                HostLifecycleHookResult {
                    status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                    output_preview: None,
                    output_truncated: false,
                    error_code: Some("hook_capability_unavailable".to_string()),
                }
            } else if invocation.handler.execution_mode
                == nola_core::lifecycle_hooks::LifecycleHookExecutionMode::Async
            {
                HostLifecycleHookResult {
                    status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                    output_preview: None,
                    output_truncated: false,
                    error_code: Some("hook_async_executor_unavailable".to_string()),
                }
            } else if let Some(handler) = self.lifecycle_hook_handler.as_ref().cloned() {
                let invocation_for_handler = invocation.clone();
                let event_for_handler = event.clone();
                let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let result = handler
                        .lock()
                        .map_err(|_| "hook_handler_lock_failed".to_string())
                        .and_then(|mut handler| {
                            handler(&invocation_for_handler, &event_for_handler)
                        });
                    let _ = result_tx.send(result);
                });
                match result_rx
                    .recv_timeout(std::time::Duration::from_millis(invocation.timeout_ms))
                {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => HostLifecycleHookResult {
                        status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                        output_preview: None,
                        output_truncated: false,
                        error_code: Some("hook_handler_failed".to_string()),
                    },
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => HostLifecycleHookResult {
                        status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::TimedOut,
                        output_preview: None,
                        output_truncated: false,
                        error_code: Some("hook_timed_out".to_string()),
                    },
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        HostLifecycleHookResult {
                            status:
                                nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                            output_preview: None,
                            output_truncated: false,
                            error_code: Some("hook_handler_disconnected".to_string()),
                        }
                    }
                }
            } else {
                HostLifecycleHookResult {
                    status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                    output_preview: None,
                    output_truncated: false,
                    error_code: Some("hook_handler_unavailable".to_string()),
                }
            };
            let duration_ms = started.elapsed().as_millis().min(180_000) as u64;
            let timed_out = result.status
                == nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::TimedOut
                || duration_ms > invocation.timeout_ms;
            let succeeded = !timed_out && result.status
                == nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Succeeded;
            let mut preview = result.output_preview;
            let preview_was_truncated = preview
                .as_ref()
                .is_some_and(|value| value.chars().count() > 4_096);
            if preview_was_truncated {
                preview = preview.map(|value| value.chars().take(4_096).collect());
            }
            let terminal = nola_core::lifecycle_hooks::project_lifecycle_hook_execution(
                &nola_core::lifecycle_hooks::LifecycleHookExecutionInput {
                    schema_version: nola_core::lifecycle_hooks::LIFECYCLE_HOOK_SCHEMA_VERSION,
                    registry_revision_sha256: self
                        .lifecycle_hook_registry_revision_sha256
                        .clone()
                        .unwrap_or_default(),
                    invocation,
                    status: if timed_out {
                        nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::TimedOut
                    } else if result.status
                        == nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Running
                    {
                        nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed
                    } else {
                        result.status
                    },
                    duration_ms: Some(duration_ms),
                    output_preview: preview,
                    output_truncated: result.output_truncated || preview_was_truncated,
                    error_code: if timed_out {
                        Some("hook_timed_out".to_string())
                    } else {
                        result.error_code
                    },
                },
            );
            if require_success && terminal.is_err() {
                return Err("session-end hook terminal projection is invalid; outcome is unconfirmed".into());
            }
            if let Ok(projection) = terminal {
                CliAgentHostDriver::emit(
                    request,
                    sequence,
                    HostResponse::LifecycleHookUpdated { projection },
                    emit,
                )?;
            }
            if require_success && !succeeded {
                return Err(if timed_out {
                    "session-end hook timed out; external execution may still be active"
                } else {
                    "session-end hook did not succeed; session closure is unconfirmed"
                }.into());
            }
        }
        Ok(())
    }


}
const DEFAULT_COMMAND_HOOK_TIMEOUT_MS: u64 = 10_000;
const MIN_COMMAND_HOOK_TIMEOUT_MS: u64 = 100;
const MAX_COMMAND_HOOK_TIMEOUT_MS: u64 = 30_000;
const MAX_COMMAND_HOOK_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_COMMAND_HOOK_PREVIEW_CHARS: usize = 4_096;
const MAX_CONFIGURED_COMMAND_HOOKS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandHookEvent {
    PreToolUse,
    PostToolUse,
    Stop,
}

impl CommandHookEvent {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pretooluse" | "pre_tool_use" | "pre-tool-use" => Some(Self::PreToolUse),
            "posttooluse" | "post_tool_use" | "post-tool-use" => Some(Self::PostToolUse),
            "stop" | "turn_end" | "session_end" => Some(Self::Stop),
            _ => None,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::Stop => "Stop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfiguredHookSource {
    User,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredCommandHook {
    pub id: String,
    pub event: CommandHookEvent,
    pub source: ConfiguredHookSource,
    pub matchers: Vec<String>,
    pub command: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
    pub working_dir: Option<PathBuf>,
}

impl ConfiguredCommandHook {
    pub(crate) fn matches_tool(&self, tool_name: Option<&str>) -> bool {
        if self.matchers.is_empty() {
            return true;
        }
        if self.matchers.iter().any(|pattern| pattern.trim() == "*") {
            return true;
        }
        let Some(tool_name) = tool_name else {
            return false;
        };
        self.matchers.iter().any(|matcher| {
            matcher
                .split('|')
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .any(|candidate| command_hook_glob_match(tool_name, candidate))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookExecutionDecision {
    pub decision: nola_core::lifecycle_hooks::LifecycleHookDecision,
    pub status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub additional_context: Option<String>,
    pub error_code: Option<String>,
}

impl HookExecutionDecision {
    pub(crate) fn into_outcome(
        self,
        invocation_id: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> nola_core::lifecycle_hooks::LifecycleHookOutcome {
        nola_core::lifecycle_hooks::LifecycleHookOutcome {
            invocation_id: invocation_id.into(),
            rule_id: rule_id.into(),
            status: self.status,
            decision: self.decision,
            output_preview: self.stdout_preview.or(self.stderr_preview),
            error_code: self.error_code,
        }
    }
}

/// Loads configured command hooks from `~/.nola/hooks.json` (user scope) and
/// `<workspace_root>/.nola/hooks.json` (project scope) when present.
pub(crate) fn load_configured_hooks(
    workspace_root: Option<&std::path::Path>,
) -> Result<Vec<ConfiguredCommandHook>, String> {
    load_configured_hooks_with_user_home(
        workspace_root,
        crate::platform_paths::user_home_dir().as_deref(),
    )
}

pub(crate) fn load_configured_hooks_with_user_home(
    workspace_root: Option<&std::path::Path>,
    user_home: Option<&std::path::Path>,
) -> Result<Vec<ConfiguredCommandHook>, String> {
    let mut hooks = Vec::new();
    let working_dir = workspace_root.map(std::path::Path::to_path_buf);

    if let Some(home) = user_home {
        let user_path = home.join(".nola").join("hooks.json");
        if user_path.is_file() {
            let mut user_hooks = parse_hooks_file(
                &user_path,
                ConfiguredHookSource::User,
                working_dir.clone(),
            )?;
            hooks.append(&mut user_hooks);
        }
    }

    if let Some(root) = workspace_root {
        let project_path = root.join(".nola").join("hooks.json");
        if project_path.is_file() {
            let mut project_hooks = parse_hooks_file(
                &project_path,
                ConfiguredHookSource::Project,
                working_dir,
            )?;
            hooks.append(&mut project_hooks);
        }
    }

    if hooks.len() > MAX_CONFIGURED_COMMAND_HOOKS {
        return Err(format!(
            "configured lifecycle hooks exceed limit of {MAX_CONFIGURED_COMMAND_HOOKS}"
        ));
    }

    Ok(hooks)
}

fn parse_hooks_file(
    path: &std::path::Path,
    source: ConfiguredHookSource,
    working_dir: Option<PathBuf>,
) -> Result<Vec<ConfiguredCommandHook>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))?;
    let Some(root) = parsed.as_object() else {
        return Err(format!(
            "{} must contain a JSON object",
            path.display()
        ));
    };

    let mut hooks = Vec::new();
    let mut next_index = 0usize;

    if let Some(hooks_value) = root.get("hooks") {
        match hooks_value {
            Value::Object(event_map) => {
                for (event_key, entries) in event_map {
                    parse_hook_event_entries(
                        path,
                        source,
                        working_dir.as_ref(),
                        event_key,
                        entries,
                        &mut hooks,
                        &mut next_index,
                    )?;
                }
            }
            Value::Array(items) => {
                for item in items {
                    let Some(obj) = item.as_object() else {
                        return Err(format!(
                            "hook entry in {} must be an object",
                            path.display()
                        ));
                    };
                    let event_key = obj
                        .get("event")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            format!("hook entry in {} missing event field", path.display())
                        })?;
                    let event = CommandHookEvent::parse(event_key).ok_or_else(|| {
                        format!(
                            "unsupported hook event '{event_key}' in {} (expected PreToolUse, PostToolUse, or Stop)",
                            path.display()
                        )
                    })?;
                    let matchers = extract_hook_matchers(obj, &[])?;
                    let hook = build_command_hook_entry(
                        path,
                        source,
                        working_dir.as_ref(),
                        event,
                        matchers,
                        obj,
                        &mut next_index,
                    )?;
                    hooks.push(hook);
                }
            }
            _ => {
                return Err(format!(
                    "'hooks' in {} must be an object or array",
                    path.display()
                ));
            }
        }
    } else {
        for (event_key, entries) in root {
            if CommandHookEvent::parse(event_key).is_some() {
                parse_hook_event_entries(
                    path,
                    source,
                    working_dir.as_ref(),
                    event_key,
                    entries,
                    &mut hooks,
                    &mut next_index,
                )?;
            }
        }
    }

    Ok(hooks)
}

fn parse_hook_event_entries(
    path: &std::path::Path,
    source: ConfiguredHookSource,
    working_dir: Option<&PathBuf>,
    event_key: &str,
    entries: &Value,
    out: &mut Vec<ConfiguredCommandHook>,
    next_index: &mut usize,
) -> Result<(), String> {
    let event = CommandHookEvent::parse(event_key).ok_or_else(|| {
        format!(
            "unsupported hook event '{event_key}' in {} (expected PreToolUse, PostToolUse, or Stop)",
            path.display()
        )
    })?;
    let Some(items) = entries.as_array() else {
        return Err(format!(
            "hook event '{event_key}' in {} must map to an array",
            path.display()
        ));
    };

    for item in items {
        let Some(obj) = item.as_object() else {
            return Err(format!(
                "hook entry under '{event_key}' in {} must be an object",
                path.display()
            ));
        };
        let parent_matchers = extract_hook_matchers(obj, &[])?;
        if let Some(nested) = obj.get("hooks") {
            let Some(nested_items) = nested.as_array() else {
                return Err(format!(
                    "nested 'hooks' under '{event_key}' in {} must be an array",
                    path.display()
                ));
            };
            for nested_item in nested_items {
                let Some(nested_obj) = nested_item.as_object() else {
                    return Err(format!(
                        "nested hook item under '{event_key}' in {} must be an object",
                        path.display()
                    ));
                };
                let matchers = extract_hook_matchers(nested_obj, &parent_matchers)?;
                let hook = build_command_hook_entry(
                    path,
                    source,
                    working_dir,
                    event,
                    matchers,
                    nested_obj,
                    next_index,
                )?;
                out.push(hook);
            }
        } else {
            let hook = build_command_hook_entry(
                path,
                source,
                working_dir,
                event,
                parent_matchers,
                obj,
                next_index,
            )?;
            out.push(hook);
        }
    }

    Ok(())
}

fn extract_hook_matchers(
    obj: &serde_json::Map<String, Value>,
    fallback: &[String],
) -> Result<Vec<String>, String> {
    let mut matchers = Vec::new();
    if let Some(value) = obj.get("matcher") {
        match value {
            Value::String(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    matchers.push(trimmed.to_string());
                }
            }
            Value::Array(items) => {
                for item in items {
                    let text = item
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| "hook matcher array items must be non-empty strings".to_string())?;
                    matchers.push(text.to_string());
                }
            }
            Value::Null => {}
            _ => return Err("hook matcher must be a string or array of strings".to_string()),
        }
    }
    for key in ["matchers", "toolNames", "tool_names"] {
        if let Some(value) = obj.get(key) {
            let Some(items) = value.as_array() else {
                return Err(format!("hook '{key}' must be an array of strings"));
            };
            for item in items {
                let text = item
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("hook '{key}' entries must be non-empty strings"))?;
                matchers.push(text.to_string());
            }
        }
    }
    if matchers.is_empty() {
        Ok(fallback.to_vec())
    } else {
        Ok(matchers)
    }
}

fn build_command_hook_entry(
    path: &std::path::Path,
    source: ConfiguredHookSource,
    working_dir: Option<&PathBuf>,
    event: CommandHookEvent,
    matchers: Vec<String>,
    obj: &serde_json::Map<String, Value>,
    next_index: &mut usize,
) -> Result<ConfiguredCommandHook, String> {
    if let Some(kind) = obj.get("type").and_then(Value::as_str) {
        if !kind.trim().eq_ignore_ascii_case("command") {
            return Err(format!(
                "unsupported hook type '{kind}' in {} (only 'command' is supported)",
                path.display()
            ));
        }
    }
    let command = obj
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("command hook in {} requires a non-empty 'command'", path.display()))?
        .to_string();

    let mut args = Vec::new();
    if let Some(raw_args) = obj.get("args") {
        let Some(items) = raw_args.as_array() else {
            return Err(format!("command hook 'args' in {} must be an array", path.display()));
        };
        for item in items {
            let arg = item
                .as_str()
                .ok_or_else(|| format!("command hook 'args' in {} must contain strings", path.display()))?;
            args.push(arg.to_string());
        }
    }

    let timeout_ms = parse_hook_timeout_ms(obj)?;
    let scope_prefix = match source {
        ConfiguredHookSource::User => "user",
        ConfiguredHookSource::Project => "project",
    };
    let index = *next_index;
    *next_index += 1;
    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{scope_prefix}.{}.{}", event.as_str().to_ascii_lowercase(), index));

    Ok(ConfiguredCommandHook {
        id,
        event,
        source,
        matchers,
        command,
        args,
        timeout_ms,
        working_dir: working_dir.cloned(),
    })
}

fn parse_hook_timeout_ms(obj: &serde_json::Map<String, Value>) -> Result<u64, String> {
    let raw_ms = if let Some(value) = obj.get("timeoutMs").or_else(|| obj.get("timeout_ms")) {
        value
            .as_u64()
            .ok_or_else(|| "hook timeoutMs must be a positive integer".to_string())?
    } else if let Some(value) = obj
        .get("timeoutSeconds")
        .or_else(|| obj.get("timeout_secs"))
        .or_else(|| obj.get("timeout"))
    {
        let seconds = value
            .as_u64()
            .ok_or_else(|| "hook timeout must be a positive integer".to_string())?;
        seconds.saturating_mul(1_000)
    } else {
        DEFAULT_COMMAND_HOOK_TIMEOUT_MS
    };
    Ok(raw_ms.clamp(MIN_COMMAND_HOOK_TIMEOUT_MS, MAX_COMMAND_HOOK_TIMEOUT_MS))
}

fn command_hook_glob_match(value: &str, pattern: &str) -> bool {
    let value = value.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    let value_bytes = value.as_bytes();
    let pattern_bytes = pattern.as_bytes();
    let (mut vi, mut pi) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_value = 0usize;
    while vi < value_bytes.len() {
        if pi < pattern_bytes.len()
            && (pattern_bytes[pi] == b'?' || pattern_bytes[pi] == value_bytes[vi])
        {
            vi += 1;
            pi += 1;
        } else if pi < pattern_bytes.len() && pattern_bytes[pi] == b'*' {
            star = Some(pi);
            star_value = vi;
            pi += 1;
        } else if let Some(star_index) = star {
            pi = star_index + 1;
            star_value += 1;
            vi = star_value;
        } else {
            return false;
        }
    }
    while pi < pattern_bytes.len() && pattern_bytes[pi] == b'*' {
        pi += 1;
    }
    pi == pattern_bytes.len()
}

fn build_command_hook_process(hook: &ConfiguredCommandHook) -> std::process::Command {
    let mut cmd = if !hook.args.is_empty() {
        let mut direct = std::process::Command::new(&hook.command);
        direct.args(&hook.args);
        direct
    } else {
        #[cfg(unix)]
        {
            let mut shell = std::process::Command::new("sh");
            shell.arg("-c").arg(&hook.command);
            shell
        }
        #[cfg(windows)]
        {
            let mut shell = std::process::Command::new("cmd");
            shell.arg("/C").arg(&hook.command);
            shell
        }
    };
    if let Some(dir) = &hook.working_dir {
        cmd.current_dir(dir);
    }
    crate::native_sandbox::scrub_sensitive_env(&mut cmd);
    for extra_secret in ["GITHUB_TOKEN", "GH_TOKEN", "NPM_TOKEN", "GITLAB_TOKEN"] {
        cmd.env_remove(extra_secret);
    }
    cmd.env("NOLA_HOOK_EVENT", hook.event.as_str());
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    cmd
}

fn bounded_preview(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let clean = trimmed.replace('\0', "");
    if clean.chars().count() > MAX_COMMAND_HOOK_PREVIEW_CHARS {
        Some(clean.chars().take(MAX_COMMAND_HOOK_PREVIEW_CHARS).collect())
    } else {
        Some(clean)
    }
}

fn kill_command_hook_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        if let Ok(pgid) = i32::try_from(child.id()) {
            if pgid > 0 {
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Spawns a configured command hook with the JSON payload on stdin, scrubs
/// sensitive environment variables, enforces the hook timeout by killing the
/// child process cleanly, and parses either stdout JSON decisions or exit code 2
/// (`Block` with stderr as reason).
pub(crate) fn execute_command_hook(
    hook: &ConfiguredCommandHook,
    payload: &Value,
) -> HookExecutionDecision {
    let started = std::time::Instant::now();
    let timeout_ms = hook
        .timeout_ms
        .clamp(MIN_COMMAND_HOOK_TIMEOUT_MS, MAX_COMMAND_HOOK_TIMEOUT_MS);
    let timeout = std::time::Duration::from_millis(timeout_ms);

    let mut cmd = build_command_hook_process(hook);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            return HookExecutionDecision {
                decision: nola_core::lifecycle_hooks::LifecycleHookDecision::Allow,
                status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                exit_code: None,
                duration_ms: started.elapsed().as_millis().min(180_000) as u64,
                stdout_preview: None,
                stderr_preview: bounded_preview(&error.to_string()),
                additional_context: None,
                error_code: Some("hook_spawn_failed".to_string()),
            };
        }
    };

    let payload_bytes = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || {
            use std::io::Write as _;
            let _ = stdin.write_all(&payload_bytes);
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
        });
    }

    let (stdout_tx, stdout_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(1);
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            use std::io::Read as _;
            let mut buf = Vec::new();
            let _ = stdout
                .take(MAX_COMMAND_HOOK_OUTPUT_BYTES as u64)
                .read_to_end(&mut buf);
            let _ = stdout_tx.send(buf);
        });
    } else {
        let _ = stdout_tx.send(Vec::new());
    }

    let (stderr_tx, stderr_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(1);
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::Read as _;
            let mut buf = Vec::new();
            let _ = stderr
                .take(MAX_COMMAND_HOOK_OUTPUT_BYTES as u64)
                .read_to_end(&mut buf);
            let _ = stderr_tx.send(buf);
        });
    } else {
        let _ = stderr_tx.send(Vec::new());
    }

    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    kill_command_hook_child(&mut child);
                    break None;
                }
                let remaining = timeout.saturating_sub(started.elapsed());
                std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
            }
            Err(_) => {
                kill_command_hook_child(&mut child);
                return HookExecutionDecision {
                    decision: nola_core::lifecycle_hooks::LifecycleHookDecision::Allow,
                    status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
                    exit_code: None,
                    duration_ms: started.elapsed().as_millis().min(180_000) as u64,
                    stdout_preview: None,
                    stderr_preview: None,
                    additional_context: None,
                    error_code: Some("hook_wait_failed".to_string()),
                };
            }
        }
    };

    let duration_ms = started.elapsed().as_millis().min(180_000) as u64;
    let Some(exit_status) = exit_status else {
        return HookExecutionDecision {
            decision: nola_core::lifecycle_hooks::LifecycleHookDecision::Allow,
            status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::TimedOut,
            exit_code: None,
            duration_ms,
            stdout_preview: None,
            stderr_preview: None,
            additional_context: None,
            error_code: Some("hook_timed_out".to_string()),
        };
    };

    let drain_timeout = std::time::Duration::from_millis(200);
    let stdout_bytes = stdout_rx.recv_timeout(drain_timeout).unwrap_or_default();
    let stderr_bytes = stderr_rx.recv_timeout(drain_timeout).unwrap_or_default();
    let stdout_text = String::from_utf8_lossy(&stdout_bytes);
    let stderr_text = String::from_utf8_lossy(&stderr_bytes);
    let exit_code = exit_status.code();

    // Claude Code hook convention: exit code 2 blocks with stderr as reason.
    if exit_code == Some(2) {
        let reason = bounded_preview(&stderr_text)
            .or_else(|| bounded_preview(&stdout_text))
            .unwrap_or_else(|| "blocked by lifecycle hook (exit code 2)".to_string());
        return HookExecutionDecision {
            decision: nola_core::lifecycle_hooks::LifecycleHookDecision::Block { reason },
            status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Succeeded,
            exit_code,
            duration_ms,
            stdout_preview: bounded_preview(&stdout_text),
            stderr_preview: bounded_preview(&stderr_text),
            additional_context: None,
            error_code: None,
        };
    }

    if !exit_status.success() {
        let error_code = match exit_code {
            Some(code) => format!("hook_exit_{code}"),
            None => "hook_terminated_by_signal".to_string(),
        };
        return HookExecutionDecision {
            decision: nola_core::lifecycle_hooks::LifecycleHookDecision::Allow,
            status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Failed,
            exit_code,
            duration_ms,
            stdout_preview: bounded_preview(&stdout_text),
            stderr_preview: bounded_preview(&stderr_text),
            additional_context: None,
            error_code: Some(error_code),
        };
    }

    let (decision, additional_context) = parse_command_hook_stdout_decision(&stdout_text);
    HookExecutionDecision {
        decision,
        status: nola_core::lifecycle_hooks::LifecycleHookExecutionStatus::Succeeded,
        exit_code,
        duration_ms,
        stdout_preview: bounded_preview(&stdout_text),
        stderr_preview: bounded_preview(&stderr_text),
        additional_context,
        error_code: None,
    }
}

fn parse_command_hook_stdout_decision(
    stdout_text: &str,
) -> (
    nola_core::lifecycle_hooks::LifecycleHookDecision,
    Option<String>,
) {
    let trimmed = stdout_text.trim();
    if trimmed.is_empty() {
        return (
            nola_core::lifecycle_hooks::LifecycleHookDecision::Allow,
            None,
        );
    }
    let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
        return (
            nola_core::lifecycle_hooks::LifecycleHookDecision::Allow,
            None,
        );
    };
    let Some(obj) = parsed.as_object() else {
        return (
            nola_core::lifecycle_hooks::LifecycleHookDecision::Allow,
            None,
        );
    };
    let specific = obj
        .get("hookSpecificOutput")
        .or_else(|| obj.get("hook_specific_output"))
        .and_then(Value::as_object);

    let extract_str = |keys: &[&str]| -> Option<String> {
        for key in keys {
            if let Some(value) = obj
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(value.to_string());
            }
            if let Some(nested) = specific {
                if let Some(value) = nested
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    return Some(value.to_string());
                }
            }
        }
        None
    };

    let reason = extract_str(&["reason", "stopReason", "stop_reason", "message"]);
    let additional_context = extract_str(&["additionalContext", "additional_context", "text"]);
    let updated_input = obj
        .get("updatedInput")
        .or_else(|| obj.get("updated_input"))
        .or_else(|| {
            specific.and_then(|nested| {
                nested
                    .get("updatedInput")
                    .or_else(|| nested.get("updated_input"))
            })
        })
        .cloned();

    if obj.get("continue").and_then(Value::as_bool) == Some(false) {
        return (
            nola_core::lifecycle_hooks::LifecycleHookDecision::Block {
                reason: reason.unwrap_or_else(|| "blocked by lifecycle hook".to_string()),
            },
            additional_context,
        );
    }

    let raw_decision = extract_str(&["decision", "permissionDecision", "action"])
        .map(|value| value.to_ascii_lowercase());

    let decision = match raw_decision.as_deref() {
        Some("block" | "deny") => nola_core::lifecycle_hooks::LifecycleHookDecision::Block {
            reason: reason.unwrap_or_else(|| "blocked by lifecycle hook".to_string()),
        },
        Some("modify" | "modify_input") => {
            if let Some(input) = updated_input {
                nola_core::lifecycle_hooks::LifecycleHookDecision::ModifyInput { input, reason }
            } else if let Some(text) = additional_context.clone() {
                nola_core::lifecycle_hooks::LifecycleHookDecision::InjectContext { text }
            } else {
                nola_core::lifecycle_hooks::LifecycleHookDecision::Allow
            }
        }
        Some("inject_context" | "context") => {
            if let Some(text) = additional_context.clone() {
                nola_core::lifecycle_hooks::LifecycleHookDecision::InjectContext { text }
            } else {
                nola_core::lifecycle_hooks::LifecycleHookDecision::Allow
            }
        }
        _ => {
            if let Some(input) = updated_input {
                nola_core::lifecycle_hooks::LifecycleHookDecision::ModifyInput { input, reason }
            } else if let Some(text) = additional_context.clone() {
                nola_core::lifecycle_hooks::LifecycleHookDecision::InjectContext { text }
            } else {
                nola_core::lifecycle_hooks::LifecycleHookDecision::Allow
            }
        }
    };

    (decision, additional_context)
}

/// Runs all matching configured command hooks for an event and aggregates
/// their decisions using Core's deterministic precedence (`Block` > `ModifyInput`
/// > `InjectContext` > `Allow`).
pub(crate) fn run_configured_command_hooks(
    hooks: &[ConfiguredCommandHook],
    event: CommandHookEvent,
    tool_name: Option<&str>,
    payload: &Value,
) -> (
    nola_core::lifecycle_hooks::LifecycleHookDecision,
    Vec<nola_core::lifecycle_hooks::LifecycleHookOutcome>,
) {
    let mut outcomes = Vec::new();
    for (index, hook) in hooks
        .iter()
        .filter(|hook| hook.event == event && hook.matches_tool(tool_name))
        .enumerate()
    {
        let result = execute_command_hook(hook, payload);
        let is_block = matches!(
            result.decision,
            nola_core::lifecycle_hooks::LifecycleHookDecision::Block { .. }
        );
        let outcome = result.into_outcome(
            format!("cmd_hook_{}_{}", event.as_str().to_ascii_lowercase(), index),
            hook.id.clone(),
        );
        outcomes.push(outcome);
        if is_block {
            break;
        }
    }
    let aggregate = nola_core::lifecycle_hooks::evaluate_hook_decisions(&outcomes);
    (aggregate, outcomes)
}

#[derive(Default)]
pub(crate) struct LifecycleOnlyDriver {
    executor: LifecycleHookExecutor,
}
impl CliAgentHostDriver {
    pub(crate) fn lifecycle_only_driver(&self) -> LifecycleOnlyDriver {
        LifecycleOnlyDriver { executor: self.lifecycle.clone() }
    }
}
impl AgentHostDriver for LifecycleOnlyDriver {
    type Protocol = nola_core::agent::host::AgentRuntimeProtocol;
    type Error = String;
    fn dispatch(&mut self, request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        let HostRequest::RunLifecycleHook { event } = &request.payload else {
            return Err("lifecycle-only driver refuses non-lifecycle work".into());
        };
        if event.name != nola_core::lifecycle_hooks::LifecycleHookEventName::SessionEnd
            || event.session_id != request.session_id || event.turn_id != request.turn_id {
            return Err("lifecycle event identity mismatch".into());
        }
        let mut sequence = CliAgentHostDriver::response_sequence(&request);
        if request.delivery.is_some() && !request.attempt_started {
            CliAgentHostDriver::emit(&request, &mut sequence, HostResponse::ExecutionStarted, emit)?;
        }
        self.executor.run_lifecycle_hooks(&request, &mut sequence, event.name,
            Some(&event.id), event.tool_name.as_deref(), event.tool_call_id.as_deref(), true, true, emit)?;
        CliAgentHostDriver::emit(&request, &mut sequence, HostResponse::Completed {
            output: serde_json::json!({}), usage: None, full_output_ref: None, artifacts: Vec::new(),
        }, emit)
    }
}

#[cfg(test)]
mod lifecycle_close_failure_tests {
    use super::*;
    use nola_core::lifecycle_hooks::*;

    fn request() -> HostRequestEnvelope {
        serde_json::from_value(serde_json::json!({
            "attempt_started":false,"attempt_id":"close-attempt","request_id":"close-request",
            "effect_id":"close-effect","host_id":"close-host","session_id":"close-session",
            "turn_id":"close-turn","catalog_revision":1,"availability_epoch":1,"created_at_ms":1,
            "payload":{"type":"run_lifecycle_hook","event":{"id":"close-event",
                "name":"session_end","sessionId":"close-session","turnId":"close-turn"}}
        })).unwrap()
    }

    fn executor(status: LifecycleHookExecutionStatus) -> LifecycleHookExecutor {
        LifecycleHookExecutor {
            lifecycle_hook_rules: vec![LifecycleHookRule {
                id:"close-rule".into(), event:LifecycleHookEventName::SessionEnd,
                source:LifecycleHookSource::User, trust:LifecycleHookTrustStatus::Trusted,
                handler:LifecycleHookHandlerReference {id:"close-handler".into(),
                    kind:LifecycleHookHandlerKind::Internal, execution_mode:LifecycleHookExecutionMode::Sync,
                    display_name:None}, enabled:true, matcher:Default::default(), display_order:0, timeout_ms:100,
                blocking:false,
            }],
            lifecycle_hook_registry_revision_sha256:Some("a".repeat(64)),
            lifecycle_hook_handler:Some(std::sync::Arc::new(std::sync::Mutex::new(Box::new(move |_,_| {
                Ok(HostLifecycleHookResult {status,output_preview:None,output_truncated:false,error_code:None})
            })))),
            ..Default::default()
        }
    }

    #[test]
    fn failed_or_timed_out_session_end_never_emits_completed_or_replays() {
        for status in [LifecycleHookExecutionStatus::Failed, LifecycleHookExecutionStatus::TimedOut] {
            let mut driver = LifecycleOnlyDriver {executor:executor(status)};
            let mut responses=Vec::new();
            assert!(driver.dispatch(request(), &mut |value| {responses.push(value);Ok(())}).is_err());
            assert!(responses.iter().any(|value| matches!(&value.payload,
                HostResponse::LifecycleHookUpdated {projection} if projection.status==status)));
            assert!(!responses.iter().any(|value| matches!(value.payload, HostResponse::Completed {..})));
            let count=responses.len();
            assert!(driver.dispatch(request(), &mut |value| {responses.push(value);Ok(())}).is_err());
            assert_eq!(responses.len(),count,"retry cannot turn prior failure into success");
        }
    }

    #[test]
    fn invalid_session_end_plan_and_registry_do_not_complete() {
        for invalid_event in [true,false] {
            let mut driver=LifecycleOnlyDriver {executor:executor(LifecycleHookExecutionStatus::Succeeded)};
            let mut request=request();
            if invalid_event {
                if let HostRequest::RunLifecycleHook {event}=&mut request.payload {event.id=String::new();}
            } else {driver.executor.lifecycle_hook_rules[0].id=String::new();}
            let mut responses=Vec::new();
            assert!(driver.dispatch(request,&mut |value| {responses.push(value);Ok(())}).is_err());
            assert!(!responses.iter().any(|value| matches!(value.payload,HostResponse::Completed {..})));
        }
    }

    #[test]
    fn ordinary_observation_hook_failure_remains_nonblocking() {
        let mut executor=executor(LifecycleHookExecutionStatus::Failed);
        executor.run_lifecycle_hooks(&request(), &mut 0, LifecycleHookEventName::SessionEnd,
            Some("close-event"),None,None,true,false,&mut |_| Ok(())).unwrap();
    }

    #[test]
    fn still_running_session_end_timeout_never_claims_completion() {
        let (release_tx,release_rx)=std::sync::mpsc::channel();
        let (finished_tx,finished_rx)=std::sync::mpsc::channel();
        let mut executor=executor(LifecycleHookExecutionStatus::Succeeded);
        executor.lifecycle_hook_handler=Some(std::sync::Arc::new(std::sync::Mutex::new(Box::new(move |_,_| {
            release_rx.recv().map_err(|error|error.to_string())?;
            let _=finished_tx.send(());
            Ok(HostLifecycleHookResult {status:LifecycleHookExecutionStatus::Succeeded,
                output_preview:None,output_truncated:false,error_code:None})
        }))));
        let mut driver=LifecycleOnlyDriver {executor};
        let mut responses=Vec::new();
        let result=driver.dispatch(request(),&mut |value| {responses.push(value);Ok(())});
        // Explicitly release/join the fixture handler; no orphan background work.
        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
        assert!(result.unwrap_err().contains("external execution may still be active"));
        assert!(!responses.iter().any(|value| matches!(value.payload,HostResponse::Completed {..})));
    }
}

#[cfg(test)]
mod command_hook_execution_tests {
    use super::*;
    use nola_core::lifecycle_hooks::{
        LifecycleHookDecision, LifecycleHookExecutionStatus,
    };

    #[test]
    fn loads_project_and_user_hooks_json_with_matchers_and_capped_timeouts() {
        let user_dir = tempfile::tempdir().unwrap();
        let workspace_dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(user_dir.path().join(".nola")).unwrap();
        fs::create_dir_all(workspace_dir.path().join(".nola")).unwrap();

        fs::write(
            user_dir.path().join(".nola").join("hooks.json"),
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "PreToolUse": [
                        {
                            "matcher": "bash|pty_*",
                            "hooks": [
                                {
                                    "type": "command",
                                    "command": "echo '{\"decision\":\"allow\"}'",
                                    "timeoutMs": 999999
                                }
                            ]
                        }
                    ],
                    "Stop": [
                        {
                            "command": "echo done"
                        }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        fs::write(
            workspace_dir.path().join(".nola").join("hooks.json"),
            serde_json::to_string_pretty(&json!({
                "PostToolUse": [
                    {
                        "id": "lint.after.edit",
                        "matcher": "edit_*",
                        "command": "echo checked",
                        "timeoutSeconds": 5
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let hooks = load_configured_hooks_with_user_home(
            Some(workspace_dir.path()),
            Some(user_dir.path()),
        )
        .expect("hooks should parse cleanly");

        assert_eq!(hooks.len(), 3);
        assert_eq!(hooks[0].source, ConfiguredHookSource::User);
        assert_eq!(hooks[0].event, CommandHookEvent::PreToolUse);
        assert_eq!(hooks[0].timeout_ms, MAX_COMMAND_HOOK_TIMEOUT_MS);
        assert!(hooks[0].matches_tool(Some("bash")));
        assert!(hooks[0].matches_tool(Some("pty_exec")));
        assert!(!hooks[0].matches_tool(Some("read_file")));

        assert_eq!(hooks[1].source, ConfiguredHookSource::User);
        assert_eq!(hooks[1].event, CommandHookEvent::Stop);
        assert_eq!(hooks[1].timeout_ms, DEFAULT_COMMAND_HOOK_TIMEOUT_MS);
        assert!(hooks[1].matches_tool(None));

        assert_eq!(hooks[2].source, ConfiguredHookSource::Project);
        assert_eq!(hooks[2].id, "lint.after.edit");
        assert_eq!(hooks[2].event, CommandHookEvent::PostToolUse);
        assert_eq!(hooks[2].timeout_ms, 5_000);
        assert!(hooks[2].matches_tool(Some("edit_file")));
        assert!(!hooks[2].matches_tool(Some("bash")));
    }

    #[test]
    fn command_hook_process_scrubs_sensitive_environment_variables() {
        let hook = ConfiguredCommandHook {
            id: "env.check".to_string(),
            event: CommandHookEvent::PreToolUse,
            source: ConfiguredHookSource::Project,
            matchers: vec!["*".to_string()],
            command: "echo ok".to_string(),
            args: Vec::new(),
            timeout_ms: 5_000,
            working_dir: None,
        };
        let cmd = build_command_hook_process(&hook);
        let removed: Vec<&std::ffi::OsStr> = cmd
            .get_envs()
            .filter_map(|(k, v)| v.is_none().then_some(k))
            .collect();
        for key in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "NOLA_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "BASH_ENV",
            "GITHUB_TOKEN",
        ] {
            assert!(
                removed.iter().any(|item| *item == std::ffi::OsStr::new(key)),
                "expected sensitive env var {key} to be scrubbed"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn command_hook_supports_allow_exit_code_2_block_json_modify_and_context() {
        let allow_hook = ConfiguredCommandHook {
            id: "allow.hook".to_string(),
            event: CommandHookEvent::PreToolUse,
            source: ConfiguredHookSource::Project,
            matchers: vec!["bash".to_string()],
            command: r#"printf '{"decision":"allow"}'"#.to_string(),
            args: Vec::new(),
            timeout_ms: 5_000,
            working_dir: None,
        };
        let allow_result = execute_command_hook(&allow_hook, &json!({"tool": "bash"}));
        assert_eq!(allow_result.status, LifecycleHookExecutionStatus::Succeeded);
        assert_eq!(allow_result.decision, LifecycleHookDecision::Allow);

        let exit2_block_hook = ConfiguredCommandHook {
            id: "block.exit2".to_string(),
            event: CommandHookEvent::PreToolUse,
            source: ConfiguredHookSource::Project,
            matchers: vec!["bash".to_string()],
            command: "echo 'rm -rf is prohibited by repo policy' >&2; exit 2".to_string(),
            args: Vec::new(),
            timeout_ms: 5_000,
            working_dir: None,
        };
        let block_result = execute_command_hook(&exit2_block_hook, &json!({"tool": "bash"}));
        assert_eq!(block_result.status, LifecycleHookExecutionStatus::Succeeded);
        assert_eq!(block_result.exit_code, Some(2));
        assert_eq!(
            block_result.decision,
            LifecycleHookDecision::Block {
                reason: "rm -rf is prohibited by repo policy".to_string(),
            }
        );

        let modify_hook = ConfiguredCommandHook {
            id: "modify.json".to_string(),
            event: CommandHookEvent::PreToolUse,
            source: ConfiguredHookSource::Project,
            matchers: vec!["bash".to_string()],
            command: r#"grep -q "git clean" && printf '{"decision":"modify","reason":"added --dry-run","updatedInput":{"command":"git clean --dry-run"},"additionalContext":"dry-run enforced"}'"#.to_string(),
            args: Vec::new(),
            timeout_ms: 5_000,
            working_dir: None,
        };
        let modify_result =
            execute_command_hook(&modify_hook, &json!({"tool": "bash", "input": {"command": "git clean -fd"}}));
        assert_eq!(modify_result.status, LifecycleHookExecutionStatus::Succeeded);
        assert_eq!(
            modify_result.decision,
            LifecycleHookDecision::ModifyInput {
                input: json!({"command": "git clean --dry-run"}),
                reason: Some("added --dry-run".to_string()),
            }
        );
        assert_eq!(
            modify_result.additional_context.as_deref(),
            Some("dry-run enforced")
        );

        let (aggregate, outcomes) = run_configured_command_hooks(
            &[allow_hook, modify_hook, exit2_block_hook],
            CommandHookEvent::PreToolUse,
            Some("bash"),
            &json!({"tool": "bash", "input": {"command": "git clean -fd"}}),
        );
        assert_eq!(outcomes.len(), 3);
        assert!(matches!(
            outcomes[1].decision,
            LifecycleHookDecision::ModifyInput { .. }
        ));
        assert_eq!(
            aggregate,
            LifecycleHookDecision::Block {
                reason: "rm -rf is prohibited by repo policy".to_string(),
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn command_hook_timeout_kills_child_cleanly_without_hanging() {
        let slow_hook = ConfiguredCommandHook {
            id: "slow.hook".to_string(),
            event: CommandHookEvent::PreToolUse,
            source: ConfiguredHookSource::Project,
            matchers: vec!["*".to_string()],
            command: "sleep 30".to_string(),
            args: Vec::new(),
            timeout_ms: 150,
            working_dir: None,
        };
        let started = std::time::Instant::now();
        let result = execute_command_hook(&slow_hook, &json!({"tool": "bash"}));
        let elapsed = started.elapsed();

        assert_eq!(result.status, LifecycleHookExecutionStatus::TimedOut);
        assert_eq!(result.decision, LifecycleHookDecision::Allow);
        assert_eq!(result.error_code.as_deref(), Some("hook_timed_out"));
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "timed-out hook should be killed promptly, elapsed={elapsed:?}"
        );
    }
}

