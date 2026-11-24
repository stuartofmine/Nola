fn handle_data_source_command(prompt: &str, config: &Config) -> Result<(), String> {
    use crate::data_sources::{DataSourceDraft, DataSourceStore, HostDataSourceKind};
    use nola_core::data_source::DataSourceAccessPolicy;

    let command = prompt.split_whitespace().next().unwrap_or("/sources");
    let rest = prompt.get(command.len()..).unwrap_or_default().trim_start();
    let mut arguments = rest.splitn(2, char::is_whitespace);
    let action = arguments
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let argument = arguments.next().unwrap_or_default().trim();
    let action = if action.is_empty() {
        "list"
    } else {
        action.as_str()
    };
    let mut store = DataSourceStore::open_default()?;

    match action {
        "list" => {
            terminal_notice!(
                "{}",
                interactive_layer_rule(Some(ui_text(UiText::DataSourcesTitle)))
            );
            if store.sources().is_empty() {
                terminal_notice!("  {}", ui_text(UiText::DataSourceEmpty));
            } else {
                let conversation_id = config.session_id.as_deref().unwrap_or_default();
                for source in store.sources() {
                    let resolution = store.resolve(source, conversation_id);
                    terminal_notice!(
                        "  {}  {} · {} · {}: {:?} · {} · {} {}",
                        if resolution.usable { "●" } else { "○" },
                        source.name,
                        source.id,
                        ui_text(UiText::DataSourcePolicy),
                        source.access_policy,
                        ui_text(UiText::DataSourceVerified),
                        source.validation_contract.record_count,
                        ui_text(UiText::DataSourceRecords),
                    );
                    terminal_notice!(
                        "      {} · {}",
                        source.location,
                        interactive_toggle_text(resolution.enabled),
                    );
                }
            }
            if !store.usage_records().is_empty() {
                terminal_notice!(
                    "  {}: {} {}",
                    ui_text(UiText::DataSourcesUsage),
                    store.usage_records().len(),
                    ui_text(UiText::DataSourceRecords),
                );
            }
        }
        "add" => {
            if argument.is_empty() {
                return print_data_source_usage();
            }
            let kind = if argument.starts_with("http://") || argument.starts_with("https://") {
                HostDataSourceKind::Http
            } else {
                HostDataSourceKind::LocalFile
            };
            let name = data_source_display_name(argument);
            let source = store.validate_and_save(&DataSourceDraft {
                id: None,
                name,
                kind,
                location: argument.to_string(),
                access_policy: DataSourceAccessPolicy::ConversationOptIn,
                maximum_age_minutes: 60,
            })?;
            terminal_notice!(
                "  {} · {} · {}",
                ui_text(UiText::DataSourceAdded),
                source.name,
                source.id,
            );
        }
        "test" => {
            if argument.is_empty() {
                return print_data_source_usage();
            }
            let source = store.refresh(argument)?;
            terminal_notice!(
                "  {} · {} · {} {}",
                ui_text(UiText::DataSourceTested),
                source.name,
                source.validation_contract.record_count,
                ui_text(UiText::DataSourceRecords),
            );
        }
        "remove" => {
            if argument.is_empty() {
                return print_data_source_usage();
            }
            if store.remove(argument)? {
                terminal_notice!("  {} · {argument}", ui_text(UiText::DataSourceRemoved));
            } else {
                return Err(format!("unknown data source: {argument}"));
            }
        }
        "enable" | "disable" | "inherit" => {
            if argument.is_empty() {
                return print_data_source_usage();
            }
            let Some(conversation_id) = config.session_id.as_deref() else {
                terminal_notice!("  {}", ui_text(UiText::DataSourceNoSession));
                return Ok(());
            };
            let enabled = match action {
                "enable" => Some(true),
                "disable" => Some(false),
                _ => None,
            };
            store.set_conversation_override(conversation_id, argument, enabled)?;
            let message = match enabled {
                Some(true) => UiText::DataSourceEnabled,
                Some(false) => UiText::DataSourceDisabled,
                None => UiText::DataSourceInherited,
            };
            terminal_notice!("  {} · {argument}", ui_text(message));
        }
        _ => {
            terminal_notice!("  {}: {action}", ui_text(UiText::DataSourceUnknownAction));
            print_data_source_usage()?;
        }
    }
    Ok(())
}

fn print_data_source_usage() -> Result<(), String> {
    terminal_notice!(
        "  {}: /sources [list | add <path-or-url> | test <id> | remove <id> | enable <id> | disable <id> | inherit <id>]",
        ui_text(UiText::DataSourcesUsage),
    );
    Ok(())
}

fn data_source_display_name(location: &str) -> String {
    let trimmed = location.trim_end_matches('/');
    std::path::Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            trimmed
                .split("//")
                .nth(1)
                .and_then(|value| value.split('/').next())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| ui_text(UiText::DataSourcesTitle).to_string())
}

fn print_goal_mode_summary(goal: &str, constraints: Option<&str>) {
    let mut lines = vec![interactive_layer_rule(Some(ui_text(UiText::AutonomousGoal))),
        format!("  {}: {goal}", ui_text(UiText::AutonomousGoal))];
    if let Some(constraints) = constraints.filter(|value| !value.trim().is_empty()) {
        lines.push(format!("  {}: {constraints}", ui_text(UiText::Constraints)));
    }
    lines.push(format!("  {}", ui_text(UiText::GoalLifecycle)));
    crate::terminal_ui::commit_history(&format!("{}\n", lines.join("\n")));
}

fn begin_interactive_goal_input(target: &str, config: &Config,
    goal_target: &mut Option<String>, goal_constraints: &mut Option<String>) -> Result<SlashCommandResult, String> {
    if target.trim().is_empty() { return Ok(SlashCommandResult::GoalEntry); }
    if interactive_goal_snapshot(config, goal_target.as_deref(), goal_constraints.as_deref())?.is_some() {
        return Err(tr(Message::InteractiveGoalAlreadyActive).to_string());
    }
    // This is an explicit Goal form. Its text is an objective, never slash
    // syntax, flags, or a replacement for an ordinary chat message.
    *goal_target = Some(target.trim().to_string());
    *goal_constraints = None;
    print_goal_mode_summary(target.trim(), None);
    Ok(SlashCommandResult::RunGoalPrompt(target.trim().to_string()))
}

#[derive(Debug, Clone)]
struct InteractiveGoalSnapshot {
    target: String,
    constraints: String,
    status: String,
    stop_reason: Option<String>,
    attempt_count: u32,
    completed_requirements: usize,
    total_requirements: usize,
    next_wake_at_epoch_ms: Option<i64>,
}

fn goal_stop_reason_label(reason: nola_core::agent::goal::GoalStopReason) -> &'static str {
    use nola_core::agent::goal::GoalStopReason;
    match (i18n::current_language(), reason) {
        (i18n::Language::ZhHans, GoalStopReason::Completed) => "已完成",
        (i18n::Language::ZhHans, GoalStopReason::Incomplete) => "未完成",
        (i18n::Language::ZhHans, GoalStopReason::Suspended) => "已暂停",
        (i18n::Language::ZhHans, GoalStopReason::Scheduled) => "已计划重试",
        (i18n::Language::ZhHans, GoalStopReason::AwaitingUser) => "等待用户输入",
        (i18n::Language::ZhHans, GoalStopReason::Failed) => "执行失败",
        (i18n::Language::ZhHans, GoalStopReason::Cancelled) => "已取消",
        (i18n::Language::ZhHant, GoalStopReason::Completed) => "已完成",
        (i18n::Language::ZhHant, GoalStopReason::Incomplete) => "未完成",
        (i18n::Language::ZhHant, GoalStopReason::Suspended) => "已暫停",
        (i18n::Language::ZhHant, GoalStopReason::Scheduled) => "已排程重試",
        (i18n::Language::ZhHant, GoalStopReason::AwaitingUser) => "等待使用者輸入",
        (i18n::Language::ZhHant, GoalStopReason::Failed) => "執行失敗",
        (i18n::Language::ZhHant, GoalStopReason::Cancelled) => "已取消",
        (_, GoalStopReason::Completed) => "completed",
        (_, GoalStopReason::Incomplete) => "incomplete",
        (_, GoalStopReason::Suspended) => "suspended",
        (_, GoalStopReason::Scheduled) => "scheduled",
        (_, GoalStopReason::AwaitingUser) => "awaiting_user",
        (_, GoalStopReason::Failed) => "failed",
        (_, GoalStopReason::Cancelled) => "cancelled",
    }
}

fn interactive_goal_snapshot_from_state(
    goal: &nola_core::agent::goal::GoalState,
) -> InteractiveGoalSnapshot {
    let status = match goal.status {
        GoalStatus::Running => "running",
        GoalStatus::Paused => "paused",
        GoalStatus::NeedsUser => "needs_user",
        GoalStatus::Completed => "completed",
        GoalStatus::Failed => "failed",
        GoalStatus::Cancelled => "cancelled",
    };
    let completed_requirements = goal
        .requirements
        .values()
        .filter(|req| req.status == nola_core::agent::GoalRecordStatus::Completed)
        .count();
    let total_requirements = goal.requirements.len();
    InteractiveGoalSnapshot {
        target: goal.objective.clone(),
        constraints: goal.constraints.join("\n"),
        status: status.to_string(),
        stop_reason: goal
            .stop_reason
            .map(|r| goal_stop_reason_label(r).to_string()),
        attempt_count: goal.attempt_count,
        completed_requirements,
        total_requirements,
        next_wake_at_epoch_ms: goal.next_wake_at_epoch_ms,
    }
}

fn interactive_goal_snapshot(
    config: &Config,
    goal_target: Option<&str>,
    goal_constraints: Option<&str>,
) -> Result<Option<InteractiveGoalSnapshot>, String> {
    if let Some(session_id) = config.session_id.as_deref() {
        if let Some(projection) = session::load_session_goal_projection(session_id)? {
            if let Some(goal) = projection.active.filter(|goal| !goal.status.is_terminal()) {
                return Ok(Some(interactive_goal_snapshot_from_state(&goal)));
            }
            // Persisted completion/cancellation wins over a stale local draft.
            return Ok(None);
        }
    }
    Ok(goal_target.map(|target| InteractiveGoalSnapshot {
        target: target.to_string(),
        constraints: goal_constraints.unwrap_or_default().to_string(),
        status: "running".to_string(),
        stop_reason: None,
        attempt_count: 0,
        completed_requirements: 0,
        total_requirements: 0,
        next_wake_at_epoch_ms: None,
    }))
}

fn interactive_goal_snapshot_from_projection(
    projection: &nola_core::agent::goal::GoalProjection,
) -> Option<InteractiveGoalSnapshot> {
    let goal = projection
        .active
        .as_ref()
        .filter(|goal| !goal.status.is_terminal())?;
    Some(interactive_goal_snapshot_from_state(goal))
}

fn format_goal_wake_time(wake_at_epoch_ms: i64) -> String {
    let now_ms = goal_now_epoch_ms();
    let delta_secs = ((wake_at_epoch_ms - now_ms).max(0) + 500) / 1000;
    if delta_secs == 0 {
        return match i18n::current_language() {
            i18n::Language::ZhHans => "即将恢复".to_string(),
            i18n::Language::ZhHant => "即將恢復".to_string(),
            i18n::Language::De => "sofort".to_string(),
            i18n::Language::Es => "en breve".to_string(),
            i18n::Language::Fr => "imminent".to_string(),
            i18n::Language::Ja => "まもなく".to_string(),
            i18n::Language::Ko => "곧".to_string(),
            i18n::Language::Ru => "скоро".to_string(),
            i18n::Language::En => "shortly".to_string(),
        };
    }
    let formatted = format_elapsed_duration(Duration::from_secs(delta_secs as u64));
    match i18n::current_language() {
        i18n::Language::ZhHans => format!("{formatted} 后"),
        i18n::Language::ZhHant => format!("{formatted} 後"),
        i18n::Language::De => format!("in {formatted}"),
        i18n::Language::Es => format!("en {formatted}"),
        i18n::Language::Fr => format!("dans {formatted}"),
        i18n::Language::Ja => format!("{formatted} 後"),
        i18n::Language::Ko => format!("{formatted} 후"),
        i18n::Language::Ru => format!("через {formatted}"),
        i18n::Language::En => format!("in {formatted}"),
    }
}

