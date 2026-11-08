// All durable interactive feedback joins the same transcript. Raw cursor
// writes remain confined to the editor/renderer, never slash-command output.
macro_rules! terminal_notice {
    () => {{ if !crate::terminal_ui::owns_terminal() { println!(); } }};
    ($($arg:tt)*) => {{
        if crate::terminal_ui::owns_terminal() {
            crate::terminal_ui::commit_history(&format!($($arg)*));
        } else {
            println!($($arg)*);
        }
    }};
}

fn run_interactive_prompt(config: &Config) -> Result<(), String> {
    run_interactive_prompt_with_recovery(config, config.session_id.is_some())
}

fn run_interactive_prompt_with_recovery(config: &Config, resume_pending: bool) -> Result<(), String> {
    let mut selected = config.clone();
    let mut recover = resume_pending;
    loop {
        match run_interactive_session(&selected, recover)? {
            InteractiveSessionExit::Quit => return Ok(()),
            InteractiveSessionExit::Switch(next) => {
                selected = *next;
                recover = true;
            }
        }
    }
}

enum InteractiveSessionExit {
    Quit,
    Switch(Box<Config>),
}

fn interactive_composer_key(config: &Config) -> Result<Option<String>, String> {
    config.session_id.as_ref().map(|session| {
        let database = crate::platform_paths::application_database_path()?;
        serde_json::to_string(&(database, session)).map_err(|error| error.to_string())
    }).transpose()
}

fn open_interactive_view_state(config: &Config) -> (
    Option<crate::cli_view_state::ViewStore>, Option<crate::cli_view_state::ViewState>,
) {
    if !config.save_session || config.json_output { return (None, None); }
    let Some(id) = config.session_id.as_deref() else { return (None, None); };
    match crate::cli_view_state::ViewStore::open(id) {
        Ok((store, view)) => (Some(store), Some(view)),
        Err(error) => {
            terminal_notice!("无法恢复 CLI 显示状态（原文件保留）：{error}");
            (None, None)
        }
    }
}

fn persist_interactive_view_state(store: &mut Option<crate::cli_view_state::ViewStore>, draft: String, cursor: usize) {
    let Some(active) = store.as_mut() else { return; };
    let view = crate::cli_view_state::ViewState {
        draft, cursor, viewport: crate::terminal_ui::capture_viewport_bookmark(),
    };
    if let Err(error) = active.save(view) {
        terminal_notice!("无法保存 CLI 显示状态：{error}");
        *store = None;
    }
}

fn run_interactive_session(config: &Config, resume_pending: bool) -> Result<InteractiveSessionExit, String> {
    onboarding::run_if_needed(config)?;
    let mut ui_guard = InteractiveSessionUiStateGuard::new(!config.json_output);
    llm::configure_local_debug_log(config.debug_mode, &config.project_root);
    if !config.json_output {
        reset_interactive_terminal_state();
        // Install the terminal event source before publishing the first frame.
        // Otherwise a resize immediately after restore can precede SIGWINCH
        // registration and remain invisible until the next keystroke.
        crate::terminal_input::poll(Duration::ZERO).map_err(|error| error.to_string())?;
        crate::terminal_ui::begin_interactive_session();
    }
    let mut interactive_config = config.clone();
    if interactive_config.session_id.is_none() && interactive_config.continue_last {
        interactive_config.session_id = resolve_continue_session_id(&interactive_config)?;
        interactive_config.continue_last = false;
    }
    interaction::select_interactive_composer(interactive_composer_key(&interactive_config)?);
    let mut started = false;
    let mut turn = 1usize;
    let mut input_history: Vec<String> = Vec::new();
    let mut history_cursor: Option<usize> = None;
    let mut goal_target: Option<String> = None;
    let mut goal_constraints: Option<String> = None;
    let mut previous_workspace: Option<PathBuf> = None;
    let mut queued_user_inputs: VecDeque<String> = VecDeque::new();
    let mut pending_draft: Option<String> = None;
    let mut view_session = interactive_config.session_id.clone();
    let (mut view_store, saved_view) = open_interactive_view_state(&interactive_config);
    let _active_view = crate::cli_view_state::ActiveView::bind(view_store.clone());
    if let Some(view) = &saved_view {
        if !view.draft.is_empty() { pending_draft = Some(view.draft.clone()); }
        interaction::update_interactive_composer(&view.draft, view.cursor);
    }
    let mut last_context_usage: Option<nola_core::agent::context_window::ContextUsageSnapshot> =
        None;

    crate::terminal_ui::set_restoring_view(true);
    let restored = restore_interactive_session_messages(
        &interactive_config,
        &mut started,
        &mut turn,
        &mut input_history,
    )?;
    if let Some(session_id) = interactive_config.session_id.as_deref() {
        last_context_usage = restored_agent_context_usage(session_id);
    }
    if let Some(saved) = restored.as_ref() {
        if !interactive_config.json_output && !saved.messages.is_empty() {
            terminal_notice!();
            print_interactive_session_entry(saved, &interactive_config);
            print_interactive_session_replay(saved);
            if let Some(session_id) = interactive_config.session_id.as_deref() {
                if let Ok(total_message_count) = session::session_message_count(session_id) {
                    print_interactive_session_replay_notice(
                        saved.messages.len(),
                        total_message_count,
                    );
                }
            }
        }
    }

    if let Some(view) = &saved_view {
        if !crate::terminal_ui::restore_viewport_bookmark(&view.viewport) {
            // The recent-tail budget is a startup optimization, not a boundary
            // on a saved reading position. Reuse the canonical /history read
            // when the source anchor is outside that tail; never synthesize
            // history from the host-local view file.
            let expanded = load_interactive_session_for_command(&interactive_config)?;
            if let Some(history) = expanded {
                crate::terminal_ui::clear_replayed_history();
                crate::terminal_ui::set_restoring_view(true);
                print_interactive_session_entry(&history, &interactive_config);
                print_interactive_session_replay(&history);
            }
            if !crate::terminal_ui::restore_viewport_bookmark(&view.viewport) {
                terminal_notice!("原阅读内容已不可用；草稿已保留，当前显示最近记录。");
            }
        }
    }
    crate::terminal_ui::set_restoring_view(false);
    let mut recovery_blocked = false;
    if resume_pending {
        let mut emitter = Emitter::new(interactive_config.json_output).with_quiet_status(true);
        recovery_blocked = match resume_selected_session_before_composer(&interactive_config, &mut emitter) {
            Ok(ready) => !ready,
            Err(error) => {
                terminal_notice!("{}", format_error(&error));
                true
            }
        };
        if recovery_blocked {
            terminal_notice!("恢复尚未完成；可用 /resume 重试或切换会话，/exit 退出。原请求已保留。");
        }
        // Recovery is a real execution phase: input typed while its effects
        // drain has the same composer handoff as an ordinary completed turn.
        emitter.wait_for_execution_input_release();
        queued_user_inputs.extend(emitter.take_queued_user_inputs());
        if let Some(session_id) = emitter.session_id() {
            core_agent_runtime::drain_parent_agent_task_inbox(session_id)?;
        }
        if let Some(draft) = emitter.take_draft_user_input() { pending_draft = Some(draft); }
        if let Some(usage) = emitter.take_context_usage() { last_context_usage = Some(usage); }
    }
    if let Some(view) = &saved_view {
        if pending_draft.as_deref() == Some(view.draft.as_str()) {
            interaction::update_interactive_composer(&view.draft, view.cursor);
        }
    }

    if interactive_config.session_id.is_some() {
        if let Some(goal) = interactive_goal_snapshot(&interactive_config, None, None)? {
            goal_target = Some(goal.target);
            goal_constraints = (!goal.constraints.trim().is_empty()).then_some(goal.constraints);
            if !interactive_config.json_output {
                if let Some(target) = goal_target.as_deref() {
                    print_goal_mode_summary(target, goal_constraints.as_deref());
                    crate::terminal_ui::commit_history(&format!(
                        "  {} {}\n",
                        tr(Message::InteractiveGoalStatePrefix),
                        interactive_goal_state_text(&goal.status)
                    ));
                }
            }
        }
    }

    if !interactive_config.json_output {
        if let Some(saved) = restored.as_ref() {
            let plan = agent::session_plan_summary_for_session(&saved.id, &saved.messages);
            let background_tasks = tools::background_bash_task_summaries(
                std::slice::from_ref(&interactive_config.project_root),
                interactive_config.session_id.as_deref(),
            )
            .unwrap_or_default();
            let running_process = crate::session_process_observation::session_running_process(&saved.id);
            print_resumed_execution_state(plan.as_ref(), &background_tasks, running_process);
        }
    }

    if !interactive_config.json_output && !started {
        print_interactive_welcome(&interactive_config)?;
    }

    let mut offer_user_questions = true;
    let mut include_child_questions = false;
    loop {
        if view_session != interactive_config.session_id {
            view_session = interactive_config.session_id.clone();
            interaction::select_interactive_composer(interactive_composer_key(&interactive_config)?);
            pending_draft = None;
            let (store, view) = open_interactive_view_state(&interactive_config);
            view_store = store;
            crate::cli_view_state::ActiveView::replace(view_store.clone());
            if let Some(view) = view {
                if !view.draft.is_empty() { pending_draft = Some(view.draft.clone()); }
                interaction::update_interactive_composer(&view.draft, view.cursor);
            }
        }
        if let Some(chosen) = LIVE_PERMISSION_CHANGE.with(|pending| pending.take()) {
            chosen.apply(&mut interactive_config);
            crate::conversation_invocation::persist(&interactive_config)?;
        }
        crate::conversation_invocation::refresh_unmodified(&mut interactive_config)?;
        if offer_user_questions && !interactive_config.json_output {
            offer_user_questions = false;
            if let Some(conversation) = interactive_config.session_id.as_deref() {
                let include_children = std::mem::take(&mut include_child_questions);
                match crate::user_input_wait::present_pending(
                    conversation,
                    include_children,
                    &interaction::TerminalInteraction,
                ) {
                    Ok(outcome) => {
                        if let Some(error) = outcome.notice { terminal_notice!("{}", format_error(&error)); }
                        if let Some(run_id) = outcome.resume_run {
                            let mut emitter = Emitter::new(false);
                            let result = interactive_config.host_invocation_request().and_then(|request|
                                with_cli_execution_control(&interactive_config, interaction::ExecutionCancellation::interactive(), &mut emitter,
                                    |control, emitter| core_agent_runtime::resume_agent_core_expected_with_cancellation(
                                        &request, emitter, control, &run_id)));
                            emitter.wait_for_execution_input_release();
                            queued_user_inputs.extend(emitter.take_queued_user_inputs());
                            if let Some(draft) = emitter.take_draft_user_input() { pending_draft = Some(draft); }
                            if let Some(usage) = emitter.take_context_usage() { last_context_usage = Some(usage); }
                            match result {
                                Ok(_) => { offer_user_questions = !outcome.deferred; continue; }
                                Err(error) => terminal_notice!("{}", format_error(&error)),
                            }
                        }
                    }
                    Err(error) => terminal_notice!("{}", format_error(&error)),
                }
            }
        }
        queued_user_inputs.extend(interaction::take_interactive_queued_inputs());
        let prompt_footer = if !interactive_config.json_output {
            interactive_prompt_footer_line(
                &interactive_config,
                interactive_config.session_id.as_deref(),
                last_context_usage,
            )
        } else {
            None
        };
        let maybe_prompt = if let Some(queued) = queued_user_inputs.pop_front() {
            if !interactive_config.json_output {
                commit_interactive_user_history_line(&queued);
            }
            Some(queued)
        } else {
            read_interactive_prompt_line(
                &input_history,
                &mut history_cursor,
                !interactive_config.json_output,
                prompt_footer.as_deref(),
                &interactive_config,
                pending_draft.take().as_deref(),
                &mut view_store,
            )
            .map_err(|err| format!("{} {err}", tr(Message::FailedReadPromptPrefix)))?
        };
        let Some(prompt) = maybe_prompt else {
            break;
        };
        if prompt.is_empty() {
            continue;
        }
        if prompt == "/questions" {
            offer_user_questions = true;
            include_child_questions = true;
            continue;
        }
        if is_exit_command(prompt.as_str()) {
            if !interactive_config.json_output {
                clear_interactive_prompt_area(prompt_footer.as_deref())
                    .map_err(|err| format!("{} {err}", tr(Message::FailedFlushPromptPrefix)))?;
            }
            break;
        }
        if is_resume_command(prompt.as_str()) || is_continue_command(prompt.as_str()) {
            match select_interactive_session_restore(&interactive_config, is_continue_command(prompt.as_str())) {
                Ok(Some(next)) => return Ok(InteractiveSessionExit::Switch(Box::new(next))),
                Ok(None) => {},
                Err(error) => terminal_notice!("{}", format_error(&error)),
            }
            crate::terminal_ui::invalidate_presented_screen();
            continue;
        }
        if recovery_blocked && !prompt.starts_with('/') {
            terminal_notice!("恢复尚未完成，未发送此消息。请用 /resume 重试或选择其他会话。");
            pending_draft = Some(prompt);
            continue;
        }
        if prompt == "/" {
            print_interactive_help();
            continue;
        }
        if !prompt.starts_with('/') && interactive_config.save_session {
            if let Some(session)=interactive_config.session_id.as_deref() {
                if let Err(error)=crate::boundary_input_intent::ensure_no_pending(session) {
                    terminal_notice!("{}",format_error(&error));
                    pending_draft=Some(prompt);
                    continue;
                }
            }
        }
        match handle_interactive_slash_command(
            &prompt,
            &mut interactive_config,
            &mut goal_target,
            &mut goal_constraints,
            &mut previous_workspace,
            &mut last_context_usage,
            turn,
        )? {
            SlashCommandResult::RunGoalPrompt(run_prompt) => {
                if recovery_blocked {
                    terminal_notice!("恢复尚未完成，未启动新任务。请用 /resume 重试或选择其他会话。");
                    pending_draft = Some(prompt);
                    continue;
                }
                if started
                    && interactive_config.session_id.is_none()
                    && !interactive_config.continue_last
                {
                    interactive_config.continue_last = true;
                }
                let run_result = if let Some(goal) = goal_target.clone() {
                    let requirements = goal_constraints
                        .clone()
                        .unwrap_or_else(|| default_goal_requirements(&interactive_config, &goal));
                    let goal_result = run_goal_prompt_cycle(
                        &mut interactive_config,
                        run_prompt.as_str(),
                        &goal,
                        requirements.as_str(),
                    );
                    if let Ok(outcome) = &goal_result {
                        let handoff = match outcome {
                            GoalCycleOutcome::Completed(handoff) => {
                                goal_target = None;
                                goal_constraints = None;
                                Some(handoff)
                            }
                            GoalCycleOutcome::Yielded(handoff) => Some(handoff),
                            GoalCycleOutcome::Paused => None,
                        };
                        if let Some(handoff) = handoff {
                            queued_user_inputs.extend(handoff.queued_user_inputs.iter().cloned());
                            if let Some(draft) = handoff.draft_user_input.as_ref() {
                                pending_draft = Some(draft.clone());
                            }
                            if let Some(context_usage) = handoff.context_usage {
                                last_context_usage = Some(context_usage);
                            }
                            reset_interactive_terminal_state();
                        }
                    }
                    goal_result.map(|_| ())
                } else {
                    Err("goal mode is not enabled".to_string())
                };
                if let Err(err) = run_result {
                    terminal_notice!("{}", format_error(&err));
                }
                started = true;
                turn = turn.saturating_add(1);
                continue;
            }
            SlashCommandResult::Handled => continue,
            SlashCommandResult::WorkspaceChanged | SlashCommandResult::SessionReset => {
                interaction::select_interactive_composer(interactive_composer_key(&interactive_config)?);
                pending_draft = None;
                recovery_blocked = false;
                offer_user_questions = true;
                started = false;
                turn = 1;
                history_cursor = None;
                queued_user_inputs.clear();
                continue;
            }
            SlashCommandResult::PassThrough => {}
        }
        if started && interactive_config.session_id.is_none() && !interactive_config.continue_last {
            interactive_config.continue_last = true;
        }
        // A persisted Goal is durable work state, not an implicit input mode.
        // Only an explicit `/goal ...` control above may enter the autonomous
        // cycle. Every ordinary composer submission remains one visible user
        // Turn, even while a Goal is paused, running in the background, or
        // awaiting user input.
        let run_result = match run_single_prompt_with_cancellation(
            &interactive_config,
            prompt.as_str(),
        ) {
            Ok(result) => {
                offer_user_questions = true;
                if !result.run_outcome.is_terminal() {
                    eprintln!("Run remains pending ({:?}); no completion or permission decision was recorded.", result.run_outcome.turn_status);
                }
                if interactive_config.session_id.is_none() {
                    interactive_config.session_id = result.session_id.clone();
                    interactive_config.continue_last = false;
                }
                queued_user_inputs.extend(result.queued_user_inputs);
                pending_draft = result.draft_user_input;
                if let Some(context_usage) = result.context_usage {
                    last_context_usage = Some(context_usage);
                }
                Ok(())
            }
            Err(err) => Err(err),
        };
        if let Err(err) = run_result {
            terminal_notice!("{}", format_error(&err));
        }
        if input_history
            .last()
            .is_none_or(|value| value.as_str() != prompt)
        {
            input_history.push(prompt.to_string());
            if input_history.len() > 128 {
                input_history.drain(0..(input_history.len() - 128));
            }
        }
        history_cursor = None;
        started = true;
        turn = turn.saturating_add(1);
    }

    if let Some(guard) = ui_guard.as_mut() {
        guard.restore();
    }
    if !interactive_config.json_output && started {
        print_interactive_session_exit_hint(&interactive_config);
    }
    Ok(InteractiveSessionExit::Quit)
}

