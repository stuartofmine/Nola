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
}

fn interactive_goal_snapshot(
    config: &Config,
    goal_target: Option<&str>,
    goal_constraints: Option<&str>,
) -> Result<Option<InteractiveGoalSnapshot>, String> {
    if let Some(session_id) = config.session_id.as_deref() {
        if let Some(projection) = session::load_session_goal_projection(session_id)? {
            if let Some(goal) = projection.active.filter(|goal| !goal.status.is_terminal()) {
                let status = match goal.status {
                    GoalStatus::Running => "running",
                    GoalStatus::Paused => "paused",
                    GoalStatus::NeedsUser => "needs_user",
                    GoalStatus::Completed => "completed",
                    GoalStatus::Failed => "failed",
                    GoalStatus::Cancelled => "cancelled",
                };
                return Ok(Some(InteractiveGoalSnapshot {
                    target: goal.objective,
                    constraints: goal.constraints.join("\n"),
                    status: status.to_string(),
                }));
            }
            // Persisted completion/cancellation wins over a stale local draft.
            return Ok(None);
        }
    }
    Ok(goal_target.map(|target| InteractiveGoalSnapshot {
        target: target.to_string(),
        constraints: goal_constraints.unwrap_or_default().to_string(),
        status: "running".to_string(),
    }))
}

fn interactive_goal_snapshot_from_projection(
    projection: &nola_core::agent::goal::GoalProjection,
) -> Option<InteractiveGoalSnapshot> {
    let goal = projection
        .active
        .as_ref()
        .filter(|goal| !goal.status.is_terminal())?;
    let status = match goal.status {
        GoalStatus::Running => "running",
        GoalStatus::Paused => "paused",
        GoalStatus::NeedsUser => "needs_user",
        GoalStatus::Completed => "completed",
        GoalStatus::Failed => "failed",
        GoalStatus::Cancelled => "cancelled",
    };
    Some(InteractiveGoalSnapshot {
        target: goal.objective.clone(),
        constraints: goal.constraints.join("\n"),
        status: status.to_string(),
    })
}