fn submit_interactive_goal_control(config: &Config, action: nola_application::goal_control_intent::GoalControlAction) -> Result<bool, String> {
    let session = config.session_id.as_deref().ok_or("Goal requires a conversation")?;
    let projection = session::load_session_goal_projection(session)?.ok_or("Goal state is unavailable")?;
    let goal = projection.active.as_ref().filter(|goal| !goal.status.is_terminal())
        .ok_or_else(|| tr(Message::InteractiveGoalNoResumable).to_string())?;
    let intent = nola_application::goal_control_intent::PrepareGoalControlIntent {
        conversation_id: session.into(), operation_id: format!("cli-goal:{}", uuid::Uuid::new_v4()),
        action: action.clone(), expected_goal_id: Some(goal.id.clone()), expected_control_token: projection.control_token(),
        objective: String::new(), constraints: Vec::new(), wake_at_epoch_ms: None,
    };
    let confirmed = if matches!(action, nola_application::goal_control_intent::GoalControlAction::Resume)
        && goal.status != GoalStatus::Running
    {
        crate::core_agent_runtime::rebind_and_resume_goal(&config.host_invocation_request()?, intent)?
    } else {
        crate::goal_control_port::submit_existing(intent)?
    };
    if !confirmed { terminal_notice!("  {}", tr(Message::InteractiveGoalControlPending)); }
    Ok(confirmed)
}

fn interactive_goal_state_text(status: &str) -> String {
    match status {
        "running" => tr(Message::InteractiveGoalStateRunning).to_string(),
        "paused" => tr(Message::InteractiveGoalStatePaused).to_string(),
        "needs_user" => tr(Message::InteractiveGoalStateNeedsUser).to_string(),
        "failed" => tr(Message::InteractiveGoalStateFailed).to_string(),
        _ => status.to_string(),
    }
}

/// Parse only explicit command syntax; preserve the objective's Unicode,
/// spacing and newlines. Quoted examples and text after `--` are literal.
fn parse_goal_target_and_constraints(input: &str) -> Result<(String, Option<String>), String> {
    let input = input.trim();
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if escaped { escaped = false; continue; }
        if ch == '\\' { escaped = true; continue; }
        if matches!(ch, '\'' | '"') {
            if quote == Some(ch) { quote = None; }
            else if quote.is_none() && (index == 0 || input[..index].ends_with(char::is_whitespace)) { quote = Some(ch); }
            continue;
        }
        if quote.is_some() || (index > 0 && !input[..index].ends_with(char::is_whitespace)) { continue; }
        let rest = &input[index..];
        let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..token_end];
        if token == "--" {
            let target = format!("{}{}", &input[..index], rest[token_end..].trim_start());
            return validate_goal_input(target.trim().to_string(), None);
        }
        let constraints = if matches!(token, "-c" | "--constraints") {
            Some(rest[token_end..].trim())
        } else { token.strip_prefix("--constraints=").or_else(|| token.strip_prefix("-c="))
            .map(|_| rest.split_once('=').unwrap().1.trim()) };
        if let Some(constraints) = constraints {
            if constraints.is_empty() { return Err(tr(Message::InteractiveSlashGoalUsage).to_string()); }
            return validate_goal_input(input[..index].trim().to_string(), Some(constraints.to_string()));
        }
    }
    validate_goal_input(input.to_string(), None)
}

fn validate_goal_input(target: String, constraints: Option<String>) -> Result<(String, Option<String>), String> {
    if target.trim().is_empty() { return Err(tr(Message::InteractiveSlashGoalUsage).to_string()); }
    Ok((target, constraints))
}


fn select_interactive_model(config: &mut Config, model_id: &str) -> Result<(), String> {
    if let Some(id) = model_id.strip_prefix("configured:") {
        crate::conversation_invocation::select_profile(config, id)?;
        terminal_notice!("  {} {}", tr(Message::InteractiveModelStatusPrefix), config.model);
        return Ok(());
    }
    if !local_models::contains_model(model_id) {
        if let Ok(profiles) =
            crate::store::NolaStore::open_default().and_then(|store| store.model_profiles())
        {
            let needle = model_id.trim();
            if let Some(matched) = profiles.into_iter().find(|p| {
                p.enabled
                    && p.deleted_at_ms.is_none()
                    && p.runtime == "remote"
                    && (p.id == needle || p.model_name.eq_ignore_ascii_case(needle))
            }) {
                crate::conversation_invocation::select_profile(config, &matched.id)?;
                terminal_notice!("  {} {}", tr(Message::InteractiveModelStatusPrefix), config.model);
                return Ok(());
            }
        }
    }
    if config.runtime.model_binding.is_some() && !local_models::contains_model(model_id) {
        return Err("Use /model to select a configured connection and model".into());
    }
    if local_models::contains_model(model_id) {
        local_models::select_model(model_id)?;
        let profile = local_models::profile(model_id);
        terminal_notice!(
            "  {} {} · {} · {}",
            local_model_cli_text(LocalModelCliText::Selected),
            profile.map(|value| value.name).unwrap_or(model_id),
            if profile.map(local_models::is_installed).unwrap_or(true) {
                local_model_cli_text(LocalModelCliText::Installed)
            } else {
                local_model_cli_text(LocalModelCliText::NotInstalled)
            },
            local_model_cli_text(LocalModelCliText::StartHint)
        );
        return Ok(());
    }
    if local_models::running_model_id().is_some() {
        local_models::shutdown();
        restore_interactive_provider(config);
    }
    config.model = model_id.to_string();
    config.runtime.model = Some(model_id.to_string());
    terminal_notice!(
        "  {} {}",
        tr(Message::InteractiveModelStatusPrefix),
        config.model
    );
    Ok(())
}

fn start_interactive_local_model(config: &mut Config, model_id: &str) -> Result<(), String> {
    if !local_models::contains_model(model_id) {
        return Err(format!(
            "{}: {model_id}",
            local_model_cli_text(LocalModelCliText::UnknownModel)
        ));
    }
    if let Some(profile) =
        local_models::profile(model_id).filter(|profile| !local_models::is_installed(profile))
    {
        return Err(format!(
            "{} · {} ({}) · {}: /model install {}",
            profile.name,
            local_model_cli_text(LocalModelCliText::NotInstalled),
            local_models::format_bytes(profile.download_bytes),
            local_model_cli_text(LocalModelCliText::Usage),
            profile.id
        ));
    }
    local_models::select_model(model_id)?;
    remember_interactive_provider(config);
    let endpoint = local_models::start_model(model_id)?;
    config.endpoint = endpoint.base_url.clone();
    config.api_key = endpoint.api_key.clone();
    config.model = endpoint.model_id.clone();
    config.runtime.endpoint = Some(endpoint.base_url);
    config.runtime.api_key = Some(endpoint.api_key);
    config.runtime.model = Some(endpoint.model_id.clone());
    let engine = local_models::model_runtime(&endpoint.model_id).ok_or("Local model disappeared")?;
    config.runtime.provider = Some(if engine == "llama.cpp" { "llamacpp".into() } else { engine.clone() });
    config.runtime.siwc = None;
    config.runtime.selected_profile = None;
    config.runtime.model_binding = None;
    config.runtime.native_model_route = None;
    config.runtime.compaction.context_window_tokens = Some(endpoint.context_window_tokens);
    let runtime = local_models::profile(&endpoint.model_id)
        .map(|profile| {
            format!(
                " · ~{} runtime",
                local_models::format_bytes(profile.estimated_runtime_bytes)
            )
        })
        .unwrap_or_default();
    terminal_notice!(
        "  {} {} · {} · {}K{}",
        tr(Message::InteractiveModelStatusPrefix),
        endpoint.model_id,
        engine,
        endpoint.context_window_tokens / 1024,
        runtime
    );
    Ok(())
}

fn remember_interactive_provider(config: &Config) {
    let storage = INTERACTIVE_PROVIDER_BEFORE_LOCAL.get_or_init(|| Mutex::new(None));
    if let Ok(mut value) = storage.lock() {
        if value.is_none() {
            *value = Some(InteractiveProviderSnapshot {
                endpoint: config.endpoint.clone(),
                api_key: config.api_key.clone(),
                model: config.model.clone(),
                runtime_endpoint: config.runtime.endpoint.clone(),
                runtime_api_key: config.runtime.api_key.clone(),
                runtime_model: config.runtime.model.clone(),
                runtime_provider: config.runtime.provider.clone(),
                context_window_tokens: config.runtime.compaction.context_window_tokens,
                siwc: config.runtime.siwc.clone(),
                selected_profile: config.runtime.selected_profile.clone(),
                model_binding: config.runtime.model_binding.clone(),
                native_model_route: config.runtime.native_model_route.clone(),
            });
        }
    }
}

fn restore_interactive_provider(config: &mut Config) {
    let Some(storage) = INTERACTIVE_PROVIDER_BEFORE_LOCAL.get() else {
        return;
    };
    let Ok(mut value) = storage.lock() else {
        return;
    };
    let Some(previous) = value.take() else {
        return;
    };
    config.endpoint = previous.endpoint;
    config.api_key = previous.api_key;
    config.model = previous.model;
    config.runtime.endpoint = previous.runtime_endpoint;
    config.runtime.api_key = previous.runtime_api_key;
    config.runtime.model = previous.runtime_model;
    config.runtime.provider = previous.runtime_provider;
    config.runtime.compaction.context_window_tokens = previous.context_window_tokens;
    config.runtime.siwc = previous.siwc;
    config.runtime.selected_profile = previous.selected_profile;
    config.runtime.model_binding = previous.model_binding;
    config.runtime.native_model_route = previous.native_model_route;
}

fn configure_interactive_local_model(model_id: &str, assignments: &[&str]) -> Result<(), String> {
    if !local_models::contains_model(model_id) {
        return Err(format!("Unknown local model: {model_id}"));
    }
    let profile = local_models::profile(model_id);
    let mut parameters = local_models::parameters(model_id)?;
    if assignments.is_empty() {
        terminal_notice!(
            "  {} · context={} temperature={} top_p={} top_k={} min_p={} batch={} threads={} gpu_layers={} flash_attention={}",
            profile.map(|value| value.name).unwrap_or(model_id),
            parameters.context_tokens,
            parameters.temperature,
            parameters.top_p,
            parameters.top_k,
            parameters.min_p,
            parameters.batch_size,
            if parameters.threads == 0 { "auto".to_string() } else { parameters.threads.to_string() },
            parameters.gpu_layers,
            parameters.flash_attention
        );
        terminal_notice!(
            "  {}: /model config <id> top_k=40 min_p=0.05 context=65536 ...",
            local_model_cli_text(LocalModelCliText::Usage)
        );
        return Ok(());
    }
    for assignment in assignments {
        let (key, value) = assignment
            .split_once('=')
            .ok_or_else(|| format!("Invalid model parameter: {assignment}"))?;
        match key {
            "context" | "context_tokens" => {
                parameters.context_tokens = value
                    .parse()
                    .map_err(|_| format!("Invalid context: {value}"))?
            }
            "temperature" | "temp" => {
                parameters.temperature = value
                    .parse()
                    .map_err(|_| format!("Invalid temperature: {value}"))?
            }
            "top_p" => {
                parameters.top_p = value
                    .parse()
                    .map_err(|_| format!("Invalid top_p: {value}"))?
            }
            "top_k" => {
                parameters.top_k = value
                    .parse()
                    .map_err(|_| format!("Invalid top_k: {value}"))?
            }
            "min_p" => {
                parameters.min_p = value
                    .parse()
                    .map_err(|_| format!("Invalid min_p: {value}"))?
            }
            "batch" | "batch_size" => {
                parameters.batch_size = value
                    .parse()
                    .map_err(|_| format!("Invalid batch size: {value}"))?
            }
            "threads" => {
                parameters.threads = value
                    .parse()
                    .map_err(|_| format!("Invalid threads: {value}"))?
            }
            "gpu_layers" => {
                parameters.gpu_layers = value
                    .parse()
                    .map_err(|_| format!("Invalid GPU layers: {value}"))?
            }
            "flash_attention" | "flash" => {
                parameters.flash_attention = matches!(value, "1" | "true" | "on" | "yes")
            }
            _ => return Err(format!("Unknown model parameter: {key}")),
        }
    }
    let saved = local_models::save_parameters(model_id, parameters)?;
    terminal_notice!(
        "  {} {} · context={} · top_k={} · min_p={}",
        local_model_cli_text(LocalModelCliText::SavedParameters),
        profile.map(|value| value.name).unwrap_or(model_id),
        saved.context_tokens,
        saved.top_k,
        saved.min_p
    );
    Ok(())
}