fn print_interactive_session_entry(_saved: &session::SavedSession, config: &Config) {
    let width = terminal_columns().saturating_sub(2).max(40);
    let cwd = truncate_display_path(&config.project_root.to_string_lossy(), width / 2);
    let title = i18n::interaction_text().resume_title;
    let line = truncate_display_width(&format!("{title} · {cwd}"), width);
    let heading = if interactive_color_enabled() {
        format!(
            "  {INTERACTIVE_ACCENT}{title}{INTERACTIVE_RESET}{INTERACTIVE_DIM} · {cwd}{INTERACTIVE_RESET}"
        )
    } else {
        format!("  {line}")
    };
    crate::terminal_ui::commit_history(&format!(
        "\n{heading}\n{}\n\n",
        interactive_layer_rule(None)
    ));
}

fn interactive_session_resume_command(config: &Config) -> Option<String> {
    if !config.save_session { return None; }
    let id = config.session_id.as_deref()?;
    let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
    Some(format!("nola --session {} --project {}", quote(id), quote(&config.project_root.to_string_lossy())))
}

fn print_interactive_session_exit_hint(config: &Config) {
    let Some(command) = interactive_session_resume_command(config) else { return; };
    terminal_notice!();
    terminal_notice!("{}", interactive_layer_rule(Some("continue")));
    // A copyable command must never be truncated to the terminal width.
    terminal_notice!("  {}", match current_language() {
        Language::ZhHans => "继续此会话：",
        _ => "Resume this conversation:",
    });
    terminal_notice!("  {command}");
}

fn print_resumed_execution_state(
    plan: Option<&agent::SessionPlanSummary>,
    background_tasks: &[tools::BackgroundTaskSummary],
    running_process: Option<u32>,
) {
    let background_tasks = background_tasks.iter().filter(|task|
        !matches!(task.canonical_state.as_str(), "completed" | "failed" | "cancelled")
    ).cloned().collect::<Vec<_>>();
    if plan.is_none() && background_tasks.is_empty() && running_process.is_none() {
        return;
    }
    terminal_notice!("{}", interactive_layer_rule(Some("state")));
    if let Some(pid) = running_process {
        if interactive_color_enabled() {
            terminal_notice!(
                "  {INTERACTIVE_LABEL}{}{INTERACTIVE_RESET}  {INTERACTIVE_ACCENT}{}{INTERACTIVE_RESET}{INTERACTIVE_DIM} · pid={pid} · {}{INTERACTIVE_RESET}",
                ui_text(UiText::Session), ui_text(UiText::SessionBusy), ui_text(UiText::WaitInOriginalTerminal)
            );
        } else {
            terminal_notice!(
                "  {}  {} · pid={pid} · {}",
                ui_text(UiText::Session),
                ui_text(UiText::SessionBusy),
                ui_text(UiText::WaitInOriginalTerminal)
            );
        }
    }
    if let Some(plan) = plan {
        let current = plan
            .current
            .as_deref()
            .map(|value| truncate_display_width(value, 100))
            .unwrap_or_else(|| {
                if plan.completed == plan.total {
                    ui_text(UiText::AllComplete).to_string()
                } else {
                    ui_text(UiText::WaitingNextStep).to_string()
                }
            });
        if interactive_color_enabled() {
            terminal_notice!(
                "  {INTERACTIVE_LABEL}{}{INTERACTIVE_RESET}  {INTERACTIVE_VALUE}{}/{}{INTERACTIVE_RESET}{INTERACTIVE_DIM} · {current}{INTERACTIVE_RESET}",
                ui_text(UiText::Plan), plan.completed, plan.total
            );
        } else {
            terminal_notice!(
                "  {}  {}/{} · {current}",
                ui_text(UiText::Plan),
                plan.completed,
                plan.total
            );
        }
    }
    if !background_tasks.is_empty() {
        let running = background_tasks
            .iter()
            .filter(|task| task.status.to_ascii_lowercase().starts_with("running"))
            .count();
        if interactive_color_enabled() {
            terminal_notice!(
                "  {INTERACTIVE_LABEL}{}{INTERACTIVE_RESET}  {INTERACTIVE_VALUE}{} {}{INTERACTIVE_RESET}{INTERACTIVE_DIM} · {} {} · {}{INTERACTIVE_RESET}",
                ui_text(UiText::Background), background_tasks.len(), ui_text(UiText::TasksUnit), running, ui_text(UiText::RunningUnit), ui_text(UiText::ManageJobs)
            );
        } else {
            terminal_notice!(
                "  {}  {} {} · {running} {} · {}",
                ui_text(UiText::Background),
                background_tasks.len(),
                ui_text(UiText::TasksUnit),
                ui_text(UiText::RunningUnit),
                ui_text(UiText::ManageJobs)
            );
        }
        for task in background_tasks.iter().take(3) {
            let state = if task.status.to_ascii_lowercase().starts_with("running") {
                "●"
            } else {
                "○"
            };
            let command = truncate_display_width(&task.command, 88);
            let elapsed =
                format_elapsed_duration(Duration::from_secs_f64(task.elapsed_secs.max(0.0)));
            if interactive_color_enabled() {
                terminal_notice!(
                    "  {INTERACTIVE_DIM}{state} {} · pid={} · {elapsed} · {command}{INTERACTIVE_RESET}",
                    task.id, task.pid
                );
            } else {
                terminal_notice!(
                    "  {state} {} · pid={} · {elapsed} · {command}",
                    task.id, task.pid
                );
            }
        }
        if background_tasks.len() > 3 {
            if interactive_color_enabled() {
                terminal_notice!(
                    "  {INTERACTIVE_DIM}{} {}{INTERACTIVE_RESET}",
                    background_tasks.len() - 3,
                    ui_text(UiText::MoreTasks)
                );
            } else {
                terminal_notice!(
                    "  {} {}",
                    background_tasks.len() - 3,
                    ui_text(UiText::MoreTasks)
                );
            }
        }
    }
}

fn format_elapsed_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if days > 0 {
        return if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        };
    }
    if hours > 0 {
        return if minutes > 0 {
            format!("{hours}h {minutes}m")
        } else {
            format!("{hours}h")
        };
    }
    if minutes > 0 {
        return if seconds > 0 {
            format!("{minutes}m {seconds}s")
        } else {
            format!("{minutes}m")
        };
    }
    format!("{seconds}s")
}

fn print_interactive_welcome(config: &Config) -> Result<(), String> {
    let width = terminal_columns().saturating_sub(4).clamp(30, 72);
    let use_color = interactive_color_enabled();
    let reset = if use_color { INTERACTIVE_RESET } else { "" };
    let _ = config;
    let tagline = tr(Message::InteractiveWelcomeTagline);
    let help = tr(Message::InteractiveWelcomeHint);
    let rendered = if use_color {
        format!(
            "  {INTERACTIVE_ACCENT}Nola{reset}{INTERACTIVE_DIM}  {tagline}{reset}\n  {INTERACTIVE_DIM}{}{reset}\n\n",
            truncate_display_width(help, width)
        )
    } else {
        format!(
            "  Nola  {tagline}\n  {}\n\n",
            truncate_display_width(help, width)
        )
    };
    crate::terminal_ui::commit_history(&rendered);
    Ok(())
}

fn read_interactive_prompt_line(
    input_history: &[String],
    history_cursor: &mut Option<usize>,
    show_prompt: bool,
    prompt_footer: Option<&str>,
    config: &Config,
    initial_draft: Option<&str>,
    view_store: &mut Option<crate::cli_view_state::ViewStore>,
) -> io::Result<Option<String>> {
    if !io::stdin().is_terminal() {
        let mut raw_line = String::new();
        let read = io::stdin().read_line(&mut raw_line)?;
        if read == 0 {
            return Ok(None);
        }
        return Ok(normalize_interactive_prompt(
            &raw_line,
            input_history,
            history_cursor,
        ));
    }
    let Ok(_raw_mode) = InteractiveRawModeGuard::new() else {
        let mut raw_line = String::new();
        let read = io::stdin().read_line(&mut raw_line)?;
        if read == 0 {
            return Ok(None);
        }
        return Ok(normalize_interactive_prompt(
            &raw_line,
            input_history,
            history_cursor,
        ));
    };
    // terminal_ui remains the sole owner of the bottom surface in both idle
    // and running phases. The former hand-off to a second ANSI renderer was
    // responsible for blank gaps, cursor drift and a composer that vanished
    // after Enter.
    let inherited_surface_rows = 0;
    let snapshot = interaction::interactive_composer_snapshot();
    let mut line = initial_draft.unwrap_or(&snapshot.draft).to_string();
    let mut cursor = if snapshot.draft == line { snapshot.cursor } else { line.len() };
    let mut pasted_contents = Vec::new();
    INTERACTIVE_COMPOSER_CURSOR_ROW.with(|row| row.set(0));
    INTERACTIVE_COMPOSER_FRAME_ROWS.with(|rows| rows.set(inherited_surface_rows));
    if show_prompt {
        draw_interactive_prompt_area(&line, cursor, prompt_footer, config)?;
    }
    _raw_mode.enable_bracketed_paste()?;
    let mut background_notification_pending = true;
    let mut scheduler_event_cursor = 0_i64;

    loop {
        let expanded = expand_interactive_pasted_content(&line, &pasted_contents);
        let expanded_cursor = expand_interactive_pasted_content(&line[..cursor], &pasted_contents).len();
        persist_interactive_view_state(view_store, expanded, expanded_cursor);
        if show_prompt && background_notification_pending {
            // Snapshot before reconciliation. Events appended concurrently
            // after this read remain above the consumed cursor and cause the
            // next wake instead of being accidentally swallowed.
            let observed_scheduler_cursor =
                task_runtime::latest_scheduler_event_cursor().unwrap_or(scheduler_event_cursor);
            // A durable task completion is also a scheduler wake-up. Reconcile
            // before projecting notifications so a Shell or Agent releasing a
            // shared slot immediately advances the next fair queued task. Do
            // not make progress depend on the user entering another command.
            let invocation_request = config.host_invocation_request().map_err(io::Error::other)?;
            task_runtime::reconcile_supervised_tasks(&invocation_request);
            if config.save_session && config.session_id.is_some() {
                if let Err(error) = config.host_invocation_request()
                    .and_then(|request| core_agent_runtime::drain_idle_agent_core_commands(&request)) {
                    llm::log_local_debug_payload(
                        "idle_agent_command_drain_failed",
                        &json!({
                            "type": "idle_agent_command_drain_failed",
                            "session_id": config.session_id,
                            "error": error,
                        }),
                    );
                }
            }
            // Notification replay is durable-session behavior. An ephemeral
            // composer must become interactive immediately and must not probe
            // the Store merely because it has an in-memory runtime ID.
            if config.save_session {
                if let Ok(notifications) = tools::take_background_task_notifications(
                    &config.allowed_dirs,
                    config.session_id.as_deref(),
                ) {
                    if !notifications.is_empty() {
                        clear_interactive_prompt_area(prompt_footer)?;
                        print_background_task_notifications(&notifications)?;
                        for notification in &notifications {
                            llm::log_local_debug_payload(
                                "background_task_notification",
                                &json!({
                                    "type": "background_task_notification",
                                    "task_id": notification.id,
                                    "status": notification.status,
                                    "command": notification.command,
                                    "elapsed_secs": notification.elapsed_secs,
                                }),
                            );
                        }
                        draw_interactive_prompt_area(&line, cursor, prompt_footer, config)?;
                    }
                }
            }
            if let Ok(notifications) = task_runtime::take_runtime_task_notifications(&invocation_request) {
                if !notifications.is_empty() {
                    clear_interactive_prompt_area(prompt_footer)?;
                    let mut passive_notifications = Vec::new();
                    for notification in notifications {
                        llm::log_local_debug_payload(
                            "runtime_task_notification",
                            &json!({
                                "type": "runtime_task_notification",
                                "task_id": notification.id,
                                "status": notification.status,
                                "agent_type": notification.agent_type,
                                "elapsed_secs": notification.elapsed_secs,
                            }),
                        );
                        if notification.status == "waiting_permission" {
                            if let Some(prompt) =
                                task_runtime::permission_prompt_for_task(&invocation_request, &notification.id)
                            {
                                if notification.delivery.occurrence_id != format!("permission:{}", prompt.request_id()) {
                                    continue;
                                }
                                let choice = interaction::prompt_permission(&prompt.prompt);
                                restore_interactive_raw_mode()?;
                                let outcome = task_runtime::resolve_task_permission_choice(
                                    &invocation_request,
                                    &prompt,
                                    choice,
                                );
                                if outcome.error {
                                    terminal_notice!("\r  {}", format_error(&outcome.display));
                                } else {
                                    terminal_notice!("\r  {}", outcome.display);
                                }
                                // A resolved permission no longer needs a passive
                                // notification; the resolution is its canonical exit.
                                continue;
                            }
                            // The exact permission occurrence was resolved
                            // after this list was read. Do not turn its stale
                            // snapshot into a passive approval notification.
                            continue;
                        }
                        passive_notifications.push(notification);
                    }
                    if !passive_notifications.is_empty() {
                        print_runtime_task_notifications(&passive_notifications)?;
                    }
                    draw_interactive_prompt_area(&line, cursor, prompt_footer, config)?;
                }
            }
            scheduler_event_cursor = scheduler_event_cursor.max(observed_scheduler_cursor);
            background_notification_pending = false;
        }
        // Event cursors provide the fast cross-process wake-up path. Keep a
        // bounded authoritative-state probe as a lost-edge fallback: a worker
        // can append its terminal event just before releasing the shared
        // scheduler slot, and external Agent commands may not append a runtime
        // task event at all. This applies to ephemeral and saved CLIs alike.
        let probe_durable_commands = show_prompt;
        if !wait_for_interactive_input_or_background(
            &mut scheduler_event_cursor,
            probe_durable_commands,
        )
        .map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("{} {err}", tr(Message::FailedReadKeyEventPrefix)),
            )
        })? {
            background_notification_pending = true;
            continue;
        }
        let event = read().map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("{} {err}", tr(Message::FailedReadKeyEventPrefix)),
            )
        })?;
        if let Event::Resize(_, _) = event {
            // Keep the draft, cursor and footer in one projection when the
            // terminal changes size. The prior frame is cleared using its
            // recorded geometry before the new frame is laid out against the
            // current terminal dimensions.
            draw_interactive_prompt_area(&line, cursor, prompt_footer, config)?;
            // Equal composer text does not mean equal terminal geometry.
            crate::terminal_ui::invalidate_presented_screen();
            continue;
        }
        if let Event::Key(key) = event {
            if crate::terminal_ui::scroll_history_key(&key) { continue; }
            if crate::composer_media::paste_key(&key) {
                match crate::composer_media::paste_image() {
                    Ok(marker) => { line.insert_str(cursor, &marker); cursor += marker.len(); }
                    Err(error) => terminal_notice!("{}", crate::composer_media::paste_error(&error)),
                }
                redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                continue;
            }
            if is_interactive_submit_event(&key) {
                persist_interactive_view_state(view_store, String::new(), 0);
                finish_interactive_prompt_line(&line, prompt_footer)?;
                interaction::clear_interactive_execution_draft();
                if line.is_empty() {
                    return Ok(Some(String::new()));
                }
                let prompt = expand_interactive_pasted_content(&line, &pasted_contents)
                    .trim()
                    .to_string();
                return Ok(Some(prompt));
            }
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if crate::composer::edit(&mut line, &mut cursor, &key) {
                redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                continue;
            }
            match key.code {
                KeyCode::Esc => {
                    line.clear();
                    cursor = 0;
                    pasted_contents.clear();
                    redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                }
                KeyCode::Tab => {
                    if let Some(command) = first_matching_slash_completion(&line, config) {
                        line = command.name.to_string();
                        cursor = line.len();
                        pasted_contents.clear();
                        redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    }
                }
                KeyCode::Up => {
                    if let Some(history) =
                        interactive_history_prompt("\x1b[A", input_history, history_cursor)
                    {
                        line = history;
                        cursor = line.len();
                        pasted_contents.clear();
                        redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    }
                }
                KeyCode::Down => {
                    if let Some(history) =
                        interactive_history_prompt("\x1b[B", input_history, history_cursor)
                    {
                        line = history;
                        cursor = line.len();
                        pasted_contents.clear();
                        redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    } else {
                        line.clear();
                        cursor = 0;
                        pasted_contents.clear();
                        redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    }
                }
                KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    interaction::clear_interactive_execution_draft();
                    if !line.is_empty() {
                        line.clear();
                        cursor = 0;
                        pasted_contents.clear();
                        *history_cursor = None;
                        redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                        continue;
                    }
                    finish_interactive_prompt_line("", prompt_footer)?;
                    return Ok(None);
                }
                KeyCode::Char('d' | 'D') if key.modifiers.contains(KeyModifiers::CONTROL) && line.is_empty() => {
                    finish_interactive_prompt_line("", prompt_footer)?;
                    return Ok(None);
                }
                _ => {}
            }
        } else if let Event::Paste(text) = event {
            match crate::composer_media::paste_path(&text) {
                Ok(Some(marker)) => {
                    line.insert_str(cursor, &marker); cursor += marker.len();
                    redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    continue;
                }
                Err(error) => {
                    terminal_notice!("{}", crate::composer_media::paste_error(&error));
                    redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    continue;
                }
                Ok(None) => {}
            }
            let sanitized = normalize_interactive_paste(&text);
            if sanitized.chars().any(|ch| !ch.is_whitespace()) {
                let placeholder = format!("[Pasted Content {} chars]", sanitized.chars().count());
                line.insert_str(cursor, &placeholder);
                cursor += placeholder.len();
                pasted_contents.push(InteractivePastedContent {
                    placeholder,
                    content: sanitized,
                });
                redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
            }
        }
    }
}

