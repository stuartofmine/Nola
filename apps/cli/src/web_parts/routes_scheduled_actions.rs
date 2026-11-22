// SPDX-License-Identifier: Apache-2.0

fn list_scheduled_actions_response() -> Response<Box<dyn Read + Send>> {
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let mut actions = match store.load_web_scheduled_actions() {
        Ok(values) => values
            .into_iter()
            .filter_map(|value| serde_json::from_value::<WebScheduledAction>(value).ok())
            .collect::<Vec<_>>(),
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let results = match store.load_web_execution_results() {
        Ok(values) => values
            .into_iter()
            .filter_map(|value| serde_json::from_value::<WebExecutionResult>(value).ok())
            .collect::<Vec<_>>(),
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    for action in &mut actions {
        action.latest_result = latest_result_for_action_values(&results, &action.id);
        action.next_scheduled_at = estimate_next_scheduled_at(
            &action.frequency,
            &action.time,
            action.weekday,
            action.enabled,
        );
    }
    actions.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    json_response(
        StatusCode(200),
        json!({"actions": actions, "localFirst": true, "backend": "cli-rust", "executionCapabilities": {"manualRun": true, "scheduledDispatch": true, "timezone": "local", "promptAutomations": true}}),
    )
}

fn create_scheduled_action_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let input = match read_json::<ScheduledActionInput>(request) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let action = match build_web_scheduled_action(input) {
        Ok(action) => action,
        Err(error) => return json_response(StatusCode(400), json!({"success":false,"error":error})),
    };
    let mut store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let action = match store.upsert_web_scheduled_action(&json!(action)) {
        Ok(action) => action,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    json_response(StatusCode(200), json!({"success":true,"action":action,"localFirst":true,"backendSynced":false}))
}

fn build_web_scheduled_action(input: ScheduledActionInput) -> Result<WebScheduledAction, String> {
    web_automation_spec(&input.execution_spec)?;
    let format = input
        .format
        .as_deref()
        .unwrap_or("agent")
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        format.as_str(),
        "agent" | "html" | "search" | "rss" | "json" | "icalendar" | "file" | "folder" | "script"
    ) {
        return Err("无效任务类型".into());
    }
    let frequency = input
        .frequency
        .as_deref()
        .unwrap_or("daily")
        .trim()
        .to_ascii_lowercase();
    if !matches!(frequency.as_str(), "daily" | "weekly") {
        return Err("无效执行频率".into());
    }
    let name = normalize_title(input.name.as_deref());
    let schedule_time = input.time.as_deref().unwrap_or("09:00");
    let schedule_timezone = input.timezone.as_deref().unwrap_or("local");
    validate_web_schedule(&frequency, schedule_time, input.weekday, schedule_timezone)?;
    let url = scheduled_action_url(&format, &input)?;
    // Approval is a separate, revision-bound operation, never a form field.
    let approved = false;
    let source_kind = source_kind_for_format(&format, &url);
    let now = now_string();
    let script = input
        .script
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    let user_prompt = if format == "script" {
        build_script_metadata(
            &script,
            input
                .cwd
                .as_deref()
                .unwrap_or(""),
            input.user_prompt.as_deref().unwrap_or(""),
            approved,
        )?
    } else {
        input.user_prompt.unwrap_or_default()
    };
    let script_meta = (format == "script").then(|| load_script_metadata(&user_prompt));
    let weekday = input.weekday;
    let time = input.time.unwrap_or_else(|| "09:00".to_string());
    let enabled = format != "script" || approved;
    let next_scheduled_at = estimate_next_scheduled_at(&frequency, &time, weekday, enabled);
    let action = WebScheduledAction {
        execution_spec: input.execution_spec,
        id: new_id("act"),
        revision: 0,
        name,
        url: url.clone(),
        format: format.clone(),
        source_kind: source_kind.to_string(),
        source_kind_label: source_kind_label(source_kind).to_string(),
        local_only: matches!(format.as_str(), "file" | "folder" | "script"),
        user_prompt,
        frequency: frequency.clone(),
        weekday,
        time,
        timezone: input.timezone.unwrap_or_else(|| "local".to_string()),
        enabled,
        next_scheduled_at,
        updated_at: now,
        latest_result: None,
        script_approved: (format == "script").then_some(approved),
        script_runtime: script_meta
            .as_ref()
            .and_then(|meta| meta.get("runtime").and_then(Value::as_str))
            .map(ToOwned::to_owned),
        script_cwd: script_meta
            .as_ref()
            .and_then(|meta| meta.get("cwd").and_then(Value::as_str))
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| (format == "script").then(|| url.clone())),
        script_preview: script_meta.as_ref().map(script_preview),
    };
    Ok(action)
}