fn print_local_model_status() {
    let memory = local_models::physical_memory_bytes()
        .map(local_models::format_bytes)
        .unwrap_or_else(|| "unknown".to_string());
    let backend = local_models::llama_server_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| local_model_cli_text(LocalModelCliText::NotInstalled).to_string());
    terminal_notice!(
        "  {} · llama.cpp: {backend} · {}: {memory} · {}: 64K",
        local_model_cli_text(LocalModelCliText::LocalModels),
        local_model_cli_text(LocalModelCliText::Memory),
        local_model_cli_text(LocalModelCliText::Context)
    );
    if let Some(models) = local_models::catalog_json()["models"].as_array() {
        for model in models {
            let id = model["id"].as_str().unwrap_or("unknown");
            let state = if model["running"].as_bool() == Some(true) {
                local_model_cli_text(LocalModelCliText::Running)
            } else if model["installed"].as_bool() == Some(true) {
                local_model_cli_text(LocalModelCliText::Installed)
            } else {
                local_model_cli_text(LocalModelCliText::NotInstalled)
            };
            let recommendation = if id == local_models::recommended_model_id() {
                local_model_cli_text(LocalModelCliText::Recommended)
            } else {
                ""
            };
            let selected = if model["selected"].as_bool() == Some(true) {
                local_model_cli_text(LocalModelCliText::SelectedState)
            } else {
                ""
            };
            let download = model["downloadBytes"]
                .as_u64()
                .map(local_models::format_bytes)
                .unwrap_or_else(|| "—".to_string());
            let runtime = model["estimatedRuntimeBytes"]
                .as_u64()
                .map(local_models::format_bytes)
                .unwrap_or_else(|| "—".to_string());
            terminal_notice!(
                "    {:<22} {:<13} {} {} · {} ~{}{}{}",
                id,
                state,
                local_model_cli_text(LocalModelCliText::Download),
                download,
                local_model_cli_text(LocalModelCliText::Runtime),
                runtime,
                recommendation,
                selected
            );
        }
    }
    if local_models::llama_server_path().is_none() {
        terminal_notice!("    {}", local_models::llama_install_hint());
    }
}

#[derive(Clone, Copy)]
enum LocalModelCliText {
    LocalModels,
    Selected,
    Installed,
    NotInstalled,
    Running,
    StartHint,
    Memory,
    Context,
    Download,
    Runtime,
    Recommended,
    SelectedState,
    Usage,
    SavedParameters,
    Stopped,
    Downloading,
    InstalledAction,
    RemovedAction,
    Api,
    Active,
    SelectBeforeStart,
    SelectBeforeConfigure,
    UnknownModel,
}

fn local_model_cli_text(key: LocalModelCliText) -> &'static str {
    let values = match key {
        LocalModelCliText::LocalModels => [
            "本地模型",
            "Local models",
            "Lokale Modelle",
            "Modelos locales",
            "Modèles locaux",
            "ローカルモデル",
            "Локальные модели",
        ],
        LocalModelCliText::Selected => [
            "已选择",
            "Selected",
            "Ausgewählt",
            "Seleccionado",
            "Sélectionné",
            "選択済み",
            "Выбрано",
        ],
        LocalModelCliText::Installed => [
            "已下载",
            "Downloaded",
            "Heruntergeladen",
            "Descargado",
            "Téléchargé",
            "ダウンロード済み",
            "Загружено",
        ],
        LocalModelCliText::NotInstalled => [
            "未下载",
            "Not downloaded",
            "Nicht heruntergeladen",
            "No descargado",
            "Non téléchargé",
            "未ダウンロード",
            "Не загружено",
        ],
        LocalModelCliText::Running => [
            "运行中",
            "Running",
            "Läuft",
            "En ejecución",
            "En cours",
            "実行中",
            "Запущено",
        ],
        LocalModelCliText::StartHint => [
            "使用 /model start 明确启动",
            "use /model start to launch",
            "mit /model start starten",
            "usa /model start para iniciar",
            "utilisez /model start pour démarrer",
            "/model start で起動",
            "запустите через /model start",
        ],
        LocalModelCliText::Memory => [
            "内存",
            "memory",
            "Speicher",
            "memoria",
            "mémoire",
            "メモリ",
            "память",
        ],
        LocalModelCliText::Context => [
            "上下文",
            "context",
            "Kontext",
            "contexto",
            "contexte",
            "コンテキスト",
            "контекст",
        ],
        LocalModelCliText::Download => [
            "下载",
            "download",
            "Download",
            "descarga",
            "téléchargement",
            "ダウンロード",
            "загрузка",
        ],
        LocalModelCliText::Runtime => [
            "运行估算",
            "runtime estimate",
            "Laufzeitschätzung",
            "estimación de ejecución",
            "estimation d’exécution",
            "実行時推定",
            "оценка памяти",
        ],
        LocalModelCliText::Recommended => [
            " · 推荐",
            " · recommended",
            " · empfohlen",
            " · recomendado",
            " · recommandé",
            " · 推奨",
            " · рекомендуется",
        ],
        LocalModelCliText::SelectedState => [
            " · 已选择",
            " · selected",
            " · ausgewählt",
            " · seleccionado",
            " · sélectionné",
            " · 選択済み",
            " · выбрано",
        ],
        LocalModelCliText::Usage => [
            "用法",
            "Usage",
            "Verwendung",
            "Uso",
            "Utilisation",
            "使い方",
            "Использование",
        ],
        LocalModelCliText::SavedParameters => [
            "已保存参数",
            "Saved parameters for",
            "Parameter gespeichert für",
            "Parámetros guardados para",
            "Paramètres enregistrés pour",
            "パラメータを保存",
            "Параметры сохранены для",
        ],
        LocalModelCliText::Stopped => [
            "本地模型已停止，已恢复之前的 API 配置",
            "Local model stopped; previous API configuration restored",
            "Lokales Modell gestoppt; vorherige API-Konfiguration wiederhergestellt",
            "Modelo local detenido; se restauró la configuración API anterior",
            "Modèle local arrêté ; configuration API précédente restaurée",
            "ローカルモデルを停止し、以前の API 設定を復元しました",
            "Локальная модель остановлена; предыдущая конфигурация API восстановлена",
        ],
        LocalModelCliText::Downloading => [
            "正在下载",
            "Downloading",
            "Wird heruntergeladen",
            "Descargando",
            "Téléchargement",
            "ダウンロード中",
            "Загрузка",
        ],
        LocalModelCliText::InstalledAction => [
            "已安装",
            "Installed",
            "Installiert",
            "Instalado",
            "Installé",
            "インストール済み",
            "Установлено",
        ],
        LocalModelCliText::RemovedAction => [
            "已删除",
            "Removed",
            "Entfernt",
            "Eliminado",
            "Supprimé",
            "削除済み",
            "Удалено",
        ],
        LocalModelCliText::Api => ["API", "API", "API", "API", "API", "API", "API"],
        LocalModelCliText::Active => [
            "已启用",
            "active",
            "aktiv",
            "activo",
            "actif",
            "有効",
            "активна",
        ],
        LocalModelCliText::SelectBeforeStart => [
            "请先选择一个本地模型，再明确启动。",
            "Select a local model before starting it.",
            "Wähle vor dem Start ein lokales Modell aus.",
            "Selecciona un modelo local antes de iniciarlo.",
            "Sélectionnez un modèle local avant de le démarrer.",
            "起動する前にローカルモデルを選択してください。",
            "Перед запуском выберите локальную модель.",
        ],
        LocalModelCliText::SelectBeforeConfigure => [
            "请先选择一个本地模型，再配置参数。",
            "Select a local model before configuring it.",
            "Wähle vor der Konfiguration ein lokales Modell aus.",
            "Selecciona un modelo local antes de configurarlo.",
            "Sélectionnez un modèle local avant de le configurer.",
            "設定する前にローカルモデルを選択してください。",
            "Перед настройкой выберите локальную модель.",
        ],
        LocalModelCliText::UnknownModel => [
            "未知的本地模型",
            "Unknown local model",
            "Unbekanntes lokales Modell",
            "Modelo local desconocido",
            "Modèle local inconnu",
            "不明なローカルモデル",
            "Неизвестная локальная модель",
        ],
    };
    match current_language() {
        Language::ZhHans => values[0],
        Language::De => values[2],
        Language::Es => values[3],
        Language::Fr => values[4],
        Language::Ja => values[5],
        Language::Ru => values[6],
        Language::En | _ => values[1],
    }
}

fn interactive_select_model(config: &Config) -> Result<Option<String>, String> {
    let models = available_interactive_models(Some(config), "");
    if models.is_empty() {
        terminal_notice!("  {}", ui_text(UiText::NoAvailableModels));
        return Ok(None);
    }

    let preferred_model = local_models::selected_model_id().unwrap_or_else(|| config.model.clone());
    let labels: Vec<String> = models.iter().map(|model| interactive_model_picker_label(model, config)).collect();
    let mut selected = models
        .iter()
        .enumerate()
        .position(|(idx, value)| {
            value == &preferred_model
                || config
                    .cli_profile_selection
                    .as_ref()
                    .is_some_and(|(id, _)| value == &format!("configured:{id}"))
                || labels
                    .get(idx)
                    .is_some_and(|label| label.starts_with(&format!("{} · ", config.model)))
        })
        .unwrap_or(0);

    let _modal = crate::interaction::acquire_terminal_modal();
    let _raw_mode = InteractiveRawModeGuard::new()
        .map_err(|err| format!("{} {err}", tr(Message::InteractiveRawModeFailedPrefix)))?;

    loop {
        draw_interactive_model_picker(&labels, selected)
            .map_err(|err| format!("{} {err}", tr(Message::FailedReadKeyEventPrefix)))?;

        let event =
            read().map_err(|err| format!("{0} {err}", tr(Message::FailedReadKeyEventPrefix)))?;
        if let Event::Key(key) = event {
            match crate::terminal_input::modal_key(&key) {
                crate::terminal_input::ModalKey::Cancel => {
                    clear_interactive_list_lines();
                    return Ok(None);
                }
                crate::terminal_input::ModalKey::Ignore => continue,
                crate::terminal_input::ModalKey::Action => {}
            }
            match key.code {
                KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => {
                    if selected == 0 {
                        selected = models.len() - 1;
                    } else {
                        selected -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => {
                    if selected + 1 >= models.len() {
                        selected = 0;
                    } else {
                        selected += 1;
                    }
                }
                KeyCode::Home => selected = 0,
                KeyCode::End => selected = models.len() - 1,
                KeyCode::PageUp => selected = selected.saturating_sub(10),
                KeyCode::PageDown => selected = selected.saturating_add(10).min(models.len() - 1),
                KeyCode::Enter => {
                    clear_interactive_list_lines();
                    return Ok(Some(models[selected].clone()));
                }
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    clear_interactive_list_lines();
                    return Ok(None);
                }
                _ => {}
            }
        }
    }
}

fn draw_interactive_model_picker(
    models: &[String],
    selected: usize,
) -> io::Result<()> {
    let (columns, rows) = crossterm::terminal::size()?;
    let panel_width = usize::from(columns).saturating_sub(2).clamp(6, 78);
    let inner_width = panel_width.saturating_sub(4).max(1);
    let use_color = interactive_color_enabled();
    let top = format!("╭{}╮", INTERACTIVE_PANEL_BORDER.repeat(panel_width - 2));
    let bottom = format!(
        "{INTERACTIVE_PANEL_BOTTOM_LEFT}{}{}",
        INTERACTIVE_PANEL_BORDER.repeat(panel_width - 2),
        INTERACTIVE_PANEL_BOTTOM_RIGHT
    );
    let header = tr(Message::InteractiveAvailableModels).trim_end_matches(':');
    let hint = crate::i18n::interaction_text().selection_hint;
    let visible = model_picker_visible_range(models.len(), selected, usize::from(rows));
    let mut frame = vec![top, interactive_picker_row(
        &truncate_display_width(&format!(" {header} · {}/{}", selected + 1, models.len()), inner_width),
        panel_width, use_color,
    )];
    for index in visible {
        let model = &models[index];
        let label = model;
        let marker = if index == selected { "›" } else { " " };
        frame.push(interactive_picker_row(
            &truncate_display_width(&format!("{marker} {label}"), inner_width),
            panel_width, use_color,
        ));
    }
    frame.push(interactive_picker_row(hint, panel_width, use_color));
    frame.push(bottom);
    crate::terminal_ui::present_permission_frame(&frame.join("\n"), true)
}

fn model_picker_visible_range(count: usize, selected: usize, rows: usize) -> std::ops::Range<usize> {
    let visible = rows.saturating_sub(6).clamp(1, 12).min(count);
    let start = selected.saturating_sub(visible / 2).min(count.saturating_sub(visible));
    start..start + visible
}

#[cfg(test)]
#[test]
fn model_picker_window_is_bounded_and_keeps_selection_visible() {
    for count in [1, 3, 20, 500] {
        for rows in [1, 8, 24, 100] {
            for selected in 0..count {
                let range = model_picker_visible_range(count, selected, rows);
                assert!(range.contains(&selected));
                assert!(range.len() <= 12);
                assert!(range.end <= count);
            }
        }
    }
    assert_eq!(model_picker_visible_range(0, 0, 24), 0..0);
}

fn interactive_model_picker_label(model_id: &str, config: &Config) -> String {
    let active_suffix = match current_language() {
        Language::ZhHans => "当前",
        _ => "current",
    };
    if let Some(id) = model_id.strip_prefix("configured:") {
        if let Ok(Some(profile)) = crate::store::NolaStore::open_default().and_then(|store| store.model_profile(id)) {
            let source: serde_json::Value = serde_json::from_str(&profile.source_json).unwrap_or_default();
            let provider = source["connectionID"]
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(profile.provider.as_str());
            let is_current = config
                .cli_profile_selection
                .as_ref()
                .is_some_and(|(selected_id, _)| selected_id == id)
                || config.model == profile.model_name;
            let current = if is_current {
                format!(" ({active_suffix})")
            } else {
                String::new()
            };
            return format!("{} · {}{current}", profile.model_name, provider);
        }
    }
    if let Some(profile) = local_models::profile(model_id) {
        let state = if local_models::running_model_id().as_deref() == Some(profile.id) {
            local_model_cli_text(LocalModelCliText::Running)
        } else if local_models::is_installed(profile) {
            local_model_cli_text(LocalModelCliText::Installed)
        } else {
            local_model_cli_text(LocalModelCliText::NotInstalled)
        };
        let selected = if local_models::selected_model_id().as_deref() == Some(profile.id) {
            format!(" · {}", local_model_cli_text(LocalModelCliText::Selected))
        } else {
            String::new()
        };
        return format!(
            "{} · {} · {}{}",
            profile.name,
            local_model_cli_text(LocalModelCliText::LocalModels),
            state,
            selected
        );
    }
    if local_models::contains_model(model_id) {
        let state = if local_models::running_model_id().as_deref() == Some(model_id) {
            local_model_cli_text(LocalModelCliText::Running)
        } else {
            local_model_cli_text(LocalModelCliText::Installed)
        };
        let selected = if local_models::selected_model_id().as_deref() == Some(model_id) {
            format!(" · {}", local_model_cli_text(LocalModelCliText::Selected))
        } else {
            String::new()
        };
        return format!(
            "{model_id} · {} · {state}{selected}",
            local_model_cli_text(LocalModelCliText::LocalModels)
        );
    }

    let active = if model_id == config.model {
        format!(" · {}", local_model_cli_text(LocalModelCliText::Active))
    } else {
        String::new()
    };
    format!(
        "{model_id} · {}{active}",
        local_model_cli_text(LocalModelCliText::Api)
    )
}

fn interactive_picker_row(text: &str, panel_width: usize, use_color: bool) -> String {
    let inner_width = panel_width.saturating_sub(4).max(1);
    let plain = truncate_display_width(text, inner_width);
    let padded = pad_right_width(&plain, inner_width);
    if use_color {
        if plain.starts_with('›') {
            format!(
                "{INTERACTIVE_DIM}│{INTERACTIVE_RESET}{INTERACTIVE_ACCENT} {padded} {INTERACTIVE_RESET}{INTERACTIVE_DIM}│{INTERACTIVE_RESET}"
            )
        } else {
            format!(
                "{INTERACTIVE_DIM}│{INTERACTIVE_RESET}{INTERACTIVE_VALUE} {padded} {INTERACTIVE_RESET}{INTERACTIVE_DIM}│{INTERACTIVE_RESET}"
            )
        }
    } else {
        format!("│ {padded} │")
    }
}

fn clear_interactive_list_lines() {
    crate::terminal_ui::clear_permission_frame();
}

fn interactive_toggle_text(enabled: bool) -> &'static str {
    if enabled {
        tr(Message::InteractiveStatusOn)
    } else {
        tr(Message::InteractiveStatusOff)
    }
}