/// Terminal implementations do not agree on how a return key is reported
/// after raw-mode ownership changes. Crossterm normally emits `Enter`, while
/// PTYs may emit Ctrl-M, Ctrl-J, or a newline character. Unmodified presses
/// submit; modified Enter edits the draft. Repeat/release events must never
/// manufacture a second submission from the same physical key press.
fn is_interactive_submit_event(key: &KeyEvent) -> bool {
    crate::composer::is_submit(key)
}

#[cfg(test)]
#[test]
fn interactive_submit_accepts_terminal_return_variants_but_not_release() {
    assert!(is_interactive_submit_event(&KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(!is_interactive_submit_event(&KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        KeyEventKind::Repeat,
    )));
    assert!(is_interactive_submit_event(&KeyEvent::new(
        KeyCode::Char('m'),
        KeyModifiers::CONTROL,
    )));
    assert!(is_interactive_submit_event(&KeyEvent::new(
        KeyCode::Char('j'),
        KeyModifiers::CONTROL,
    )));
    assert!(!is_interactive_submit_event(&KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    )));
    assert!(!is_interactive_submit_event(&KeyEvent::new(
        KeyCode::Char('m'),
        KeyModifiers::NONE,
    )));
}

fn normalize_interactive_paste(text: &str) -> String {
    crate::composer::normalize_paste(text)
}

fn expand_interactive_pasted_content(
    input: &str,
    pasted_contents: &[InteractivePastedContent],
) -> String {
    let mut expanded = input.to_string();
    for pasted in pasted_contents {
        if let Some(start) = expanded.find(&pasted.placeholder) {
            let end = start + pasted.placeholder.len();
            expanded.replace_range(start..end, &pasted.content);
        }
    }
    expanded
}

#[cfg(unix)]
fn wait_for_interactive_input_or_background(
    scheduler_event_cursor: &mut i64,
    probe_durable_commands: bool,
) -> io::Result<bool> {
    let Some(notification_fd) = tools::background_task_notification_fd() else {
        return Ok(true);
    };
    let mut poll_fds = [
        libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: notification_fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];

    let next_durable_command_probe = Instant::now() + Duration::from_secs(1);
    loop {
        // Resize and other terminal events can live in Crossterm's queue
        // without making stdin readable. Recheck after timeout or EINTR,
        // rather than waiting for a key or a background-task notification.
        if crate::terminal_input::poll(Duration::ZERO)? {
            return Ok(true);
        }
        let ready =
            unsafe { libc::poll(poll_fds.as_mut_ptr(), poll_fds.len() as libc::nfds_t, 250) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready == 0 {
            match task_runtime::latest_scheduler_event_cursor() {
                Ok(latest) if latest > *scheduler_event_cursor => return Ok(false),
                Ok(_) if probe_durable_commands && Instant::now() >= next_durable_command_probe => {
                    // Runtime task events have an immediate cross-process
                    // wake-up path. The bounded probe also covers external
                    // Agent commands and the narrow window where a terminal
                    // event becomes visible just before its slot is released.
                    return Ok(false);
                }
                Ok(_) => continue,
                // Preserve recovery behavior if the cursor probe itself
                // fails. The following reconcile will surface durable state
                // once the Store is available again.
                Err(_) => return Ok(false),
            }
        }

        let notification_events = poll_fds[1].revents;
        if notification_events & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
            return Ok(false);
        }
        let input_events = poll_fds[0].revents;
        if input_events & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
            return Ok(true);
        }
    }
}

#[cfg(not(unix))]
fn wait_for_interactive_input_or_background(
    scheduler_event_cursor: &mut i64,
    probe_durable_commands: bool,
) -> io::Result<bool> {
    let next_durable_command_probe = Instant::now() + Duration::from_secs(1);
    loop {
        if crate::terminal_input::poll(Duration::from_millis(250))? {
            return Ok(true);
        }
        match task_runtime::latest_scheduler_event_cursor() {
            Ok(latest) if latest > *scheduler_event_cursor => return Ok(false),
            Ok(_) if probe_durable_commands && Instant::now() >= next_durable_command_probe => {
                return Ok(false)
            }
            Ok(_) => {}
            Err(_) => return Ok(false),
        }
    }
}

fn print_background_task_notifications(
    notifications: &[tools::BackgroundTaskNotification],
) -> io::Result<()> {
    let width = terminal_columns().saturating_sub(8).max(24);
    for notification in notifications {
        let success = notification.canonical_state == "completed";
        let marker = if success { "✓" } else { "!" };
        let label = background_task_notification_label(i18n::current_language(), &notification.canonical_state);
        let elapsed = if notification.elapsed_secs >= 60.0 {
            format!(
                "{}m {}s",
                (notification.elapsed_secs / 60.0) as u64,
                notification.elapsed_secs as u64 % 60
            )
        } else {
            format!("{:.1}s", notification.elapsed_secs)
        };
        let command = truncate_display_width(&notification.command, width);
        let text = if interactive_color_enabled() {
            let color = if success {
                "\x1b[38;5;114m"
            } else {
                "\x1b[38;5;203m"
            };
            format!(
                "  {color}{marker}  {label}{INTERACTIVE_RESET}  {INTERACTIVE_VALUE}{command}{INTERACTIVE_RESET}  {INTERACTIVE_DIM}{elapsed}{INTERACTIVE_RESET}"
            )
        } else {
            format!("  {marker}  {label}  {command}  {elapsed}")
        };
        tools::deliver_background_task_notification(notification, || crate::terminal_ui::deliver_history(&text)).map_err(io::Error::other)?;
    }
    io::stdout().flush()
}

fn print_runtime_task_notifications(
    notifications: &[task_runtime::RuntimeTaskNotification],
) -> io::Result<()> {
    let width = terminal_columns().saturating_sub(8).max(24);
    for notification in notifications {
        let waiting_permission = matches!(notification.status.as_str(), "waiting_permission" | "waiting_input");
        let action = if notification.status == "waiting_input" { "/questions".to_owned() } else { format!("/tasks {}", notification.id) };
        let success = notification.status == "completed";
        let marker = if waiting_permission {
            "◆"
        } else if success {
            "✓"
        } else {
            "!"
        };
        let label = runtime_task_notification_label(&notification.status);
        let elapsed =
            format_elapsed_duration(Duration::from_secs_f64(notification.elapsed_secs.max(0.0)));
        let title = truncate_display_width(&notification.title, width);
        let text = if interactive_color_enabled() {
            let color = if waiting_permission {
                "\x1b[38;5;215m"
            } else if success {
                "\x1b[38;5;114m"
            } else {
                "\x1b[38;5;203m"
            };
            format!(
                "  {color}{marker}  {label}{INTERACTIVE_RESET}  {INTERACTIVE_VALUE}{title}{INTERACTIVE_RESET}  {INTERACTIVE_DIM}{} · {elapsed} · {}{INTERACTIVE_RESET}",
                notification.id,
                action
            )
        } else {
            format!(
                "  {marker}  {label}  {title}  {} · {elapsed} · {}",
                notification.id,
                action
            )
        };
        task_runtime::deliver_runtime_task_notification(notification, || crate::terminal_ui::deliver_history(&text))
            .map_err(io::Error::other)?;
    }
    io::stdout().flush()
}

fn background_task_notification_label(language: i18n::Language, state: &str) -> &'static str {
    match (language, state) {
        (i18n::Language::ZhHans, "completed") => "后台任务完成",
        (i18n::Language::ZhHans, "failed") => "后台任务失败",
        (i18n::Language::ZhHans, "cancelled") => "后台任务已取消",
        (i18n::Language::ZhHans, _) => "后台任务状态未确认",
        (i18n::Language::De, "completed") => "Hintergrundaufgabe abgeschlossen",
        (i18n::Language::De, "failed") => "Hintergrundaufgabe fehlgeschlagen",
        (i18n::Language::De, "cancelled") => "Hintergrundaufgabe abgebrochen",
        (i18n::Language::De, _) => "Status der Hintergrundaufgabe unbestätigt",
        (i18n::Language::Es, "completed") => "Tarea en segundo plano completada",
        (i18n::Language::Es, "failed") => "Tarea en segundo plano fallida",
        (i18n::Language::Es, "cancelled") => "Tarea en segundo plano cancelada",
        (i18n::Language::Es, _) => "Estado de la tarea sin confirmar",
        (i18n::Language::Fr, "completed") => "Tâche en arrière-plan terminée",
        (i18n::Language::Fr, "failed") => "Échec de la tâche en arrière-plan",
        (i18n::Language::Fr, "cancelled") => "Tâche en arrière-plan annulée",
        (i18n::Language::Fr, _) => "État de la tâche non confirmé",
        (i18n::Language::Ja, "completed") => "バックグラウンドタスクが完了しました",
        (i18n::Language::Ja, "failed") => "バックグラウンドタスクが失敗しました",
        (i18n::Language::Ja, "cancelled") => "バックグラウンドタスクがキャンセルされました",
        (i18n::Language::Ja, _) => "バックグラウンドタスクの状態は未確認です",
        (i18n::Language::Ru, "completed") => "Фоновая задача выполнена",
        (i18n::Language::Ru, "failed") => "Фоновая задача завершилась ошибкой",
        (i18n::Language::Ru, "cancelled") => "Фоновая задача отменена",
        (i18n::Language::Ru, _) => "Статус фоновой задачи не подтверждён",
        (i18n::Language::En, "completed") => "Background task completed",
        (i18n::Language::En, "failed") => "Background task failed",
        (i18n::Language::En, "cancelled") => "Background task cancelled",
        (i18n::Language::En, _) => "Background task status unconfirmed",
    }
}

fn runtime_task_notification_label(status: &str) -> &'static str {
    match (i18n::current_language(), status) {
        (i18n::Language::ZhHans, "waiting_input") => "子代理等待回答",
        (i18n::Language::De, "waiting_input") => "Agent-Aufgabe wartet auf Antwort",
        (i18n::Language::Es, "waiting_input") => "La tarea del agente espera respuesta",
        (i18n::Language::Fr, "waiting_input") => "La tâche de l’agent attend une réponse",
        (i18n::Language::Ja, "waiting_input") => "エージェントタスクは回答を待っています",
        (i18n::Language::Ru, "waiting_input") => "Задача агента ожидает ответа",
        (i18n::Language::En, "waiting_input") => "Agent task needs an answer",
        (i18n::Language::ZhHans, "waiting_permission") => "子代理等待授权",
        (i18n::Language::ZhHans, "sealed") => "子代理执行已封存",
        (i18n::Language::ZhHans, "failed") => "子代理失败",
        (i18n::Language::ZhHans, "cancelled") => "子代理已取消",
        (i18n::Language::De, "waiting_permission") => "Agent-Aufgabe wartet auf Berechtigung",
        (i18n::Language::De, "sealed") => "Agent-Ausführung beendet",
        (i18n::Language::De, "failed") => "Agent-Aufgabe fehlgeschlagen",
        (i18n::Language::De, "cancelled") => "Agent-Aufgabe abgebrochen",
        (i18n::Language::Es, "waiting_permission") => "La tarea del agente espera permiso",
        (i18n::Language::Es, "sealed") => "Ejecución del agente finalizada",
        (i18n::Language::Es, "failed") => "La tarea del agente falló",
        (i18n::Language::Es, "cancelled") => "Tarea del agente cancelada",
        (i18n::Language::Fr, "waiting_permission") => "La tâche de l’agent attend une autorisation",
        (i18n::Language::Fr, "sealed") => "Exécution de l’agent terminée",
        (i18n::Language::Fr, "failed") => "La tâche de l’agent a échoué",
        (i18n::Language::Fr, "cancelled") => "Tâche de l’agent annulée",
        (i18n::Language::Ja, "waiting_permission") => "エージェントタスクは権限を待っています",
        (i18n::Language::Ja, "sealed") => "エージェントの実行が終了しました",
        (i18n::Language::Ja, "failed") => "エージェントタスクが失敗しました",
        (i18n::Language::Ja, "cancelled") => "エージェントタスクがキャンセルされました",
        (i18n::Language::Ru, "waiting_permission") => "Задача агента ожидает разрешения",
        (i18n::Language::Ru, "sealed") => "Выполнение агента завершено",
        (i18n::Language::Ru, "failed") => "Задача агента завершилась с ошибкой",
        (i18n::Language::Ru, "cancelled") => "Задача агента отменена",
        (i18n::Language::En, "waiting_permission") => "Agent task needs permission",
        (i18n::Language::En, "sealed") => "Agent execution sealed",
        (i18n::Language::En, "failed") => "Agent task failed",
        (i18n::Language::En, "cancelled") => "Agent task cancelled",
        (i18n::Language::ZhHans, _) => "子代理状态已更新",
        (i18n::Language::De, _) => "Agent-Aufgabe aktualisiert",
        (i18n::Language::Es, _) => "Tarea del agente actualizada",
        (i18n::Language::Fr, _) => "Tâche de l’agent mise à jour",
        (i18n::Language::Ja, _) => "エージェントタスクが更新されました",
        (i18n::Language::Ru, _) => "Задача агента обновлена",
        (i18n::Language::En, _) => "Agent task updated",
    }
}

fn take_interactive_composer_frame() -> (usize, usize) {
    let cursor_row = INTERACTIVE_COMPOSER_CURSOR_ROW.with(|row| {
        let current = row.get();
        row.set(0);
        current
    });
    let frame_rows = INTERACTIVE_COMPOSER_FRAME_ROWS.with(|rows| {
        let current = rows.get();
        rows.set(0);
        current
    });
    (cursor_row, frame_rows)
}

fn clear_interactive_composer_frame() -> usize {
    let (_, frame_rows) = take_interactive_composer_frame();
    crate::terminal_ui::clear_idle_frame();
    frame_rows
}

fn draw_interactive_prompt_area(
    line: &str,
    cursor: usize,
    footer: Option<&str>,
    config: &Config,
) -> io::Result<()> {
    let completion_line = composer_slash_completion_line(line, config);
    let dynamic_footer = interactive_footer_with_background_status(footer, config);
    draw_composer_frame(
        line,
        cursor,
        dynamic_footer.as_deref(),
        completion_line.as_deref(),
    )
}

fn draw_composer_frame(
    line: &str,
    cursor: usize,
    footer: Option<&str>,
    completion_line: Option<&str>,
) -> io::Result<()> {
    interaction::update_interactive_composer(line, cursor);
    let input_line = composer_input_line(line, cursor);
    let (cursor_row, cursor_col) = composer_cursor_position(line, cursor);
    let mut frame_lines = input_line
        .lines()
        .map(|value| value.trim_end_matches('\r').to_string())
        .collect::<Vec<_>>();
    if frame_lines.is_empty() {
        frame_lines.push(String::new());
    }
    crate::terminal_ui::set_footer(footer.unwrap_or_default());
    if let Some(completion_line) = completion_line {
        frame_lines.extend(
            completion_line
                .lines()
                .map(|value| value.trim_end_matches('\r').to_string()),
        );
    }

    let (terminal_columns, terminal_rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let frame_rows = frame_lines.len().min(usize::from(terminal_rows.max(1)));
    let absolute_cursor_column = (cursor_col.saturating_add(1) as u16)
        .min(terminal_columns.max(1));
    crate::terminal_ui::present_idle_frame(
        frame_lines,
        cursor_row,
        absolute_cursor_column,
    );
    INTERACTIVE_COMPOSER_CURSOR_ROW.with(|row| row.set(cursor_row));
    INTERACTIVE_COMPOSER_FRAME_ROWS.with(|rows| rows.set(frame_rows));
    Ok(())
}

fn redraw_interactive_prompt_line(
    line: &str,
    cursor: usize,
    footer: Option<&str>,
    config: &Config,
) -> io::Result<()> {
    draw_interactive_prompt_area(line, cursor, footer, config)
}

fn finish_interactive_prompt_line(line: &str, _footer: Option<&str>) -> io::Result<()> {
    clear_interactive_composer_frame();
    crate::terminal_ui::clear_last_wrapped_block();
    if !line.trim().is_empty() {
        commit_interactive_user_history_line(line);
    }
    io::stdout().flush()
}

fn commit_interactive_user_history_line(line: &str) {
    let submitted = interactive_session_replay_user_line(line);
    // One deliberate blank row separates durable conversation cells. Both a
    // normal Enter and input queued while a turn is finishing use this exact
    // history path, so fast follow-ups cannot bypass terminal ownership and
    // disappear when the next live surface redraws.
    crate::terminal_ui::commit_history(&format!("\n{submitted}\n\n"));
}

fn clear_interactive_prompt_area(_footer: Option<&str>) -> io::Result<()> {
    clear_interactive_composer_frame();
    io::stdout().flush()
}

fn composer_input_line(line: &str, cursor: usize) -> String {
    let line_width = terminal_columns().saturating_sub(1).max(1);
    let prefix = if line_width > display_width(INTERACTIVE_PROMPT_PREFIX) {
        INTERACTIVE_PROMPT_PREFIX
    } else {
        ""
    };
    let input_width = line_width.saturating_sub(display_width(prefix));
    let (visible_lines, _cursor_row, _cursor_col) =
        composer_input_multiline(line, cursor, input_width, prefix);
    // 用 \n 连接多行，每行用 interactive_input_bar 渲染
    let continuation_prefix = " ".repeat(display_width(prefix));
    visible_lines
        .iter()
        .enumerate()
        .map(|(i, (text, is_placeholder))| {
            let clear = i == 0;
            let row_prefix = if i == 0 {
                prefix
            } else {
                continuation_prefix.as_str()
            };
            interactive_input_bar_line(text, *is_placeholder, clear, row_prefix, line_width)
        })
        .collect::<Vec<_>>()
        .join("\n\r")
}

fn composer_cursor_position(line: &str, cursor: usize) -> (usize, usize) {
    let line_width = terminal_columns().saturating_sub(1).max(1);
    let prefix = if line_width > display_width(INTERACTIVE_PROMPT_PREFIX) {
        INTERACTIVE_PROMPT_PREFIX
    } else {
        ""
    };
    let input_width = line_width.saturating_sub(display_width(prefix));
    let (_visible_lines, cursor_row, cursor_col) =
        composer_input_multiline(line, cursor, input_width, prefix);
    (cursor_row, cursor_col)
}

/// 将输入文本按终端宽度折行，返回 (各行文本, 光标所在行号(0-based), 光标所在列)
fn composer_input_multiline(
    line: &str,
    cursor: usize,
    width: usize,
    prefix: &str,
) -> (Vec<(String, bool)>, usize, usize) {
    let cursor = cursor.min(line.len());
    let cursor = if line.is_char_boundary(cursor) {
        cursor
    } else {
        previous_char_boundary(line, cursor).unwrap_or(0)
    };
    let prefix_width = display_width(prefix);

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    let mut cursor_row = 0usize;
    let mut cursor_col = prefix_width;
    let mut found_cursor = false;
    // 光标在当前行中之前字符的累计宽度
    let mut cursor_line_width_before = 0usize;

    for (idx, ch) in line.char_indices() {
        // 显式换行符：强制换行
        if ch == '\n' {
            if !found_cursor && idx >= cursor {
                cursor_row = lines.len();
                cursor_col = prefix_width + cursor_line_width_before;
                found_cursor = true;
            }
            lines.push(std::mem::take(&mut current));
            current_width = 0;
            cursor_line_width_before = 0;
            continue;
        }
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width > 0 && current_width + ch_width > width {
            // 当前行满了，换行
            if !found_cursor && idx > cursor {
                cursor_row = lines.len();
                cursor_col = prefix_width + cursor_line_width_before;
                found_cursor = true;
            }
            lines.push(std::mem::take(&mut current));
            current_width = 0;
            cursor_line_width_before = 0;
            if !found_cursor && idx == cursor {
                cursor_row = lines.len();
                cursor_col = prefix_width;
                found_cursor = true;
            }
        }
        if !found_cursor && idx < cursor {
            cursor_line_width_before += ch_width;
        }
        current.push(ch);
        current_width += ch_width;
    }

    // 光标在末尾（或光标位置超出文本长度）
    if !found_cursor {
        cursor_row = lines.len();
        cursor_col = prefix_width + cursor_line_width_before;
    }

    // 最后一行
    lines.push(current);

    let placeholder = line.is_empty();
    let result: Vec<(String, bool)> = lines
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            // 只有第一行为空时才显示 placeholder
            (s, placeholder && i == 0)
        })
        .collect();

    (result, cursor_row, cursor_col)
}