fn delete_scheduled_action_response(action_id: &str) -> Response<Box<dyn Read + Send>> {
    let mut store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let mut actions = match store.load_web_scheduled_actions() {
        Ok(values) => values,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let before = actions.len();
    actions.retain(|item| item.get("id").and_then(Value::as_str) != Some(action_id));
    if actions.len() == before {
        return json_response(
            StatusCode(404),
            json!({"success": false, "detail": "任务不存在"}),
        );
    }
    if let Err(err) = store.delete_web_scheduled_action(action_id) {
        return json_response(StatusCode(500), json!({"error": err}));
    }
    json_response(
        StatusCode(200),
        json!({"success": true, "localFirst": true}),
    )
}

fn update_scheduled_action_response(
    request: &mut Request,
    action_id: &str,
) -> Response<Box<dyn Read + Send>> {
    let input = match read_json::<ScheduledActionUpdateInput>(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if input.enabled.is_some() == input.definition.is_some() {
        return json_response(
            StatusCode(400),
            json!({"success":false,"error":"Specify either enabled or definition"}),
        );
    }
    let expected_revision = input.expected_revision;
    if expected_revision < 1 {
        return json_response(
            StatusCode(400),
            json!({"success":false,"code":"automation_revision_required"}),
        );
    }
    let mut store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let actions = match store.load_web_scheduled_actions() {
        Ok(values) => values,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let Some(index) = actions
        .iter()
        .position(|item| item.get("id").and_then(Value::as_str) == Some(action_id))
    else {
        return json_response(
            StatusCode(404),
            json!({"success": false, "error": "任务不存在"}),
        );
    };
    let mut action = match serde_json::from_value::<WebScheduledAction>(actions[index].clone()) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(500), json!({"error": err.to_string()})),
    };
    if action.revision != expected_revision {
        return json_response(StatusCode(409), json!({"success":false,"code":"automation_revision_conflict"}));
    }
    if let Some(definition) = input.definition {
        action = match edit_web_scheduled_action(&action, definition) {
            Ok(action) => action,
            Err(error) => return json_response(StatusCode(400), json!({"success":false,"error":error})),
        };
    }
    if action.format == "script" && input.enabled == Some(true) && action.script_approved != Some(true) {
        return json_response(
            StatusCode(409),
            json!({"success": false, "error": "脚本自动化需要先完成审批"}),
        );
    }
    if let Some(enabled) = input.enabled { action.enabled = enabled; }
    action.updated_at = now_string();
    action.next_scheduled_at = estimate_next_scheduled_at(
        &action.frequency,
        &action.time,
        action.weekday,
        action.enabled,
    );
    let action =
        match store.update_web_scheduled_action_if_revision(&json!(action), expected_revision) {
            Ok(Some(action)) => action,
            Ok(None) => {
                return json_response(
                    StatusCode(409),
                    json!({"success":false,"code":"automation_revision_conflict"}),
                )
            }
            Err(err) => return json_response(StatusCode(500), json!({"error": err})),
        };
    json_response(
        StatusCode(200),
        json!({"success": true, "action": action, "localFirst": true}),
    )
}

fn edit_web_scheduled_action(previous: &WebScheduledAction, input: ScheduledActionInput) -> Result<WebScheduledAction, String> {
    let mut next = build_web_scheduled_action(input)?;
    next.id = previous.id.clone();
    next.revision = previous.revision;
    next.latest_result = previous.latest_result.clone();
    // A revised script must be reviewed against its new revision. Editing does
    // not implicitly resume a paused task or transfer old script approval.
    next.enabled = previous.enabled && next.format != "script";
    next.next_scheduled_at = estimate_next_scheduled_at(&next.frequency, &next.time, next.weekday, next.enabled);
    Ok(next)
}

fn approve_script_action_response(
    request: &mut Request,
    action_id: &str,
) -> Response<Box<dyn Read + Send>> {
    let value = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if value.get("confirmSafety").and_then(Value::as_bool) != Some(true) {
        return json_response(
            StatusCode(400),
            json!({"success": false, "detail": "请确认已审查脚本、安全边界、允许目录和命令白名单"}),
        );
    }
    let Some(expected_revision) = value
        .get("expectedRevision")
        .and_then(Value::as_i64)
        .filter(|revision| *revision > 0)
    else {
        return json_response(
            StatusCode(400),
            json!({"success":false,"code":"automation_revision_required"}),
        );
    };
    let mut store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let mut actions = match store.load_web_scheduled_actions() {
        Ok(values) => values
            .into_iter()
            .filter_map(|value| serde_json::from_value::<WebScheduledAction>(value).ok())
            .collect::<Vec<_>>(),
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let Some(action) = actions.iter_mut().find(|item| item.id == action_id) else {
        return json_response(
            StatusCode(404),
            json!({"success": false, "detail": "任务不存在"}),
        );
    };
    if action.format != "script" {
        return json_response(
            StatusCode(400),
            json!({"success": false, "errorCode": "approval_not_required"}),
        );
    }
    action.user_prompt = match approve_script_metadata(&action.user_prompt) {
        Ok(value) => value,
        Err(err) => {
            return json_response(
                StatusCode(400),
                json!({"success": false, "errorCode": "invalid_script", "technicalDetail": err}),
            )
        }
    };
    let meta = load_script_metadata(&action.user_prompt);
    action.enabled = true;
    action.next_scheduled_at = estimate_next_scheduled_at(
        &action.frequency,
        &action.time,
        action.weekday,
        action.enabled,
    );
    action.script_approved = Some(true);
    action.script_preview = Some(script_preview(&meta));
    action.updated_at = now_string();
    let response_action = action.clone();
    let response_action = match store
        .update_web_scheduled_action_if_revision(&json!(response_action), expected_revision)
    {
        Ok(Some(action)) => action,
        Ok(None) => {
            return json_response(
                StatusCode(409),
                json!({"success":false,"code":"automation_revision_conflict"}),
            )
        }
        Err(err) => return json_response(StatusCode(500), json!({"error":err})),
    };
    json_response(
        StatusCode(200),
        json!({"success": true, "action": response_action, "localFirst": true}),
    )
}

fn web_automation_spec(
    definition: &nola_domain::automation_execution::AutomationDefinitionSpec,
) -> Result<&nola_domain::automation_execution::AutomationExecutionSpec, String> {
    definition.validate()?;
    let spec = definition.agent()?;
    if !matches!(spec.execution_scope, nola_domain::automation_execution::AutomationExecutionScope::LocalWorkspace { .. })
        || !spec.context_refs.is_empty() {
        return Err("Web automation requires a local workspace and currently supports no data-source artifacts".into());
    }
    Ok(spec)
}

fn automation_skill_runtime(config: &Config) -> Result<crate::skill_runtime::SkillRuntimeContext, String> {
    let store = NolaStore::open_default()?;
    let settings = load_agent_settings_from_store(&store)?;
    web_skill_runtime_for_settings(config.skill_dirs.as_deref(), &settings)
}

fn automation_skills_response(config: &Config) -> Response<Box<dyn Read + Send>> {
    match automation_skill_runtime(config).and_then(|runtime| runtime.index()) {
        Ok(items) => json_response(StatusCode(200), json!({"skills":items.into_iter().map(|item| json!({
            "id":item["id"], "name":item["name"], "description":item["description"]
        })).collect::<Vec<_>>()})),
        Err(_) => json_response(StatusCode(503), json!({"error":"Automation skill catalog unavailable"})),
    }
}

fn automation_skill_artifacts(runtime: &crate::skill_runtime::SkillRuntimeContext, ids: &[String]) -> Result<Vec<crate::host_invocation_request::AutomationArtifact>, String> {
    use crate::host_invocation_request::{AutomationArtifact, AutomationArtifactKind};
    use sha2::{Digest, Sha256};
    use std::io::Read;
    if ids.is_empty() { return Ok(Vec::new()); }
    let catalog = runtime.index()?;
    let mut seen = std::collections::HashSet::new();
    let mut remaining = 16 * 1024 * 1024usize;
    ids.iter().map(|id| {
        if !seen.insert(id) { return Err("Duplicate automation skill".into()); }
        let item = catalog.iter().find(|item| item.get("id").and_then(Value::as_str) == Some(id))
            .ok_or("Selected automation skill is unavailable or disabled")?;
        let root = item.get("base_directory").and_then(Value::as_str).ok_or("Skill has no instruction source")?;
        let path = std::path::Path::new(root).join("SKILL.md").canonicalize().map_err(|_| "Skill instruction source unavailable")?;
        let mut bytes = Vec::new();
        std::fs::File::open(&path).map_err(|_| "Skill instruction source unavailable")?
            .take(remaining.min(4 * 1024 * 1024) as u64 + 1).read_to_end(&mut bytes).map_err(|_| "Skill instruction read failed")?;
        if bytes.len() > remaining.min(4 * 1024 * 1024) { return Err("Skill exceeds automation admission budget".into()); }
        remaining -= bytes.len();
        std::str::from_utf8(&bytes).map_err(|_| "Skill instruction is not UTF-8")?;
        Ok(AutomationArtifact { kind: AutomationArtifactKind::Skill, id: id.clone(), path,
            sha256: format!("{:x}", Sha256::digest(&bytes)), source: None })
    }).collect()
}

fn web_automation_observation_response(store: &NolaStore, id: &str, error: Option<String>) -> Response<Box<dyn Read + Send>> {
    let receipt = match store.reconcile_web_automation_execution(id, Utc::now().timestamp_millis()) {
        Ok(receipt) => receipt,
        Err(error) => return json_response(StatusCode(202), json!({"occurrenceId":id,"outcomeUnknown":true,"error":error})),
    };
    match store.observe_web_automation_execution(id) {
        Ok(execution) => json_response(StatusCode(if receipt.is_some() { 200 } else { 202 }),
            json!({"occurrenceId":id,"execution":execution,"receipt":receipt,"error":error})),
        Err(error) => json_response(StatusCode(202), json!({"occurrenceId":id,"outcomeUnknown":true,"error":error})),
    }
}

fn resolve_web_automation_config(
    store: &NolaStore,
    config: &Config,
    conversation_id: &str,
) -> Result<Config, String> {
    if !config.model.trim().is_empty() && !config.endpoint.trim().is_empty() {
        return Ok(config.clone());
    }
    let (resolution, _) = mobile_default_chat_model_projection(store)?;
    if resolution.get("kind").and_then(Value::as_str) == Some("resolved") {
        if let Some(profile_id) = resolution.get("profile_id").and_then(Value::as_str) {
            if let Some(profile) = store.model_profile(profile_id)? {
                let source: Value = serde_json::from_str(&profile.source_json)
                    .map_err(|_| "Invalid default Host model profile source".to_string())?;
                let secret = crate::host_credentials::for_profile(store, &profile, None)?;
                let mut resolved = resolve_remote_profile_settings(
                    &profile,
                    &json!({}),
                    &config.runtime,
                    &[(
                        source["baseURL"].as_str().unwrap_or(""),
                        secret.as_deref().unwrap_or(""),
                    )],
                )?;
                crate::model_profile_resolution::bind_siwc_profile(&mut resolved, &profile, store)?;
                return Ok(conversation_execution_config(config, resolved, conversation_id));
            }
        }
    }
    let state = load_state().unwrap_or_default();
    if let Ok(resolved) = resolve_llm_settings(&json!({}), config, Some(&state.agent_settings), None) {
        return Ok(conversation_execution_config(config, resolved, conversation_id));
    }
    Err("No default model is configured for Automations; configure a model profile first.".into())
}

fn execute_claimed_web_automation_occurrence(
    store: &NolaStore,
    config: &Config,
    run_id: &str,
    expected_revision: i64,
    lease: nola_domain::automation_execution::AutomationExecutionLease,
    spec: &nola_domain::automation_execution::AutomationExecutionSpec,
    skills: &crate::skill_runtime::SkillRuntimeContext,
    artifacts: Vec<crate::host_invocation_request::AutomationArtifact>,
) -> Option<String> {
    use nola_domain::automation_execution::*;
    let result = (|| -> Result<(), String> {
        if !store.observe_web_automation_worker(lease.clone(), AutomationWorkerObservation::Preparing, Utc::now().timestamp_millis())? {
            return Err("automation preparation fact was not committed".into());
        }
        let identity = AutomationExecutionIdentity::for_occurrence(run_id);
        let exec_config = resolve_web_automation_config(store, config, &identity.conversation_id)?;
        let mut invocation = exec_config.host_invocation_request()?;
        let AutomationExecutionScope::LocalWorkspace { root } = &spec.execution_scope else { return Err("local workspace required".into()) };
        let profile = match spec.permission_profile {
            AutomationPermissionProfile::Cautious => crate::permissions::PermissionProfile::Cautious,
            AutomationPermissionProfile::AllowAll => crate::permissions::PermissionProfile::AllowAll,
        };
        invocation.tool_settings.project_root = std::path::PathBuf::from(root);
        invocation.tool_settings.allowed_dirs = vec![invocation.tool_settings.project_root.clone()];
        invocation.tool_settings.permission_profile = Some(profile);
        invocation.tool_settings.auto_approve = profile.auto_approve();
        invocation.tool_settings.network_policy = profile.network_policy().into();
        invocation.tool_settings.bash_prefixes.clear();
        invocation.runtime.permissions.clear();
        invocation.runtime.core_permission_rules.clear();
        invocation.session.session_id = Some(identity.conversation_id);
        invocation.session.host_submission_id = Some(identity.submission_id);
        invocation.session.host_user_message_id = Some(identity.user_message_id);
        invocation.session.host_assistant_message_id = Some(identity.assistant_message_id);
        invocation.session.expected_configuration_revision = None;
        invocation.session.expected_parent_message_id = None;
        invocation.session.permission_scope_id = None;
        invocation.session.save_session = true;
        invocation.session.resume_snapshot = None;
        invocation.session.desktop_resume = None;
        invocation.session.desktop_cancel = None;
        invocation.session.host_task_execution = None;
        invocation.session.automation_execution_lease = Some(lease.clone());
        invocation.session.automation_execution_spec = Some(spec.clone());
        invocation.tool_settings.skill_dirs = skills.configured_dirs().map(|dirs| dirs.to_vec());
        invocation.session.automation_artifacts = artifacts;
        invocation.input.message = spec.prompt.clone();
        invocation.input.host_input_origin = nola_core::agent::protocol::AgentInputOrigin::ScheduledAction;
        invocation.input.host_turn_context = None;
        invocation.input.host_context_components.clear();
        invocation.input.host_attachments.clear();
        invocation.input.context_files.clear();
        invocation.input.expected_artifact_refs.clear();
        crate::core_agent_runtime::admit_automation_artifacts(&mut invocation)?;
        let mut output = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        // A synchronous caller has no interactive approval channel.
        // Preserve the committed permission wait; never prompt server stdin.
        let _permission_transport = crate::tools::enter_host_permission_resolver(std::sync::Arc::new(|_, _, _| {
            crate::tools::HostPermissionResolution::TransportInterrupted(
                nola_host_protocol::PermissionTransportInterruptionReason::Unavailable)
        }));
        if !store.observe_web_automation_worker(lease.clone(), AutomationWorkerObservation::AdmissionRequested, Utc::now().timestamp_millis())? {
            return Err("automation admission request fact was not committed".into());
        }
        crate::core_agent_runtime::run_agent_core(&invocation, &spec.prompt, &mut output)?;
        Ok(())
    })();
    let _ = expected_revision;
    let error = result.err();
    if let Some(error) = &error {
        match store.observe_web_automation_worker(lease,
            AutomationWorkerObservation::LaunchFailed { detail: error.chars().take(8_000).collect() },
            Utc::now().timestamp_millis()) {
            Ok(true) => {}
            Ok(false) => return Some("Automation failure fact was not committed; reconcile this exact occurrence".into()),
            Err(persistence_error) => return Some(format!("Automation failure fact could not be persisted: {persistence_error}")),
        }
    }
    error
}

const MAX_CONCURRENT_SCHEDULED_AUTOMATIONS: usize = 8;

struct WebAutomationSchedulerGuard(Arc<std::sync::atomic::AtomicBool>);

impl Drop for WebAutomationSchedulerGuard {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

struct InFlightAutomationGuard {
    in_flight: Arc<Mutex<HashSet<String>>>,
    action_id: String,
}

impl Drop for InFlightAutomationGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.in_flight.lock() {
            guard.remove(&self.action_id);
        }
    }
}

fn reconcile_completed_web_automation_execution(store: &NolaStore, run_id: &str) {
    if let Err(error) = store.reconcile_web_automation_execution(run_id, Utc::now().timestamp_millis()) {
        eprintln!("web automation scheduler: reconcile {run_id} failed: {error}");
    }
}

fn reconcile_pending_web_automation_executions(store: &NolaStore) {
    let mut after: Option<String> = None;
    for _ in 0..16 {
        let pending = match store.list_pending_web_automation_executions(after.as_deref(), 256) {
            Ok(ids) => ids,
            Err(error) => {
                eprintln!("web automation scheduler: list pending executions failed: {error}");
                return;
            }
        };
        if pending.is_empty() {
            return;
        }
        let page_len = pending.len();
        after = pending.last().cloned();
        for occurrence_id in pending {
            reconcile_completed_web_automation_execution(store, &occurrence_id);
        }
        if page_len < 256 {
            return;
        }
    }
}

fn start_web_automation_scheduler(config: &Config) -> Option<WebAutomationSchedulerGuard> {
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_stopped = Arc::clone(&stopped);
    let config = config.clone();
    if std::thread::Builder::new()
        .name("web-automation-scheduler".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
            if let Ok(store) = NolaStore::open_default() {
                reconcile_pending_web_automation_executions(&store);
            }
            while !worker_stopped.load(std::sync::atomic::Ordering::SeqCst) {
                for _ in 0..30 {
                    if worker_stopped.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(StdDuration::from_secs(1));
                }
                let Ok(store) = NolaStore::open_default() else { continue };
                reconcile_pending_web_automation_executions(&store);
                let Ok(raw_actions) = store.load_web_scheduled_actions() else { continue };
                let now_ms = Utc::now().timestamp_millis();
                let lookback_ms = now_ms.saturating_sub(15 * 60 * 1_000);
                for value in raw_actions {
                    if worker_stopped.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    let Ok(action) = serde_json::from_value::<WebScheduledAction>(value) else { continue };
                    if !action.enabled || (action.format == "script" && action.script_approved != Some(true)) {
                        continue;
                    }
                    {
                        let Ok(guard) = in_flight.lock() else { continue };
                        if guard.contains(&action.id) || guard.len() >= MAX_CONCURRENT_SCHEDULED_AUTOMATIONS {
                            continue;
                        }
                    }
                    let Ok(spec) = web_automation_spec(&action.execution_spec).cloned() else { continue };
                    let tz = if action.timezone.trim().is_empty() || action.timezone.eq_ignore_ascii_case("local") {
                        "system".to_string()
                    } else {
                        action.timezone.clone()
                    };
                    let Ok(page) = nola_application::automation_occurrence::query(
                        &nola_application::automation_occurrence::OccurrenceQuery {
                            automation_id: action.id.clone(),
                            timezone: tz,
                            frequency: action.frequency.clone(),
                            time: Some(action.time.clone()),
                            weekday: action.weekday.map(|day| u32::from((day + 5) % 7 + 1)),
                            once_at_ms: None,
                            after_ms: lookback_ms,
                            through_ms: now_ms,
                            limit: 4,
                        },
                    ) else {
                        continue;
                    };
                    let Some(occurrence) = page.occurrences.last() else { continue };
                    let occurrence_key = format!("scheduled:{}:{}", action.revision, occurrence.scheduled_for_ms);
                    if store.find_web_automation_run(&action.id, &occurrence_key).ok().flatten().is_some() {
                        continue;
                    }
                    let Ok((skills, artifacts)) = automation_skill_runtime(&config).and_then(|skills| {
                        let artifacts = automation_skill_artifacts(&skills, &spec.skill_refs)?;
                        Ok((skills, artifacts))
                    }) else {
                        continue;
                    };
                    let Ok(run) = store.enqueue_web_automation_run(
                        &action.id,
                        &occurrence_key,
                        "scheduled",
                        occurrence.scheduled_for_ms,
                        action.revision,
                    ) else {
                        continue;
                    };
                    let lease_token = new_id("lease");
                    let worker_id = format!("cli-web-scheduler:{}", std::process::id());
                    if !store
                        .claim_web_automation_run(
                            &run.id,
                            &worker_id,
                            &lease_token,
                            Utc::now().timestamp_millis(),
                            Utc::now().timestamp_millis().saturating_add(5 * 60 * 1_000),
                        )
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let lease = nola_domain::automation_execution::AutomationExecutionLease {
                        occurrence_id: run.id.clone(),
                        definition_revision: action.revision,
                        worker_id,
                        lease_token,
                    };
                    let in_flight_guard = {
                        let Ok(mut guard) = in_flight.lock() else { continue };
                        if guard.contains(&action.id) || guard.len() >= MAX_CONCURRENT_SCHEDULED_AUTOMATIONS {
                            continue;
                        }
                        guard.insert(action.id.clone());
                        InFlightAutomationGuard {
                            in_flight: Arc::clone(&in_flight),
                            action_id: action.id.clone(),
                        }
                    };
                    let worker_config = config.clone();
                    let run_id = run.id.clone();
                    let revision = action.revision;
                    if let Err(error) = std::thread::Builder::new()
                        .name(format!("web-automation-run-{}", action.id))
                        .stack_size(16 * 1024 * 1024)
                        .spawn(move || {
                            let _guard = in_flight_guard;
                            let Ok(worker_store) = NolaStore::open_default() else {
                                eprintln!("web automation scheduler: failed to open store for {run_id}");
                                return;
                            };
                            let _ = execute_claimed_web_automation_occurrence(
                                &worker_store,
                                &worker_config,
                                &run_id,
                                revision,
                                lease,
                                &spec,
                                &skills,
                                artifacts,
                            );
                            reconcile_completed_web_automation_execution(&worker_store, &run_id);
                        })
                    {
                        eprintln!("web automation scheduler: failed to spawn worker for {}: {error}", run.id);
                    }
                }
            }
        })
        .is_ok()
    {
        Some(WebAutomationSchedulerGuard(stopped))
    } else {
        None
    }
}