fn submit_interactive_goal_control(config: &Config, action: nola_application::goal_control_intent::GoalControlAction) -> Result<bool, String> {
    let session = config.session_id.as_deref().ok_or("Goal requires a conversation")?;
    let projection = session::load_session_goal_projection(session)?.ok_or("Goal state is unavailable")?;
    let goal = projection.active.as_ref().filter(|goal| !goal.status.is_terminal())
        .ok_or_else(|| tr(Message::InteractiveGoalNoResumable).to_string())?;
    let confirmed = crate::goal_control_port::submit_existing(nola_application::goal_control_intent::PrepareGoalControlIntent {
        conversation_id: session.into(), operation_id: format!("cli-goal:{}", uuid::Uuid::new_v4()),
        action, expected_goal_id: Some(goal.id.clone()), expected_control_token: projection.control_token(),
        objective: String::new(), constraints: Vec::new(), wake_at_epoch_ms: None,
    })?;
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
    config.runtime.provider = Some("llamacpp".to_string());
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
        "  {} {} · llama.cpp · {}K{}",
        tr(Message::InteractiveModelStatusPrefix),
        endpoint.model_id,
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
        Language::En => values[1],
        Language::De => values[2],
        Language::Es => values[3],
        Language::Fr => values[4],
        Language::Ja => values[5],
        Language::Ru => values[6],
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
        .position(|value| value == &preferred_model)
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
    if let Some(id) = model_id.strip_prefix("configured:") {
        if let Ok(Some(profile)) = crate::store::NolaStore::open_default().and_then(|store| store.model_profile(id)) {
            let source: serde_json::Value = serde_json::from_str(&profile.source_json).unwrap_or_default();
            return format!("{} · {}", profile.model_name, source["connectionID"].as_str().unwrap_or(id));
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
        "on"
    } else {
        "off"
    }
}

fn parse_interactive_toggle(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn interactive_toggle_hint() -> String {
    "on/off".to_string()
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
    Select(usize),
    SelectAndConfirm(usize),
    Confirm,
    Cancel,
    Ignore,
}

fn permission_picker_action(key: KeyEvent, selected: usize) -> PermissionPickerAction {
    use PermissionPickerAction::*;
    match crate::terminal_input::modal_key(&key) {
        crate::terminal_input::ModalKey::Cancel => return Cancel,
        crate::terminal_input::ModalKey::Ignore => return Ignore,
        crate::terminal_input::ModalKey::Action => {}
    }
    match key.code {
        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => Select((selected + ALL_PERMISSION_LEVELS.len() - 1) % ALL_PERMISSION_LEVELS.len()),
        KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => Select((selected + 1) % ALL_PERMISSION_LEVELS.len()),
        KeyCode::Char('1') => SelectAndConfirm(0),
        KeyCode::Char('2') => SelectAndConfirm(1),
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

    let mut rendered_lines = 0usize;
    draw_permission_picker_frame(&levels, selected_idx, current, rendered_lines)?;
    rendered_lines = render_permission_picker(&levels, selected_idx, current)
        .lines()
        .count();

    loop {
        match read()? {
            Event::Key(key) => match permission_picker_action(key, selected_idx) {
                PermissionPickerAction::Select(index) => {
                    selected_idx = index;
                    draw_permission_picker_frame(&levels, selected_idx, current, rendered_lines)?;
                    rendered_lines = render_permission_picker(&levels, selected_idx, current)
                        .lines()
                        .count();
                }
                PermissionPickerAction::Confirm => {
                    clear_permission_picker_frame(rendered_lines)?;
                    apply_interactive_permission_level(config, levels[selected_idx]);
                    return Ok(());
                }
                PermissionPickerAction::SelectAndConfirm(index) => {
                    clear_permission_picker_frame(rendered_lines)?;
                    apply_interactive_permission_level(config, levels[index]);
                    return Ok(());
                }
                PermissionPickerAction::Cancel => {
                    clear_permission_picker_frame(rendered_lines)?;
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
                draw_permission_picker_frame(&levels, selected_idx, current, rendered_lines)?;
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
    previous_lines: usize,
) -> io::Result<()> {
    crate::terminal_ui::present_permission_frame(
        &render_permission_picker(levels, selected_idx, current), previous_lines > 0,
    )
}

fn clear_permission_picker_frame(_rendered_lines: usize) -> io::Result<()> {
    crate::terminal_ui::clear_permission_frame();
    Ok(())
}

fn render_permission_picker(
    levels: &[PermissionLevel],
    selected_idx: usize,
    current: PermissionLevel,
) -> String {
    let color = interactive_color_enabled();
    let accent = INTERACTIVE_ACCENT;
    let header = tr(Message::InteractiveAllowPanelTitle);
    let localized = crate::i18n::interaction_text();
    let hint = localized.selection_hint;

    let mut plain_rows: Vec<String> = Vec::new();
    plain_rows.push(format!("⚙  {header}"));
    plain_rows.push(hint.to_string());
    plain_rows.push(format!(
        "  {} {} ({})",
        localized.current_label,
        current.display(),
        current.description()
    ));
    plain_rows.push(String::new());
    let option_start = plain_rows.len();
    for (i, level) in levels.iter().enumerate() {
        let marker = if i == selected_idx { "›" } else { " " };
        plain_rows.push(format!("{marker} {}  {}", i + 1, level.display()));
        plain_rows.push(format!("     {}", level.description()));
    }

    let terminal_width = terminal_columns().max(12);
    let max_content = terminal_width.saturating_sub(6).max(8);
    let preferred_min = 32.min(max_content);
    let inner = plain_rows
        .iter()
        .map(|row| display_width(row))
        .max()
        .unwrap_or(preferred_min)
        .min(max_content)
        .max(preferred_min);
    let mut out = String::new();
    out.push('\n');
    push_picker_border(&mut out, "╭", "╮", inner, accent, color);
    for (index, row) in plain_rows.iter().enumerate() {
        let rendered = truncate_display_width(row, inner);
        let padded = pad_to_display_width(&rendered, inner);
        let selected = index >= option_start && (index - option_start) / 2 == selected_idx;
        let row_color = if index == 0 {
            accent
        } else if index >= option_start {
            if (index - option_start) % 2 == 0 {
                INTERACTIVE_ACCENT
            } else {
                INTERACTIVE_DIM
            }
        } else {
            INTERACTIVE_DIM
        };
        push_picker_line(&mut out, &padded, row_color, color, selected);
    }
    push_picker_border(&mut out, "╰", "╯", inner, accent, color);
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
        out.push_str("\x1b[2;38;5;245m│\x1b[0m ");
        if selected {
            out.push_str("\x1b[48;5;24m");
        }
        out.push_str(row_color);
        out.push_str(text);
        out.push_str("\x1b[0m");
        out.push_str(" \x1b[2;38;5;245m│\x1b[0m\n");
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

fn print_interactive_help() {
    terminal_notice!("{}", tr(Message::InteractiveAvailableCommands));
    let rows = interactive_help_rows();
    let command_width = rows
        .iter()
        .map(|(command, _)| display_width(command))
        .max()
        .unwrap_or(0);
    for (command, hint) in rows {
        terminal_notice!("  {}  {hint}", pad_right_width(&command, command_width));
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
            "/plan".to_string(),
            tr(Message::InteractiveSlashPlanHint).to_string(),
        ),
        ("/agent".to_string(), ui_text(UiText::HelpAgent).to_string()),
        ("/room".to_string(), "Room messages · @recipient + Tab, or automatic routing".to_string()),
        ("/work".to_string(), ui_text(UiText::HelpWork).to_string()),
        ("/recover".to_string(), crate::unknown_effect_recovery::usage().to_string()),
        ("/tasks".to_string(), ui_text(UiText::HelpTasks).to_string()),
        ("/jobs".to_string(), ui_text(UiText::HelpJobs).to_string()),
        (
            "/resume".to_string(),
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
        (
            "/quit".to_string(),
            tr(Message::InteractiveSlashQuitHint).to_string(),
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
}

fn interactive_color_enabled() -> bool {
    (io::stdout().is_terminal() || io::stderr().is_terminal())
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM")
            .map(|value| value != "dumb")
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
}

#[derive(Debug)]
struct LiveCliCommands {
    config: std::sync::Mutex<Config>,
    permission_change: std::sync::Mutex<Option<PermissionLevel>>,
}
impl interaction::ClientCommandHandler for LiveCliCommands {
    fn execute(&self, input: &str) {
        let mut config = self.config.lock().unwrap_or_else(|p| p.into_inner());
        let command = input.split_whitespace().next().unwrap_or("");
        let command = normalize_interactive_command(command).unwrap_or(command);
        let result = match command {
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
            "/help" => { print_interactive_help(); Ok(()) }
            "/status" => print_interactive_prompt_card(&config, 0),
            "/history" => print_interactive_session_history(&config),
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
        config: std::sync::Mutex::new(config.clone()), permission_change: Default::default(),
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
    result
}

fn run_single_prompt(config: &Config, prompt: &str) -> Result<PromptRunResult, String> {
    let selected_config = crate::conversation_invocation::prepare_submission(config)?;
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
    let run_outcome = with_cli_execution_control(config, cancellation, &mut emitter, |control, emitter|
        core_agent_runtime::run_agent_core_with_cancellation(&request, prompt, emitter, control))?;
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
    let selected_config = crate::conversation_invocation::prepare_submission(config)?;
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

struct CliGoalRoundDriver { config: Config }
impl CliGoalRoundDriver {
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
    fn resume(&mut self, request: &crate::host_invocation_request::HostInvocationRequest, run_id: &str) -> Result<Option<PromptRunResult>, String> {
        let config = self.round_config(request);
        let cancellation = if config.json_output { interaction::ExecutionCancellation::default() }
            else { interaction::ExecutionCancellation::interactive() };
        let mut emitter = Emitter::new(config.json_output).with_quiet_status(true);
        let session = request.session.session_id.as_deref().ok_or("Goal recovery requires a session")?;
        let run_outcome = with_cli_execution_control(&config, cancellation, &mut emitter, |control, emitter|
            crate::core_agent_runtime::resume_frozen_core_agent_run(session, run_id, emitter, control))?;
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
fn run_goal_prompt_cycle(config: &mut Config, user_prompt: &str, objective: &str, constraints: &str)
    -> Result<GoalCycleOutcome, String> {
    let selected = crate::conversation_invocation::prepare_submission(config)?;
    let mut request = selected.host_invocation_request()?;
    let mut driver = CliGoalRoundDriver { config: selected };
    let result = crate::goal_runtime::run_goal_cycle(&mut request, user_prompt, objective, constraints, &mut driver);
    config.session_id = request.session.session_id;
    config.continue_last = false;
    config.save_session = request.session.save_session;
    config.resume_snapshot = request.session.resume_snapshot;
    if matches!(&result, Ok(GoalCycleOutcome::Paused)) && !config.json_output {
        terminal_notice!("  {}", ui_text(UiText::GoalPaused));
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

fn read_prompt(args_message: &str) -> io::Result<String> {
    if !args_message.trim().is_empty() {
        return Ok(args_message.to_string());
    }

    if io::stdin().is_terminal() {
        return Ok(String::new());
    }

    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer)?;
    Ok(buffer)
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