fn interactive_input_bar_line(
    text: &str,
    is_placeholder: bool,
    clear: bool,
    prefix: &str,
    line_width: usize,
) -> String {
    interactive_input_bar_line_with_color(text, is_placeholder, clear, prefix, line_width, interactive_color_enabled())
}

fn interactive_input_bar_line_with_color(
    text: &str, is_placeholder: bool, clear: bool, prefix: &str, line_width: usize, color: bool,
) -> String {
    let visible_line =
        truncate_display_width_tail(text, line_width.saturating_sub(display_width(prefix)));
    let use_placeholder = is_placeholder && visible_line.is_empty();
    let visible = if use_placeholder {
        tr(Message::InteractiveInputPlaceholder)
    } else {
        visible_line.as_str()
    };
    let content_width = display_width(prefix).saturating_add(display_width(visible));
    let padding = " ".repeat(line_width.saturating_sub(content_width));
    let clear_prefix = if clear { "\x1b[2K" } else { "" };
    if color {
        let value_color = if use_placeholder {
            INTERACTIVE_DIM
        } else {
            INTERACTIVE_VALUE
        };
        format!(
            "{clear_prefix}{INTERACTIVE_COMPOSER_BG}{INTERACTIVE_ACCENT}{prefix}{value_color}{visible}{INTERACTIVE_COMPOSER_BG}{padding}{INTERACTIVE_RESET}"
        )
    } else if visible.is_empty() {
        format!("{clear_prefix}{prefix}")
    } else {
        format!("{clear_prefix}{prefix}{visible}")
    }
}

fn previous_char_boundary(line: &str, cursor: usize) -> Option<usize> {
    line.get(..cursor.min(line.len()))?
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
}

fn next_char_boundary(line: &str, cursor: usize) -> Option<usize> {
    let cursor = cursor.min(line.len());
    let ch = line.get(cursor..)?.chars().next()?;
    Some(cursor + ch.len_utf8())
}

fn previous_word_boundary(line: &str, cursor: usize) -> usize {
    let before = line.get(..cursor.min(line.len())).unwrap_or_default();
    let trimmed = before.trim_end_matches(char::is_whitespace);
    trimmed
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0)
}

fn interactive_input_bar(line: &str, placeholder: Option<&str>, clear: bool) -> String {
    let line_width = terminal_columns().saturating_sub(1).max(1);
    let prefix = if line_width > display_width(INTERACTIVE_PROMPT_PREFIX) {
        INTERACTIVE_PROMPT_PREFIX
    } else {
        ""
    };
    let input_width = line_width.saturating_sub(display_width(prefix));
    let visible_line = truncate_display_width_tail(line, input_width);
    let use_placeholder = visible_line.is_empty();
    let visible = if use_placeholder {
        placeholder.unwrap_or("")
    } else {
        visible_line.as_str()
    };
    let content_width = display_width(prefix).saturating_add(display_width(visible));
    let padding = " ".repeat(line_width.saturating_sub(content_width));
    let clear_prefix = if clear { "\x1b[2K" } else { "" };
    if interactive_color_enabled() {
        let value_color = if use_placeholder {
            INTERACTIVE_DIM
        } else {
            INTERACTIVE_VALUE
        };
        format!(
            "{clear_prefix}{INTERACTIVE_COMPOSER_BG}{INTERACTIVE_ACCENT}{prefix}{value_color}{visible}{INTERACTIVE_COMPOSER_BG}{padding}{INTERACTIVE_RESET}"
        )
    } else if visible.is_empty() {
        format!("{clear_prefix}{prefix}")
    } else {
        format!("{clear_prefix}{prefix}{visible}")
    }
}

fn composer_prompt_footer_line(footer: &str) -> String {
    let footer_lines: Vec<&str> = footer.lines().collect();
    let terminal_width = terminal_columns().saturating_sub(1).max(1);
    let mut lines: Vec<String> = footer_lines
        .into_iter()
        .map(|line| {
            let prefix = "  ";
            let content_width = terminal_width.saturating_sub(display_width(&prefix));
            let line = truncate_display_width(line, content_width);
            if interactive_color_enabled() {
                format!(
                    "\r{INTERACTIVE_RESET}\x1b[2K  {INTERACTIVE_DIM}{}{INTERACTIVE_RESET}",
                    line
                )
            } else {
                format!("\r\x1b[2K  {}", line)
            }
        })
        .collect();
    if lines.is_empty() {
        lines.push(format!("\x1b[2K"));
    }
    lines.join("\n")
}

fn composer_slash_completion_line(line: &str, config: &Config) -> Option<String> {
    let completions = matching_slash_commands_with_config(line, config);
    if completions.is_empty() {
        return None;
    }
    // Keep the final terminal column untouched. Writing exactly through the
    // right edge puts many terminals into deferred-wrap state, which makes the
    // next composer redraw advance one physical row even though our logical
    // line count did not change.
    let width = terminal_columns().saturating_sub(3).max(1);
    let mut parts = completions
        .into_iter()
        .map(|item| interactive_room_completion_label(&item.name).unwrap_or(item.name))
        .collect::<Vec<_>>();
    if parts.len() > 4 {
        parts.truncate(4);
    }
    let text = format!("tab {}", parts.join("   "));
    let text = truncate_display_width(&text, width);
    if interactive_color_enabled() {
        Some(format!(
            "{INTERACTIVE_RESET}\x1b[2K  {INTERACTIVE_DIM}{text}{INTERACTIVE_RESET}"
        ))
    } else {
        Some(format!("\x1b[2K  {text}"))
    }
}

struct InteractiveRawModeGuard;

impl InteractiveRawModeGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode().map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("{} {err}", tr(Message::InteractiveRawModeFailedPrefix)),
            )
        })?;
        Ok(Self)
    }

    fn enable_bracketed_paste(&self) -> io::Result<()> {
        execute!(io::stdout(), crossterm::event::EnableBracketedPaste)
    }
}

fn restore_interactive_raw_mode() -> io::Result<()> {
    enable_raw_mode().map_err(|err| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("{} {err}", tr(Message::InteractiveRawModeFailedPrefix)),
        )
    })?;
    execute!(io::stdout(), crossterm::event::EnableBracketedPaste)?;
    Ok(())
}

impl Drop for InteractiveRawModeGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), crossterm::event::DisableBracketedPaste);
        let _ = disable_raw_mode();
    }
}

fn reset_interactive_terminal_state() {
    let _ = disable_raw_mode();
    let _ = print!(
        "{INTERACTIVE_RESET}\x1b[?25h\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l"
    );
    let _ = io::stdout().flush();
}

struct InteractiveSessionUiStateGuard {
    enabled: bool,
}

impl InteractiveSessionUiStateGuard {
    fn new(enabled: bool) -> Option<Self> {
        enabled.then_some(Self { enabled: true })
    }

    fn restore(&mut self) {
        if self.enabled {
            crate::terminal_ui::end_interactive_session();
            reset_interactive_terminal_state();
            self.enabled = false;
        }
    }
}

impl Drop for InteractiveSessionUiStateGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn interactive_provider_label(config: &Config) -> String {
    if config.endpoint.trim().is_empty() {
        tr(Message::InteractiveProviderLocal).to_string()
    } else {
        let endpoint = config.endpoint.trim();
        if let Some((_, host_part)) = endpoint.split_once("://") {
            host_part.split('/').next().unwrap_or(host_part).to_string()
        } else {
            endpoint.to_string()
        }
    }
}

fn interactive_param_line(config: &Config) -> String {
    let mode = tr(Message::InteractiveAutoValue);
    let tools_label = tr(Message::InteractiveToolsLabel);
    let network_label = tr(Message::InteractiveNetworkLabel);
    if config.debug_mode {
        let tool_set = interactive_tool_set_text(config.tools_enabled, &config.force_tool_set);
        return format!(
            "{mode} · {tools_label}={tool_set} · {network_label}={}",
            interactive_network_policy_text(config.network_policy.as_str())
        );
    }
    mode.to_string()
}

fn interactive_tool_set_text(tools_enabled: bool, force_tool_set: &str) -> String {
    if !tools_enabled {
        return tr(Message::InteractiveStatusOff).to_string();
    }
    let force_tool_set = force_tool_set.trim();
    if force_tool_set.is_empty() {
        tr(Message::InteractiveAutoValue).to_string()
    } else {
        force_tool_set.to_string()
    }
}

fn interactive_network_policy_text(policy: &str) -> &str {
    match policy {
        "allow" => tr(Message::InteractiveNetworkAllowValue),
        "ask" => tr(Message::InteractiveNetworkAskValue),
        "deny" => tr(Message::InteractiveNetworkDenyValue),
        other => other,
    }
}

fn interactive_footer_with_background_status(
    footer: Option<&str>,
    _config: &Config,
) -> Option<String> {
    footer.map(str::to_string)
}

fn format_interactive_background_footer(footer: &str, running: usize, width: usize) -> String {
    if running == 0 {
        return footer.to_string();
    }
    let indicator = format!(
        "● {}: {running} · /tasks",
        tr(Message::InteractiveJobsTitle)
    );
    let indicator = truncate_display_width(&indicator, width);
    let mut lines = footer.lines();
    let first = lines.next().unwrap_or_default();
    let first = truncate_display_width_middle(first, width);
    std::iter::once(first)
        .filter(|line| !line.trim().is_empty())
        .chain(std::iter::once(indicator))
        .chain(lines.map(str::to_string))
        .collect::<Vec<_>>()
        .join("\n")
}

fn interactive_prompt_footer_line(
    config: &Config,
    session_id: Option<&str>,
    context_usage: Option<nola_core::agent::context_window::ContextUsageSnapshot>,
) -> Option<String> {
    let width = terminal_columns().saturating_sub(2).max(1);
    let model = config.model.trim();
    let model = if model.is_empty() {
        tr(Message::InteractiveDefaultModelLabel)
    } else {
        model
    };
    let params = interactive_param_line(config);
    let working_dir = compact_home_path(&config.project_root.to_string_lossy());
    let configured_window = config.runtime.compaction.context_window_tokens.unwrap_or(0);
    let context_info = context_usage
        .map(|usage| {
            agent::format_context_usage_footer(
                usage.used_tokens,
                if usage.context_window_tokens > 0 {
                    usage.context_window_tokens
                } else {
                    configured_window
                },
            )
        })
        .or_else(|| {
            session_id.and_then(|sid| {
                session::load_session(sid).ok().and_then(|saved| {
                    let used = context_budget::estimate_tokens(&saved.messages);
                    Some(agent::format_context_usage_footer(used, configured_window))
                })
            })
        })
        .or_else(|| {
            (configured_window > 0)
                .then(|| agent::format_context_usage_footer(0, configured_window))
        })
        .unwrap_or_else(|| format!("{} 0% · 0/—", tr(Message::InteractiveContextLabel)));
    let perm = PermissionLevel::from_config(config);
    let perm_label = compact_permission_label(perm.display());
    let mut primary_parts = Vec::new();
    primary_parts.push(context_info);
    let executor =
        crate::external_executor::execution_backend_display_name(&config.execution_backend);
    primary_parts.push(truncate_display_width(model, width.min(28)));
    if executor.trim() != "Nola" {
        primary_parts.push(format!(
            "{}: {executor}",
            tr(Message::InteractiveExecutorLabel)
        ));
    }
    if !config.debug_mode {
        primary_parts.push(truncate_display_width(&params, width.min(32)));
    }
    primary_parts.push(format!("[{perm_label}]"));
    if let Some(cache) = context_usage.and_then(|usage| usage.cache_usage) {
        let rate = cache.hit_rate_bps.map(|bps| format!("{:.1}%", f64::from(bps) / 100.0)).unwrap_or_else(|| "—".into());
        let scope = if i18n::current_language() == i18n::Language::ZhHans { "本次请求" } else { "Latest request" };
        primary_parts.insert(1.min(primary_parts.len()), format!("{scope} {} {rate}", ui_text(UiText::CacheHitRate)));
    }
    // Keep model and permission on a stable row even after a terminal resize.
    // Context and optional generation diagnostics use their own row.
    let mut primary = {
        let safety = format!("[{perm_label}]");
        let model_width = width.saturating_sub(display_width(&safety) + 3);
        let model_part = truncate_display_width(model, width.min(28));
        let details = primary_parts.iter().filter(|part| **part != safety && **part != model_part)
            .cloned().collect::<Vec<_>>().join(" · ");
        format!("{} · {safety}\n{}", truncate_display_width(model, model_width),
            truncate_display_width_middle(&details, width))
    };
    if executor.trim() != "Nola" {
        primary.push_str(&format!("\n{}: {executor}", tr(Message::InteractiveExecutorLabel)));
    }
    let task_line = config
        .save_session
        .then(|| task_runtime::core_task_footer_summary(session_id))
        .flatten()
        .map(|line| truncate_display_width(&line, width));
    if !config.debug_mode {
        return Some(match task_line {
            Some(task_line) => format!("{primary}\n{task_line}"),
            None => primary,
        });
    }

    let provider = interactive_provider_label(config);
    let provider_label = tr(Message::InteractiveProviderLabel);
    let endpoint_label = tr(Message::InteractiveEndpointLabel);
    let cwd_label = tr(Message::InteractiveWorkingDirLabel);
    // An ephemeral Agent turn still has a runtime session identifier, but it
    // does not own durable session state. Never let debug-only presentation
    // turn that identifier into a Store/Application Service read.
    let prompt_cache_diagnostic = config
        .save_session
        .then(|| session_id.and_then(restored_agent_prompt_cache_diagnostic))
        .flatten();
    let diagnostics = format_interactive_footer_diagnostics(
        provider_label,
        &provider,
        endpoint_label,
        &config.endpoint,
        cwd_label,
        &working_dir,
        config.debug_mode.then_some(params.as_str()),
        prompt_cache_diagnostic.as_ref(),
        width,
    );
    Some(match task_line {
        Some(task_line) => format!("{primary}\n{task_line}\n{diagnostics}"),
        None => format!("{primary}\n{diagnostics}"),
    })
}

fn compact_permission_label(label: &str) -> &str {
    label
        .split_once('（')
        .map(|(value, _)| value.trim())
        .or_else(|| label.split_once(" (").map(|(value, _)| value.trim()))
        .unwrap_or(label)
}