fn run_scheduled_action_now_response(request: &mut Request, action_id: &str, config: &Config) -> Response<Box<dyn Read + Send>> {
    use nola_domain::automation_execution::*;
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Input { occurrence_key: String, expected_revision: i64 }
    let input = match read_json::<Input>(request) { Ok(value) => value, Err(response) => return response };
    if input.occurrence_key.trim().is_empty() || input.occurrence_key.len() > 256 || input.expected_revision < 1 {
        return json_response(StatusCode(400), json!({"error":"stable occurrenceKey and positive expectedRevision required"}));
    }
    let store = match NolaStore::open_default() { Ok(store) => store, Err(error) => return json_response(StatusCode(500),json!({"error":error})) };
    // The exact revision belongs to the caller's stable manual occurrence; no
    // random replacement submission may be created after an uncertain response.
    let occurrence_key = format!("manual:{}:{}", input.expected_revision, input.occurrence_key);
    let now_ms = Utc::now().timestamp_millis();
    // Observe a prior request before validating the mutable definition. A
    // replay retains its original Run even if the definition was edited.
    let existing = match store.find_web_automation_run(action_id, &occurrence_key) {
        Ok(value) => value,
        Err(error) => return json_response(StatusCode(500),json!({"error":error})),
    };
    if let Some(run) = &existing {
        if let Err(error) = store.fence_expired_unadmitted_web_automation(run, now_ms) {
            return json_response(StatusCode(202),json!({"occurrenceId":run.id,"outcomeUnknown":true,"error":error}));
        }
        match store.observe_web_automation_execution(&run.id) {
            Ok(value) if value.binding.is_some() || value.receipt.is_some() || value.phase == AutomationExecutionPhase::LaunchFailed =>
                return web_automation_observation_response(&store, &run.id, None),
            Ok(_) => {},
            Err(error) => return json_response(StatusCode(202),json!({"occurrenceId":run.id,"outcomeUnknown":true,"error":error})),
        }
    }
    let actions = match store.load_web_scheduled_actions() { Ok(value) => value, Err(error) => return json_response(StatusCode(500),json!({"error":error})) };
    let Some(value) = actions.into_iter().find(|value| value.get("id").and_then(Value::as_str)==Some(action_id)) else {
        return json_response(StatusCode(404),json!({"errorCode":"action_not_found"}));
    };
    let action: WebScheduledAction = match serde_json::from_value(value) { Ok(value) => value, Err(error) => return json_response(StatusCode(500),json!({"error":error.to_string()})) };
    let spec = match web_automation_spec(&action.execution_spec) { Ok(value) => value.clone(), Err(error) => return json_response(StatusCode(400),json!({"error":error})) };
    if action.revision != input.expected_revision {
        return json_response(StatusCode(409),json!({"errorCode":"automation_revision_conflict"}));
    }
    if !action.enabled || (action.format == "script" && action.script_approved != Some(true)) {
        return json_response(StatusCode(409),json!({"errorCode":"automation_not_enabled"}));
    }
    // Resolve the current enabled catalog before claiming an occurrence. Missing
    // skills are a rejected admission, not a misleading pending execution.
    let (skills, artifacts) = match automation_skill_runtime(config).and_then(|skills| {
        let artifacts = automation_skill_artifacts(&skills, &spec.skill_refs)?;
        Ok((skills, artifacts))
    }) {
        Ok(value) => value,
        Err(_) => return json_response(StatusCode(409), json!({"code":"automation_skill_unavailable","error":"Selected automation skills could not be loaded"})),
    };
    let run = match existing.map(Ok).unwrap_or_else(|| store.enqueue_web_automation_run(action_id, &occurrence_key, "manual", now_ms, input.expected_revision)) {
        Ok(run) => run, Err(error) => return json_response(StatusCode(500),json!({"error":error})),
    };
    match store.observe_web_automation_execution(&run.id) {
        Ok(observed) if observed.binding.is_some() || observed.receipt.is_some() =>
            return web_automation_observation_response(&store, &run.id, None),
        Err(error) => return json_response(StatusCode(202),json!({"occurrenceId":run.id,"outcomeUnknown":true,"error":error})),
        _ => {}
    }
    let lease_token = new_id("lease");
    let worker_id = format!("cli-web:{}", std::process::id());
    match store.claim_web_automation_run(&run.id, &worker_id, &lease_token, now_ms, now_ms.saturating_add(5*60*1_000)) {
        Ok(true) => {},
        Ok(false) => return web_automation_observation_response(&store, &run.id, None),
        Err(error) => return json_response(StatusCode(202),json!({"occurrenceId":run.id,"outcomeUnknown":true,"error":error})),
    }
    let lease = AutomationExecutionLease {
        occurrence_id:run.id.clone(), definition_revision:input.expected_revision, worker_id, lease_token,
    };
    let error = execute_claimed_web_automation_occurrence(
        &store,
        config,
        &run.id,
        input.expected_revision,
        lease,
        &spec,
        &skills,
        artifacts,
    );
    web_automation_observation_response(&store, &run.id, error)
}