fn parse_interactive_toggle(value: &str) -> Option<bool> {
    let lower = value.trim().to_lowercase();
    match lower.as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => {
            let localized_on = tr(Message::InteractiveStatusOn).trim().to_lowercase();
            let localized_off = tr(Message::InteractiveStatusOff).trim().to_lowercase();
            if !localized_on.is_empty() && lower == localized_on {
                Some(true)
            } else if !localized_off.is_empty() && lower == localized_off {
                Some(false)
            } else {
                None
            }
        }
    }
}

fn interactive_toggle_hint() -> String {
    format!(
        "{}/{}",
        tr(Message::InteractiveStatusOn),
        tr(Message::InteractiveStatusOff)
    )
}

type PermissionLevel = crate::permissions::PermissionProfile;

trait PermissionLevelPresentation {
    fn display(self) -> &'static str;
    fn description(self) -> &'static str;
    fn from_config(config: &Config) -> Self;
    fn apply(self, config: &mut Config);
}

impl PermissionLevelPresentation for PermissionLevel {
    fn display(self) -> &'static str {
        match self {
            Self::Cautious => tr(Message::InteractiveAllowPanelCautious),
            Self::AllowAll => tr(Message::InteractiveAllowPanelAllowAll),
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Cautious => tr(Message::InteractiveAllowPanelCautiousDesc),
            Self::AllowAll => tr(Message::InteractiveAllowPanelAllowAllDesc),
        }
    }

    fn from_config(config: &Config) -> Self {
        config.permission_profile.unwrap_or_default()
    }

    fn apply(self, config: &mut Config) {
        config.permission_profile = Some(self);
        std::env::set_var("NOLA_PERMISSION_PROFILE", self.canonical_name());
        config.auto_approve = self.auto_approve();
        config.network_policy = self.network_policy().to_string();
    }
}

const ALL_PERMISSION_LEVELS: [PermissionLevel; 2] = PermissionLevel::ALL;

#[derive(Debug, PartialEq, Eq)]
enum PermissionPickerAction {
    Scroll(bool),
    Select(usize),
    Confirm,
    Cancel,
    Ignore,
}

fn permission_picker_action(key: KeyEvent, selected: usize) -> PermissionPickerAction {
    use PermissionPickerAction::*;
    if let Some(down) = crate::interaction::modal_scroll_direction(&key) {
        return Scroll(down);
    }
    match crate::terminal_input::modal_key(&key) {
        crate::terminal_input::ModalKey::Cancel => return Cancel,
        crate::terminal_input::ModalKey::Ignore => return Ignore,
        crate::terminal_input::ModalKey::Action => {}
    }
    match key.code {
        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => Select((selected + ALL_PERMISSION_LEVELS.len() - 1) % ALL_PERMISSION_LEVELS.len()),
        KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => Select((selected + 1) % ALL_PERMISSION_LEVELS.len()),
        KeyCode::Char('1') => Select(0),
        KeyCode::Char('2') => Select(1),
        KeyCode::Enter => Confirm,
        KeyCode::Esc => Cancel,
        _ => Ignore,
    }
}

fn interactive_permission_panel(config: &mut Config) -> io::Result<()> {
    let current = PermissionLevel::from_config(config);
    llm::log_local_debug_payload(
        "permission_mode_panel",
        &json!({
            "type": "permission_mode_panel",
            "phase": "presented",
            "current": current.canonical_name(),
        }),
    );
    let mut selected_idx = ALL_PERMISSION_LEVELS
        .iter()
        .position(|l| *l == current)
        .unwrap_or(0);
    let levels = ALL_PERMISSION_LEVELS;

    let _modal = crate::interaction::acquire_terminal_modal();
    let _raw_mode = InteractiveRawModeGuard::new()?;

    draw_permission_picker_frame(&levels, selected_idx, current)?;

    loop {
        match read()? {
            Event::Key(key) => match permission_picker_action(key, selected_idx) {
                PermissionPickerAction::Scroll(down) => crate::terminal_ui::scroll_permission_frame(down),
                PermissionPickerAction::Select(index) => {
                    selected_idx = index;
                    draw_permission_picker_frame(&levels, selected_idx, current)?;
                }
                PermissionPickerAction::Confirm => {
                    crate::terminal_ui::clear_permission_frame();
                    apply_interactive_permission_level(config, levels[selected_idx]);
                    return Ok(());
                }
                PermissionPickerAction::Cancel => {
                    crate::terminal_ui::clear_permission_frame();
                    llm::log_local_debug_payload(
                        "permission_mode_panel",
                        &json!({
                            "type": "permission_mode_panel",
                            "phase": "cancelled",
                            "current": current.canonical_name(),
                        }),
                    );
                    crate::terminal_ui::commit_history(tr(Message::InteractiveAllowPanelCancelled));
                    return Ok(());
                }
                _ => {}
            },
            Event::Resize(_, _) => {
                draw_permission_picker_frame(&levels, selected_idx, current)?;
            }
            _ => {}
        }
    }
}

fn apply_interactive_permission_level(config: &mut Config, chosen: PermissionLevel) {
    chosen.apply(config);
    llm::log_local_debug_payload(
        "permission_mode_panel",
        &json!({
            "type": "permission_mode_panel",
            "phase": "selected",
            "selected": chosen.canonical_name(),
        }),
    );
    crate::terminal_ui::commit_history(&format!(
        "{} {}",
        tr(Message::InteractiveAllowPanelSelected),
        chosen.display()
    ));
}

fn draw_permission_picker_frame(
    levels: &[PermissionLevel],
    selected_idx: usize,
    current: PermissionLevel,
) -> io::Result<()> {
    let text = render_permission_picker(levels, selected_idx, current, terminal_columns());
    crate::terminal_ui::present_permission_frame(&text, true)
}

fn render_permission_picker(
    levels: &[PermissionLevel],
    selected_idx: usize,
    current: PermissionLevel,
    columns: usize,
) -> String {
    let color = interactive_color_enabled();
    let localized = crate::i18n::interaction_text();
    let inner = columns.saturating_sub(6).clamp(1, 76);
    let mut out = String::new();
    push_picker_border(&mut out, "╭", "╮", inner, INTERACTIVE_ACCENT, color);
    let mut row = |text: &str, tone: &str, selected: bool| {
        for line in crate::terminal_ui::wrap_styled_line(text, inner) {
            push_picker_line(&mut out, &pad_to_display_width(&line, inner), tone, color, selected);
        }
    };
    row(tr(Message::InteractiveAllowPanelTitle), INTERACTIVE_ACCENT, false);
    row(&format!("{} {}", localized.current_label, current.display()), INTERACTIVE_DIM, false);
    for (i, level) in levels.iter().enumerate() {
        let selected = i == selected_idx;
        let marker = if selected { "›" } else { " " };
        row(&format!("{marker} {}  {}", i + 1, level.display()), INTERACTIVE_ACCENT, selected);
    }
    row(levels[selected_idx].description(), INTERACTIVE_DIM, false);
    row(ui_text(UiText::PermissionDigitHint), INTERACTIVE_DIM, false);
    push_picker_border(&mut out, "╰", "╯", inner, INTERACTIVE_ACCENT, color);
    out
}

fn pad_to_display_width(value: &str, width: usize) -> String {
    let visible = display_width(value);
    if visible >= width {
        return value.to_string();
    }
    format!("{value}{}", " ".repeat(width - visible))
}

fn push_picker_border(
    out: &mut String,
    left: &str,
    right: &str,
    inner: usize,
    accent: &str,
    color: bool,
) {
    if color {
        out.push_str(accent);
    }
    out.push_str(left);
    out.push_str(&"─".repeat(inner + 2));
    out.push_str(right);
    if color {
        out.push_str("\x1b[0m");
    }
    out.push('\n');
}

fn push_picker_line(out: &mut String, text: &str, row_color: &str, color: bool, selected: bool) {
    if color {
        out.push_str("\x1b[2;39m│\x1b[0m ");
        if selected {
            out.push_str("\x1b[7;1m");
        } else {
            out.push_str(row_color);
        }
        out.push_str(text);
        out.push_str("\x1b[0m");
        out.push_str(" \x1b[2;39m│\x1b[0m\n");
    } else {
        out.push_str("│ ");
        out.push_str(text);
        out.push_str(" │\n");
    }
}

#[allow(dead_code)]
fn print_interactive_models(config: &Config) {
    terminal_notice!("{}", tr(Message::InteractiveAvailableModels));
    for model in available_interactive_models(Some(config), "") {
        if model == config.model {
            terminal_notice!("  * {model}");
        } else {
            terminal_notice!("    {model}");
        }
    }
    terminal_notice!("{}", tr(Message::InteractiveModelUsageHint));
}

fn print_interactive_help(query: &str) {
    let query = query.trim();
    let rows = interactive_help_rows();
    let visible: Vec<_> = if query.is_empty() {
        ["/new", "/resume", "/edit", "/model", "/allow", "/goal", "/guide"]
            .into_iter()
            .filter_map(|name| rows.iter().find(|(usage, _)| usage.split_whitespace().next() == Some(name)))
            .collect()
    } else if query.eq_ignore_ascii_case("all") {
        rows.iter().collect()
    } else {
        let requested = format!("/{}", query.trim_start_matches('/'));
        let command = normalize_interactive_command(&requested).unwrap_or(&requested);
        rows.iter().filter(|(usage, _)| usage.split_whitespace().next() == Some(command)).collect()
    };
    if visible.is_empty() {
        terminal_notice!("  {query} · {}", tr(Message::InteractiveUnknownCommandHint));
    } else {
        terminal_notice!("{}", tr(Message::InteractiveAvailableCommands));
        let visible: Vec<_> = visible.into_iter().map(|(usage, description)| {
            let label = if query.is_empty() { usage.split_whitespace().next().unwrap_or(usage) } else { usage };
            (label, description)
        }).collect();
        let width = visible.iter().map(|(label, _)| display_width(label)).max().unwrap_or(0);
        for (label, description) in visible {
            terminal_notice!("  {}  {description}", pad_right_width(label, width));
            if !query.is_empty() && !query.eq_ignore_ascii_case("all") {
                match label {
                    "/goal" => terminal_notice!("  {}", tr(Message::InteractiveSlashGoalUsage)),
                    "/model" => terminal_notice!("  {}", tr(Message::InteractiveModelUsageHint)),
                    _ => {}
                }
            }
        }
    }
    if !query.eq_ignore_ascii_case("all") {
        terminal_notice!("  {}", ui_text(UiText::HelpMoreCommands));
    }
}