fn format_interactive_footer_diagnostics(
    provider_label: &str,
    provider: &str,
    endpoint_label: &str,
    endpoint: &str,
    cwd_label: &str,
    working_dir: &str,
    compact_prefix: Option<&str>,
    prompt_cache_diagnostic: Option<&nola_core::prompt_cache::PromptCacheDiagnostic>,
    width: usize,
) -> String {
    if width < 96 {
        let mut diagnostics = compact_prefix
            .filter(|value| !value.trim().is_empty())
            .map(|value| vec![value.to_string()])
            .unwrap_or_default();
        diagnostics.push(provider.to_string());
        diagnostics.push(truncate_display_path(working_dir, width.min(44)));
        return truncate_display_width_middle(&diagnostics.join(" · "), width);
    }
    let mut diagnostics = compact_prefix
        .filter(|value| !value.trim().is_empty())
        .map(|value| vec![value.to_string()])
        .unwrap_or_default();
    diagnostics.push(format!("{provider_label}: {provider}"));
    // The provider already communicates the host. On narrow terminals an
    // abbreviated endpoint adds noise and pushes the working directory out.
    // A full endpoint is only useful when there is enough room to keep it
    // readable. In ordinary split panes the provider host already identifies
    // the route; a middle-truncated URL is diagnostic noise.
    if width >= 144 && !endpoint.trim().is_empty() {
        diagnostics.push(format!(
            "{endpoint_label}: {}",
            compact_endpoint_for_footer(endpoint)
        ));
    }
    diagnostics.push(format!(
        "{cwd_label}: {}",
        truncate_display_path(working_dir, width.min(44))
    ));
    if let Some(diagnostic) = prompt_cache_diagnostic {
        diagnostics.push(format_prompt_cache_footer(diagnostic));
    }
    truncate_display_width_middle(&diagnostics.join(" · "), width)
}

fn format_prompt_cache_footer(
    diagnostic: &nola_core::prompt_cache::PromptCacheDiagnostic,
) -> String {
    use nola_core::prompt_cache::PromptCacheDiagnosticStatus;

    let status = match diagnostic.status {
        PromptCacheDiagnosticStatus::Disabled => ui_text(UiText::CacheDisabled),
        PromptCacheDiagnosticStatus::InvalidIdentity => ui_text(UiText::CacheInvalidIdentity),
        PromptCacheDiagnosticStatus::FirstRequest => ui_text(UiText::CacheFirstRequest),
        PromptCacheDiagnosticStatus::Hit => ui_text(UiText::CacheHit),
        PromptCacheDiagnosticStatus::Write => ui_text(UiText::CacheStored),
        PromptCacheDiagnosticStatus::Miss => ui_text(UiText::CacheMiss),
        PromptCacheDiagnosticStatus::StableCandidate => ui_text(UiText::CacheStableCandidate),
        PromptCacheDiagnosticStatus::Unknown => ui_text(UiText::CacheUnknown),
    };
    let mut parts = vec![format!("{}: {status}", ui_text(UiText::PromptCache))];
    if let Some(tokens) = diagnostic.cache_read_input_tokens {
        parts.push(format!("{} {tokens}", ui_text(UiText::CacheRead)));
    }
    if let Some(tokens) = diagnostic.cache_write_input_tokens {
        parts.push(format!("{} {tokens}", ui_text(UiText::CacheWrite)));
    }
    if !diagnostic.fragment_changes.is_empty() {
        parts.push(format!(
            "{} {}",
            ui_text(UiText::CacheSections),
            diagnostic.fragment_changes.len()
        ));
    }
    parts.join(" ")
}

fn restored_agent_context_usage(
    session_id: &str,
) -> Option<nola_core::agent::context_window::ContextUsageSnapshot> {
    let checkpoint =
        session::SessionStore::load_agent_checkpoint_for_session(session_id).ok()??;
    let state = checkpoint.get("state")?;
    let context = state
        .pointer("/conversation/context_window")
        .or_else(|| state.get("context_window"))?;
    let used = context.get("used_tokens")?.as_u64()? as usize;
    let window = context
        .get("config")?
        .get("context_window_tokens")?
        .as_u64()? as usize;
    let prompt_token_limit = context
        .get("config")?
        .get("prompt_token_limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(window);
    let usage_is_estimated = context
        .get("usage_origin")
        .and_then(Value::as_str)
        .map(|origin| origin == "estimated")
        .unwrap_or(true);
    (window > 0).then_some(nola_core::agent::context_window::ContextUsageSnapshot {
        cache_usage: context.pointer("/cumulative_usage/cache_usage").and_then(|value| serde_json::from_value(value.clone()).ok()),
        used_tokens: used,
        context_window_tokens: window,
        prompt_token_limit,
        usage_percent: used as f64 / window as f64 * 100.0,
        usage_is_estimated,
        ..Default::default()
    })
}

fn restored_agent_prompt_cache_diagnostic(
    session_id: &str,
) -> Option<nola_core::prompt_cache::PromptCacheDiagnostic> {
    let checkpoint =
        session::SessionStore::load_agent_checkpoint_for_session(session_id).ok()??;
    let state = checkpoint.get("state")?;
    let value = state
        .pointer("/conversation/context_window/cumulative_usage/prompt_cache_diagnostic")
        .or_else(|| state.pointer("/context_window/cumulative_usage/prompt_cache_diagnostic"))
        .or_else(|| state.get("prompt_cache_diagnostic"))?;
    serde_json::from_value(value.clone()).ok()
}

fn print_interactive_prompt_card(config: &Config, turn: usize) -> Result<(), String> {
    let working_dir = compact_home_path(&config.project_root.to_string_lossy());
    let workspace_root = config.project_root.display().to_string();
    let width = terminal_columns().max(20);
    let max_value_width = width.saturating_sub(INTERACTIVE_LABEL_WIDTH + 3).max(8);
    let use_color = interactive_color_enabled();
    let dim = if use_color { INTERACTIVE_DIM } else { "" };
    let label_color = if use_color { INTERACTIVE_LABEL } else { "" };
    let value_color = if use_color { INTERACTIVE_VALUE } else { "" };
    let reset = if use_color { INTERACTIVE_RESET } else { "" };

    let model = config.model.trim();
    let model = if model.is_empty() {
        tr(Message::InteractiveDefaultModelLabel)
    } else {
        model
    };
    let provider = interactive_provider_label(config);
    let params = interactive_param_line(config);
    let cwd = truncate_display_path(&working_dir, max_value_width);
    let workspace = truncate_display_path(&workspace_root, max_value_width);
    let mode = tr(Message::InteractiveAutoValue);
    let network = interactive_network_policy_text(config.network_policy.as_str());

    let mut rows: Vec<(String, String)> = Vec::new();
    rows.push((
        tr(Message::InteractiveModelLabel).to_string(),
        truncate_display_width(model, max_value_width),
    ));
    rows.push((
        tr(Message::InteractiveParamsLabel).to_string(),
        truncate_display_width(&params, max_value_width),
    ));
    rows.push((
        tr(Message::InteractiveWorkingDirLabel).to_string(),
        truncate_display_path(&cwd, max_value_width),
    ));
    if config.debug_mode {
        rows.push((
            tr(Message::InteractiveProviderLabel).to_string(),
            truncate_display_path(&provider, max_value_width),
        ));
    } else if workspace_root != working_dir {
        rows.push((
            tr(Message::InteractiveWorkspaceLabel).to_string(),
            truncate_display_path(&workspace, max_value_width),
        ));
    }
    if config.debug_mode {
        rows.push((
            tr(Message::InteractiveModeFieldLabel).to_string(),
            format!("{mode}"),
        ));
        rows.push((
            tr(Message::InteractiveNetworkFieldLabel).to_string(),
            format!("{network}"),
        ));
        rows.push((
            tr(Message::InteractiveEndpointLabel).to_string(),
            truncate_display_path(&config.endpoint, max_value_width),
        ));
    }

    if !config.debug_mode {
        let compact = interactive_prompt_compact_line(
            turn,
            model,
            &params,
            &cwd,
            &workspace,
            &provider,
            workspace_root.as_str() != working_dir,
            width,
        );
        let line = compact.unwrap_or_else(|| {
            let base = format!("nola[{turn}]");
            let parts = vec![
                truncate_display_width(model, 24),
                truncate_display_width(&params, 24),
                truncate_display_width(&cwd, 40),
            ];
            truncate_display_width(
                &format!("{} {}", base, parts.join("  ")),
                width.saturating_sub(2),
            )
        });
        let rendered = if use_color {
            format!("{label_color}{line}{reset}\n")
        } else {
            format!("{line}\n")
        };
        crate::terminal_ui::commit_history(&rendered);
        return Ok(());
    }

    let panel_width = width.saturating_sub(2).max(20).min(120);
    let label_column = rows
        .iter()
        .map(|(label, _)| display_width(label))
        .max()
        .unwrap_or(INTERACTIVE_LABEL_WIDTH);
    let inner_width = panel_width.saturating_sub(2);
    let value_column = inner_width.saturating_sub(label_column + 6).max(12);

    let title = format!(" nola[{}] ", turn);
    let title_width = display_width(&title);
    let inner_fill = inner_width.saturating_sub(title_width + 2);
    let left_fill = inner_fill / 2;
    let right_fill = inner_fill.saturating_sub(left_fill);
    let mut title_line = String::new();
    title_line.push_str(INTERACTIVE_PANEL_TOP_LEFT);
    title_line.push_str(&INTERACTIVE_PANEL_BORDER.repeat(left_fill));
    if inner_fill > 0 {
        title_line.push(' ');
    }
    title_line.push_str(&title);
    if inner_fill > 0 {
        title_line.push(' ');
    }
    title_line.push_str(&INTERACTIVE_PANEL_BORDER.repeat(right_fill));
    title_line.push_str(INTERACTIVE_PANEL_TOP_RIGHT);
    let mut output = Vec::with_capacity(rows.len().saturating_add(2));
    if use_color {
        output.push(format!(
            "{label_color}{title_line}{reset}",
            label_color = label_color,
            reset = reset
        ));
    } else {
        output.push(title_line);
    }

    for (label, value) in rows {
        let label = pad_right_width(&label, label_column);
        let value = truncate_display_width(&value, value_column);
        if use_color {
            output.push(format!(
                "{side} {label_color}{label}{reset} : {dim}{value_color}{value}{reset} {side}",
                side = INTERACTIVE_PANEL_SIDE,
                label_color = label_color,
                value_color = value_color,
                label = label,
                dim = dim,
                value = value,
                reset = reset
            ));
        } else {
            output.push(format!(
                "{side} {label} : {value} {side}",
                side = INTERACTIVE_PANEL_SIDE
            ));
        }
    }

    let mut bottom = String::new();
    bottom.push_str(INTERACTIVE_PANEL_BOTTOM_LEFT);
    bottom.push_str(&INTERACTIVE_PANEL_BORDER.repeat(inner_width));
    bottom.push_str(INTERACTIVE_PANEL_BOTTOM_RIGHT);
    if use_color {
        output.push(format!("{label_color}{bottom}{reset}"));
    } else {
        output.push(bottom);
    }
    crate::terminal_ui::commit_history(&format!("{}\n", output.join("\n")));
    Ok(())
}

fn interactive_prompt_compact_line(
    turn: usize,
    model: &str,
    params: &str,
    cwd: &str,
    workspace: &str,
    provider: &str,
    show_workspace: bool,
    terminal_width: usize,
) -> Option<String> {
    let mut parts = vec![
        format!(
            "{} {}",
            tr(Message::InteractiveModelLabel),
            truncate_display_width(model, 28)
        ),
        format!(
            "{} {}",
            tr(Message::InteractiveParamsLabel),
            truncate_display_width(params, 22)
        ),
        format!(
            "{} {}",
            tr(Message::InteractiveWorkingDirLabel),
            truncate_display_path(cwd, 32)
        ),
    ];

    if show_workspace {
        parts.push(format!(
            "{} {}",
            tr(Message::InteractiveWorkspaceLabel),
            truncate_display_path(workspace, 22)
        ));
    }
    if !provider.trim().is_empty() {
        parts.push(format!(
            "{} {}",
            tr(Message::InteractiveProviderLabel),
            truncate_display_path(provider, 24)
        ));
    }

    let label = format!("nola[{turn}] ");
    let details = parts.join("  ");
    let budget = terminal_width
        .saturating_sub(display_width(&label))
        .saturating_sub(2);
    if budget == 0 {
        return None;
    }

    let details = truncate_display_width(&details, budget);
    let line = format!("{label}{details}");
    if display_width(&line) > terminal_width.saturating_sub(2) {
        return None;
    }

    Some(line)
}

struct SlashCommand {
    name: &'static str,
}

const INTERACTIVE_SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand { name: "/help" },
    SlashCommand { name: "/model" },
    SlashCommand { name: "/executor" },
    SlashCommand { name: "/debug" },
    SlashCommand { name: "/allow" },
    SlashCommand { name: "/sources" },
    SlashCommand { name: "/goal" },
    SlashCommand { name: "/init" },
    SlashCommand { name: "/cd" },
    SlashCommand { name: "/compact" },
    SlashCommand { name: "/new" },
    SlashCommand { name: "/plan" },
    SlashCommand { name: "/agent" },
    SlashCommand { name: "/room" },
    SlashCommand { name: "/work" },
    SlashCommand { name: "/recover" },
    SlashCommand { name: "/tasks" },
    SlashCommand { name: "/jobs" },
    SlashCommand { name: "/resume" },
    SlashCommand { name: "/continue" },
    SlashCommand { name: "/history" },
    SlashCommand { name: "/status" },
    SlashCommand { name: "/exit" },
    SlashCommand { name: "/quit" },
];

fn normalize_interactive_command(command: &str) -> Option<&'static str> {
    if !command.starts_with('/') {
        return None;
    }
    let lower = command.to_ascii_lowercase();
    Some(match lower.as_str() {
        "/help" | "/h" => "/help",
        "/model" | "/m" => "/model",
        "/executor" | "/exec" => "/executor",
        "/debug" | "/d" => "/debug",
        "/allow" | "/a" => "/allow",
        "/sources" | "/data" => "/sources",
        "/goal" | "/g" => "/goal",
        "/init" | "/i" => "/init",
        "/cd" => "/cd",
        "/compact" => "/compact",
        "/new" | "/n" => "/new",
        "/plan" | "/p" => "/plan",
        "/agent" => "/agent",
        "/room" => "/room",
        "/work" | "/w" => "/work",
        "/recover" => "/recover",
        "/tasks" | "/t" => "/tasks",
        "/jobs" | "/j" => "/jobs",
        "/resume" | "/r" => "/resume",
        "/continue" | "/c" => "/continue",
        "/history" | "/hist" => "/history",
        "/status" | "/s" | "/st" => "/status",
        "/exit" | "/quit" | "/q" | "/e" => "/exit",
        _ => return None,
    })
}

fn is_resume_command(prompt: &str) -> bool {
    let mut parts = prompt.trim().split_whitespace();
    let Some(command) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    normalize_interactive_command(command).is_some_and(|value| value == "/resume")
}

fn is_continue_command(prompt: &str) -> bool {
    let mut parts = prompt.trim().split_whitespace();
    let Some(command) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    normalize_interactive_command(command).is_some_and(|value| value == "/continue")
}

fn interactive_init_command_args(prompt: &str) -> Option<Vec<String>> {
    let mut parts = prompt.trim().split_whitespace();
    let command = parts.next()?;
    let is_init = normalize_interactive_command(command).is_some_and(|value| value == "/init");
    if is_init {
        Some(parts.map(str::to_string).collect())
    } else {
        None
    }
}

fn run_interactive_init_command(config: &Config, args: &[String]) -> Result<(), String> {
    let mut init_config = config.clone();
    apply_interactive_init_args(&mut init_config, args)?;
    reset_interactive_terminal_state();
    if !init_config.json_output {
        terminal_notice!("{}", interactive_layer_rule(Some("init")));
        terminal_notice!();
    } else {
        return init::run_init(&init_config);
    }
    let mut active_progress = None;
    let result = init::run_init_with_progress(&init_config, |progress| {
        update_interactive_init_progress(&mut active_progress, &progress);
    });
    if let Some(progress) = active_progress.take() {
        progress.clear();
    }
    result
}

fn update_interactive_init_progress(
    active: &mut Option<InteractiveProgressSpinner>,
    progress: &init::InitProgress,
) {
    let label = match progress.phase {
        init::InitProgressPhase::Output => ui_text(UiText::InitCheckOutput),
        init::InitProgressPhase::Scan => ui_text(UiText::InitScanFiles),
        init::InitProgressPhase::Wiki => ui_text(UiText::InitAnalyzeProject),
        init::InitProgressPhase::Memory => ui_text(UiText::InitGenerateMemory),
        init::InitProgressPhase::Write => ui_text(UiText::InitWriteNola),
    };
    match progress.state {
        init::InitProgressState::Started => {
            if let Some(previous) = active.take() {
                previous.clear();
            }
            *active = Some(InteractiveProgressSpinner::start(
                label,
                progress.detail.trim(),
            ));
        }
        init::InitProgressState::Completed => {
            if let Some(current) = active.take() {
                current.finish(label, progress.detail.trim());
            } else {
                print_interactive_progress_completion(label, progress.detail.trim());
            }
        }
    }
}