fn activity_read_response(command: nola_application::ApplicationCommand) -> Response<Box<dyn Read + Send>> {
    match crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<Value>(command)) {
        Ok(value) => json_response(StatusCode(200), value),
        Err(error) => json_response(StatusCode(409), json!({"error": error})),
    }
}

fn list_results_response() -> Response<Box<dyn Read + Send>> {
    let mut results =
        match NolaStore::open_default().and_then(|store| store.load_web_execution_results()) {
            Ok(values) => values
                .into_iter()
                .filter_map(|value| serde_json::from_value::<WebExecutionResult>(value).ok())
                .collect::<Vec<_>>(),
            Err(err) => return json_response(StatusCode(500), json!({"error": err})),
        };
    results.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.result_id.cmp(&b.result_id))
    });
    let summaries = results
        .iter()
        .take(100)
        .map(result_summary_value)
        .collect::<Vec<_>>();
    json_response(
        StatusCode(200),
        json!({"results": summaries, "localFirst": true, "backend": "cli-rust"}),
    )
}

fn result_detail_response(result_id: &str) -> Response<Box<dyn Read + Send>> {
    if result_id.trim().is_empty() || result_id.bytes().any(|byte| byte.is_ascii_control()) {
        return json_response(StatusCode(400), json!({"error": "无效执行记录标识"}));
    }
    let results = match NolaStore::open_default().and_then(|store| store.load_web_execution_results()) {
        Ok(values) => values,
        Err(error) => return json_response(StatusCode(500), json!({"error":error})),
    };
    let Some(result) = results.into_iter().find(|value|value.get("resultId").and_then(Value::as_str)==Some(result_id)) else {
        return json_response(StatusCode(404), json!({"error":"执行记录不存在"}));
    };
    // Preserve the complete typed receipt in detail; presentation summaries
    // must not replace canonical result evidence.
    json_response(StatusCode(200), result)
}