fn interactive_help_rows() -> Vec<(String, String)> {
    vec![
        (
            "/help".to_string(),
            tr(Message::InteractiveSlashHelpHint).to_string(),
        ),
        (
            "/model".to_string(),
            tr(Message::InteractiveSlashModelHint).to_string(),
        ),
        (
            "/executor".to_string(),
            tr(Message::InteractiveSlashExecutorHint).to_string(),
        ),
        (
            format!("/debug {}", interactive_toggle_hint()),
            tr(Message::InteractiveSlashDebugHint).to_string(),
        ),
        (
            "/allow".to_string(),
            tr(Message::InteractiveSlashAllowHint).to_string(),
        ),
        (
            "/sources".to_string(),
            ui_text(UiText::HelpSources).to_string(),
        ),
        (
            "/attach <path>".to_string(),
            match current_language() {
                Language::ZhHans => "附加图片、PDF 或文本文件到当前消息",
                Language::ZhHant => "附加圖片、PDF 或文字檔案到目前訊息",
                Language::De => "Bild, PDF oder Textdatei an die Nachricht anhängen",
                Language::Es => "Adjuntar imagen, PDF o archivo de texto al mensaje",
                Language::Fr => "Joindre une image, un PDF ou un fichier texte au message",
                Language::Ja => "画像・PDF・テキストファイルをメッセージに添付",
                Language::Ko => "이미지, PDF 또는 텍스트 파일을 메시지에 첨부",
                Language::Ru => "Прикрепить изображение, PDF или текстовый файл к сообщению",
                Language::En => "Attach an image, PDF, or text file to the message",
            }
            .to_string(),
        ),
        ("/edit [message-id]".into(), ui_text(UiText::HelpEdit).into()),
        ("/cancel-edit".into(), ui_text(UiText::HelpCancelEdit).into()),
        (
            "/goal".to_string(),
            tr(Message::InteractiveSlashGoalHint).to_string(),
        ),
        (
            "/init [opts]".to_string(),
            tr(Message::InteractiveSlashInitHint).to_string(),
        ),
        (
            "/cd PATH".to_string(),
            tr(Message::InteractiveSlashCdHint).to_string(),
        ),
        (
            "/compact".to_string(),
            tr(Message::InteractiveSlashCompactHint).to_string(),
        ),
        (
            "/new".to_string(),
            tr(Message::InteractiveSlashNewHint).to_string(),
        ),
        (
            "/clear".to_string(),
            match current_language() {
                Language::ZhHans => "清空当前显示并开启新会话",
                _ => "Clear display and start a fresh conversation",
            }
            .to_string(),
        ),
        (
            "/plan".to_string(),
            tr(Message::InteractiveSlashPlanHint).to_string(),
        ),
        (
            "/mode [plan|build]".to_string(),
            match current_language() {
                Language::ZhHans => "切换只读 Plan 模式与可写 Build 模式（物理只读隔离）",
                _ => "Toggle read-only Plan mode and writable Build mode (physically enforced)",
            }
            .to_string(),
        ),
        (
            "/diff [--stat|PATH]".to_string(),
            match current_language() {
                Language::ZhHans => "查看当前工作区未提交的 Git 变更与 Diff",
                _ => "Inspect uncommitted Git status and colorized workspace diff",
            }
            .to_string(),
        ),
        (
            "/undo".to_string(),
            match current_language() {
                Language::ZhHans => "回滚工作区文件到上一轮对话前的快照（可用 /redo 恢复）",
                _ => "Restore workspace files to the pre-turn checkpoint (reversible with /redo)",
            }
            .to_string(),
        ),
        (
            "/redo".to_string(),
            match current_language() {
                Language::ZhHans => "恢复最近一次 /undo 撤销的工作区文件变更",
                _ => "Re-apply the most recently undone workspace checkpoint",
            }
            .to_string(),
        ),
        (
            "/checkpoints".to_string(),
            match current_language() {
                Language::ZhHans => "列出当前会话已记录的轮次文件快照",
                _ => "List captured per-turn workspace file checkpoints",
            }
            .to_string(),
        ),
        ("/agent".to_string(), ui_text(UiText::HelpAgent).to_string()),
        ("/room".to_string(), ui_text(UiText::HelpRoom).to_string()),
        ("/work".to_string(), ui_text(UiText::HelpWork).to_string()),
        ("/recover".to_string(), crate::unknown_effect_recovery::usage().to_string()),
        ("/tasks".to_string(), ui_text(UiText::HelpTasks).to_string()),
        ("/jobs".to_string(), ui_text(UiText::HelpJobs).to_string()),
        (
            "/stop".to_string(),
            match current_language() {
                Language::ZhHans => "停止当前运行或指定后台任务",
                _ => "Stop the active run or a background task",
            }
            .to_string(),
        ),
        (
            "/questions".to_string(),
            match current_language() {
                Language::ZhHans => "查看并回答待处理问题（含子任务问题）",
                _ => "Answer pending questions (including child tasks)",
            }
            .to_string(),
        ),
        (
            "/skills [list|enable|disable]".to_string(),
            match current_language() {
                Language::ZhHans => "查看或启用/停用 Skills",
                _ => "List, enable, or disable skills",
            }
            .to_string(),
        ),
        (
            "/mcp [list|enable|disable]".to_string(),
            match current_language() {
                Language::ZhHans => "查看或启用/停用 MCP 服务",
                _ => "List, enable, or disable MCP servers",
            }
            .to_string(),
        ),
        (
            "/memory [list|search|add]".to_string(),
            match current_language() {
                Language::ZhHans => "查看、搜索或添加长期记忆",
                _ => "List, search, or add memory entries",
            }
            .to_string(),
        ),
        (
            "/config".to_string(),
            crate::config_files::overview_label().to_string(),
        ),
        (
            "/guide".to_string(),
            match current_language() {
                Language::ZhHans => "显示快捷键与交互操作指南",
                _ => "Show keyboard shortcuts and interactive guide",
            }
            .to_string(),
        ),
        (
            "/resume [id]".to_string(),
            tr(Message::InteractiveSlashResumeHint).to_string(),
        ),
        (
            "/continue".to_string(),
            tr(Message::InteractiveSlashContinueHint).to_string(),
        ),
        (
            "/history".to_string(),
            tr(Message::InteractiveSlashHistoryHint).to_string(),
        ),
        (
            "/status".to_string(),
            tr(Message::InteractiveSlashStatusHint).to_string(),
        ),
        (
            "/exit".to_string(),
            tr(Message::InteractiveSlashExitHint).to_string(),
        ),
    ]
}

#[cfg(test)]
mod interactive_terminal_ui_contract_tests {
    use super::*;