struct InteractiveProgressSpinner {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl InteractiveProgressSpinner {
    fn start(label: &str, detail: &str) -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = std::sync::Arc::clone(&stop);
        let label = label.to_string();
        let detail = detail.to_string();
        if crate::terminal_ui::reduced_motion() {
            print_interactive_progress_frame("•", &label, &detail, false);
            return Self { stop, handle: None };
        }
        print_interactive_progress_frame(INTERACTIVE_SPINNER_FRAMES[0], &label, &detail, false);
        let handle = std::thread::spawn(move || {
            let mut frame = 1usize;
            while !thread_stop.load(std::sync::atomic::Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(80));
                if thread_stop.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                if interaction::permission_prompt_active() {
                    continue;
                }
                print_interactive_progress_frame(
                    INTERACTIVE_SPINNER_FRAMES[frame % INTERACTIVE_SPINNER_FRAMES.len()],
                    &label,
                    &detail,
                    false,
                );
                frame = frame.saturating_add(1);
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    fn finish(mut self, label: &str, detail: &str) {
        self.stop();
        print_interactive_progress_frame("✓", label, detail, true);
    }

    fn clear(mut self) {
        self.stop();
        clear_interactive_progress_line();
    }
}

impl Drop for InteractiveProgressSpinner {
    fn drop(&mut self) {
        self.stop();
    }
}

const INTERACTIVE_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn print_interactive_progress_completion(label: &str, detail: &str) {
    print_interactive_progress_frame("✓", label, detail, true);
}

fn print_interactive_progress_frame(marker: &str, label: &str, detail: &str, newline: bool) {
    let width = terminal_columns().saturating_sub(8).max(12);
    let detail = truncate_display_width_middle(detail.trim(), width / 2);
    let detail = (!detail.is_empty())
        .then(|| format!("  ·  {detail}"))
        .unwrap_or_default();
    if crate::terminal_ui::owns_terminal() {
        if newline {
            crate::terminal_ui::clear_activity();
            terminal_notice!("  {marker} {label}{detail}");
        } else {
            crate::terminal_ui::set_activity("interactive-progress", marker, &format!("{label}{detail}"));
        }
        return;
    }
    if interactive_color_enabled() {
        print!(
            "\r\x1b[2K  {INTERACTIVE_ACCENT}{marker}{INTERACTIVE_RESET} {INTERACTIVE_VALUE}{label}{INTERACTIVE_RESET}{INTERACTIVE_DIM}{detail}{INTERACTIVE_RESET}"
        );
    } else {
        print!("\r\x1b[2K  {marker} {label}{detail}");
    }
    if newline {
        terminal_notice!();
    }
    let _ = io::stdout().flush();
}

fn clear_interactive_progress_line() {
    if crate::terminal_ui::owns_terminal() {
        crate::terminal_ui::clear_activity();
        return;
    }
    print!("\r{INTERACTIVE_RESET}\x1b[2K");
    let _ = io::stdout().flush();
}

fn apply_interactive_init_args(config: &mut Config, args: &[String]) -> Result<(), String> {
    let mut index = 0usize;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "--dry-run" => {
                config.dry_run = true;
                index += 1;
            }
            "--force" => {
                config.force = true;
                index += 1;
            }
            "--output" | "-o" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(format!("{}: --output PATH", interactive_init_usage()));
                };
                config.output_path = Some(PathBuf::from(value));
                index += 2;
            }
            "--help" | "-h" => return Err(interactive_init_usage()),
            _ if arg.starts_with("--output=") => {
                let value = arg.trim_start_matches("--output=");
                if value.trim().is_empty() {
                    return Err(format!("{}: --output PATH", interactive_init_usage()));
                }
                config.output_path = Some(PathBuf::from(value));
                index += 1;
            }
            _ => {
                return Err(format!(
                    "unsupported /init option: {arg}\n{}",
                    interactive_init_usage()
                ));
            }
        }
    }
    Ok(())
}

fn interactive_init_usage() -> String {
    "usage: /init [--dry-run] [--force] [--output PATH]".to_string()
}

fn resolve_interactive_workspace_path(
    raw: &str,
    current: &Path,
    previous: Option<&Path>,
) -> Result<PathBuf, String> {
    let raw = raw.trim();
    let candidate = if raw == "-" {
        previous
            .map(Path::to_path_buf)
            .ok_or_else(|| "no previous workspace is available".to_string())?
    } else {
        let raw = raw
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                raw.strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(raw);
        let expanded = crate::config_paths::expand_home(Path::new(raw));
        if expanded.is_absolute() {
            expanded
        } else {
            current.join(expanded)
        }
    };
    let resolved = candidate
        .canonicalize()
        .map_err(|err| format!("resolve workspace {} failed: {err}", candidate.display()))?;
    if !resolved.is_dir() {
        return Err(format!(
            "workspace is not a directory: {}",
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn select_interactive_session_restore(config: &Config, continue_last: bool) -> Result<Option<Config>, String> {
    let mut selected = config.clone();
    selected.continue_last = continue_last;
    selected.session_id = None;
    let Some(id) = select_interactive_resume_session(&selected)? else { return Ok(None); };
    selected.session_id = Some(id);
    selected.continue_last = false;
    selected.resume_session = false;
    crate::conversation_invocation::initialize_or_restore(&mut selected)?;
    Ok(Some(selected))
}

fn interactive_toggle_option_labels() -> [(&'static str, &'static str); 2] {
    [
        ("on", tr(Message::InteractiveStatusOn)),
        ("off", tr(Message::InteractiveStatusOff)),
    ]
}

fn interactive_toggle_matches<'a>(query: &'a str, include_empty: bool) -> Vec<&'static str> {
    let query = query.trim();
    let ascii_query = query.to_ascii_lowercase();
    let mut result = Vec::new();

    if include_empty && query.is_empty() {
        return interactive_toggle_option_labels()
            .iter()
            .map(|(_, label)| *label)
            .collect();
    }

    for (canonical, label) in interactive_toggle_option_labels() {
        if canonical.starts_with(&ascii_query) || label.starts_with(query) {
            result.push(label);
        }
    }

    result
}

struct SlashCompletion {
    name: String,
}

fn matching_slash_commands_with_config(input: &str, config: &Config) -> Vec<SlashCompletion> {
    matching_slash_commands_inner(input, Some(config))
}

fn matching_slash_commands_inner(input: &str, config: Option<&Config>) -> Vec<SlashCompletion> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') {
        return Vec::new();
    }
    let mut segments = trimmed.splitn(2, char::is_whitespace);
    let raw_command = segments.next().unwrap_or("");
    let rest = segments.next().unwrap_or("").trim();
    let command = normalize_interactive_command(raw_command).unwrap_or(raw_command);

    if command == "/room" && !rest.is_empty() {
        return interactive_room_completions(rest);
    }

    if command == "/model" {
        let (prefix, query) = interactive_model_completion_parts(rest);
        return available_interactive_models(config, query)
            .into_iter()
            .map(|model| SlashCompletion {
                name: format!("{prefix}{model}"),
            })
            .take(4)
            .collect();
    }
    if command == "/executor" {
        let query = rest.to_ascii_lowercase();
        // Completion is display-only. Discovery probes versions, authentication
        // and model catalogs by starting external processes; never do it for a
        // keystroke. The explicit picker/submission still checks availability.
        return nola_domain::execution_backend::ExecutionBackendKind::ALL
            .into_iter()
            .filter(|backend| query.is_empty() || backend.id().starts_with(&query))
            .map(|backend| SlashCompletion {
                name: format!("/executor {}", backend.id()),
            })
            .take(4)
            .collect();
    }
    if command == "/goal" && rest.is_empty() {
        return interactive_goal_subcommand_matches(rest);
    }
    if command == "/goal" {
        return interactive_goal_subcommand_matches(rest);
    }
    if command == "/debug" {
        return interactive_toggle_matches(rest, true)
            .into_iter()
            .map(|value| SlashCompletion {
                name: format!("/debug {value}"),
            })
            .collect();
    }
    if command == "/allow" {
        return Vec::new();
    }

    let command = if command == raw_command {
        trimmed
    } else {
        command
    };

    INTERACTIVE_SLASH_COMMANDS
        .iter()
        .filter(|item| item.name.starts_with(command))
        .map(|command| SlashCompletion {
            name: command.name.to_string(),
        })
        .take(4)
        .collect()
}

fn first_matching_slash_completion(input: &str, config: &Config) -> Option<SlashCompletion> {
    // The visible candidate list and Tab must use the same contract; independent
    // resolvers previously showed /executor candidates that Tab could not insert.
    matching_slash_commands_with_config(input, config).into_iter().next()
}

fn available_interactive_models(config: Option<&Config>, query: &str) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    if config.is_some() {
        if let Ok(profiles) = crate::store::NolaStore::open_default().and_then(|store| store.model_profiles()) {
            for profile in profiles.into_iter().filter(|p| p.enabled && p.deleted_at_ms.is_none() && p.runtime == "remote") {
                models.push(format!("configured:{}", profile.id));
            }
        }
        if !models.is_empty() { return models; }
    }
    if let Some(config) = config {
        for item in [
            &config.model,
            config.runtime.model.as_ref().unwrap_or(&config.model),
        ] {
            let item = item.trim();
            if !item.is_empty() && !models.iter().any(|model: &String| model.as_str() == item) {
                models.push(item.to_string());
            }
        }
    }
    if let Some(previous_model) = interactive_previous_provider_model() {
        if !models.iter().any(|model| model == &previous_model) {
            models.push(previous_model);
        }
    }
    for model in local_models::model_ids() {
        if !models.iter().any(|candidate| candidate == model.as_str()) {
            models.push(model.to_string());
        }
    }
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return models;
    }
    models
        .into_iter()
        .filter(|model| model.to_ascii_lowercase().contains(&query))
        .collect()
}

fn interactive_previous_provider_model() -> Option<String> {
    let storage = INTERACTIVE_PROVIDER_BEFORE_LOCAL.get()?;
    let snapshot = storage.lock().ok()?;
    let model = snapshot.as_ref()?.model.trim();
    if model.is_empty() {
        None
    } else {
        Some(model.to_string())
    }
}

fn interactive_model_completion_parts(rest: &str) -> (&str, &str) {
    for action in ["install", "remove", "start", "config"] {
        if rest == action {
            return (
                match action {
                    "install" => "/model install ",
                    "remove" => "/model remove ",
                    "start" => "/model start ",
                    _ => "/model config ",
                },
                "",
            );
        }
        if let Some(query) = rest.strip_prefix(&format!("{action} ")) {
            return (
                match action {
                    "install" => "/model install ",
                    "remove" => "/model remove ",
                    "start" => "/model start ",
                    _ => "/model config ",
                },
                query,
            );
        }
    }
    ("/model ", rest)
}

fn interactive_goal_subcommand_matches(query: &str) -> Vec<SlashCompletion> {
    let query = query.trim().to_ascii_lowercase();
    ["status", "resume", "off"]
        .into_iter()
        .filter(|subcommand| query.is_empty() || subcommand.starts_with(&query))
        .map(|subcommand| SlashCompletion {
            name: format!("/goal {subcommand}"),
        })
        .collect()
}

enum SlashCommandResult {
    Handled,
    PassThrough,
    RunGoalPrompt(String),
    WorkspaceChanged,
    SessionReset,
}

enum ResumePreflight {
    Continue(nola_application::ResumeSnapshot),
    StayInteractive,
}

fn prepare_resume_snapshot_interactively(
    config: &Config,
    project_root: &str,
    mut snapshot: nola_application::ResumeSnapshot,
) -> Result<ResumePreflight, String> {
    loop {
        let has_observed_results =
            snapshot
                .supervisor_reconciliation
                .side_effects
                .iter()
                .any(|item| {
                    item.disposition
                        == nola_application::SupervisorSideEffectDisposition::CommitObservedResult
                });
        if has_observed_results {
            let reconciled =
                crate::work_step_port::reconcile_observed_work_step_transactions()?;
            if !reconciled.is_empty() {
                terminal_notice!(
                    "  {} {}",
                    resume_recovery_text(ResumeRecoveryText::ObservedCommitted),
                    reconciled.len()
                );
            }
            snapshot =
                session::load_resume_snapshot_for_project(project_root, goal_now_epoch_ms())?
                    .ok_or_else(|| "resumable Goal disappeared during recovery".to_string())?;
            continue;
        }

        let uncertain = snapshot
            .incomplete_work_step_transactions
            .iter()
            .filter(|transaction| {
                snapshot
                    .supervisor_reconciliation
                    .side_effects
                    .iter()
                    .any(|item| {
                        item.transaction_id == transaction.id
                            && item.disposition
                                == nola_application::SupervisorSideEffectDisposition::RequiresIntervention
                    })
            })
            .collect::<Vec<_>>();
        if !uncertain.is_empty() {
            terminal_notice!(
                "  {}",
                resume_recovery_text(ResumeRecoveryText::UncertainBlocked)
            );
            for transaction in uncertain {
                let capability = transaction
                    .request
                    .get("capability_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                terminal_notice!(
                    "  - {} · {} · {:?}",
                    transaction.id, capability, transaction.phase
                );
            }
            terminal_notice!(
                "  {}",
                resume_recovery_text(ResumeRecoveryText::InspectWork)
            );
            return Ok(ResumePreflight::StayInteractive);
        }

        if !snapshot.pending_permission_requests.is_empty() {
            let mut task_ids = Vec::new();
            for request in &snapshot.pending_permission_requests {
                if !task_ids.contains(&request.task_id) {
                    task_ids.push(request.task_id.clone());
                }
            }
            for task_id in task_ids {
                let Some(prompt) = task_runtime::permission_prompt_for_task(&config.host_invocation_request()?, &task_id)
                else {
                    terminal_notice!(
                        "  {} {task_id}",
                        resume_recovery_text(ResumeRecoveryText::PermissionUnavailable)
                    );
                    return Ok(ResumePreflight::StayInteractive);
                };
                let choice = interaction::prompt_permission(&prompt.prompt);
                restore_interactive_raw_mode().map_err(|error| error.to_string())?;
                let stops_resume = matches!(
                    &choice,
                    crate::runtime_interaction::PermissionChoice::Custom(_)
                        | crate::runtime_interaction::PermissionChoice::ReturnToComposer
                        | crate::runtime_interaction::PermissionChoice::Deny
                        | crate::runtime_interaction::PermissionChoice::TransportInterrupted(_)
                );
                let outcome =
                    task_runtime::resolve_task_permission_choice(&config.host_invocation_request()?, &prompt, choice);
                if outcome.error {
                    return Err(outcome.text);
                }
                task_runtime::reconcile_after_permission_resolution(&config.host_invocation_request()?);
                terminal_notice!("  {}", outcome.display);
                if stops_resume {
                    return Ok(ResumePreflight::StayInteractive);
                }
            }
            snapshot =
                session::load_resume_snapshot_for_project(project_root, goal_now_epoch_ms())?
                    .ok_or_else(|| {
                        "resumable Goal disappeared after permission resolution".to_string()
                    })?;
            continue;
        }

        for task in snapshot
            .supervisor_reconciliation
            .tasks
            .iter()
            .filter(|item| {
                item.disposition == nola_application::SupervisorTaskDisposition::RetryScheduled
            })
        {
            terminal_notice!(
                "  {} {}",
                resume_recovery_text(ResumeRecoveryText::RetryScheduled),
                task.task_id
            );
        }
        return Ok(ResumePreflight::Continue(snapshot));
    }
}

#[derive(Clone, Copy)]
enum ResumeRecoveryText {
    ObservedCommitted,
    UncertainBlocked,
    InspectWork,
    PermissionUnavailable,
    RetryScheduled,
}

fn resume_recovery_text(text: ResumeRecoveryText) -> &'static str {
    use i18n::Language;
    match (i18n::current_language(), text) {
        (Language::ZhHans, ResumeRecoveryText::ObservedCommitted) => "已提交恢复到的工具结果：",
        (Language::ZhHans, ResumeRecoveryText::UncertainBlocked) => {
            "检测到执行结果不确定的操作，已停止自动恢复以避免重复副作用："
        }
        (Language::ZhHans, ResumeRecoveryText::InspectWork) => {
            "使用 /work 查看详情；确认真实外部状态后再处理。"
        }
        (Language::ZhHans, ResumeRecoveryText::PermissionUnavailable) => "无法恢复待授权任务：",
        (Language::ZhHans, ResumeRecoveryText::RetryScheduled) => "任务已安排重试，不会重复启动：",
        (Language::De, ResumeRecoveryText::ObservedCommitted) => {
            "Wiederhergestellte Werkzeugergebnisse gespeichert:"
        }
        (Language::De, ResumeRecoveryText::UncertainBlocked) => {
            "Automatische Wiederaufnahme gestoppt: Der Ausgang dieser Vorgänge ist ungewiss:"
        }
        (Language::De, ResumeRecoveryText::InspectWork) => {
            "Details mit /work prüfen und zuerst den tatsächlichen externen Zustand bestätigen."
        }
        (Language::De, ResumeRecoveryText::PermissionUnavailable) => {
            "Ausstehende Berechtigung konnte nicht wiederhergestellt werden:"
        }
        (Language::De, ResumeRecoveryText::RetryScheduled) => {
            "Aufgabe ist zur Wiederholung eingeplant und wird nicht doppelt gestartet:"
        }
        (Language::Es, ResumeRecoveryText::ObservedCommitted) => {
            "Resultados recuperados confirmados:"
        }
        (Language::Es, ResumeRecoveryText::UncertainBlocked) => {
            "Reanudación automática detenida porque estas operaciones tienen un resultado incierto:"
        }
        (Language::Es, ResumeRecoveryText::InspectWork) => {
            "Usa /work y confirma primero el estado externo real."
        }
        (Language::Es, ResumeRecoveryText::PermissionUnavailable) => {
            "No se pudo recuperar el permiso pendiente:"
        }
        (Language::Es, ResumeRecoveryText::RetryScheduled) => {
            "La tarea tiene un reintento programado y no se iniciará dos veces:"
        }
        (Language::Fr, ResumeRecoveryText::ObservedCommitted) => "Résultats récupérés validés :",
        (Language::Fr, ResumeRecoveryText::UncertainBlocked) => {
            "Reprise automatique arrêtée car le résultat de ces opérations est incertain :"
        }
        (Language::Fr, ResumeRecoveryText::InspectWork) => {
            "Utilisez /work et confirmez d'abord l'état externe réel."
        }
        (Language::Fr, ResumeRecoveryText::PermissionUnavailable) => {
            "Impossible de restaurer l'autorisation en attente :"
        }
        (Language::Fr, ResumeRecoveryText::RetryScheduled) => {
            "La tâche a une nouvelle tentative planifiée et ne sera pas relancée en double :"
        }
        (Language::Ja, ResumeRecoveryText::ObservedCommitted) => {
            "復元したツール結果を確定しました:"
        }
        (Language::Ja, ResumeRecoveryText::UncertainBlocked) => {
            "結果が不確定な操作があるため、自動再開を停止しました:"
        }
        (Language::Ja, ResumeRecoveryText::InspectWork) => {
            "/work で詳細を確認し、実際の外部状態を先に確認してください。"
        }
        (Language::Ja, ResumeRecoveryText::PermissionUnavailable) => {
            "保留中の権限要求を復元できません:"
        }
        (Language::Ja, ResumeRecoveryText::RetryScheduled) => {
            "タスクは再試行予定で、重複起動しません:"
        }
        (Language::Ru, ResumeRecoveryText::ObservedCommitted) => {
            "Восстановленные результаты инструментов зафиксированы:"
        }
        (Language::Ru, ResumeRecoveryText::UncertainBlocked) => {
            "Автовозобновление остановлено: результат этих операций неопределён:"
        }
        (Language::Ru, ResumeRecoveryText::InspectWork) => {
            "Откройте /work и сначала подтвердите фактическое внешнее состояние."
        }
        (Language::Ru, ResumeRecoveryText::PermissionUnavailable) => {
            "Не удалось восстановить ожидающий запрос разрешения:"
        }
        (Language::Ru, ResumeRecoveryText::RetryScheduled) => {
            "Повтор задачи уже запланирован; дублирующий запуск не выполняется:"
        }
        (Language::En, ResumeRecoveryText::ObservedCommitted) => {
            "Committed recovered tool results:"
        }
        (Language::En, ResumeRecoveryText::UncertainBlocked) => {
            "Automatic resume stopped because these operations have an uncertain outcome:"
        }
        (Language::En, ResumeRecoveryText::InspectWork) => {
            "Use /work and confirm the real external state before resolving them."
        }
        (Language::En, ResumeRecoveryText::PermissionUnavailable) => {
            "Could not restore pending permission:"
        }
        (Language::En, ResumeRecoveryText::RetryScheduled) => {
            "Task retry is scheduled and will not be started twice:"
        }
    }
}