    #[test]
    fn allow_all_picker_updates_typed_core_configuration_and_can_restrict_again() {
        let _lock = crate::store::test_env_lock();
        let directory = tempfile::tempdir().unwrap();
        struct RestoreEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for RestoreEnvironment {
            fn drop(&mut self) {
                for (name, previous) in &self.0 {
                    match previous {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
        let _environment = RestoreEnvironment(["NOLA_HOME", "NOLA_PERMISSION_PROFILE"].into_iter()
            .map(|name| (name, std::env::var_os(name))).collect());
        std::env::set_var("NOLA_HOME", directory.path());
        std::env::remove_var("NOLA_PERMISSION_PROFILE");
        let mut config = Config::from_args([
            "run".into(), "--project".into(), directory.path().to_string_lossy().into_owned(),
            "--endpoint".into(), "http://127.0.0.1:1/v1".into(),
            "--model".into(), "permission-picker-fixture".into(),
            "--api-key".into(), "fixture-not-used".into(),
        ]).unwrap();
        let mut observed = Vec::new();
        for profile in PermissionLevel::ALL {
            profile.apply(&mut config);
            observed.push((config.permission_profile, config.auto_approve, config.network_policy.clone()));
        }
        assert_eq!(observed, vec![
            (Some(PermissionLevel::Cautious), true, "ask".into()),
            (Some(PermissionLevel::AllowAll), true, "allow".into()),
        ]);
    }

    #[test]
    fn disabled_tools_override_the_requested_tool_set() {
        assert_eq!(
            interactive_tool_set_text(false, "all"),
            tr(Message::InteractiveStatusOff)
        );
    }

    #[test]
    fn returning_to_composer_is_a_goal_pause_terminal_reason() {
        assert!(terminal_reason_pauses_goal("cancelled"));
        assert!(terminal_reason_pauses_goal("returned_to_composer"));
        assert!(!terminal_reason_pauses_goal("failed"));
        assert!(!terminal_reason_pauses_goal("unknown"));
    }

    #[test]
    fn narrow_debug_footer_prioritizes_provider_and_working_directory() {
        let footer = format_interactive_footer_diagnostics(
            "provider",
            "api.example.com",
            "endpoint",
            "https://api.example.com/v1/chat/completions",
            "cwd",
            "~/Desktop/Nola/cli",
            Some("auto · tools=off · network=allow"),
            None,
            78,
        );
        assert!(footer.contains("tools=off"));
        assert!(footer.contains("api.example.com"));
        assert!(footer.contains("~/Desktop/Nola/cli"));
        assert!(!footer.contains("provider:"));
        assert!(!footer.contains("cwd:"));
        assert!(!footer.contains("endpoint:"));
        assert!(display_width(&footer) <= 78);
    }

    #[test]
    fn help_rows_share_one_description_column() {
        let rows = interactive_help_rows();
        let width = rows
            .iter()
            .map(|(command, _)| display_width(command))
            .max()
            .unwrap();
        let starts = rows
            .iter()
            .map(|(command, _)| display_width(&pad_right_width(command, width)) + 2)
            .collect::<Vec<_>>();
        assert!(starts.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn debug_toggle_accepts_canonical_and_localized_values() {
        let _lock = crate::store::test_env_lock();
        let previous = current_language();
        struct RestoreLang(Language);
        impl Drop for RestoreLang {
            fn drop(&mut self) {
                i18n::set_language(self.0);
            }
        }
        let _restore = RestoreLang(previous);

        i18n::set_language(Language::En);
        assert_eq!(parse_interactive_toggle("on"), Some(true));
        assert_eq!(parse_interactive_toggle("OFF"), Some(false));
        assert_eq!(interactive_toggle_text(true), tr(Message::InteractiveStatusOn));
        assert_eq!(interactive_toggle_text(false), tr(Message::InteractiveStatusOff));
        assert_eq!(interactive_toggle_hint(), "on/off");

        i18n::set_language(Language::ZhHans);
        assert_eq!(parse_interactive_toggle("开启"), Some(true));
        assert_eq!(parse_interactive_toggle("关闭"), Some(false));
        assert_eq!(parse_interactive_toggle("on"), Some(true));
        assert_eq!(parse_interactive_toggle("off"), Some(false));
        assert_eq!(parse_interactive_toggle("invalid"), None);
        assert_eq!(interactive_toggle_text(true), "开启");
        assert_eq!(interactive_toggle_text(false), "关闭");
        assert_eq!(interactive_toggle_hint(), "开启/关闭");
    }

    #[test]
    fn footer_prints_external_executor_once() {
        let _lock = crate::store::test_env_lock();
        let mut config = Config::for_test().unwrap();
        config.model = "gpt-5".into();
        config.execution_backend = nola_domain::execution_backend::ExecutionBackendKind::Codex.id().to_string();
        let footer = interactive_prompt_footer_line(&config, None, None).unwrap();
        let label = format!("{}:", tr(Message::InteractiveExecutorLabel));
        assert_eq!(footer.matches(&label).count(), 1, "footer should print Executor once: {footer}");
    }

    #[test]
    fn canonical_slash_commands_and_help_exclude_removed_aliases() {
        let forbidden = ["/permissions", "/resources", "/quit", "/experts"];
        for alias in forbidden {
            assert!(
                !INTERACTIVE_SLASH_COMMANDS.iter().any(|cmd| cmd.name == alias),
                "INTERACTIVE_SLASH_COMMANDS should not contain removed alias {alias}"
            );
            assert!(
                !interactive_help_rows()
                    .iter()
                    .any(|(usage, _)| usage.split_whitespace().next() == Some(alias)),
                "interactive_help_rows should not contain removed alias {alias}"
            );
            assert!(
                normalize_interactive_command(alias).is_none(),
                "removed alias {alias} should not normalize"
            );
        }
        assert_eq!(normalize_interactive_command("/EDIT"), Some("/edit"));
        assert_eq!(normalize_interactive_command("/Cancel-Edit"), Some("/cancel-edit"));
        assert_eq!(normalize_interactive_command("/QUESTIONS"), Some("/questions"));
        assert!(is_resume_command("/resume"));
        assert!(is_resume_command("/resume ses-42"));
    }
}

fn interactive_color_enabled() -> bool {
    io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM")
            .map(|value| !value.eq_ignore_ascii_case("dumb"))
            .unwrap_or(true)
}

fn terminal_columns() -> usize {
    if let Ok((columns, _)) = crossterm::terminal::size() {
        if columns > 0 {
            return columns as usize;
        }
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(100)
}

fn truncate_display_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let mut out = String::new();
    let mut width = 0usize;
    let budget = max_width.saturating_sub(1);

    for ch in text.chars() {
        let ch_width = unicode_char_width(ch);
        if width.saturating_add(ch_width) > budget {
            break;
        }
        width += ch_width;
        out.push(ch);
    }

    if out.is_empty() {
        "…".to_string()
    } else {
        out.push('…');
        out
    }
}

fn truncate_display_width_tail(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let budget = max_width.saturating_sub(1);
    let mut width = 0usize;
    let mut chars = Vec::new();
    for ch in text.chars().rev() {
        let ch_width = unicode_char_width(ch);
        if width.saturating_add(ch_width) > budget {
            break;
        }
        width += ch_width;
        chars.push(ch);
    }
    if chars.is_empty() {
        return "…".to_string();
    }
    chars.reverse();
    let mut out = String::from("…");
    out.extend(chars);
    out
}

fn truncate_display_width_middle(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let left_budget = max_width / 2;
    let right_budget = max_width.saturating_sub(left_budget).saturating_sub(1);
    let left = truncate_display_width_no_ellipsis(text, left_budget);
    let right = truncate_display_width_tail(text, right_budget);
    format!("{}…{}", left, right.trim_start_matches('…'))
}

fn compact_endpoint_for_footer(endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    if let Some((scheme, rest)) = endpoint.split_once("://") {
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        if path.is_empty() {
            format!("{scheme}://{host}")
        } else {
            format!(
                "{scheme}://{host}/{}",
                truncate_display_width_tail(path, 28)
            )
        }
    } else {
        truncate_display_width_tail(endpoint, 48)
    }
}

fn truncate_display_width_no_ellipsis(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(text) <= max_width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = unicode_char_width(ch);
        if width.saturating_add(ch_width) > max_width {
            break;
        }
        width += ch_width;
        out.push(ch);
    }
    out
}

fn is_path_separator(ch: char) -> bool {
    ch == '/' || ch == '\\'
}

fn compact_home_path(text: &str) -> String {
    let Ok(home) = std::env::var("HOME") else {
        return text.to_string();
    };
    if home.is_empty() || home == "/" {
        return text.to_string();
    }
    if text == home {
        return "~".to_string();
    }
    let Some(rest) = text.strip_prefix(&home) else {
        return text.to_string();
    };
    if rest.starts_with('/') {
        format!("~{rest}")
    } else {
        text.to_string()
    }
}

fn truncate_display_path(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let compacted = compact_home_path(text);
    let text = compacted.as_str();
    if text.is_empty() || display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let sep = text
        .char_indices()
        .rev()
        .find_map(|(index, ch)| is_path_separator(ch).then_some((index, ch)));
    let Some((sep_index, sep_char)) = sep else {
        return truncate_display_width(text, max_width);
    };
    let sep_len = sep_char.len_utf8();
    let filename_start = sep_index + sep_len;
    if filename_start >= text.len() {
        return truncate_display_width(text, max_width);
    }

    let separator = &text[sep_index..filename_start];
    let directory = &text[..sep_index];
    let filename = &text[filename_start..];
    let filename_width = display_width(filename);
    let separator_width = display_width(separator);
    if filename_width + separator_width >= max_width.saturating_sub(1) {
        return truncate_display_width(filename, max_width);
    }

    let budget = max_width
        .saturating_sub(separator_width)
        .saturating_sub(filename_width)
        .saturating_sub(1);
    let prefix = if budget > 0 {
        truncate_display_width_no_ellipsis(directory, budget)
    } else {
        String::new()
    };
    if prefix.is_empty() {
        if separator_width + filename_width <= max_width {
            return format!("{separator}{filename}");
        }
        return truncate_display_width(filename, max_width.saturating_sub(separator_width));
    }
    format!("{prefix}…{separator}{filename}")
}

use crate::goal_runtime::{PromptRunResult, GoalCycleOutcome, GoalCyclePause,
    goal_now_epoch_ms};
#[cfg(test)]
use crate::goal_runtime::{terminal_reason_pauses_goal, compose_goal_prompt, goal_retry_delay_for_attempt, goal_requirement_records};

thread_local! {
    static LIVE_PERMISSION_CHANGE: std::cell::Cell<Option<PermissionLevel>> = const { std::cell::Cell::new(None) };
    static LIVE_MODEL_CHANGE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static LIVE_THINKING_CHANGE: std::cell::RefCell<Option<Option<nola_core::agent::reasoning::ReasoningSelection>>> = const { std::cell::RefCell::new(None) };
}

#[derive(Debug)]
struct LiveCliCommands {
    config: std::sync::Mutex<Config>,
    permission_change: std::sync::Mutex<Option<PermissionLevel>>,
    model_change: std::sync::Mutex<Option<String>>,
    thinking_change: std::sync::Mutex<Option<Option<nola_core::agent::reasoning::ReasoningSelection>>>,
    runtime: crate::runtime_control::RuntimeExecutionControl,
    execution_id: u64,
}
impl interaction::ClientCommandHandler for LiveCliCommands {
    fn execute(&self, input: &str) {
        let mut config = self.config.lock().unwrap_or_else(|p| p.into_inner());
        let command = input.split_whitespace().next().unwrap_or("");
        let command = normalize_interactive_command(command).unwrap_or(command);
        let result = match command {
            "/stop" => {
                self.runtime.request();
                crate::terminal_ui::mark_turn_cancelling(self.execution_id);
                crate::terminal_ui::commit_history(match current_language() {
                    Language::ZhHans => "  正在停止当前运行…",
                    _ => "  Stopping active run…",
                });
                Ok(())
            }
            "/allow" => {
                let before = PermissionLevel::from_config(&config);
                let result = interactive_permission_panel(&mut config).map_err(|e| e.to_string());
                let after = PermissionLevel::from_config(&config);
                if before != after {
                    *self.permission_change.lock().unwrap_or_else(|p| p.into_inner()) = Some(after);
                    crate::terminal_ui::commit_history(match current_language() {
                        Language::ZhHans => "权限模式将在下一轮请求生效；当前待审批操作仍需单独确认。",
                        _ => "Permission mode applies to the next turn; pending operations still require their own decision.",
                    });
                }
                result
            }
            "/model" => {
                let mut parts = input.split_whitespace();
                let _ = parts.next();
                let chosen: Result<Option<String>, String> = match parts.next() {
                    None => interactive_select_model(&config),
                    Some(arg)
                        if arg.eq_ignore_ascii_case("status")
                            || arg.eq_ignore_ascii_case("local") =>
                    {
                        print_local_model_status();
                        Ok(None)
                    }
                    Some(arg)
                        if matches!(
                            arg.to_ascii_lowercase().as_str(),
                            "install" | "remove" | "start" | "stop" | "config"
                        ) =>
                    {
                        crate::terminal_ui::commit_history(&format!(
                            "{} {}",
                            input.trim(),
                            match current_language() {
                                Language::ZhHans => "· 本地模型管理命令请等当前任务结束或按 Esc 后执行。",
                                _ => "· Wait for the active turn to finish or press Esc before running local model management commands.",
                            }
                        ));
                        Ok(None)
                    }
                    Some(arg) => Ok(Some(arg.to_string())),
                };
                match chosen {
                    Ok(Some(selected_model)) => {
                        let label = interactive_model_picker_label(&selected_model, &config);
                        *self.model_change.lock().unwrap_or_else(|p| p.into_inner()) = Some(selected_model);
                        crate::terminal_ui::commit_history(&match current_language() {
                            Language::ZhHans => format!("  模型已设为 {label}，将在下一轮请求生效。"),
                            _ => format!("  Model set to {label}; applies to the next turn."),
                        });
                        Ok(())
                    }
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                }
            }
            "/thinking" => {
                let mut parts = input.split_whitespace();
                let _ = parts.next();
                match parts.next() {
                    None => {
                        let current = config
                            .runtime
                            .reasoning_effort_display()
                            .unwrap_or_else(|| "default".to_string());
                        crate::terminal_ui::commit_history(&match current_language() {
                            Language::ZhHans => format!("  当前思考等级：{current} · 用法：/thinking <off|low|medium|high|xhigh|max|default>"),
                            _ => format!("  Reasoning effort: {current} · Usage: /thinking <off|low|medium|high|xhigh|max|default>"),
                        });
                        Ok(())
                    }
                    Some(raw) => match crate::runtime_config::parse_reasoning_effort_selection(raw) {
                        Ok(selection) => {
                            config.runtime.reasoning_selection = selection.clone();
                            *self.thinking_change.lock().unwrap_or_else(|p| p.into_inner()) = Some(selection);
                            let label = config
                                .runtime
                                .reasoning_effort_display()
                                .unwrap_or_else(|| "default".to_string());
                            crate::terminal_ui::commit_history(&match current_language() {
                                Language::ZhHans => format!("  思考等级已设为 {label}，将在下一轮请求生效。"),
                                _ => format!("  Reasoning effort set to {label}; applies to the next turn."),
                            });
                            Ok(())
                        }
                        Err(error) => Err(error),
                    },
                }
            }
            "/help" => {
                print_interactive_help(input.trim().split_once(char::is_whitespace).map_or("", |(_, query)| query));
                Ok(())
            }
            "/status" => print_interactive_prompt_card(&config, 0),
            "/history" => print_interactive_session_history(&config),
            "/resume" => {
                self.runtime.request();
                crate::terminal_ui::mark_turn_cancelling(self.execution_id);
                crate::terminal_ui::commit_history(match current_language() {
                    Language::ZhHans => "  正在停止当前任务并切换会话…",
                    _ => "  Stopping active run to switch session…",
                });
                Ok(())
            }
            _ => {
                crate::terminal_ui::commit_history(&format!("{} {}", input.trim(), match current_language() {
                    Language::ZhHans => "· 当前任务运行中，请先按 Esc 返回输入后执行此命令；命令未排队。",
                    _ => "· A task is active. Press Esc to return to the prompt before using this command; it was not queued.",
                }));
                Ok(())
            }
        };
        if let Err(error) = result { crate::terminal_ui::commit_history(&format_error(&error)); }
    }
}

/// Terminal ownership stays in the product adapter. The listener is dropped
/// before callers drain drafts or wait for input release, including on errors.
fn with_cli_execution_control<T>(
    config: &Config,
    cancellation: interaction::ExecutionCancellation,
    emitter: &mut Emitter,
    run: impl FnOnce(crate::runtime_control::RuntimeExecutionControl, &mut Emitter) -> Result<T, String>,
) -> Result<T, String> {
    if let Some(owner) = interactive_composer_key(config)? {
        interaction::adopt_interactive_composer(owner);
    }
    // A newly submitted conversation may only receive its ID during
    // prepare_submission, after the outer composer opened its view store.
    let _new_session_view = if crate::cli_view_state::active_store().is_none()
        && config.save_session && !config.json_output
    {
        let (store, _) = open_interactive_view_state(config);
        Some(crate::cli_view_state::ActiveView::bind(store))
    } else { None };
    let commands = std::sync::Arc::new(LiveCliCommands {
        config: std::sync::Mutex::new(config.clone()),
        permission_change: Default::default(),
        model_change: Default::default(),
        thinking_change: Default::default(),
        runtime: cancellation.runtime_control(),
        execution_id: cancellation.execution_id(),
    });
    let cancellation = cancellation.with_client_commands(commands.clone());
    let _scope = cancellation.enter();
    let _signals = crate::execution_signals::ExecutionSignals::install(
        cancellation.runtime_control(), None,
    )?;
    let _listener = interaction::start_execution_cancel_listener(&cancellation);
    emitter.attach_execution_cancellation(cancellation.clone());
    let result = run(cancellation.runtime_control(), emitter);
    drop(_listener);
    if let Some(change) = commands.permission_change.lock().unwrap_or_else(|p| p.into_inner()).take() {
        LIVE_PERMISSION_CHANGE.with(|pending| pending.set(Some(change)));
    }
    if let Some(model) = commands.model_change.lock().unwrap_or_else(|p| p.into_inner()).take() {
        LIVE_MODEL_CHANGE.with(|pending| *pending.borrow_mut() = Some(model));
    }
    if let Some(thinking) = commands.thinking_change.lock().unwrap_or_else(|p| p.into_inner()).take() {
        LIVE_THINKING_CHANGE.with(|pending| *pending.borrow_mut() = Some(thinking));
    }
    result
}

fn prepare_cli_prompt<'a>(config: &Config, prompt: &'a str) -> Result<(Config, std::borrow::Cow<'a, str>), String> {
    let selected_config = crate::conversation_invocation::prepare_submission(config)?;
    if selected_config.main_expert_selection.is_some() && (!config.save_session || !config.tools_enabled) {
        return Err("Expert mentions require a saved conversation with tools enabled.".into());
    }
    Ok((selected_config, std::borrow::Cow::Borrowed(prompt)))
}

/// The terminal listener has joined before this handoff. Every Enter it
/// accepted is now either in the predecessor's Core checkpoint or in the
/// original Application FIFO. Only the latter still needs a foreground run.
fn finish_cli_foreground(
    config: &Config,
    mut cancellation: interaction::ExecutionCancellation,
    emitter: &mut Emitter,
    mut outcome: core_agent_runtime::CoreRunOutcome,
) -> Result<core_agent_runtime::CoreRunOutcome, String> {
    let Some(session) = config.session_id.as_deref().filter(|_| config.save_session) else {
        return Ok(outcome);
    };
    let mut admitted = std::collections::BTreeSet::new();
    while outcome.turn_status == nola_core::agent::AgentTurnStatus::Completed && !cancellation.is_requested() {
        admitted.extend(cancellation.runtime_control().take_admitted_followups(session));
        if admitted.is_empty() { break; }
        core_agent_runtime::drain_parent_agent_task_inbox(session)?;
        let Some(lease) = crate::agent_command_port::claim_authored_followup(session, &admitted)? else { break; };
        let command_id = lease.command_id.clone();
        let lease_owner = lease.lease_owner.clone();
        let request_id = format!("queued-admission:{}:{}", command_id, lease.delivery_attempt);
        // Each new turn observes its own configuration and transcript parent.
        // A mention on the preceding message belongs only to that message.
        let mut next = config.clone();
        next.main_expert_selection = None;
        cancellation = interaction::ExecutionCancellation::interactive();
        let result = (|| {
            let next = crate::conversation_invocation::prepare_submission(&next)?;
            let request = next.host_invocation_request()?;
            with_cli_execution_control(&next, cancellation.clone(), emitter, |control, emitter|
                core_agent_runtime::run_queued_user_input(&request, lease, request_id, emitter, control,
                    nola_core::context_engine::ContextPlatform::Cli, Vec::new()))
        })();
        if let Err(error) = &result {
            if nola_service_client::ApplicationServiceCommandError::unresolved_from_boundary_message(error).is_none() {
                crate::agent_command_port::release_agent_command_lease(session, &command_id, &lease_owner)?;
            }
        }
        outcome = result?;
        admitted.remove(&command_id);
    }
    Ok(outcome)
}

fn run_single_prompt(config: &Config, prompt: &str) -> Result<PromptRunResult, String> {
    let (selected_config, text) = prepare_cli_prompt(config, prompt)?;
    let prompt = text.as_ref();
    let config = &selected_config;
    // Debug mode enriches the durable trace and log files; it must not expose
    // internal lifecycle messages or change the interactive rendering model.
    let (turn_config, used_data_sources) =
        prepare_cli_data_source_context_or_fallback(config, prompt);
    let mut emitter = Emitter::new(config.json_output).with_quiet_status(true);
    // Give the terminal immediate feedback while Core prepares context and
    // waits for the first model event. Without this, a later turn can appear
    // inert for several seconds even though its Run was already accepted.
    emitter.status("processing", "");
    let cancellation = if config.json_output {
        interaction::ExecutionCancellation::default()
    } else {
        interaction::ExecutionCancellation::interactive()
    };
    let request = turn_config.host_invocation_request()?;
    let run_outcome = with_cli_execution_control(config, cancellation.clone(), &mut emitter, |control, emitter|
        core_agent_runtime::run_agent_core_with_cancellation(&request, prompt, emitter, control))?;
    let run_outcome = finish_cli_foreground(config, cancellation, &mut emitter, run_outcome)?;
    record_cli_data_source_usage(config, &used_data_sources);
    emitter.wait_for_execution_input_release();
    let session_id = emitter.session_id().map(str::to_string);
    let terminal_reason = emitter.take_terminal_reason();
    // Custom permission intentionally ends the current tool run and hands
    // control back to the main composer. Input typed during that hand-off is
    // still normal user input and must be replayed exactly once on the next
    // loop iteration rather than being discarded here.
    let queued_user_inputs = emitter.take_queued_user_inputs();
    if let Some(session_id) = session_id.as_deref() {
        core_agent_runtime::drain_parent_agent_task_inbox(session_id)?;
    }
    let draft_user_input = emitter.take_draft_user_input();
    let context_usage = emitter.take_context_usage();
    let output = emitter.take_markdown_text();
    Ok(PromptRunResult {
        goal_signal: run_outcome.goal_signal.clone(),
        run_outcome,
        output,
        terminal_reason,
        session_id,
        queued_user_inputs,
        draft_user_input,
        context_usage,
    })
}

fn run_single_prompt_with_execution_cancellation(
    config: &Config,
    prompt: &str,
    cancellation: interaction::ExecutionCancellation,
) -> Result<PromptRunResult, String> {
    let (selected_config, text) = prepare_cli_prompt(config, prompt)?;
    let prompt = text.as_ref();
    let config = &selected_config;
    let prompt_started_at = std::time::Instant::now();
    let (turn_config, used_data_sources) =
        prepare_cli_data_source_context_or_fallback(config, prompt);
    let data_sources_ready_at = std::time::Instant::now();
    if std::env::var_os("NOLA_CLI_DEBUG").is_some() {
        eprintln!(
            "nola_prompt_preparation_timing data_sources_ms={} total_ms={}",
            data_sources_ready_at
                .duration_since(prompt_started_at)
                .as_millis(),
            data_sources_ready_at
                .duration_since(prompt_started_at)
                .as_millis(),
        );
    }
    let mut emitter = Emitter::new(config.json_output).with_quiet_status(true);
    emitter.status("processing", "");
    let request = turn_config.host_invocation_request()?;
    let run_outcome = with_cli_execution_control(config, cancellation, &mut emitter, |control, emitter|
        core_agent_runtime::run_agent_core_with_cancellation(&request, prompt, emitter, control))?;
    record_cli_data_source_usage(config, &used_data_sources);
    emitter.wait_for_execution_input_release();
    let session_id = emitter.session_id().map(str::to_string);
    let queued_user_inputs = emitter.take_queued_user_inputs();
    let draft_user_input = emitter.take_draft_user_input();
    let context_usage = emitter.take_context_usage();
    let output = emitter.take_markdown_text();
    Ok(PromptRunResult {
        goal_signal: run_outcome.goal_signal.clone(),
        run_outcome,
        output,
        terminal_reason: emitter.take_terminal_reason(),
        session_id,
        queued_user_inputs,
        draft_user_input,
        context_usage,
    })
}

fn resume_cli_core_pending(
    config: &Config,
    emitter: &mut Emitter,
    control: crate::runtime_control::RuntimeExecutionControl,
) -> Result<Option<crate::core_agent_runtime::CoreRunOutcome>, String> {
    let session = config.session_id.as_deref().ok_or("resume requires an explicit canonical session")?;
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    if let Some(active) = application.active_run(session)? {
        return crate::core_agent_runtime::resume_frozen_core_agent_run(session, &active.run_id, emitter, control);
    }
    if control.is_requested() { return Ok(None); }
    // Promotion preserves the original accepted input and its execution
    // descriptor. Reopening a conversation must not replace either with this
    // process's default tool/model settings.
    let successor: Option<nola_application::AcceptedAssistantTurn> = application.execute(
        nola_application::ApplicationCommand::PromoteQueuedAssistantTurn {
            input: nola_application::PromoteQueuedAssistantTurn {
                conversation_id: session.into(), created_at_ms: goal_now_epoch_ms(),
            },
        })?;
    match successor {
        Some(successor) => crate::core_agent_runtime::resume_frozen_core_agent_run(
            session, &successor.begin.run_id, emitter, control),
        None => Ok(None),
    }
}

#[derive(Debug, Clone)]
struct UsedCliDataSource {
    id: String,
    connector_id: String,
    truncated: bool,
}

fn prepare_cli_data_source_context_or_fallback(
    config: &Config,
    prompt: &str,
) -> (Config, Vec<UsedCliDataSource>) {
    match prepare_cli_data_source_context(config, prompt) {
        Ok(prepared) => prepared,
        Err(error) => {
            // Context is optional evidence, not the transaction boundary for
            // the user's message. A malformed or temporarily unavailable
            // source fails closed by contributing no data; it must not abort
            // the Turn before Core can durably accept the submission.
            if config.debug_mode {
                eprintln!("debug: context_prepare_failed error={error}");
            }
            (config.clone(), Vec::new())
        }
    }
}

fn prepare_cli_data_source_context(
    config: &Config,
    _prompt: &str,
) -> Result<(Config, Vec<UsedCliDataSource>), String> {
    use nola_core::context_engine::{
        ContextComponent, ContextComponentKind, ContextComponentSource, ContextPlatform,
    };

    let mut turn_config = config.clone();
    // Explicit Skill roots are a Host capability snapshot and replace the
    // standalone embedded catalog. This keeps metadata discovery and the
    // later exact-id load on the same filesystem/provenance boundary.
    let skill_runtime = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
        turn_config.skill_dirs.clone(),
    );
    let skills = skill_runtime.index()?;
    let skill_catalog = crate::tools::skill_catalog_prompt_from_items(&skills);
    // Catalog metadata is not a filesystem grant. The Host binds a successful
    // Skill activation to its actual resource directory at the execution boundary.
    turn_config.host_context_components.push(ContextComponent {
        kind: ContextComponentKind::Skill,
        id: "skill-catalog".to_string(),
        content: Value::String(skill_catalog),
        source: Some(ContextComponentSource {
            platform: Some(ContextPlatform::Shared),
            title: Some("Available Skills".to_string()),
            mime_type: Some("text/plain".to_string()),
            ..ContextComponentSource::default()
        }),
    });
    if !config.save_session {
        return Ok((turn_config, Vec::new()));
    }
    let store = crate::data_sources::DataSourceStore::open_default()?;
    let conversation_id = config.session_id.as_deref().unwrap_or("pending");
    let source_ids = store
        .sources()
        .iter()
        .map(|source| source.id.clone())
        .collect::<Vec<_>>();
    let mut used = Vec::new();

    for source_id in source_ids {
        let Some(source) = store.source(&source_id).cloned() else {
            continue;
        };
        let resolution = store.resolve(&source, conversation_id);
        if !resolution.enabled || !(resolution.usable || resolution.should_refresh) {
            continue;
        }
        let Ok(snapshot) = store.read_context_snapshot(&source.id, conversation_id) else {
            continue;
        };
        turn_config.host_context_components.push(ContextComponent {
            kind: ContextComponentKind::PlatformData,
            id: format!("data-source:{}", source.id),
            content: json!({
                "sourceId": source.id,
                "title": source.name,
                "content": snapshot.content,
                "observedAt": snapshot.observed_at,
                "truncated": snapshot.truncated,
            }),
            source: Some(ContextComponentSource {
                platform: Some(ContextPlatform::Host),
                uri: Some(source.location.clone()),
                title: Some(source.name.clone()),
                mime_type: None,
                observed_at: Some(snapshot.observed_at),
                authorization: Some(snapshot.authorization),
                maximum_age_minutes: Some(snapshot.maximum_age_minutes),
            }),
        });
        used.push(UsedCliDataSource {
            id: source.id,
            connector_id: source.validation_contract.connector_id,
            truncated: snapshot.truncated,
        });
    }
    Ok((turn_config, used))
}

fn record_cli_data_source_usage(config: &Config, sources: &[UsedCliDataSource]) {
    if sources.is_empty() {
        return;
    }
    let Ok(mut store) = crate::data_sources::DataSourceStore::open_default() else {
        return;
    };
    let disclosed_to_cloud = endpoint_discloses_to_cloud(&config.endpoint);
    for source in sources {
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert("truncated".to_string(), Value::Bool(source.truncated));
        metadata.insert(
            "connectorId".to_string(),
            Value::String(source.connector_id.clone()),
        );
        let _ = store.record_usage(
            &source.id,
            "cli_chat",
            "conversation_context",
            config.session_id.as_deref(),
            disclosed_to_cloud,
            metadata,
        );
    }
}

fn endpoint_discloses_to_cloud(endpoint: &str) -> bool {
    let endpoint = endpoint.trim().to_ascii_lowercase();
    !(endpoint.starts_with("http://127.0.0.1")
        || endpoint.starts_with("http://localhost")
        || endpoint.starts_with("http://[::1]")
        || endpoint.starts_with("https://127.0.0.1")
        || endpoint.starts_with("https://localhost")
        || endpoint.starts_with("https://[::1]"))
}

fn run_single_prompt_with_cancellation(
    config: &Config,
    prompt: &str,
) -> Result<PromptRunResult, String> {
    // The runtime owns an execution-scoped terminal listener. Keeping the Agent
    // on this thread avoids a second stdin owner and guarantees Esc/Ctrl+C can
    // reach the cancellation token for the full lifetime of the run.
    run_single_prompt(config, prompt)
}

struct CliGoalRoundDriver {
    config: Config,
    stop_requested: std::sync::atomic::AtomicBool,
}
impl CliGoalRoundDriver {
    fn new(config: Config) -> Self {
        Self {
            config,
            stop_requested: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn round_config(&self, request: &crate::host_invocation_request::HostInvocationRequest) -> Config {
        let mut config = self.config.clone();
        config.session_id = request.session.session_id.clone();
        config.continue_last = false;
        config.save_session = request.session.save_session;
        config.resume_snapshot = request.session.resume_snapshot.clone();
        config.host_submission_id = request.session.host_submission_id.clone();
        config.host_user_message_id = request.session.host_user_message_id.clone();
        config.host_assistant_message_id = request.session.host_assistant_message_id.clone();
        config.host_input_origin = request.input.host_input_origin;
        config.host_turn_context = request.input.host_turn_context.clone();
        config.host_context_components = request.input.host_context_components.clone();
        config.project_root = request.tool_settings.project_root.clone();
        config
    }
}
impl crate::goal_runtime::GoalRoundDriver for CliGoalRoundDriver {
    fn stop_requested(&self) -> bool {
        self.stop_requested.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn restores_exact_run_configuration(&self) -> bool {
        true
    }

    fn resume(&mut self, request: &crate::host_invocation_request::HostInvocationRequest, run_id: &str) -> Result<Option<PromptRunResult>, String> {
        let config = self.round_config(request);
        let cancellation = if config.json_output { interaction::ExecutionCancellation::default() }
            else { interaction::ExecutionCancellation::interactive() };
        let mut emitter = Emitter::new(config.json_output).with_quiet_status(true);
        let session = request.session.session_id.as_deref().ok_or("Goal recovery requires a session")?;
        let run_outcome = with_cli_execution_control(&config, cancellation.clone(), &mut emitter, |control, emitter|
            crate::core_agent_runtime::resume_frozen_core_agent_run(session, run_id, emitter, control))?;
        if cancellation.is_requested() {
            self.stop_requested.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        emitter.wait_for_execution_input_release();
        Ok(run_outcome.map(|run_outcome| PromptRunResult {
            goal_signal: run_outcome.goal_signal.clone(), run_outcome,
            output: emitter.take_markdown_text(), terminal_reason: emitter.take_terminal_reason(),
            session_id: Some(session.into()), queued_user_inputs: emitter.take_queued_user_inputs(),
            draft_user_input: emitter.take_draft_user_input(), context_usage: emitter.take_context_usage(),
        }))
    }
}
fn submit_goal_command(config: &Config, operation_id: String, command: GoalRuntimeCommand)
    -> Result<nola_core::agent::protocol::AgentControlSnapshot, String> {
    crate::goal_runtime::submit_goal_command(&config.host_invocation_request()?, operation_id, command)
}
fn pause_goal(config: &Config, cause: GoalCyclePause) -> Result<GoalCycleOutcome, String> {
    crate::goal_runtime::pause_goal(&config.host_invocation_request()?, cause)
}

fn print_goal_cycle_outcome_banner(outcome: &GoalCycleOutcome) {
    let lang = i18n::current_language();
    match outcome {
        GoalCycleOutcome::Completed(_) => {
            let label = match lang {
                i18n::Language::ZhHans => "目标已完成",
                i18n::Language::ZhHant => "目標已完成",
                i18n::Language::De => "Ziel abgeschlossen",
                i18n::Language::Es => "Objetivo completado",
                i18n::Language::Fr => "Objectif atteint",
                i18n::Language::Ja => "目標を完了しました",
                i18n::Language::Ko => "목표가 완료되었습니다",
                i18n::Language::Ru => "Цель достигнута",
                i18n::Language::En => "Goal completed",
            };
            if interactive_color_enabled() {
                terminal_notice!("  \x1b[32m✓  {label}{INTERACTIVE_RESET}");
            } else {
                terminal_notice!("  ✓  {label}");
            }
        }
        GoalCycleOutcome::Failed { reason, .. } => {
            let label = match lang {
                i18n::Language::ZhHans => "目标失败",
                i18n::Language::ZhHant => "目標失敗",
                i18n::Language::De => "Ziel fehlgeschlagen",
                i18n::Language::Es => "Objetivo fallido",
                i18n::Language::Fr => "Échec de l’objectif",
                i18n::Language::Ja => "目標に失敗しました",
                i18n::Language::Ko => "목표 실패",
                i18n::Language::Ru => "Не удалось выполнить цель",
                i18n::Language::En => "Goal failed",
            };
            let suffix = reason
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|r| format!(": {r}"))
                .unwrap_or_default();
            if interactive_color_enabled() {
                terminal_notice!("  \x1b[31m!  {label}{suffix}{INTERACTIVE_RESET}");
            } else {
                terminal_notice!("  !  {label}{suffix}");
            }
        }
        GoalCycleOutcome::Cancelled(_) => {
            let label = match lang {
                i18n::Language::ZhHans => "目标已取消",
                i18n::Language::ZhHant => "目標已取消",
                i18n::Language::De => "Ziel abgebrochen",
                i18n::Language::Es => "Objetivo cancelado",
                i18n::Language::Fr => "Objectif annulé",
                i18n::Language::Ja => "目標をキャンセルしました",
                i18n::Language::Ko => "목표가 취소되었습니다",
                i18n::Language::Ru => "Цель отменена",
                i18n::Language::En => "Goal cancelled",
            };
            terminal_notice!("  {label}");
        }
        GoalCycleOutcome::Paused(_) => {
            terminal_notice!("  {}", ui_text(UiText::GoalPaused));
        }
        GoalCycleOutcome::NeedsUser { reason, .. } => {
            let label = match lang {
                i18n::Language::ZhHans => "目标需要你的输入",
                i18n::Language::ZhHant => "目標需要您的輸入",
                i18n::Language::De => "Ziel benötigt deine Eingabe",
                i18n::Language::Es => "El objetivo necesita tu respuesta",
                i18n::Language::Fr => "L’objectif attend votre réponse",
                i18n::Language::Ja => "目標の続行には入力が必要です",
                i18n::Language::Ko => "목표 진행을 위해 입력이 필요합니다",
                i18n::Language::Ru => "Для продолжения цели нужен ваш ответ",
                i18n::Language::En => "Goal needs your input",
            };
            let suffix = reason
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|r| format!(": {r}"))
                .unwrap_or_default();
            if interactive_color_enabled() {
                terminal_notice!("  \x1b[33m◆  {label}{suffix}{INTERACTIVE_RESET}");
            } else {
                terminal_notice!("  ◆  {label}{suffix}");
            }
        }
        GoalCycleOutcome::Waiting { wake_at_epoch_ms, .. } => {
            let label = match lang {
                i18n::Language::ZhHans => "目标等待中",
                i18n::Language::ZhHant => "目標等待中",
                i18n::Language::De => "Ziel wartet",
                i18n::Language::Es => "Objetivo en espera",
                i18n::Language::Fr => "Objectif en attente",
                i18n::Language::Ja => "目標は待機中です",
                i18n::Language::Ko => "목표 대기 중",
                i18n::Language::Ru => "Цель в ожидании",
                i18n::Language::En => "Goal waiting",
            };
            if let Some(wake) = wake_at_epoch_ms {
                let schedule = format_goal_wake_time(*wake);
                terminal_notice!("  {label} ({schedule})");
            } else {
                terminal_notice!("  {label}");
            }
        }
        GoalCycleOutcome::Yielded(_) => {}
    }
}

fn run_goal_prompt_cycle(config: &mut Config, user_prompt: &str, objective: &str, constraints: &str)
    -> Result<GoalCycleOutcome, String> {
    let selected = crate::conversation_invocation::prepare_submission(config)?;
    let mut request = selected.host_invocation_request()?;
    let mut driver = CliGoalRoundDriver::new(selected);
    let result = crate::goal_runtime::run_goal_cycle(&mut request, user_prompt, objective, constraints, &mut driver);
    config.session_id = request.session.session_id;
    config.continue_last = false;
    config.save_session = request.session.save_session;
    config.resume_snapshot = request.session.resume_snapshot;
    if let Ok(outcome) = &result {
        if !config.json_output {
            print_goal_cycle_outcome_banner(outcome);
        }
    }
    result
}

fn extract_first_json_object(text: &str) -> Option<&str> {
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0usize;
    let mut start = None;
    for (index, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                if start.is_none() {
                    start = Some(index);
                }
                depth = depth.saturating_add(1);
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let start = start?;
                    return Some(&text[start..=index]);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_exit_command(input: &str) -> bool {
    let mut parts = input.trim().split_whitespace();
    let Some(command) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && normalize_interactive_command(command).is_some_and(|value| value == "/exit")
}

fn normalize_interactive_prompt(
    raw_line: &str,
    input_history: &[String],
    history_cursor: &mut Option<usize>,
) -> Option<String> {
    let trimmed = raw_line.trim_end_matches(&['\n', '\r'][..]);
    if let Some(history_prompt) = interactive_history_prompt(trimmed, input_history, history_cursor)
    {
        if history_prompt.is_empty() {
            return None;
        }
        return Some(history_prompt);
    }

    *history_cursor = None;

    let out = strip_interactive_control_sequences(trimmed);

    let prompt = out.trim();
    if prompt.is_empty() {
        None
    } else {
        Some(prompt.to_string())
    }
}

fn strip_interactive_control_sequences(raw: &str) -> String {
    let mut chars = raw.chars().peekable();
    let mut out = String::new();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek().is_none() {
                continue;
            }
            match chars.next().unwrap_or_default() {
                '[' => {
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                'O' => {
                    if chars.next().is_some() {
                        continue;
                    }
                }
                _ => {}
            }
            continue;
        }
        if ch.is_control() {
            continue;
        }
        out.push(ch);
    }

    out
}

fn interactive_history_prompt(
    trimmed_line: &str,
    input_history: &[String],
    history_cursor: &mut Option<usize>,
) -> Option<String> {
    let direction = match trimmed_line {
        "\x1b[A" | "\x1bOA" => Some(-1),
        "\x1b[B" | "\x1bOB" => Some(1),
        _ => None,
    }?;

    if input_history.is_empty() {
        return None;
    }

    let next_index = match direction {
        -1 => match *history_cursor {
            None => input_history.len().saturating_sub(1),
            Some(0) => 0,
            Some(cursor) => cursor - 1,
        },
        1 => match *history_cursor {
            None => return None,
            Some(cursor) => {
                if cursor.saturating_add(1) >= input_history.len() {
                    *history_cursor = None;
                    return None;
                }
                cursor + 1
            }
        },
        _ => return None,
    };

    *history_cursor = Some(next_index);
    input_history.get(next_index).cloned()
}

fn combine_args_and_stdin_prompt(args_message: &str, piped_stdin: &str) -> String {
    let args_trimmed = args_message.trim();
    let stdin_trimmed = piped_stdin.trim();
    match (stdin_trimmed.is_empty(), args_trimmed.is_empty()) {
        (true, true) => String::new(),
        (true, false) => args_message.to_string(),
        (false, true) => piped_stdin.to_string(),
        (false, false) => format!("{stdin_trimmed}\n\n{args_trimmed}"),
    }
}

fn read_prompt(args_message: &str) -> io::Result<String> {
    if io::stdin().is_terminal() {
        return Ok(args_message.to_string());
    }

    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer)?;
    Ok(combine_args_and_stdin_prompt(args_message, &buffer))
}

fn preview(text: &str, limit: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if display_width(&normalized) <= limit {
        return normalized;
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in normalized.chars() {
        let width = unicode_char_width(ch);
        if used.saturating_add(width) > limit.saturating_sub(1) {
            break;
        }
        used += width;
        out.push(ch);
    }
    out.push('…');
    out
}

fn unicode_char_width(ch: char) -> usize {
    if ch.is_ascii_control() {
        0
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(1)
    }
}