fn handle_interactive_slash_command(
    prompt: &str,
    config: &mut Config,
    goal_target: &mut Option<String>,
    goal_constraints: &mut Option<String>,
    previous_workspace: &mut Option<PathBuf>,
    last_context_usage: &mut Option<nola_core::agent::context_window::ContextUsageSnapshot>,
    turn: usize,
) -> Result<SlashCommandResult, String> {
    let trimmed = prompt.trim();
    if !trimmed.starts_with('/') {
        return Ok(SlashCommandResult::PassThrough);
    }
    let mut parts = trimmed.split_whitespace();
    let command = parts.next().unwrap_or("");
    let command = normalize_interactive_command(command).unwrap_or(command);
    match command {
        "/room" => {
            if let Err(error) = interactive_room_command(trimmed.split_once(char::is_whitespace).map(|(_,rest)|rest.trim()).unwrap_or(""),config) {
                terminal_notice!("{}",format_error(&error));
            }
            Ok(SlashCommandResult::Handled)
        }
        "/help" => {
            print_interactive_help();
            Ok(SlashCommandResult::Handled)
        }
        "/status" => {
            print_interactive_prompt_card(config, turn)?;
            Ok(SlashCommandResult::Handled)
        }
        "/model" => {
            let Some(model) = parts.next() else {
                if let Some(selected_model) = interactive_select_model(config)? {
                    select_interactive_model(config, &selected_model)?;
                }
                return Ok(SlashCommandResult::Handled);
            };
            if model.eq_ignore_ascii_case("status") || model.eq_ignore_ascii_case("local") {
                print_local_model_status();
                return Ok(SlashCommandResult::Handled);
            }
            if model.eq_ignore_ascii_case("install") {
                let Some(model_id) = parts.next() else {
                    terminal_notice!(
                        "  {}: /model install <model-id>",
                        local_model_cli_text(LocalModelCliText::Usage)
                    );
                    print_local_model_status();
                    return Ok(SlashCommandResult::Handled);
                };
                let mut last_percent = u64::MAX;
                let download = local_models::download_model(model_id, |downloaded, total| {
                    let percent = downloaded.saturating_mul(100) / total.max(1);
                    if percent != last_percent && (percent % 2 == 0 || percent == 100) {
                        print_interactive_progress_frame("↓", local_model_cli_text(LocalModelCliText::Downloading),
                            &format!("{model_id} {percent}%"), false);
                        last_percent = percent;
                    }
                });
                clear_interactive_progress_line();
                download?;
                terminal_notice!(
                    "\r  {} {model_id}                            ",
                    local_model_cli_text(LocalModelCliText::InstalledAction)
                );
                return Ok(SlashCommandResult::Handled);
            }
            if model.eq_ignore_ascii_case("remove") {
                let Some(model_id) = parts.next() else {
                    terminal_notice!(
                        "  {}: /model remove <model-id>",
                        local_model_cli_text(LocalModelCliText::Usage)
                    );
                    return Ok(SlashCommandResult::Handled);
                };
                local_models::remove_model(model_id)?;
                terminal_notice!(
                    "  {} {model_id}",
                    local_model_cli_text(LocalModelCliText::RemovedAction)
                );
                return Ok(SlashCommandResult::Handled);
            }
            if model.eq_ignore_ascii_case("start") {
                let model_id = parts
                    .next()
                    .map(str::to_string)
                    .or_else(local_models::selected_model_id)
                    .ok_or_else(|| {
                        local_model_cli_text(LocalModelCliText::SelectBeforeStart).to_string()
                    })?;
                start_interactive_local_model(config, &model_id)?;
                return Ok(SlashCommandResult::Handled);
            }
            if model.eq_ignore_ascii_case("stop") {
                local_models::shutdown();
                restore_interactive_provider(config);
                terminal_notice!("  {}", local_model_cli_text(LocalModelCliText::Stopped));
                return Ok(SlashCommandResult::Handled);
            }
            if model.eq_ignore_ascii_case("config") {
                let model_id = parts
                    .next()
                    .map(str::to_string)
                    .or_else(local_models::selected_model_id)
                    .ok_or_else(|| {
                        local_model_cli_text(LocalModelCliText::SelectBeforeConfigure).to_string()
                    })?;
                let assignments = parts.collect::<Vec<_>>();
                configure_interactive_local_model(&model_id, &assignments)?;
                return Ok(SlashCommandResult::Handled);
            }
            select_interactive_model(config, model)?;
            Ok(SlashCommandResult::Handled)
        }
        "/executor" => {
            let catalog = crate::external_executor::discover_execution_backends();
            let Some(requested) = parts.next() else {
                terminal_notice!("{}", tr(Message::InteractiveAvailableExecutors));
                for backend in catalog {
                    let selected = if backend.id == config.execution_backend {
                        "*"
                    } else {
                        " "
                    };
                    let status = match backend.availability {
                        nola_domain::execution_backend::ExecutionBackendAvailability::Available => "available",
                        nola_domain::execution_backend::ExecutionBackendAvailability::NotInstalled => "not installed",
                        nola_domain::execution_backend::ExecutionBackendAvailability::Unauthenticated => "not authenticated",
                        nola_domain::execution_backend::ExecutionBackendAvailability::Incompatible => "incompatible",
                        nola_domain::execution_backend::ExecutionBackendAvailability::Unreachable => "unreachable",
                    };
                    terminal_notice!("  {selected} {} · {status}", backend.display_name);
                    if let Some(diagnostic) =
                        backend.diagnostic.filter(|value| !value.trim().is_empty())
                    {
                        terminal_notice!("      {diagnostic}");
                    }
                }
                terminal_notice!("{}", tr(Message::InteractiveExecutorUsage));
                return Ok(SlashCommandResult::Handled);
            };
            let Some(backend) = catalog.into_iter().find(|backend| backend.id == requested) else {
                terminal_notice!(
                    "{} {requested}",
                    tr(Message::InteractiveExecutorUnavailable)
                );
                terminal_notice!("{}", tr(Message::InteractiveExecutorUsage));
                return Ok(SlashCommandResult::Handled);
            };
            if backend.availability
                != nola_domain::execution_backend::ExecutionBackendAvailability::Available
            {
                terminal_notice!(
                    "{} {}",
                    tr(Message::InteractiveExecutorUnavailable),
                    backend.display_name
                );
                if let Some(diagnostic) = backend.diagnostic {
                    terminal_notice!("  {diagnostic}");
                }
                return Ok(SlashCommandResult::Handled);
            }
            config.execution_backend = backend.id;
            terminal_notice!(
                "{} {}",
                tr(Message::InteractiveExecutorSelected),
                backend.display_name
            );
            Ok(SlashCommandResult::Handled)
        }
        "/debug" => {
            let Some(value) = parts.next().and_then(parse_interactive_toggle) else {
                terminal_notice!(
                    "  {} {}",
                    tr(Message::InteractiveDebugStatusPrefix),
                    interactive_toggle_text(config.debug_mode)
                );
                terminal_notice!("  {}", tr(Message::InteractiveSlashDebugUsage));
                return Ok(SlashCommandResult::Handled);
            };
            config.debug_mode = value;
            llm::configure_local_debug_log(config.debug_mode, &config.project_root);
            terminal_notice!(
                "  {} {}",
                tr(Message::InteractiveDebugStatusPrefix),
                interactive_toggle_text(config.debug_mode)
            );
            Ok(SlashCommandResult::Handled)
        }
        "/allow" => {
            if let Err(e) = interactive_permission_panel(config) {
                terminal_notice!("  {} {e}", tr(Message::InteractiveAllowPanelErrorPrefix));
            }
            return Ok(SlashCommandResult::Handled);
        }
        "/sources" => {
            handle_data_source_command(trimmed, config)?;
            return Ok(SlashCommandResult::Handled);
        }
        "/goal" => {
            let target_raw = parts.collect::<Vec<_>>().join(" ");
            let action = target_raw.trim().to_ascii_lowercase();
            if target_raw.trim().is_empty() || action == "status" {
                if let Some(goal) = interactive_goal_snapshot(
                    config,
                    goal_target.as_deref(),
                    goal_constraints.as_deref(),
                )? {
                    print_goal_mode_summary(&goal.target, Some(&goal.constraints));
                    terminal_notice!(
                        "  {} {}",
                        tr(Message::InteractiveGoalStatePrefix),
                        interactive_goal_state_text(&goal.status)
                    );
                } else {
                    terminal_notice!(
                        "  {} {}",
                        tr(Message::InteractiveGoalStatusPrefix),
                        tr(Message::InteractiveStatusOff)
                    );
                }
                terminal_notice!("  {}", tr(Message::InteractiveSlashGoalUsage));
                return Ok(SlashCommandResult::Handled);
            }

            if action == "resume" {
                let project_root = config.project_root.to_string_lossy().to_string();
                task_runtime::reconcile_for_resume_snapshot(&config.host_invocation_request()?);
                let goal = if let Some(snapshot) =
                    session::load_resume_snapshot_for_project(&project_root, goal_now_epoch_ms())?
                {
                    config.session_id = Some(snapshot.projection.conversation_id.clone());
                    let goal = interactive_goal_snapshot_from_projection(
                        &snapshot.projection.goal_projection,
                    );
                    match prepare_resume_snapshot_interactively(config, &project_root, snapshot)? {
                        ResumePreflight::Continue(snapshot) => {
                            config.resume_snapshot = Some(snapshot);
                        }
                        ResumePreflight::StayInteractive => {
                            config.resume_snapshot = None;
                            return Ok(SlashCommandResult::Handled);
                        }
                    }
                    goal
                } else {
                    interactive_goal_snapshot(
                        config,
                        goal_target.as_deref(),
                        goal_constraints.as_deref(),
                    )?
                };
                let Some(goal) = goal else {
                    terminal_notice!("  {}", tr(Message::InteractiveGoalNoResumable));
                    terminal_notice!("  {}", tr(Message::InteractiveSlashGoalUsage));
                    return Ok(SlashCommandResult::Handled);
                };

                if goal.status != "running" {
                    let session_id = config
                        .session_id
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| "Goal runtime requires a durable session id".to_string())?;
                    submit_goal_command(
                        config,
                        format!("goal:{session_id}:resume:{}", uuid::Uuid::new_v4()),
                        GoalRuntimeCommand::Resume {
                            now_epoch_ms: goal_now_epoch_ms(),
                        },
                    )?;
                }

                *goal_target = Some(goal.target.clone());
                *goal_constraints =
                    (!goal.constraints.trim().is_empty()).then_some(goal.constraints.clone());
                terminal_notice!(
                    "  {} {}",
                    tr(Message::InteractiveGoalResumeStarted),
                    goal.target
                );
                print_goal_mode_summary(&goal.target, goal_constraints.as_deref());
                // `/goal resume` resumes the durable Goal without inventing a
                // new user message. A later line typed by the user is routed
                // separately as visible steering for the active Goal.
                return Ok(SlashCommandResult::RunGoalPrompt(String::new()));
            }

            let (target, constraints) = parse_goal_target_and_constraints(&target_raw);
            if target.eq_ignore_ascii_case("off") {
                if let Some(session_id) = config.session_id.as_deref() {
                    if session::load_session_goal_projection(session_id)?
                        .and_then(|projection| projection.active)
                        .is_some_and(|goal| !goal.status.is_terminal())
                    {
                        submit_goal_command(
                            config,
                            format!("goal:{session_id}:cancel:{}", uuid::Uuid::new_v4()),
                            GoalRuntimeCommand::Cancel {
                                now_epoch_ms: goal_now_epoch_ms(),
                            },
                        )?;
                    }
                }
                *goal_target = None;
                *goal_constraints = None;
                terminal_notice!(
                    "  {} {}",
                    tr(Message::InteractiveGoalStatusPrefix),
                    tr(Message::InteractiveStatusOff)
                );
                return Ok(SlashCommandResult::Handled);
            }

            *goal_target = Some(target.to_string());
            if let Some(constraints) = constraints {
                *goal_constraints = Some(constraints);
            } else {
                *goal_constraints = None;
            }
            if let Some(goal) = goal_target.as_deref() {
                print_goal_mode_summary(goal, goal_constraints.as_deref());
            }
            Ok(SlashCommandResult::RunGoalPrompt(target.to_string()))
        }
        "/init" => {
            let init_args = interactive_init_command_args(trimmed).unwrap_or_default();
            if let Err(err) = run_interactive_init_command(config, &init_args) {
                terminal_notice!("{}", format_error(&err));
            }
            Ok(SlashCommandResult::Handled)
        }
        "/guide" => {
            onboarding::show(config);
            Ok(SlashCommandResult::Handled)
        }
        "/cd" => {
            let raw_path = trimmed
                .find(char::is_whitespace)
                .map(|index| trimmed[index..].trim())
                .unwrap_or("");
            if raw_path.trim().is_empty() {
                terminal_notice!(
                    "  {} {}",
                    tr(Message::InteractiveWorkspaceStatusPrefix),
                    config.project_root.display()
                );
                terminal_notice!("  {}", tr(Message::InteractiveCdUsage));
                return Ok(SlashCommandResult::Handled);
            }
            let target = resolve_interactive_workspace_path(
                raw_path,
                &config.project_root,
                previous_workspace.as_deref(),
            )?;
            let runtime = runtime_config::RuntimeConfig::load(&target)?;
            let old_workspace = config.project_root.clone();
            std::env::set_current_dir(&target)
                .map_err(|err| format!("change workspace to {} failed: {err}", target.display()))?;
            if let (Some(session_id), Some(_)) =
                (config.session_id.as_deref(), goal_target.as_deref())
            {
                submit_goal_command(
                    config,
                    format!(
                        "goal:{session_id}:workspace-change:{}",
                        uuid::Uuid::new_v4()
                    ),
                    GoalRuntimeCommand::Pause {
                        status: GoalStatus::Paused,
                        wake_at_epoch_ms: None,
                        now_epoch_ms: goal_now_epoch_ms(),
                    },
                )?;
            }
            let mut allowed_dirs = vec![target.clone()];
            allowed_dirs.extend(config.allowed_dirs.iter().cloned());
            allowed_dirs.extend(runtime.allowed_dirs.iter().cloned());
            let mut seen_allowed_dirs = HashSet::new();
            allowed_dirs.retain(|path| seen_allowed_dirs.insert(path.clone()));
            let mut bash_prefixes = config.bash_prefixes.clone();
            for prefix in &runtime.bash_prefixes {
                if !bash_prefixes.contains(prefix) {
                    bash_prefixes.push(prefix.clone());
                }
            }
            config.project_root = target.clone();
            config.allowed_dirs = allowed_dirs;
            config.bash_prefixes = bash_prefixes;
            config.runtime = runtime;
            config.session_id = None;
            config.continue_last = false;
            config.resume_session = false;
            *goal_target = None;
            *goal_constraints = None;
            *previous_workspace = Some(old_workspace);
            *last_context_usage = None;
            llm::configure_local_debug_log(config.debug_mode, &config.project_root);
            terminal_notice!(
                "{}",
                interactive_layer_rule(Some(tr(Message::InteractiveWorkspaceLabel)))
            );
            terminal_notice!(
                "  {}  {}",
                tr(Message::InteractiveWorkspaceLabel),
                compact_home_path(&target.to_string_lossy())
            );
            terminal_notice!(
                "  {}  {}",
                tr(Message::AgentSessionLabel),
                tr(Message::InteractiveNewSessionPending)
            );
            Ok(SlashCommandResult::WorkspaceChanged)
        }
        "/compact" => {
            let session_id = if let Some(session_id) = config.session_id.clone() {
                Some(session_id)
            } else if config.continue_last {
                resolve_continue_session_id(config)?
            } else {
                None
            };
            let Some(session_id) = session_id else {
                terminal_notice!(
                    "  {}  {}",
                    tr(Message::InteractiveCompactTitle),
                    tr(Message::InteractiveCompactNoActiveSession)
                );
                return Ok(SlashCommandResult::Handled);
            };
            let saved = session::load_session(&session_id)
                .map_err(|err| format!("{} {err}", tr(Message::InteractiveCompactFailedPrefix)))?;
            let initial_items = saved
                .messages
                .iter()
                .enumerate()
                .flat_map(|(index, message)| {
                    session_message_to_agent_items(message, &session_id, index)
                })
                .collect::<Vec<_>>();
            let mut compaction_config = config.clone();
            compaction_config.session_id = Some(session_id.clone());
            compaction_config.continue_last = false;
            let progress = (!config.json_output).then(|| {
                InteractiveProgressSpinner::start(tr(Message::InteractiveCompactTitle), "")
            });
            let report = compaction_config.host_invocation_request()
                .and_then(|request| compact_agent_core_session(&request, initial_items));
            if let Some(progress) = progress {
                progress.clear();
            }
            let report = report
                .map_err(|err| format!("{} {err}", tr(Message::InteractiveCompactFailedPrefix)))?;
            config.session_id = Some(session_id);
            config.continue_last = false;
            *last_context_usage = Some(nola_core::agent::context_window::ContextUsageSnapshot {
                used_tokens: report.context_used_tokens,
                context_window_tokens: report.context_window_tokens,
                prompt_token_limit: report.context_window_tokens,
                usage_percent: report.context_usage_percent,
                usage_is_estimated: true,
                ..Default::default()
            });
            terminal_notice!(
                "{}",
                interactive_layer_rule(Some(tr(Message::InteractiveCompactTitle)))
            );
            if report.replaced_item_count == 0 {
                terminal_notice!(
                    "  {}  {} · {} {} · ~{} {}",
                    tr(Message::InteractiveContextLabel),
                    tr(Message::InteractiveCompactAlready),
                    report.remaining_item_count,
                    tr(Message::InteractiveMessagesUnit),
                    report.context_used_tokens,
                    tr(Message::InteractiveTokensUnit)
                );
            } else {
                terminal_notice!(
                    "  {}  {} · {} {} {} · {} {} {}",
                    tr(Message::InteractiveContextLabel),
                    tr(Message::InteractiveCompactCompleted),
                    tr(Message::InteractiveCompactSummarized),
                    report.replaced_item_count,
                    tr(Message::InteractiveMessagesUnit),
                    tr(Message::InteractiveCompactKeptRecent),
                    report.remaining_item_count,
                    tr(Message::InteractiveMessagesUnit)
                );
                terminal_notice!(
                    "  {}  ~{} / ~{} · {:.1}%",
                    tr(Message::InteractiveTokensLabel),
                    report.context_used_tokens,
                    report.context_window_tokens,
                    report.context_usage_percent,
                );
                terminal_notice!("  {}", tr(Message::InteractiveCompactHistoryPreserved));
            }
            Ok(SlashCommandResult::Handled)
        }
        "/new" => {
            if let (Some(session_id), Some(_)) =
                (config.session_id.as_deref(), goal_target.as_deref())
            {
                submit_goal_command(
                    config,
                    format!("goal:{session_id}:new-session:{}", uuid::Uuid::new_v4()),
                    GoalRuntimeCommand::Pause {
                        status: GoalStatus::Paused,
                        wake_at_epoch_ms: None,
                        now_epoch_ms: goal_now_epoch_ms(),
                    },
                )?;
            }
            config.session_id = None;
            config.continue_last = false;
            config.resume_session = false;
            *goal_target = None;
            *goal_constraints = None;
            *last_context_usage = None;
            crate::conversation_invocation::reset_new_conversation_defaults(config)?;
            terminal_notice!(
                "{}",
                interactive_layer_rule(Some(tr(Message::InteractiveNewSessionTitle)))
            );
            terminal_notice!(
                "  {}  {}",
                tr(Message::AgentSessionLabel),
                tr(Message::InteractiveNewSessionPending)
            );
            Ok(SlashCommandResult::SessionReset)
        }
        "/agent" => {
            let Some(agent_type) = parts.next() else {
                terminal_notice!("{}", interactive_layer_rule(Some(ui_text(UiText::Agents))));
                for (name, description) in task_runtime::available_agent_profiles(&config.host_invocation_request()?) {
                    terminal_notice!("  {name:<10} {description}");
                }
                terminal_notice!("  {}", ui_text(UiText::AgentUsage));
                return Ok(SlashCommandResult::Handled);
            };
            let instruction = parts.collect::<Vec<_>>().join(" ");
            if instruction.trim().is_empty() {
                terminal_notice!("  {}", ui_text(UiText::AgentUsage));
                return Ok(SlashCommandResult::Handled);
            }
            if config.session_id.is_none() { config.session_id=Some(uuid::Uuid::new_v4().to_string()); }
            let mut emitter=Emitter::new(config.json_output);
            let outcome=task_runtime::spawn_user_agent(&config.host_invocation_request()?,agent_type,&instruction,&mut emitter);
            terminal_notice!("{}",interactive_layer_rule(Some(ui_text(UiText::Agent))));
            match outcome {
                Ok(outcome) => {
                    if let Some(nola_domain::run_result::RunResultValue::DelegationAccepted { task_id, .. }) = outcome.run_result {
                        let label = match i18n::current_language() {
                            i18n::Language::ZhHans => "已委派；查看子任务进度",
                            i18n::Language::En => "Delegated; view child task progress",
                            i18n::Language::De => "Delegiert; Fortschritt der Unteraufgabe anzeigen",
                            i18n::Language::Es => "Delegada; consultar el progreso de la subtarea",
                            i18n::Language::Fr => "Déléguée ; consulter la progression de la sous-tâche",
                            i18n::Language::Ja => "委任しました。子タスクの進捗を確認してください",
                            i18n::Language::Ru => "Делегировано; проверьте ход подзадачи",
                        };
                        if let Some((_, description)) = task_runtime::available_agent_profiles(&config.host_invocation_request()?)
                            .into_iter().find(|(name, _)| name == agent_type) {
                            terminal_notice!("  {agent_type} · {description}");
                        }
                        terminal_notice!("  {label}\n  /tasks {task_id}");
                    } else {
                        terminal_notice!("  {:?}", outcome.turn_status);
                    }
                },
                Err(error)=>terminal_notice!("{}",format_error(&error)),
            }
            Ok(SlashCommandResult::Handled)
        }
        "/recover" => {
            let arguments = parts.collect::<Vec<_>>();
            let body = crate::unknown_effect_recovery::execute(config, &arguments)?;
            crate::terminal_ui::commit_history(&body);
            Ok(SlashCommandResult::Handled)
        }
        "/input-recovery" => {
            if !config.save_session {return Err("Input recovery requires a persistent session".into());}
            let session=config.session_id.as_deref().ok_or("Select a session first")?;
            let arguments=parts.collect::<Vec<_>>();
            let operation=match arguments.as_slice() {
                []=>crate::boundary_input_intent::InputOperation::Inspect,
                ["resume",request]=>crate::boundary_input_intent::InputOperation::ResumePending(request),
                _=>return Err("Usage: /input-recovery [resume <original-request-id>]".into()),
            };
            let result=crate::boundary_input_intent::execute(session,operation)?;
            terminal_notice!("{}",result);
            Ok(SlashCommandResult::Handled)
        }
        "/work" => {
            let work_rule = interactive_layer_rule(Some(ui_text(UiText::Work)));
            let emit_work = |body: String| {
                crate::terminal_ui::commit_history(&format!("{work_rule}\n{body}"));
            };
            let arguments = parts.collect::<Vec<_>>();
            let (action, work_id) = match arguments.as_slice() {
                [] => ("list", None),
                [work_id] => ("status", Some(*work_id)),
                [action, work_id]
                    if matches!(
                        *action,
                        "status" | "stop" | "cancel" | "pause" | "resume" | "rerun"
                    ) =>
                {
                    (*action, Some(*work_id))
                }
                _ => {
                    emit_work(format!("  {}", ui_text(UiText::WorkUsage)));
                    return Ok(SlashCommandResult::Handled);
                }
            };
            let body = match (action, work_id) {
                ("list", _) => render_markdown_text(&work::render_list(config)?),
                ("status", Some(work_id)) => {
                    render_markdown_text(&work::render_detail(config, work_id)?)
                }
                (control, Some(work_id)) => {
                    let Some(item) = work::find(config, work_id)? else {
                        emit_work(format_error(&format!(
                            "{}: {work_id}",
                            ui_text(UiText::WorkNotFound)
                        )));
                        return Ok(SlashCommandResult::Handled);
                    };
                    if control == "rerun" && item.is_sop() {
                        let rerun = work::rerun_sop(config, work_id)?;
                        format!(
                            "  {} · `{}`",
                            ui_text(UiText::WorkRerunStarted),
                            rerun.record.run_id
                        )
                    } else if item.is_goal() {
                        let session_id = config
                            .session_id
                            .as_deref()
                            .ok_or_else(|| "Goal control requires an active session".to_string())?;
                        let command = match control {
                            "pause" => GoalRuntimeCommand::Pause {
                                status: GoalStatus::Paused,
                                wake_at_epoch_ms: None,
                                now_epoch_ms: goal_now_epoch_ms(),
                            },
                            "resume" => GoalRuntimeCommand::Resume {
                                now_epoch_ms: goal_now_epoch_ms(),
                            },
                            "stop" | "cancel" => GoalRuntimeCommand::Cancel {
                                now_epoch_ms: goal_now_epoch_ms(),
                            },
                            _ => unreachable!(),
                        };
                        submit_goal_command(
                            config,
                            format!("work:{session_id}:{control}:{}", uuid::Uuid::new_v4()),
                            command,
                        )?;
                        let message = match control {
                            "pause" => UiText::WorkPaused,
                            "resume" => UiText::WorkResumed,
                            _ => UiText::WorkStopped,
                        };
                        format!("  {} · `{work_id}`", ui_text(message))
                    } else if item.is_sop() {
                        work::control_sop(config, work_id, control)?;
                        let message = match control {
                            "pause" => UiText::WorkPaused,
                            "resume" => UiText::WorkResumed,
                            _ => UiText::WorkStopped,
                        };
                        format!("  {} · `{work_id}`", ui_text(message))
                    } else if matches!(control, "stop" | "cancel") {
                        let outcome =
                            task_runtime::manage_runtime_tasks(&config.host_invocation_request()?, "stop", Some(work_id));
                        if outcome.error {
                            format_error(&outcome.display)
                        } else {
                            format!("  {} · `{work_id}`", ui_text(UiText::WorkStopped))
                        }
                    } else {
                        format!("  {}", ui_text(UiText::WorkControlUnsupported))
                    }
                }
                _ => unreachable!(),
            };
            emit_work(body);
            Ok(SlashCommandResult::Handled)
        }
        "/tasks" => {
            let arguments = parts.collect::<Vec<_>>();
            let (action, task_id) = match arguments.as_slice() {
                [] => ("list", None),
                [task_id] => ("status", Some(*task_id)),
                [action, task_id] if matches!(*action, "status" | "stop") => {
                    (*action, Some(*task_id))
                }
                _ => {
                    terminal_notice!("  {}", ui_text(UiText::TasksUsage));
                    return Ok(SlashCommandResult::Handled);
                }
            };
            let outcome = task_runtime::manage_runtime_tasks(&config.host_invocation_request()?, action, task_id);
            let body = if outcome.error {
                format_error(&outcome.display)
            } else if action == "stop" {
                format!("  {}", outcome.display)
            } else {
                render_markdown_text(&outcome.text)
            };
            crate::terminal_ui::commit_history(&format!(
                "{}\n{}",
                interactive_layer_rule(Some(ui_text(UiText::Tasks))),
                body
            ));
            Ok(SlashCommandResult::Handled)
        }
        "/jobs" => {
            let arguments = parts.collect::<Vec<_>>();
            let (action, task_id) = match arguments.as_slice() {
                [] => ("list", None),
                [task_id] => ("status", Some(*task_id)),
                [action, task_id] if matches!(*action, "status" | "stop") => {
                    (*action, Some(*task_id))
                }
                _ => {
                    terminal_notice!("  {}", tr(Message::InteractiveJobsUsage));
                    return Ok(SlashCommandResult::Handled);
                }
            };
            let outcome = tools::manage_background_bash_tasks(
                action,
                task_id,
                &config.allowed_dirs,
                config.session_id.as_deref(),
                &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(config.skill_dirs.clone()),
            );
            let body = if outcome.error {
                format_error(&outcome.display)
            } else {
                render_markdown_text(&outcome.text)
            };
            crate::terminal_ui::commit_history(&format!(
                "{}\n{}",
                interactive_layer_rule(Some(tr(Message::InteractiveJobsTitle))),
                body
            ));
            Ok(SlashCommandResult::Handled)
        }
        "/resume" => {
            terminal_notice!("  {}", ui_text(UiText::ResumeUsage));
            Ok(SlashCommandResult::Handled)
        }
        "/continue" => {
            terminal_notice!("  {}", ui_text(UiText::ContinueUsage));
            Ok(SlashCommandResult::Handled)
        }
        "/history" => {
            print_interactive_session_history(config)?;
            Ok(SlashCommandResult::Handled)
        }
        "/plan" => {
            let arguments = parts.collect::<Vec<_>>();
            match arguments.as_slice() {
                [] | ["show"] => print_interactive_session_plan(config)?,
                ["approve", step_ids @ ..] => update_interactive_session_plan_approval(
                    config,
                    step_ids.iter().map(|id| (*id).to_string()).collect(),
                    true,
                )?,
                ["reject", step_ids @ ..] => update_interactive_session_plan_approval(
                    config,
                    step_ids.iter().map(|id| (*id).to_string()).collect(),
                    false,
                )?,
                ["edit", step_id, content @ ..] if !content.is_empty() => {
                    edit_interactive_session_plan_step(config, step_id, &content.join(" "))?
                }
                ["freeze"] => freeze_interactive_session_plan(config)?,
                ["replan", step_id, content @ ..] if !content.is_empty() => {
                    replan_interactive_session_plan_step(config, step_id, &content.join(" "))?
                }
                ["history"] => print_interactive_session_plan_history(config)?,
                ["diff", from, to] => {
                    let from = from
                        .trim_start_matches('r')
                        .parse::<i64>()
                        .map_err(|_| "from revision must be an integer".to_string())?;
                    let to = to
                        .trim_start_matches('r')
                        .parse::<i64>()
                        .map_err(|_| "to revision must be an integer".to_string())?;
                    print_interactive_session_plan_diff(config, from, to)?;
                }
                _ => terminal_notice!(
                    "  /plan [show|approve|reject|edit|freeze|replan|history|diff] [arguments ...]"
                ),
            }
            Ok(SlashCommandResult::Handled)
        }
        _ => {
            terminal_notice!(
                "  {command} · {}",
                tr(Message::InteractiveUnknownCommandHint)
            );
            Ok(SlashCommandResult::Handled)
        }
    }
}

/// Resume the selected canonical Run before accepting any fresh composer input.
fn resume_selected_session_before_composer(config: &Config, emitter: &mut Emitter) -> Result<bool, String> {
    // Opening settled history is not an execution phase. Starting the live
    // input listener here briefly exposed a second composer and consumed keys
    // before the actual restored editor owned its draft/cursor.
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    if let Some(id) = config.session_id.as_deref() {
        if application.active_run(id)?.is_none() {
            // An imported/new conversation has no execution checkpoint yet.
            if application.load_checkpoint(id)?.is_none() { return Ok(true); }
            let execution: nola_application::HostExecutionState = application.execute(
                nola_application::ApplicationCommand::LoadHostExecutionState { conversation_id: id.into() })?;
            if execution.control.queued_input_count == 0 { return Ok(true); }
        }
    }
    let cancellation = if config.json_output { interaction::ExecutionCancellation::default() }
        else { interaction::ExecutionCancellation::interactive() };
    match with_cli_execution_control(config, cancellation, emitter, |control, emitter|
        resume_cli_core_pending(config, emitter, control))? {
        Some(outcome) if outcome.turn_status == nola_core::agent::AgentTurnStatus::AwaitingInput => Ok(true),
        Some(outcome) if !outcome.is_terminal() => {
            if !config.json_output {
                eprintln!("{}", match current_language() {
                    Language::ZhHans => "操作仍在等待恢复。可再次运行 --resume；原请求已保留。",
                    _ => "The run is still waiting. Use --resume to retry; the original request is preserved.",
                });
            }
            Ok(false)
        }
        None => {
            // A busy owner can make recovery a no-op. That does not grant a
            // second composer permission to submit over its still-active Run.
            let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
            match config.session_id.as_deref() {
                Some(id) => Ok(application.active_run(id)?.is_none()),
                None => Ok(true),
            }
        }
        Some(_) => Ok(true),
    }
}
