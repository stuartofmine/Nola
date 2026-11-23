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
            terminal_notice!("{}", crate::i18n::cli_view_state_restore_failed(&error));
            (None, None)
        }
    }
}

fn persist_interactive_view_state(store: &mut Option<crate::cli_view_state::ViewStore>, draft: String, cursor: usize, expert_mentions: Vec<crate::cli_expert_mentions::Occurrence>, edit: Option<crate::cli_expert_mentions::EditTarget>) {
    let Some(active) = store.as_mut() else { return; };
    let view = crate::cli_view_state::ViewState {
        draft,
        cursor,
        expert_mentions,
        edit,
        viewport: crate::terminal_ui::capture_viewport_bookmark(),
    };
    if let Err(error) = active.save(view) {
        terminal_notice!("{}", crate::i18n::cli_view_state_save_failed(&error));
        *store = None;
    }
}

fn interactive_git_branch(project_root: &Path) -> Option<String> {
    let head = std::fs::read_to_string(project_root.join(".git").join("HEAD")).ok()?;
    let trimmed = head.trim();
    trimmed
        .strip_prefix("ref: refs/heads/")
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(str::to_string)
}

fn update_interactive_session_header(config: &Config) {
    if config.json_output {
        return;
    }
    let workspace = config
        .project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| compact_home_path(&config.project_root.to_string_lossy()));
    let detail = match interactive_git_branch(&config.project_root) {
        Some(branch) => format!("{workspace} ({branch})"),
        None => workspace,
    };
    crate::terminal_ui::set_session_header("Nola", detail);
}

fn run_interactive_session(config: &Config, resume_pending: bool) -> Result<InteractiveSessionExit, String> {
    clear_pending_task_completions();
    crate::composer_media::prune_old_staged_files();
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
    update_interactive_session_header(&interactive_config);
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
    let mut rejected_mentions = Vec::new();
    let mut goal_entry = false;
    let mut view_session = interactive_config.session_id.clone();
    let (mut view_store, saved_view) = open_interactive_view_state(&interactive_config);
    let mut edit_target = saved_view.as_ref().and_then(|v| v.edit.clone());
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
                terminal_notice!("{}", crate::i18n::cli_saved_viewport_unavailable());
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
            terminal_notice!("{}", crate::i18n::cli_recovery_incomplete_hint());
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
    let mut questions_requested = false;
    loop {
        if view_session != interactive_config.session_id {
            if view_session.is_some() {
                clear_pending_task_completions();
            }
            rejected_mentions.clear();
            view_session = interactive_config.session_id.clone();
            interaction::select_interactive_composer(interactive_composer_key(&interactive_config)?);
            pending_draft = None;
            let (store, view) = open_interactive_view_state(&interactive_config);
            edit_target = view.as_ref().and_then(|v| v.edit.clone());
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
        update_interactive_session_header(&interactive_config);
        if offer_user_questions && !goal_entry && !interactive_config.json_output {
            offer_user_questions = false;
            let requested = std::mem::take(&mut questions_requested);
            if let Some(conversation) = interactive_config.session_id.as_deref() {
                let child_waits = crate::user_input_wait::owned(conversation, None).unwrap_or_default();
                let include_children = requested || !child_waits.is_empty();
                if !child_waits.is_empty() {
                    terminal_notice!(
                        "\n  ◆ {} ({}) — /questions",
                        match current_language() {
                            Language::ZhHans => "子任务有待回答问题，正在打开交互提示",
                            _ => "Pending question(s) from child task",
                        },
                        child_waits.len()
                    );
                }
                match crate::user_input_wait::present_pending(
                    conversation,
                    include_children,
                    &interaction::TerminalInteraction,
                ) {
                    Ok(outcome) => {
                        if requested && !outcome.had_questions {
                            terminal_notice!("{}", ui_text(UiText::NoPendingQuestions));
                        }
                        if let Some(error) = outcome.notice { terminal_notice!("{}", format_error(&error)); }
                        if outcome.deferred && outcome.had_questions {
                            terminal_notice!("{}", ui_text(UiText::QuestionDeferred));
                        }
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
                                Ok(resumed) => {
                                    // A resumed answer may already be sealed by its owner. The
                                    // new terminal emitter then has no live text events to replay.
                                    if resumed.as_ref().is_none_or(|run| run.is_terminal()) {
                                        match session::load_interactive_session_snapshot(conversation) {
                                            Ok(snapshot) => replace_interactive_session_replay(&snapshot.saved, &interactive_config),
                                            Err(error) => terminal_notice!("{}", format_error(&error)),
                                        }
                                    }
                                    offer_user_questions = !outcome.deferred;
                                    continue;
                                }
                                Err(error) => terminal_notice!("{}", format_error(&error)),
                            }
                        }
                    }
                    Err(error) => terminal_notice!("{}", format_error(&error)),
                }
            } else if requested {
                terminal_notice!("{}", ui_text(UiText::NoPendingQuestions));
            }
        }
        queued_user_inputs.extend(interaction::take_interactive_queued_inputs());
        let prompt_footer = if edit_target.is_some() {
            Some(ui_text(UiText::EditInputHint).to_string())
        } else if goal_entry {
            Some(ui_text(UiText::GoalInputHint).to_string())
        } else if !interactive_config.json_output {
            interactive_prompt_footer_line(
                &interactive_config,
                interactive_config.session_id.as_deref(),
                last_context_usage,
            )
        } else {
            None
        };
        let maybe_prompt = if let Some(queued) = (!goal_entry && edit_target.is_none())
            .then(|| queued_user_inputs.pop_front())
            .flatten()
        {
            if !interactive_config.json_output {
                commit_interactive_user_history_line(&queued);
            }
            Some(InteractivePromptInput::Submitted(queued))
        } else {
            let mut goal_view_store = None;
            read_interactive_prompt_line(
                &input_history,
                &mut history_cursor,
                !interactive_config.json_output,
                prompt_footer.as_deref(),
                &interactive_config,
                pending_draft.take().as_deref(),
                if goal_entry { &mut goal_view_store } else { &mut view_store },
                std::mem::take(&mut rejected_mentions), edit_target.as_ref(), goal_entry,
            )
            .map_err(|err| format!("{} {err}", tr(Message::FailedReadPromptPrefix)))?
        };
        let mut selected_experts = None;
        let mut submitted_mentions = Vec::new();
        let prompt = match maybe_prompt {
            None => break,
            Some(InteractivePromptInput::CancelGoal) => {
                goal_entry = false;
                pending_draft = Some(String::new());
                interaction::clear_interactive_execution_draft();
                continue;
            }
            Some(InteractivePromptInput::CancelEdit) => {
                edit_target = None;
                interaction::clear_interactive_execution_draft();
                persist_interactive_view_state(&mut view_store, String::new(), 0, vec![], None);
                continue;
            }
            Some(InteractivePromptInput::Submitted(prompt)) => prompt,
            Some(InteractivePromptInput::Mentioned(prompt, selection, mentions)) => {
                selected_experts = Some(selection);
                submitted_mentions = mentions;
                prompt
            }
        };
        if (goal_entry || edit_target.is_some()) && !interactive_config.json_output {
            let footer = interactive_prompt_footer_line(
                &interactive_config,
                interactive_config.session_id.as_deref(),
                last_context_usage,
            );
            crate::terminal_ui::set_footer(footer.as_deref().unwrap_or_default());
        }
        let chat_commands = !goal_entry && edit_target.is_none();
        let first_token = prompt.split_whitespace().next().unwrap_or("");
        let normalized_command = normalize_interactive_command(first_token);
        if !goal_entry
            && normalized_command == Some("/cancel-edit")
            && prompt.split_whitespace().nth(1).is_none()
        {
            if edit_target.is_none() {
                terminal_notice!("{}", ui_text(UiText::NoActiveEdit));
                continue;
            }
            edit_target = None;
            rejected_mentions.clear();
            persist_interactive_view_state(&mut view_store, String::new(), 0, vec![], None);
            continue;
        }
        if chat_commands && normalized_command == Some("/edit") {
            let Some(conversation) = interactive_config.session_id.as_deref() else {
                terminal_notice!("{}", ui_text(UiText::NoMessageToEdit));
                continue;
            };
            let id = prompt
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest.trim())
                .unwrap_or("");
            match crate::cli_expert_mentions::EditTarget::load(
                conversation,
                (!id.is_empty()).then_some(id),
            ) {
                Ok((target, text, runs)) => {
                    edit_target = Some(target);
                    pending_draft = Some(text);
                    rejected_mentions = runs;
                }
                Err(error) => terminal_notice!("{}", error),
            }
            continue;
        }
        if prompt.is_empty() {
            continue;
        }
        if chat_commands
            && normalized_command == Some("/questions")
            && prompt.split_whitespace().nth(1).is_none()
        {
            offer_user_questions = true;
            questions_requested = true;
            continue;
        }
        if chat_commands && is_exit_command(prompt.as_str()) {
            if !interactive_config.json_output {
                clear_interactive_prompt_area(prompt_footer.as_deref())
                    .map_err(|err| format!("{} {err}", tr(Message::FailedFlushPromptPrefix)))?;
            }
            break;
        }
        if chat_commands {
            if let Some(target) = resume_command_target(prompt.as_str()) {
                match target {
                    Some(id) => {
                        match resume_interactive_session_by_id(&interactive_config, &id) {
                            Ok(next) => return Ok(InteractiveSessionExit::Switch(Box::new(next))),
                            Err(error) => terminal_notice!("{}", format_error(&error)),
                        }
                    }
                    None => {
                        match select_interactive_session_restore(&interactive_config, false) {
                            Ok(Some(next)) => return Ok(InteractiveSessionExit::Switch(Box::new(next))),
                            Ok(None) => {}
                            Err(error) => terminal_notice!("{}", format_error(&error)),
                        }
                        crate::terminal_ui::invalidate_presented_screen();
                    }
                }
                continue;
            }
            if is_continue_command(prompt.as_str()) {
                match select_interactive_session_restore(&interactive_config, true) {
                    Ok(Some(next)) => return Ok(InteractiveSessionExit::Switch(Box::new(next))),
                    Ok(None) => {}
                    Err(error) => terminal_notice!("{}", format_error(&error)),
                }
                crate::terminal_ui::invalidate_presented_screen();
                continue;
            }
        }
        if recovery_blocked && (!chat_commands || !prompt.starts_with('/')) {
            terminal_notice!("{}", ui_text(UiText::RecoveryBlockedUseResume));
            pending_draft = Some(prompt);
            continue;
        }
        if chat_commands && prompt == "/" {
            print_interactive_help("");
            continue;
        }
        if (!chat_commands || !prompt.starts_with('/')) && interactive_config.save_session {
            if let Some(session)=interactive_config.session_id.as_deref() {
                if let Err(error)=crate::boundary_input_intent::ensure_no_pending(session) {
                    terminal_notice!("{}",format_error(&error));
                    pending_draft=Some(prompt);
                    continue;
                }
            }
        }
        let submitted_command = if goal_entry {
            begin_interactive_goal_input(&prompt, &interactive_config, &mut goal_target, &mut goal_constraints)
        } else if edit_target.is_some() {
            Ok(SlashCommandResult::PassThrough)
        } else { handle_interactive_slash_command(
            &prompt,
            &mut interactive_config,
            &mut goal_target,
            &mut goal_constraints,
            &mut previous_workspace,
            &mut last_context_usage,
            turn,
        ) };
        let command_result = match submitted_command {
            Ok(result) => result,
            Err(error) => {
                terminal_notice!("{}", format_error(&error));
                pending_draft = Some(prompt);
                continue;
            }
        };
        match command_result {
            SlashCommandResult::RunGoalPrompt(run_prompt) => {
                goal_entry = false;
                if recovery_blocked {
                    terminal_notice!("{}", ui_text(UiText::RecoveryBlockedUseResume));
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
                        .unwrap_or_default();
                    let goal_result = run_goal_prompt_cycle(
                        &mut interactive_config,
                        run_prompt.as_str(),
                        &goal,
                        requirements.as_str(),
                    );
                    if let Ok(outcome) = &goal_result {
                        let handoff = match outcome {
                            GoalCycleOutcome::Completed(handoff)
                            | GoalCycleOutcome::Failed { handoff, .. }
                            | GoalCycleOutcome::Cancelled(handoff) => {
                                goal_target = None;
                                goal_constraints = None;
                                handoff.as_ref()
                            }
                            GoalCycleOutcome::Paused(handoff)
                            | GoalCycleOutcome::NeedsUser { handoff, .. }
                            | GoalCycleOutcome::Waiting { handoff, .. } => handoff.as_ref(),
                            GoalCycleOutcome::Yielded(handoff) => Some(handoff),
                        };
                        if let Some(handoff) = handoff {
                            queued_user_inputs.extend(handoff.queued_user_inputs.iter().cloned());
                            if let Some(draft) = handoff.draft_user_input.as_ref() {
                                pending_draft = Some(draft.clone());
                            }
                            if let Some(context_usage) = handoff.context_usage {
                                last_context_usage = Some(context_usage);
                            }
                        }
                        reset_interactive_terminal_state();
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
            SlashCommandResult::GoalEntry => {
                goal_entry = true;
                pending_draft = Some(String::new());
                if !interactive_config.json_output { terminal_notice!("  {}", ui_text(UiText::GoalIntro)); }
                continue;
            }
            SlashCommandResult::Draft(draft) => { pending_draft = Some(draft); continue; }
            SlashCommandResult::Handled => continue,
            SlashCommandResult::WorkspaceChanged | SlashCommandResult::SessionReset => {
                clear_pending_task_completions();
                edit_target = None;
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
        let mut turn_config = interactive_config.clone();
        turn_config.main_expert_selection = selected_experts;
        if turn_config.main_expert_selection.is_some() || edit_target.is_some() {
            let id = uuid::Uuid::new_v4().to_string();
            if let Some(target) = &edit_target {
                if let Err(error) =
                    target.prepare(&id, &prompt, turn_config.main_expert_selection.as_ref())
                {
                    terminal_notice!("{}", error);
                    pending_draft = Some(prompt);
                    rejected_mentions = submitted_mentions;
                    continue;
                }
            }
            turn_config.host_user_message_id = Some(format!("{id}:user"));
            turn_config.host_assistant_message_id = Some(format!("{id}:assistant"));
            turn_config.host_submission_id = Some(id);
        }
        let editing = edit_target.is_some();
        let pending_task_prefix = if editing {
            None
        } else {
            drain_pending_task_completion_prefix()
        };
        let effective_prompt =
            inject_completed_task_updates(prompt.as_str(), pending_task_prefix.as_deref());
        let mut replay_edited_history = false;
        let run_result = match run_single_prompt_with_cancellation(
            &turn_config,
            effective_prompt.as_str(),
        ) {
            Ok(result) => {
                replay_edited_history = editing;
                if (!submitted_mentions.is_empty() || edit_target.is_some())
                    && result.draft_user_input.is_none()
                {
                    persist_interactive_view_state(&mut view_store, String::new(), 0, vec![], None);
                }
                edit_target = None;
                offer_user_questions = true;
                let opens_question = !interactive_config.json_output
                    && (interactive_config.session_id.is_some() || result.session_id.is_some())
                    && result.run_outcome.turn_status == nola_core::agent::AgentTurnStatus::AwaitingInput;
                if !result.run_outcome.is_terminal() && !opens_question {
                    terminal_notice!(
                        "Run remains pending ({:?}); no completion or permission decision was recorded.",
                        result.run_outcome.turn_status
                    );
                }
                if interactive_config.session_id.is_none() {
                    interactive_config.session_id = result.session_id.clone();
                    interactive_config.continue_last = false;
                }
                let turn_cancelled = result.run_outcome.turn_status
                    == nola_core::agent::AgentTurnStatus::Cancelled
                    || result.terminal_reason.as_deref() == Some("cancelled");
                if turn_cancelled && !interactive_config.json_output {
                    if let Ok(req) = interactive_config.host_invocation_request() {
                        let running = task_runtime::running_task_count(&req);
                        if running > 0 {
                            print_still_running_subagents_notice(&interactive_config, running);
                        }
                    }
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
            if !submitted_mentions.is_empty() || edit_target.is_some() {
                let admitted = turn_config.session_id.as_ref().map(|conversation| {
                    crate::application_port::CliApplicationPort::persistent_default().and_then(
                        |mut app| {
                            app.execute::<Option<nola_application::ExactRunInvocation>>(
                                nola_application::ApplicationCommand::LoadExactRunInvocation {
                                    conversation_id: conversation.clone(),
                                    expected_run_id: turn_config
                                        .host_submission_id
                                        .clone()
                                        .unwrap(),
                                    assistant_message_id: turn_config
                                        .host_assistant_message_id
                                        .clone()
                                        .unwrap(),
                                },
                            )
                        },
                    )
                });
                if matches!(admitted, None | Some(Ok(None))) {
                    pending_draft = Some(prompt.clone());
                    rejected_mentions = submitted_mentions;
                    persist_interactive_view_state(
                        &mut view_store,
                        prompt.clone(),
                        prompt.len(),
                        rejected_mentions.clone(),
                        edit_target.clone(),
                    );
                } else {
                    replay_edited_history = editing && matches!(admitted, Some(Ok(Some(_))));
                    edit_target = None;
                }
            }
        }
        if replay_edited_history && !interactive_config.json_output {
            if let Some(saved) = restore_interactive_session_messages(
                &interactive_config,
                &mut started,
                &mut turn,
                &mut input_history,
            )? {
                replace_interactive_session_replay(&saved, &interactive_config);
                history_cursor = None;
                continue;
            }
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
    if let Some(summary) = crate::terminal_ui::take_exit_assistant_summary() {
        terminal_notice!();
        terminal_notice!("{}", interactive_layer_rule(Some(crate::i18n::cli_section_label("last response"))));
        for line in summary.lines() {
            terminal_notice!("  {line}");
        }
    }
    let Some(command) = interactive_session_resume_command(config) else { return; };
    terminal_notice!();
    terminal_notice!("{}", interactive_layer_rule(Some(crate::i18n::cli_section_label("continue"))));
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
    terminal_notice!("{}", interactive_layer_rule(Some(crate::i18n::cli_section_label("state"))));
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
    let width = composer_content_rule_width(terminal_columns()).max(36);
    let use_color = interactive_color_enabled();
    let bold = if use_color { "\x1b[1m" } else { "" };
    let dim = if use_color { "\x1b[2;39m" } else { "" };
    let reset = if use_color { INTERACTIVE_RESET } else { "" };

    let tagline = tr(Message::InteractiveWelcomeTagline);
    let workspace = compact_home_path(&config.project_root.to_string_lossy());
    let workspace_label = tr(Message::InteractiveWorkingDirLabel);

    let rule = "─".repeat(width);
    let ws_budget = width
        .saturating_sub(display_width(workspace_label) + 2)
        .max(16);
    let ws_val = truncate_display_path(&workspace, ws_budget);
    let quick_actions =
        truncate_display_width("/help · /model · /resume · /attach · @file", width);

    let rendered = format!(
        "\n  {bold}Nola{reset}  {dim}{}{reset}\n  {dim}{rule}{reset}\n  {dim}{workspace_label}{reset}  {ws_val}\n  {dim}{quick_actions}{reset}\n\n",
        truncate_display_width(tagline, width.saturating_sub(6)),
    );
    crate::terminal_ui::commit_history(&rendered);
    Ok(())
}

enum InteractivePromptInput {
    Submitted(String),
    Mentioned(
        String,
        nola_domain::expert_task_target::MainExpertSelectionContextV1,
        Vec<crate::cli_expert_mentions::Occurrence>,
    ),
    CancelGoal,
    CancelEdit,
}
thread_local! {
    static MENTION_MENU: std::cell::RefCell<(String, usize, Option<usize>)> = Default::default();
}
fn mention_menu_focus(line: &str, cursor: usize) -> Option<usize> {
    MENTION_MENU.with(|m| {
        let m = m.borrow();
        if m.0 == line && m.1 == cursor {
            m.2
        } else {
            Some(0)
        }
    })
}
fn set_mention_focus(line: &str, cursor: usize, index: Option<usize>) {
    MENTION_MENU.with(|m| *m.borrow_mut() = (line.into(), cursor, index));
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ComposerPurpose {
    Chat,
    Goal,
    Edit,
}

thread_local! { static COMPOSER_PURPOSE: Cell<ComposerPurpose> = const { Cell::new(ComposerPurpose::Chat) }; }
struct ComposerScope(ComposerPurpose);
impl Drop for ComposerScope {
    fn drop(&mut self) {
        COMPOSER_PURPOSE.with(|active| active.set(self.0));
    }
}

fn read_interactive_prompt_line(
    input_history: &[String],
    history_cursor: &mut Option<usize>,
    show_prompt: bool,
    prompt_footer: Option<&str>,
    config: &Config,
    initial_draft: Option<&str>,
    view_store: &mut Option<crate::cli_view_state::ViewStore>,
    recovered_mentions: Vec<crate::cli_expert_mentions::Occurrence>,
    edit_target: Option<&crate::cli_expert_mentions::EditTarget>,
    goal_entry: bool,
) -> io::Result<Option<InteractivePromptInput>> {
    crate::cli_expert_mentions::begin_composer();
    let purpose = if goal_entry {
        ComposerPurpose::Goal
    } else if edit_target.is_some() {
        ComposerPurpose::Edit
    } else {
        ComposerPurpose::Chat
    };
    let _scope = ComposerScope(COMPOSER_PURPOSE.with(|active| active.replace(purpose)));
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
        ).map(InteractivePromptInput::Submitted));
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
        ).map(InteractivePromptInput::Submitted));
    };
    // terminal_ui remains the sole owner of the bottom surface in both idle
    // and running phases. The former hand-off to a second ANSI renderer was
    // responsible for blank gaps, cursor drift and a composer that vanished
    // after Enter.
    let inherited_surface_rows = 0;
    let snapshot = interaction::interactive_composer_snapshot();
    let mut line = initial_draft.unwrap_or(&snapshot.draft).to_string();
    let mut cursor = if snapshot.draft == line { snapshot.cursor } else { line.len() };
    let mut mentions = if recovered_mentions.is_empty() {
        view_store
            .as_ref()
            .map(|s| s.mentions_for(&line))
            .unwrap_or_default()
    } else {
        recovered_mentions
    };
    mentions.retain(|r| line.get(r.start..r.end) == Some(r.text.as_str()));
    let mut undo_mention: Option<(
        String,
        usize,
        Vec<crate::cli_expert_mentions::Occurrence>,
        Vec<InteractivePastedContent>,
        String,
    )> = None;
    SLASH_MENU.with(|menu| *menu.borrow_mut() = SlashMenu::default());
    set_mention_focus(&line, cursor, Some(0));
    let mut pasted_contents: Vec<InteractivePastedContent> = Vec::new();
    *history_cursor = None;
    let mut history_draft = None;
    let mut esc_pending_clear = false;
    let mut completion_cycle: Option<ComposerCompletionCycle> = None;
    INTERACTIVE_COMPOSER_CURSOR_ROW.with(|row| row.set(0));
    INTERACTIVE_COMPOSER_FRAME_ROWS.with(|rows| rows.set(inherited_surface_rows));
    if show_prompt {
        draw_interactive_prompt_area(&line, cursor, prompt_footer, config)?;
    }
    _raw_mode.enable_bracketed_paste()?;
    let mut background_notification_pending = true;
    let mut scheduler_event_cursor = 0_i64;
    let mut last_rendered_background_jobs = if show_prompt {
        interactive_running_background_jobs_count(config)
    } else {
        0
    };

    loop {
        let effective_footer = interactive_effective_footer(prompt_footer, esc_pending_clear);
        let expanded = expand_interactive_pasted_content(&line, &pasted_contents);
        let expanded_cursor = expand_interactive_pasted_content(&line[..cursor], &pasted_contents).len();
        let mut expanded_mentions = mentions.clone();
        for r in &mut expanded_mentions {
            r.start = expand_interactive_pasted_content(&line[..r.start], &pasted_contents).len();
            r.end = r.start + r.text.len();
        }
        persist_interactive_view_state(
            view_store,
            expanded.clone(),
            expanded_cursor,
            expanded_mentions.clone(),
            edit_target.cloned(),
        );
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
            let mut redrew_prompt = false;
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
                        draw_interactive_prompt_area(&line, cursor, effective_footer.as_deref(), config)?;
                        redrew_prompt = true;
                    }
                }
            }
            if let Ok(notifications) = task_runtime::take_runtime_task_notifications(&invocation_request) {
                if !notifications.is_empty() {
                    clear_interactive_prompt_area(effective_footer.as_deref())?;
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
                    draw_interactive_prompt_area(&line, cursor, effective_footer.as_deref(), config)?;
                    redrew_prompt = true;
                }
            }
            let current_background_jobs = interactive_running_background_jobs_count(config);
            if !redrew_prompt && current_background_jobs != last_rendered_background_jobs {
                draw_interactive_prompt_area(&line, cursor, effective_footer.as_deref(), config)?;
            }
            last_rendered_background_jobs = current_background_jobs;
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
        if let Event::Mouse(mouse) = event {
            crate::terminal_ui::handle_mouse_scroll(mouse.kind, mouse.row);
            continue;
        }
        if let Event::Resize(_, _) = event {
            // Keep the draft, cursor and footer in one projection when the
            // terminal changes size. Invalidate the cached viewport before
            // drawing so the resized terminal paints once cleanly.
            crate::terminal_ui::invalidate_presented_screen();
            draw_interactive_prompt_area(&line, cursor, effective_footer.as_deref(), config)?;
            continue;
        }
        if let Event::Key(key) = event {
            if crate::terminal_ui::scroll_history_key(&key) { continue; }
            if key.kind != KeyEventKind::Release && key.code != KeyCode::Esc {
                esc_pending_clear = false;
            }
            if key.kind == KeyEventKind::Release {
                continue;
            }
            let query = active_at_file_token(&line, cursor).filter(|_| {
                !goal_entry
                    && mention_menu_focus(&line, cursor).is_some()
                    && !line.trim_start().starts_with('/')
                    && !mentions.iter().any(|r| r.start < cursor && cursor <= r.end)
            });
            let query_open = query.is_some();
            if key.code == KeyCode::Esc && query_open {
                set_mention_focus(&line, cursor, None);
                completion_cycle = None;
                draw_composer_frame(&line, cursor, prompt_footer, None)?;
                continue;
            }
            if key.code == KeyCode::Char('z') && key.modifiers == KeyModifiers::CONTROL {
                if let Some((previous, at, old_mentions, old_pastes, after)) = undo_mention.take() {
                    if line == after {
                        line = previous;
                        cursor = at;
                        mentions = old_mentions;
                        pasted_contents = old_pastes;
                        redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                        continue;
                    }
                }
            }
            if let Some((start, end, prefix)) = query.filter(|_| {
                is_interactive_submit_event(&key) || matches!(key.code, KeyCode::Down | KeyCode::Up)
            }) {
                let candidates = interactive_mention_candidates(config, prefix);
                if !candidates.is_empty() {
                    let index = mention_menu_focus(&line, cursor).unwrap_or(0) % candidates.len();
                    if is_interactive_submit_event(&key) {
                        let previous = (line.clone(), cursor, mentions.clone(), pasted_contents.clone());
                        cursor = insert_composer_candidate(&mut line, start, end, &candidates[index].0, &mut mentions, &mut pasted_contents);
                        undo_mention = Some((previous.0, previous.1, previous.2, previous.3, line.clone()));
                        set_mention_focus(&line, cursor, None);
                    } else {
                        let offset = if key.code == KeyCode::Up { candidates.len() - 1 } else { 1 };
                        set_mention_focus(&line, cursor, Some((index + offset) % candidates.len()));
                    }
                    redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    continue;
                }
            }
            let mut submit_command = None;
            let menu_handled = SLASH_MENU.with(|menu| {
                let mut menu = menu.borrow_mut();
                menu.sync(&line, cursor, config);
                if !menu.is_open() { return false; }
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, KeyModifiers::NONE) => menu.dismissed = true,
                    (KeyCode::Up, KeyModifiers::NONE) | (KeyCode::Char('p'), KeyModifiers::CONTROL)
                        | (KeyCode::BackTab, _) => menu.step(true),
                    (KeyCode::Down, KeyModifiers::NONE) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => menu.step(false),
                    (KeyCode::Tab, KeyModifiers::NONE) => {
                        let command = &menu.entries[menu.selected.unwrap_or(0)].name;
                        line = format!("{command} ");
                        cursor = line.len();
                        mentions.clear();
                        pasted_contents.clear();
                        completion_cycle = None;
                        // Subcommand suggestions, if any, belong to the newly inserted text.
                    }
                    _ if is_interactive_submit_event(&key) => {
                        submit_command = menu.selected.map(|index| menu.entries[index].name.clone());
                        return false;
                    }
                    _ => return false,
                }
                true
            });
            if menu_handled {
                redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                continue;
            }
            if !matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                completion_cycle = None;
            }
            if crate::composer_media::paste_key(&key) {
                match crate::composer_media::paste_image() {
                    Ok(marker) => {
                        adjust_pasted_ranges(&mut pasted_contents, cursor, cursor, marker.len());
                        crate::cli_expert_mentions::replace(&mut mentions, cursor, cursor, marker.len());
                        line.insert_str(cursor, &marker);
                        cursor += marker.len();
                    }
                    Err(error) => terminal_notice!("{}", crate::composer_media::paste_error(&error)),
                }
                redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                continue;
            }
            if key.kind == KeyEventKind::Press && handle_backslash_enter_continuation(&mut line, &mut cursor, &key) {
                redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                continue;
            }
            if is_interactive_submit_event(&key) {
                let (expanded, expanded_mentions) = if let Some(command) = submit_command {
                    line.clone_from(&command);
                    (command, Vec::new())
                } else {
                    (expanded, expanded_mentions)
                };
                let selection =
                    match crate::cli_expert_mentions::bind(&expanded, &expanded_mentions) {
                        Ok(value) => value,
                        Err(error) => {
                            terminal_notice!("{}", error);
                            continue;
                        }
                    };
                if selection.is_none() && edit_target.is_none() {
                    persist_interactive_view_state(view_store, String::new(), 0, vec![], None);
                }
                finish_interactive_prompt_line(&line, prompt_footer)?;
                interaction::clear_interactive_execution_draft();
                if line.is_empty() {
                    return Ok(Some(InteractivePromptInput::Submitted(String::new())));
                }
                if let Some(selection) = selection {
                    return Ok(Some(InteractivePromptInput::Mentioned(
                        expanded,
                        selection,
                        expanded_mentions,
                    )));
                }
                let prompt = expanded.trim().to_string();
                return Ok(Some(InteractivePromptInput::Submitted(prompt)));
            }
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let old_cursor = cursor;
            let old_len = line.len();
            if let Some(r) = mentions
                .iter()
                .find(|r| match key.code {
                    KeyCode::Backspace | KeyCode::Left => r.start < cursor && cursor <= r.end,
                    KeyCode::Delete | KeyCode::Right => r.start <= cursor && cursor < r.end,
                    _ => false,
                })
                .cloned()
                .filter(|_| key.modifiers.is_empty())
            {
                match key.code {
                    KeyCode::Left => cursor = r.start,
                    KeyCode::Right => cursor = r.end,
                    _ => {
                        let old = (line.clone(), cursor, mentions.clone(), pasted_contents.clone());
                        line.replace_range(r.start..r.end, "");
                        adjust_pasted_ranges(&mut pasted_contents, r.start, r.end, 0);
                        cursor = r.start;
                        crate::cli_expert_mentions::replace(&mut mentions, r.start, r.end, 0);
                        undo_mention = Some((old.0, old.1, old.2, old.3, line.clone()));
                    }
                }
                redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                continue;
            }
            // Move inside a multiline recall until the caret reaches its outer row.
            let recall_boundary = history_cursor.is_some() && key.modifiers.is_empty() && match key.code {
                KeyCode::Up => !line[..cursor].contains('\n'),
                KeyCode::Down => !line[cursor..].contains('\n'),
                _ => false,
            };
            if !recall_boundary && edit_with_paste_blocks(&mut line, &mut cursor, &mut pasted_contents, &key) {
                if old_len != line.len() {
                    let start = cursor.min(old_cursor);
                    crate::cli_expert_mentions::replace(
                        &mut mentions,
                        start,
                        start + old_len.saturating_sub(line.len()),
                        line.len().saturating_sub(old_len),
                    );
                }
                redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                continue;
            }
            match key.code {
                KeyCode::Esc => {
                    if edit_target.is_some() {
                        return Ok(Some(InteractivePromptInput::CancelEdit));
                    }
                    if goal_entry {
                        clear_interactive_composer_frame();
                        return Ok(Some(InteractivePromptInput::CancelGoal));
                    }
                    if !line.is_empty() {
                        if !esc_pending_clear {
                            esc_pending_clear = true;
                            let hint_footer = interactive_effective_footer(prompt_footer, true);
                            redraw_interactive_prompt_line(&line, cursor, hint_footer.as_deref(), config)?;
                            continue;
                        }
                        line.clear();
                        *history_cursor = None;
                        history_draft = None;
                        mentions.clear();
                        cursor = 0;
                        pasted_contents.clear();
                        esc_pending_clear = false;
                        completion_cycle = None;
                        redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    }
                }
                KeyCode::Tab | KeyCode::BackTab => {
                    if !goal_entry
                        && apply_composer_completion_cycle(
                            &mut line,
                            &mut cursor,
                            config,
                            &mut completion_cycle,
                            key.code == KeyCode::BackTab,
                            &mut mentions,
                            &mut pasted_contents,
                        )
                    {
                        set_mention_focus(&line, cursor, None);
                        redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    }
                }
                KeyCode::Up | KeyCode::Down if purpose == ComposerPurpose::Chat => {
                    let browsing = history_cursor.is_some();
                    let direction = if key.code == KeyCode::Up { "\x1b[A" } else { "\x1b[B" };
                    if let Some(history) = interactive_history_prompt(direction, input_history, history_cursor) {
                        if !browsing {
                            history_draft = Some((std::mem::take(&mut line), cursor,
                                std::mem::take(&mut mentions), std::mem::take(&mut pasted_contents)));
                        }
                        line = history;
                        cursor = line.len();
                        mentions.clear();
                        pasted_contents.clear();
                    } else if browsing {
                        if let Some(draft) = history_draft.take() {
                            (line, cursor, mentions, pasted_contents) = draft;
                        }
                    } else {
                        continue;
                    }
                    undo_mention = None;
                    if history_cursor.is_some() {
                        SLASH_MENU.with(|menu| {
                            let mut menu = menu.borrow_mut();
                            menu.sync(&line, cursor, config);
                            menu.dismissed = true;
                        });
                    }
                    set_mention_focus(&line, cursor, None);
                    redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                }
                KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    interaction::clear_interactive_execution_draft();
                    if goal_entry { return Ok(Some(InteractivePromptInput::CancelGoal)); }
                    if !line.is_empty() {
                        line.clear();
                        *history_cursor = None;
                        history_draft = None;
                        mentions.clear();
                        cursor = 0;
                        pasted_contents.clear();
                        esc_pending_clear = false;
                        completion_cycle = None;
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
            esc_pending_clear = false;
            completion_cycle = None;
            match crate::composer_media::paste_path(&text) {
                Ok(Some(marker)) => {
                    adjust_pasted_ranges(&mut pasted_contents, cursor, cursor, marker.len());
                    crate::cli_expert_mentions::replace(&mut mentions,cursor,cursor,marker.len());
                    line.insert_str(cursor, &marker); cursor += marker.len();
                    redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    continue;
                }
                Err(error) => {
                    terminal_notice!("{}", crate::composer_media::paste_path_error(&error));
                    redraw_interactive_prompt_line(&line, cursor, prompt_footer, config)?;
                    continue;
                }
                Ok(None) => {}
            }
            let sanitized = normalize_interactive_paste(&text);
            if !sanitized.is_empty() {
                if should_collapse_pasted_content(&sanitized) {
                    let chars = sanitized.chars().count();
                    let base_badge = crate::i18n::cli_pasted_content_badge(chars);
                    let mut placeholder = base_badge.clone();
                    let mut number = 1;
                    while pasted_contents.iter().any(|block| block.placeholder == placeholder) {
                        number += 1;
                        placeholder = if let Some(prefix) = base_badge.strip_suffix(']') {
                            format!("{prefix} #{number}]")
                        } else {
                            format!("{base_badge} #{number}")
                        };
                    }
                    adjust_pasted_ranges(&mut pasted_contents, cursor, cursor, placeholder.len());
                    crate::cli_expert_mentions::replace(&mut mentions, cursor, cursor, placeholder.len());
                    line.insert_str(cursor, &placeholder);
                    let start = cursor;
                    cursor += placeholder.len();
                    let index = pasted_contents.partition_point(|block| block.start < start);
                    pasted_contents.insert(index, InteractivePastedContent { start, placeholder, content: sanitized.into() });
                } else {
                    adjust_pasted_ranges(&mut pasted_contents, cursor, cursor, sanitized.len());
                    crate::cli_expert_mentions::replace(
                        &mut mentions,
                        cursor,
                        cursor,
                        sanitized.len(),
                    );
                    line.insert_str(cursor, &sanitized);
                    cursor += sanitized.len();
                }
                set_mention_focus(&line, cursor, None);
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
    assert!(!is_interactive_submit_event(&KeyEvent::new(
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

fn should_collapse_pasted_content(text: &str) -> bool {
    text.lines().count() > 5 || text.chars().count() > 300
}

fn handle_backslash_enter_continuation(
    line: &mut String,
    cursor: &mut usize,
    key: &KeyEvent,
) -> bool {
    if key.code != KeyCode::Enter || !key.modifiers.is_empty() {
        return false;
    }
    let idx = (*cursor).min(line.len());
    if !line.is_char_boundary(idx) {
        return false;
    }
    let trailing_backslashes = line[..idx].chars().rev().take_while(|ch| *ch == '\\').count();
    if trailing_backslashes % 2 == 1 {
        let remove_at = idx - 1;
        line.remove(remove_at);
        line.insert(remove_at, '\n');
        *cursor = remove_at + 1;
        true
    } else {
        false
    }
}

/// Keep the original positional identity of each folded paste as the draft changes.
fn adjust_pasted_ranges(blocks: &mut Vec<InteractivePastedContent>, start: usize, end: usize, inserted: usize) {
    blocks.retain_mut(|block| {
        if block.start >= end {
            block.start = block.start - (end - start) + inserted;
            true
        } else {
            block.start + block.placeholder.len() <= start
        }
    });
}

fn edit_with_paste_blocks(
    line: &mut String, cursor: &mut usize, blocks: &mut Vec<InteractivePastedContent>, key: &KeyEvent,
) -> bool {
    let old_len = line.len();
    let old_cursor = *cursor;
    if !crate::composer::edit(line, cursor, key) { return false; }
    if blocks.is_empty() { return true; }
    if line.len() < old_len {
        // Word deletion and Cut can end inside the displayed label. Extend the
        // deletion to the whole block, in the coordinates before the edit.
        let removed = old_len - line.len();
        let start = old_cursor.min(*cursor);
        let end = start + removed;
        let mut left = start;
        let mut right = end;
        for block in blocks.iter() {
            let block_end = block.start + block.placeholder.len();
            if block.start < end && block_end > start {
                left = left.min(block.start);
                right = right.max(block_end);
            }
        }
        line.replace_range(left..right - removed, "");
        *cursor = left;
        adjust_pasted_ranges(blocks, left, right, 0);
    } else if line.len() > old_len {
        adjust_pasted_ranges(blocks, old_cursor, old_cursor, line.len() - old_len);
    } else if *cursor != old_cursor {
        for block in blocks.iter() {
            let end = block.start + block.placeholder.len();
            if block.start < *cursor && *cursor < end {
                *cursor = if *cursor < old_cursor { block.start } else { end };
                break;
            }
        }
    }
    true
}

fn interactive_effective_footer(base_footer: Option<&str>, esc_pending_clear: bool) -> Option<String> {
    if !esc_pending_clear {
        return base_footer.map(str::to_string);
    }
    let hint = match current_language() {
        Language::ZhHans => "再按一次 Esc 清空当前输入",
        _ => "Press Esc again to clear draft",
    };
    match base_footer.map(str::trim).filter(|v| !v.is_empty()) {
        Some(existing) => Some(format!("{existing} · {hint}")),
        None => Some(hint.to_string()),
    }
}

fn expand_interactive_pasted_content(
    input: &str,
    pasted_contents: &[InteractivePastedContent],
) -> String {
    let mut expanded = String::new();
    let mut offset = 0;
    for block in pasted_contents {
        let end = block.start + block.placeholder.len();
        if end > input.len() { break; }
        if input.get(block.start..end) == Some(block.placeholder.as_str()) {
            expanded.push_str(&input[offset..block.start]);
            expanded.push_str(&block.content);
            offset = end;
        }
    }
    expanded.push_str(&input[offset..]);
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

const MAX_PENDING_TASK_COMPLETIONS: usize = 12;
const MAX_COMPLETED_TASK_SUMMARY_CHARS: usize = 320;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletedTaskDigest {
    task_id: String,
    agent_type: Option<String>,
    status: String,
    summary: String,
}

#[derive(Debug, Default)]
struct PendingTaskCompletionInbox {
    entries: Vec<CompletedTaskDigest>,
}

impl PendingTaskCompletionInbox {
    fn push(&mut self, mut digest: CompletedTaskDigest) {
        digest.summary =
            compact_task_digest_summary(&digest.summary, MAX_COMPLETED_TASK_SUMMARY_CHARS);
        if let Some(existing) = self
            .entries
            .iter()
            .position(|entry| entry.task_id == digest.task_id)
        {
            self.entries.remove(existing);
        }
        self.entries.push(digest);
        if self.entries.len() > MAX_PENDING_TASK_COMPLETIONS {
            let overflow = self.entries.len() - MAX_PENDING_TASK_COMPLETIONS;
            self.entries.drain(0..overflow);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn drain_prompt_prefix(&mut self) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let drained = std::mem::take(&mut self.entries);
        let mut lines = Vec::with_capacity(drained.len() + 2);
        lines.push("[Background task updates]".to_string());
        for entry in drained {
            let agent = entry
                .agent_type
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let meta = match agent {
                Some(agent_type) => format!("{agent_type}, {}", entry.status),
                None => entry.status,
            };
            if entry.summary.is_empty() {
                lines.push(format!("- {} ({meta})", entry.task_id));
            } else {
                lines.push(format!("- {} ({meta}): {}", entry.task_id, entry.summary));
            }
        }
        lines.push("[/Background task updates]".to_string());
        Some(lines.join("\n"))
    }
}

thread_local! {
    static PENDING_TASK_COMPLETIONS: std::cell::RefCell<PendingTaskCompletionInbox> =
        std::cell::RefCell::new(PendingTaskCompletionInbox::default());
}

fn record_completed_task_digest(digest: CompletedTaskDigest) {
    PENDING_TASK_COMPLETIONS.with(|inbox| inbox.borrow_mut().push(digest));
}

fn clear_pending_task_completions() {
    PENDING_TASK_COMPLETIONS.with(|inbox| inbox.borrow_mut().clear());
}

fn drain_pending_task_completion_prefix() -> Option<String> {
    PENDING_TASK_COMPLETIONS.with(|inbox| inbox.borrow_mut().drain_prompt_prefix())
}

fn compact_task_digest_summary(raw: &str, max_chars: usize) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{truncated}…")
}

fn inject_completed_task_updates(prompt: &str, prefix: Option<&str>) -> String {
    match prefix.filter(|value| !value.trim().is_empty()) {
        Some(block) => format!("{block}\n\n{prompt}"),
        None => prompt.to_string(),
    }
}

fn is_terminal_runtime_task_status(status: &str) -> bool {
    matches!(status, "completed" | "sealed" | "failed" | "cancelled")
}

fn runtime_task_queued_for_next_turn_hint() -> &'static str {
    match i18n::current_language() {
        i18n::Language::ZhHans => "已记录子代理结果，下一轮对话会自动带给主代理。",
        i18n::Language::ZhHant => "已記錄子代理結果，下一輪對話會自動帶給主代理。",
        i18n::Language::De => "Ergebnis des Unteragenten für den nächsten Zug vorgemerkt.",
        i18n::Language::Es => "Resultado del subagente en cola para el siguiente turno.",
        i18n::Language::Fr => "Résultat du sous-agent mis en file pour le tour suivant.",
        i18n::Language::Ja => "サブエージェントの結果を次のターンに引き継ぐよう記録しました。",
        i18n::Language::Ko => "하위 에이전트 결과를 다음 턴에 자동 전달하도록 기록했습니다.",
        i18n::Language::Ru => "Результат субагента сохранён для следующего хода.",
        i18n::Language::En => "Subagent result queued for the next turn.",
    }
}

fn runtime_task_completion_summary(
    notification: &task_runtime::RuntimeTaskNotification,
) -> String {
    let stored = crate::store::NolaStore::open_default()
        .ok()
        .and_then(|store| store.runtime_task(&notification.id).ok().flatten());
    if let Some(task) = stored {
        for candidate in [
            task.result_summary.as_deref(),
            task.error.as_deref(),
            task.progress_summary.as_deref(),
        ] {
            if let Some(text) = candidate.map(str::trim).filter(|value| !value.is_empty()) {
                return compact_task_digest_summary(text, MAX_COMPLETED_TASK_SUMMARY_CHARS);
            }
        }
    }
    compact_task_digest_summary(&notification.title, MAX_COMPLETED_TASK_SUMMARY_CHARS)
}

fn print_background_task_notifications(
    notifications: &[tools::BackgroundTaskNotification],
) -> io::Result<()> {
    use unicode_width::UnicodeWidthStr;
    let total_cols = terminal_columns();
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
        let chrome_width = 2 + UnicodeWidthStr::width(marker) + 2 + UnicodeWidthStr::width(label) + 2 + 2 + UnicodeWidthStr::width(elapsed.as_str()) + 1;
        let cmd_width = total_cols.saturating_sub(chrome_width).max(16);
        let single_line_cmd = notification.command.split_whitespace().collect::<Vec<_>>().join(" ");
        let command = truncate_display_width(&single_line_cmd, cmd_width);
        let text = if interactive_color_enabled() {
            let color = if success {
                "\x1b[32m"
            } else {
                "\x1b[31m"
            };
            format!(
                "  {color}{marker}  {label}{INTERACTIVE_RESET}  {INTERACTIVE_VALUE}{command}{INTERACTIVE_RESET}  {INTERACTIVE_DIM}{elapsed}{INTERACTIVE_RESET}"
            )
        } else {
            format!("  {marker}  {label}  {command}  {elapsed}")
        };
        tools::deliver_background_task_notification(notification, || crate::terminal_ui::deliver_history(&text)).map_err(io::Error::other)?;
        if is_terminal_runtime_task_status(&notification.canonical_state) {
            record_completed_task_digest(CompletedTaskDigest {
                task_id: notification.id.clone(),
                agent_type: Some("bash".to_string()),
                status: notification.canonical_state.clone(),
                summary: compact_task_digest_summary(
                    &notification.command,
                    MAX_COMPLETED_TASK_SUMMARY_CHARS,
                ),
            });
        }
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
        let terminal = is_terminal_runtime_task_status(&notification.status);
        let success = matches!(notification.status.as_str(), "completed" | "sealed");
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
        let raw_title = match notification
            .agent_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(agent_type) => format!("[{agent_type}] {}", notification.title),
            None => notification.title.clone(),
        };
        let title = truncate_display_width(&raw_title, width);
        let text = if interactive_color_enabled() {
            let color = if waiting_permission {
                "\x1b[33m"
            } else if success {
                "\x1b[32m"
            } else {
                "\x1b[31m"
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
        let delivered_text = if terminal {
            let hint = runtime_task_queued_for_next_turn_hint();
            if interactive_color_enabled() {
                format!("{text}\n  {INTERACTIVE_DIM}{hint}{INTERACTIVE_RESET}")
            } else {
                format!("{text}\n  {hint}")
            }
        } else {
            text
        };
        task_runtime::deliver_runtime_task_notification(notification, || {
            crate::terminal_ui::deliver_history(&delivered_text)
        })
        .map_err(io::Error::other)?;
        if terminal {
            record_completed_task_digest(CompletedTaskDigest {
                task_id: notification.id.clone(),
                agent_type: notification.agent_type.clone(),
                status: notification.status.clone(),
                summary: runtime_task_completion_summary(notification),
            });
        }
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
        (_, "completed") => "Background task completed",
        (_, "failed") => "Background task failed",
        (_, "cancelled") => "Background task cancelled",
        (_, _) => "Background task status unconfirmed",
    }
}

fn runtime_task_notification_label(status: &str) -> &'static str {
    match (i18n::current_language(), status) {
        (i18n::Language::ZhHans, "completed") => "子代理已完成",
        (i18n::Language::ZhHant, "completed") => "子代理已完成",
        (i18n::Language::De, "completed") => "Agent-Aufgabe abgeschlossen",
        (i18n::Language::Es, "completed") => "Tarea del agente completada",
        (i18n::Language::Fr, "completed") => "Tâche de l’agent terminée",
        (i18n::Language::Ja, "completed") => "エージェントタスクが完了しました",
        (i18n::Language::Ko, "completed") => "에이전트 작업이 완료되었습니다",
        (i18n::Language::Ru, "completed") => "Задача агента выполнена",
        (i18n::Language::En, "completed") => "Agent task completed",

        (i18n::Language::ZhHans, "waiting_input") => "子代理等待回答",
        (i18n::Language::ZhHant, "waiting_input") => "子代理等待回答",
        (i18n::Language::De, "waiting_input") => "Agent-Aufgabe wartet auf Antwort",
        (i18n::Language::Es, "waiting_input") => "La tarea del agente espera respuesta",
        (i18n::Language::Fr, "waiting_input") => "La tâche de l’agent attend une réponse",
        (i18n::Language::Ja, "waiting_input") => "エージェントタスクは回答を待っています",
        (i18n::Language::Ko, "waiting_input") => "에이전트 작업이 응답을 기다리고 있습니다",
        (i18n::Language::Ru, "waiting_input") => "Задача агента ожидает ответа",
        (i18n::Language::En, "waiting_input") => "Agent task needs an answer",

        (i18n::Language::ZhHans, "waiting_permission") => "子代理等待授权",
        (i18n::Language::ZhHant, "waiting_permission") => "子代理等待授權",
        (i18n::Language::De, "waiting_permission") => "Agent-Aufgabe wartet auf Berechtigung",
        (i18n::Language::Es, "waiting_permission") => "La tarea del agente espera permiso",
        (i18n::Language::Fr, "waiting_permission") => "La tâche de l’agent attend une autorisation",
        (i18n::Language::Ja, "waiting_permission") => "エージェントタスクは権限を待っています",
        (i18n::Language::Ko, "waiting_permission") => "에이전트 작업이 권한을 기다리고 있습니다",
        (i18n::Language::Ru, "waiting_permission") => "Задача агента ожидает разрешения",
        (i18n::Language::En, "waiting_permission") => "Agent task needs permission",

        (i18n::Language::ZhHans, "sealed") => "子代理执行已封存",
        (i18n::Language::ZhHant, "sealed") => "子代理執行已封存",
        (i18n::Language::De, "sealed") => "Agent-Ausführung beendet",
        (i18n::Language::Es, "sealed") => "Ejecución del agente finalizada",
        (i18n::Language::Fr, "sealed") => "Exécution de l’agent terminée",
        (i18n::Language::Ja, "sealed") => "エージェントの実行が終了しました",
        (i18n::Language::Ko, "sealed") => "에이전트 실행이 완료되었습니다",
        (i18n::Language::Ru, "sealed") => "Выполнение агента завершено",
        (i18n::Language::En, "sealed") => "Agent execution sealed",

        (i18n::Language::ZhHans, "failed") => "子代理失败",
        (i18n::Language::ZhHant, "failed") => "子代理失敗",
        (i18n::Language::De, "failed") => "Agent-Aufgabe fehlgeschlagen",
        (i18n::Language::Es, "failed") => "La tarea del agente falló",
        (i18n::Language::Fr, "failed") => "La tâche de l’agent a échoué",
        (i18n::Language::Ja, "failed") => "エージェントタスクが失敗しました",
        (i18n::Language::Ko, "failed") => "에이전트 작업이 실패했습니다",
        (i18n::Language::Ru, "failed") => "Задача агента завершилась с ошибкой",
        (i18n::Language::En, "failed") => "Agent task failed",

        (i18n::Language::ZhHans, "cancelled") => "子代理已取消",
        (i18n::Language::ZhHant, "cancelled") => "子代理已取消",
        (i18n::Language::De, "cancelled") => "Agent-Aufgabe abgebrochen",
        (i18n::Language::Es, "cancelled") => "Tarea del agente cancelada",
        (i18n::Language::Fr, "cancelled") => "Tâche de l’agent annulée",
        (i18n::Language::Ja, "cancelled") => "エージェントタスクがキャンセルされました",
        (i18n::Language::Ko, "cancelled") => "에이전트 작업이 취소되었습니다",
        (i18n::Language::Ru, "cancelled") => "Задача агента отменена",
        (i18n::Language::En, "cancelled") => "Agent task cancelled",

        (i18n::Language::ZhHans, _) => "子代理状态已更新",
        (i18n::Language::ZhHant, _) => "子代理狀態已更新",
        (i18n::Language::De, _) => "Agent-Aufgabe aktualisiert",
        (i18n::Language::Es, _) => "Tarea del agente actualizada",
        (i18n::Language::Fr, _) => "Tâche de l’agent mise à jour",
        (i18n::Language::Ja, _) => "エージェントタスクが更新されました",
        (i18n::Language::Ko, _) => "에이전트 작업 상태가 업데이트되었습니다",
        (i18n::Language::Ru, _) => "Задача агента обновлена",
        (i18n::Language::En, _) => "Agent task updated",
    }
}

fn print_still_running_subagents_notice(config: &Config, fallback_count: usize) {
    let root = config.project_root.to_string_lossy().to_string();
    let active_ids: Vec<String> = crate::store::NolaStore::open_default()
        .and_then(|store| store.list_runtime_tasks(&root))
        .unwrap_or_default()
        .into_iter()
        .filter(|task| {
            matches!(
                task.status.as_str(),
                "queued" | "running" | "waiting_input" | "waiting_permission"
            ) && (config.session_id.is_none()
                || task.owner_session_id.is_none()
                || task.owner_session_id.as_deref() == config.session_id.as_deref())
        })
        .map(|task| task.id)
        .collect();
    let count = active_ids.len().max(fallback_count);
    let header = match i18n::current_language() {
        i18n::Language::ZhHans => format!("仍有 {count} 个后台子代理在运行；可用 /stop <task-id> 停止："),
        i18n::Language::ZhHant => format!("仍有 {count} 個背景子代理正在執行；可用 /stop <task-id> 停止："),
        i18n::Language::De => format!("{count} Hintergrund-Agent-Aufgabe(n) laufen noch; mit /stop <task-id> beenden:"),
        i18n::Language::Es => format!("Aún hay {count} tarea(s) de subagente en ejecución; usa /stop <task-id> para detenerlas:"),
        i18n::Language::Fr => format!("{count} sous-agent(s) toujours en cours ; utilisez /stop <task-id> pour arrêter :"),
        i18n::Language::Ja => format!("バックグラウンドで {count} 件のサブエージェントが実行中です。/stop <task-id> で停止できます:"),
        i18n::Language::Ko => format!("백그라운드에서 {count}개의 하위 에이전트가 계속 실행 중입니다. /stop <task-id>로 중지하세요:"),
        i18n::Language::Ru => format!("В фоне всё ещё выполняется {count} подзадач(и); остановите командой /stop <task-id>:"),
        i18n::Language::En => format!("{count} background subagent(s) still running; use /stop <task-id> to stop:"),
    };
    terminal_notice!("  {header}");
    for id in active_ids.iter().take(6) {
        terminal_notice!("    /stop {id}");
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
    let completion_line = if COMPOSER_PURPOSE.with(Cell::get) == ComposerPurpose::Goal {
        None
    } else {
        composer_completion_line(line, cursor, config)
    };
    let dynamic_footer = interactive_footer_with_background_status(footer, config);
    draw_composer_frame(
        line,
        cursor,
        dynamic_footer.as_deref(),
        completion_line.as_deref(),
    )
}

const COMPOSER_TRUE_DIM: &str = "\x1b[2;39m";
const COMPOSER_CHEVRON_ACCENT: &str = "\x1b[36m";
const COMPOSER_CHEVRON_PREFIX: &str = "  ❯ ";

fn composer_content_rule_width(columns: usize) -> usize {
    columns.saturating_sub(2).max(1).min(120).saturating_sub(2).max(1)
}

fn composer_hairline(rule_width: usize, mode_pill: Option<&str>, color: bool) -> String {
    let dim = if color { COMPOSER_TRUE_DIM } else { "" };
    let accent = if color { COMPOSER_CHEVRON_ACCENT } else { "" };
    let reset = if color { INTERACTIVE_RESET } else { "" };
    if let Some(pill) = mode_pill {
        let pill_w = display_width(pill);
        if rule_width > pill_w + 4 {
            let tail = "─".repeat(rule_width.saturating_sub(pill_w + 3));
            return format!("  {dim}─ {reset}{accent}{pill}{reset}{dim} {tail}{reset}");
        }
    }
    let rule = "─".repeat(rule_width);
    format!("  {dim}{rule}{reset}")
}

fn framed_composer_input_line(
    text: &str,
    is_placeholder: bool,
    is_first_line: bool,
    prefix: &str,
    columns: usize,
    rule_width: usize,
    color: bool,
) -> String {
    let line_width = columns.saturating_sub(1).max(1);
    let prefix_width = display_width(prefix);
    let visible_line = truncate_display_width_tail(text, line_width.saturating_sub(prefix_width));
    let use_placeholder = is_placeholder && visible_line.is_empty();
    let purpose = COMPOSER_PURPOSE.with(Cell::get);
    let visible = if use_placeholder {
        if purpose == ComposerPurpose::Goal {
            ui_text(UiText::GoalPlaceholder)
        } else {
            tr(Message::InteractiveInputPlaceholder)
        }
    } else {
        visible_line.as_str()
    };

    let dim = if color { COMPOSER_TRUE_DIM } else { "" };
    let accent = if color { COMPOSER_CHEVRON_ACCENT } else { "" };
    let reset = if color { INTERACTIVE_RESET } else { "" };

    let styled_prefix = if is_first_line && prefix == COMPOSER_CHEVRON_PREFIX {
        format!("  {accent}❯{reset} ")
    } else {
        prefix.to_string()
    };

    if use_placeholder {
        let target_right = 2 + rule_width;
        let left_width = prefix_width.saturating_add(display_width(visible));
        let right_hint = if columns >= 50 && purpose == ComposerPurpose::Chat {
            Some("/help · @file")
        } else {
            None
        };
        if let Some(hint) = right_hint {
            let hint_width = display_width(hint);
            if left_width + 2 + hint_width <= target_right {
                let gap = " ".repeat(target_right - left_width - hint_width);
                return format!("{styled_prefix}{dim}{visible}{gap}{hint}{reset}");
            }
        }
        return format!("{styled_prefix}{dim}{visible}{reset}");
    }

    format!("{styled_prefix}{visible}")
}

fn draw_composer_frame(
    line: &str,
    cursor: usize,
    footer: Option<&str>,
    completion_line: Option<&str>,
) -> io::Result<()> {
    interaction::update_interactive_composer(line, cursor);
    let (terminal_columns_u16, terminal_rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let columns = usize::from(terminal_columns_u16).max(1);
    let rule_width = composer_content_rule_width(columns);
    let color = interactive_color_enabled();

    let input_line = composer_input_line(line, cursor);
    let (cursor_row, cursor_col) = composer_cursor_position(line, cursor);

    let mut frame_lines: Vec<String> = Vec::new();
    // Anchor SlashMenu / @ mention popups above the composer's top hairline so
    // the input box never jumps vertically while filtering.
    if let Some(completion_line) = completion_line {
        frame_lines.extend(
            completion_line
                .lines()
                .map(|value| value.trim_end_matches('\r').to_string()),
        );
    }

    let mode_pill = match COMPOSER_PURPOSE.with(Cell::get) {
        ComposerPurpose::Chat => None,
        ComposerPurpose::Goal => Some("goal"),
        ComposerPurpose::Edit => Some("edit"),
    };
    if completion_line.is_none() || mode_pill.is_some() {
        frame_lines.push(composer_hairline(rule_width, mode_pill, color));
    }
    let input_start_row = frame_lines.len();

    let mut input_rows = input_line
        .lines()
        .map(|value| value.trim_end_matches('\r').to_string())
        .collect::<Vec<_>>();
    if input_rows.is_empty() {
        input_rows.push(String::new());
    }
    frame_lines.extend(input_rows);
    frame_lines.push(composer_hairline(rule_width, None, color));

    crate::terminal_ui::set_footer(footer.unwrap_or_default());

    let adjusted_cursor_row = input_start_row.saturating_add(cursor_row);
    let frame_rows = frame_lines.len().min(usize::from(terminal_rows.max(1)));
    let absolute_cursor_column = (cursor_col.saturating_add(1) as u16)
        .min(terminal_columns_u16.max(1));
    crate::terminal_ui::present_idle_frame(
        frame_lines,
        adjusted_cursor_row,
        absolute_cursor_column,
    );
    INTERACTIVE_COMPOSER_CURSOR_ROW.with(|row| row.set(adjusted_cursor_row));
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
    let columns = terminal_columns();
    let line_width = columns.saturating_sub(1).max(1);
    let rule_width = composer_content_rule_width(columns);
    let prefix = if line_width > display_width(COMPOSER_CHEVRON_PREFIX) {
        COMPOSER_CHEVRON_PREFIX
    } else {
        ""
    };
    let input_width = line_width.saturating_sub(display_width(prefix));
    let (visible_lines, _cursor_row, _cursor_col) =
        composer_input_multiline(line, cursor, input_width, prefix);
    let continuation_prefix = " ".repeat(display_width(prefix));
    let color = interactive_color_enabled();
    visible_lines
        .iter()
        .enumerate()
        .map(|(i, (text, is_placeholder))| {
            let row_prefix = if i == 0 {
                prefix
            } else {
                continuation_prefix.as_str()
            };
            framed_composer_input_line(
                text,
                *is_placeholder,
                i == 0,
                row_prefix,
                columns,
                rule_width,
                color,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\r")
}

fn composer_cursor_position(line: &str, cursor: usize) -> (usize, usize) {
    let line_width = terminal_columns().saturating_sub(1).max(1);
    let prefix = if line_width > display_width(COMPOSER_CHEVRON_PREFIX) {
        COMPOSER_CHEVRON_PREFIX
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
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

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
    // 光标在当前行中之前字素簇的累计显示宽度
    let mut cursor_line_width_before = 0usize;

    for (idx, grapheme) in line.grapheme_indices(true) {
        if grapheme == "\n" || grapheme == "\r\n" {
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
        let g_width = UnicodeWidthStr::width(grapheme);
        if current_width > 0 && current_width + g_width > width {
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
            cursor_line_width_before += g_width;
        }
        current.push_str(grapheme);
        current_width += g_width;
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
        if COMPOSER_PURPOSE.with(Cell::get) == ComposerPurpose::Goal { ui_text(UiText::GoalPlaceholder) }
        else { tr(Message::InteractiveInputPlaceholder) }
    } else {
        visible_line.as_str()
    };
    let content_width = display_width(prefix).saturating_add(display_width(visible));
    let padding = " ".repeat(line_width.saturating_sub(content_width));
    let clear_prefix = if clear { "\x1b[2K" } else { "" };
    if color {
        let value_color = if use_placeholder {
            INTERACTIVE_COMPOSER_DIM_FG
        } else {
            INTERACTIVE_COMPOSER_BG
        };
        format!(
            "{clear_prefix}{INTERACTIVE_COMPOSER_PREFIX_FG}{prefix}{value_color}{visible}{INTERACTIVE_COMPOSER_BG}{padding}{INTERACTIVE_RESET}"
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
            INTERACTIVE_COMPOSER_DIM_FG
        } else {
            INTERACTIVE_COMPOSER_BG
        };
        format!(
            "{clear_prefix}{INTERACTIVE_COMPOSER_PREFIX_FG}{prefix}{value_color}{visible}{INTERACTIVE_COMPOSER_BG}{padding}{INTERACTIVE_RESET}"
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

fn composer_completion_line(line: &str, cursor: usize, config: &Config) -> Option<String> {
    let trimmed = line.trim_start();
    let slash = SLASH_MENU.with(|menu| {
        let mut menu = menu.borrow_mut();
        menu.sync(line, cursor, config);
        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
        menu.render(usize::from(width.saturating_sub(1)), usize::from(height))
    });
    if trimmed.starts_with('/') || COMPOSER_PURPOSE.with(Cell::get) == ComposerPurpose::Goal {
        return slash;
    }
    let (prefix, selected, items) = if let Some((_, _, prefix)) = active_at_file_token(line, cursor) {
        let selected = mention_menu_focus(line, cursor)?;
        let items = interactive_mention_candidates(config, prefix)
            .into_iter()
            .map(|(_, label)| label)
            .collect::<Vec<_>>();
        (prefix, selected, items)
    } else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    let count = items.len().min(5);
    let clamped_selected = selected.min(items.len().saturating_sub(1));
    let start = clamped_selected
        .saturating_sub(count / 2)
        .min(items.len().saturating_sub(count));
    let columns = terminal_columns();
    let max_box_width = composer_content_rule_width(columns);
    let color = interactive_color_enabled();
    let dim = if color { COMPOSER_TRUE_DIM } else { "" };
    let accent = if color { COMPOSER_CHEVRON_ACCENT } else { "" };
    let bold = if color { "\x1b[1m" } else { "" };
    let reset = if color { INTERACTIVE_RESET } else { "" };

    if max_box_width < 12 {
        let mut rows = Vec::new();
        for (idx, label) in items.iter().enumerate().skip(start).take(count) {
            let marker = if idx == clamped_selected { "›" } else { " " };
            rows.push(truncate_display_width(
                &format!("  {marker} {label}"),
                columns.saturating_sub(1),
            ));
        }
        return Some(rows.join("\n"));
    }

    let inner_width = max_box_width.saturating_sub(4);
    let mut rows = Vec::with_capacity(count + 2);

    let title = if prefix.is_empty() {
        "@".to_string()
    } else {
        format!("@{prefix}")
    };
    let title = truncate_display_width(&title, inner_width.saturating_sub(2));
    let title_w = display_width(&title);
    let top_fill = "─".repeat((inner_width + 2).saturating_sub(title_w + 3));
    rows.push(format!("  {dim}╭─ {title} {top_fill}╮{reset}"));

    for (idx, label) in items.iter().enumerate().skip(start).take(count) {
        let is_selected = idx == clamped_selected;
        let visible = truncate_display_width(label, inner_width.saturating_sub(2));
        let pad = " ".repeat(inner_width.saturating_sub(2 + display_width(&visible)));
        let body = if is_selected {
            format!("{accent}›{reset} {bold}{visible}{reset}{pad}")
        } else {
            format!("  {visible}{pad}")
        };
        rows.push(format!("  {dim}│{reset} {body} {dim}│{reset}"));
    }

    let hint_full = ui_text(UiText::MentionPickKeys);
    let hint_src = if display_width(hint_full) + 2 <= inner_width {
        hint_full
    } else {
        "↑↓ · Tab · Enter · Esc"
    };
    let hint = truncate_display_width(hint_src, inner_width.saturating_sub(2));
    let hint_w = display_width(&hint);
    let bot_fill = "─".repeat((inner_width + 2).saturating_sub(hint_w + 3));
    rows.push(format!("  {dim}╰─ {hint} {bot_fill}╯{reset}"));

    Some(rows.join("\n"))
}

#[allow(dead_code)]
fn composer_slash_completion_line(line: &str, config: &Config) -> Option<String> {
    composer_completion_line(line, line.len(), config)
}

struct InteractiveRawModeGuard {
    keyboard_enhanced: bool,
}

impl InteractiveRawModeGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode().map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("{} {err}", tr(Message::InteractiveRawModeFailedPrefix)),
            )
        })?;
        let keyboard_enhanced = io::stdout().is_terminal()
            && execute!(
                io::stdout(),
                crossterm::event::PushKeyboardEnhancementFlags(
                    crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                )
            )
            .is_ok();
        Ok(Self { keyboard_enhanced })
    }

    fn enable_bracketed_paste(&self) -> io::Result<()> {
        if !io::stdout().is_terminal() {
            return Ok(());
        }
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
    if io::stdout().is_terminal() {
        execute!(io::stdout(), crossterm::event::EnableBracketedPaste)?;
    }
    Ok(())
}

impl Drop for InteractiveRawModeGuard {
    fn drop(&mut self) {
        if io::stdout().is_terminal() {
            if self.keyboard_enhanced {
                let _ = execute!(io::stdout(), crossterm::event::PopKeyboardEnhancementFlags);
            }
            let _ = execute!(io::stdout(), crossterm::event::DisableBracketedPaste);
        }
        let _ = disable_raw_mode();
    }
}

fn reset_interactive_terminal_state() {
    let _ = disable_raw_mode();
    if io::stdout().is_terminal() {
        let _ = print!(
            "{INTERACTIVE_RESET}\x1b[<u\x1b[?25h\x1b[?2004l\x1b[?1007l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l"
        );
        let _ = io::stdout().flush();
    }
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

fn interactive_running_background_jobs_count(config: &Config) -> usize {
    let bash_running = tools::background_bash_task_summaries(
        &config.allowed_dirs,
        config.session_id.as_deref(),
    )
    .unwrap_or_default()
    .into_iter()
    .filter(|task| {
        matches!(
            task.canonical_state.as_str(),
            "queued" | "running" | "waiting_input" | "waiting_permission"
        )
    })
    .count();
    let runtime_running = config
        .host_invocation_request()
        .map(|req| task_runtime::running_task_count(&req))
        .unwrap_or(0);
    bash_running.max(runtime_running)
}

fn interactive_footer_with_background_status(
    footer: Option<&str>,
    config: &Config,
) -> Option<String> {
    let footer = footer?;
    let running = interactive_running_background_jobs_count(config);
    let width = terminal_columns().saturating_sub(2).max(1);
    Some(format_interactive_background_footer(footer, running, width))
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
    let columns = terminal_columns();
    let width = composer_content_rule_width(columns);
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
        .unwrap_or_else(|| format!("{} 0%", tr(Message::InteractiveContextLabel)));
    let perm = PermissionLevel::from_config(config);
    let perm_label = compact_permission_label(perm.display());
    let executor =
        crate::external_executor::execution_backend_display_name(&config.execution_backend);
    let task_line = config
        .save_session
        .then(|| task_runtime::core_task_footer_summary(session_id))
        .flatten();

    let mut left_parts = vec![truncate_display_width(model, width.min(28))];
    if let Some(effort) = config.runtime.reasoning_effort_display() {
        left_parts.push(effort);
    }
    left_parts.push(perm_label.to_string());
    if executor.trim() != "Nola" {
        left_parts.push(format!(
            "{}: {executor}",
            tr(Message::InteractiveExecutorLabel)
        ));
    }
    if let Some(task_summary) = task_line.as_deref() {
        left_parts.push(truncate_display_width(task_summary, width.min(32)));
    }

    let mut right_parts = Vec::new();
    if let Some(cache) = context_usage.and_then(|usage| usage.cache_usage) {
        let rate = cache
            .hit_rate_bps
            .map(|bps| format!("{:.1}%", f64::from(bps) / 100.0))
            .unwrap_or_else(|| "—".into());
        let scope = if i18n::current_language() == i18n::Language::ZhHans {
            "本次请求"
        } else {
            "Latest request"
        };
        right_parts.push(format!("{scope} {} {rate}", ui_text(UiText::CacheHitRate)));
    }
    right_parts.push(context_info);

    let left = left_parts.join(" · ");
    let right = right_parts.join(" · ");
    let left_w = display_width(&left);
    let right_w = display_width(&right);
    let primary_plain = if left_w + 2 + right_w <= width {
        let gap = " ".repeat(width - left_w - right_w);
        format!("{left}{gap}{right}")
    } else {
        let combined = format!("{left} · {right}");
        if display_width(&combined) <= width {
            combined
        } else {
            // Shrink model first so permission, executor, cache, and context remain readable.
            let mut compact_left = left_parts.clone();
            let non_model_w = left_w.saturating_sub(display_width(&compact_left[0]));
            let model_budget = width
                .saturating_sub(non_model_w + right_w + 3)
                .clamp(8, 28);
            compact_left[0] = truncate_display_width(model, model_budget);
            let compact_left_str = compact_left.join(" · ");
            let compact_w = display_width(&compact_left_str);
            if compact_w + 2 + right_w <= width {
                let gap = " ".repeat(width - compact_w - right_w);
                format!("{compact_left_str}{gap}{right}")
            } else {
                truncate_display_width_middle(&format!("{compact_left_str} · {right}"), width)
            }
        }
    };

    let use_color = interactive_color_enabled();
    let primary = if use_color {
        format!("{COMPOSER_TRUE_DIM}{primary_plain}{INTERACTIVE_RESET}")
    } else {
        primary_plain
    };
    if !config.debug_mode {
        return Some(primary);
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
    let diagnostics_plain = format_interactive_footer_diagnostics(
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
    let diagnostics = if use_color {
        format!("{COMPOSER_TRUE_DIM}{diagnostics_plain}{INTERACTIVE_RESET}")
    } else {
        diagnostics_plain
    };
    Some(format!("{primary}\n{diagnostics}"))
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
    let reasoning_label = interactive_reasoning_label();
    let reasoning_value = config
        .runtime
        .reasoning_effort_display()
        .unwrap_or_else(|| "default".to_string());
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
        reasoning_label.to_string(),
        truncate_display_width(&reasoning_value, max_value_width),
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
            &reasoning_value,
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
                format!("{reasoning_label}: {reasoning_value}"),
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

fn interactive_reasoning_label() -> &'static str {
    if current_language() == Language::ZhHans {
        "推理"
    } else {
        "Reasoning"
    }
}

fn interactive_prompt_compact_line(
    turn: usize,
    model: &str,
    reasoning: &str,
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
        format!("{}: {}", interactive_reasoning_label(), reasoning),
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
    SlashCommand { name: "/thinking" },
    SlashCommand { name: "/executor" },
    SlashCommand { name: "/debug" },
    SlashCommand { name: "/allow" },
    SlashCommand { name: "/sources" },
    SlashCommand { name: "/attach" },
    SlashCommand { name: "/edit" },
    SlashCommand {
        name: "/cancel-edit",
    },
    SlashCommand { name: "/goal" },
    SlashCommand { name: "/init" },
    SlashCommand { name: "/cd" },
    SlashCommand { name: "/compact" },
    SlashCommand { name: "/new" },
    SlashCommand { name: "/clear" },
    SlashCommand { name: "/plan" },
    SlashCommand { name: "/agent" },
    SlashCommand { name: "/room" },
    SlashCommand { name: "/work" },
    SlashCommand { name: "/recover" },
    SlashCommand { name: "/tasks" },
    SlashCommand { name: "/jobs" },
    SlashCommand { name: "/stop" },
    SlashCommand { name: "/questions" },
    SlashCommand { name: "/skills" },
    SlashCommand { name: "/mcp" },
    SlashCommand { name: "/memory" },
    SlashCommand { name: "/guide" },
    SlashCommand { name: "/config" },
    SlashCommand { name: "/resume" },
    SlashCommand { name: "/continue" },
    SlashCommand { name: "/history" },
    SlashCommand { name: "/status" },
    SlashCommand { name: "/exit" },
];

fn normalize_interactive_command(command: &str) -> Option<&'static str> {
    if !command.starts_with('/') {
        return None;
    }
    let lower = command.to_ascii_lowercase();
    Some(match lower.as_str() {
        "/help" | "/h" => "/help",
        "/model" | "/models" | "/m" => "/model",
        "/thinking" => "/thinking",
        "/executor" | "/exec" => "/executor",
        "/debug" | "/d" => "/debug",
        "/allow" | "/a" => "/allow",
        "/sources" | "/data" => "/sources",
        "/attach" => "/attach",
        "/edit" => "/edit",
        "/cancel-edit" => "/cancel-edit",
        "/goal" | "/g" => "/goal",
        "/init" | "/i" => "/init",
        "/cd" => "/cd",
        "/compact" => "/compact",
        "/new" | "/n" => "/new",
        "/clear" | "/cls" => "/clear",
        "/plan" | "/p" => "/plan",
        "/agent" => "/agent",
        "/room" => "/room",
        "/work" | "/w" => "/work",
        "/recover" => "/recover",
        "/tasks" | "/t" => "/tasks",
        "/jobs" | "/j" => "/jobs",
        "/stop" => "/stop",
        "/questions" => "/questions",
        "/skills" | "/skill" => "/skills",
        "/mcp" => "/mcp",
        "/memory" | "/mem" => "/memory",
        "/guide" => "/guide",
        "/config" => "/config",
        "/resume" | "/r" => "/resume",
        "/continue" | "/c" => "/continue",
        "/history" | "/hist" => "/history",
        "/status" | "/s" | "/st" => "/status",
        "/exit" | "/q" | "/e" => "/exit",
        _ => return None,
    })
}

fn resume_command_target(prompt: &str) -> Option<Option<String>> {
    let mut parts = prompt.trim().split_whitespace();
    let command = parts.next()?;
    if normalize_interactive_command(command) != Some("/resume") {
        return None;
    }
    let first = parts.next().map(str::to_string);
    if parts.next().is_some() {
        return None;
    }
    Some(first)
}

#[allow(dead_code)]
fn is_resume_command(prompt: &str) -> bool {
    resume_command_target(prompt).is_some()
}

fn resume_interactive_session_by_id(config: &Config, session_id: &str) -> Result<Config, String> {
    let saved = session::load_session(session_id)?;
    let mut next = config.clone();
    next.session_id = Some(saved.id);
    next.continue_last = false;
    next.resume_session = false;
    crate::conversation_invocation::initialize_or_restore(&mut next)?;
    Ok(next)
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
        terminal_notice!("{}", interactive_layer_rule(Some(crate::i18n::cli_section_label("init"))));
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

fn apply_workspace_runtime(config: &mut Config, mut new_runtime: runtime_config::RuntimeConfig) {
    new_runtime.endpoint = config.runtime.endpoint.clone();
    new_runtime.api_key = config.runtime.api_key.clone();
    new_runtime.model = config.runtime.model.clone();
    new_runtime.provider = config.runtime.provider.clone();
    new_runtime.siwc = config.runtime.siwc.clone();
    new_runtime.selected_profile = config.runtime.selected_profile.clone();
    new_runtime.model_binding = config.runtime.model_binding.clone();
    new_runtime.native_model_route = config.runtime.native_model_route.clone();
    new_runtime.model_capabilities = config.runtime.model_capabilities.clone();
    new_runtime.llm_fallbacks = config.runtime.llm_fallbacks.clone();
    new_runtime.reasoning_selection = config.runtime.reasoning_selection.clone();
    new_runtime.reasoning_tier = config.runtime.reasoning_tier.clone();
    new_runtime.chat_output_token_field = config.runtime.chat_output_token_field.clone();
    new_runtime.generation_options = config.runtime.generation_options.clone();
    new_runtime.temperature = config.runtime.temperature;
    new_runtime.top_p = config.runtime.top_p;
    new_runtime.max_output_tokens = config.runtime.max_output_tokens;
    new_runtime.compaction.context_window_tokens = config.runtime.compaction.context_window_tokens;
    config.runtime = new_runtime;
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

include!("composer_completion.rs");

fn available_interactive_models(config: Option<&Config>, query: &str) -> Vec<String> {
    interactive_model_candidates(config, query).into_iter().map(|item| item.name).collect()
}

fn interactive_model_candidates(config: Option<&Config>, query: &str) -> Vec<SlashCompletion> {
    let mut models = Vec::new();
    if config.is_some() {
        if let Ok(profiles) =
            crate::store::NolaStore::open_default().and_then(|store| store.model_profiles())
        {
            for profile in profiles
                .into_iter()
                .filter(|p| p.enabled && p.deleted_at_ms.is_none() && p.runtime == "remote")
            {
                let key = format!("configured:{}", profile.id);
                let source: serde_json::Value = serde_json::from_str(&profile.source_json).unwrap_or_default();
                let connection = source["connectionID"].as_str().filter(|value| !value.trim().is_empty())
                    .unwrap_or(&profile.provider);
                let mut label = format!("{} · {}", profile.model_name, connection);
                if connection != profile.provider && !profile.provider.is_empty() {
                    label.push_str(" · ");
                    label.push_str(&profile.provider);
                }
                models.push(SlashCompletion { name: key, label: Some(label) });
            }
        }
    }
    if let Some(config) = config {
        for item in [
            &config.model,
            config.runtime.model.as_ref().unwrap_or(&config.model),
        ] {
            let item = item.trim();
            if !item.is_empty() && !models.iter().any(|model| model.name == item) {
                models.push(SlashCompletion::plain(item));
            }
        }
    }
    if let Some(previous_model) = interactive_previous_provider_model() {
        if !models.iter().any(|model| model.name == previous_model) {
            models.push(SlashCompletion::plain(previous_model));
        }
    }
    for model in local_models::model_ids() {
        if !models.iter().any(|candidate| candidate.name == model.as_str()) {
            models.push(SlashCompletion::plain(model));
        }
    }
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return models;
    }
    models
        .into_iter()
        .filter(|model| {
            model.name.to_ascii_lowercase().contains(&query)
                || model.label.as_ref()
                    .is_some_and(|label| label.to_ascii_lowercase().contains(&query))
                || local_models::profile(&model.name)
                    .is_some_and(|p| p.name.to_ascii_lowercase().contains(&query))
        })
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
    ["status", "resume", "pause", "off", "start"]
        .into_iter()
        .filter(|subcommand| query.is_empty() || subcommand.starts_with(&query))
        .map(|subcommand| SlashCompletion::plain(format!("/goal {subcommand}")))
        .collect()
}

enum SlashCommandResult {
    Handled,
    PassThrough,
    RunGoalPrompt(String),
    Draft(String),
    GoalEntry,
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
        (_, ResumeRecoveryText::ObservedCommitted) => {
            "Committed recovered tool results:"
        }
        (_, ResumeRecoveryText::UncertainBlocked) => {
            "Automatic resume stopped because these operations have an uncertain outcome:"
        }
        (_, ResumeRecoveryText::InspectWork) => {
            "Use /work and confirm the real external state before resolving them."
        }
        (_, ResumeRecoveryText::PermissionUnavailable) => {
            "Could not restore pending permission:"
        }
        (_, ResumeRecoveryText::RetryScheduled) => {
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
            print_interactive_help(trimmed.split_once(char::is_whitespace).map_or("", |(_, query)| query));
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
        "/thinking" => {
            let level = parts.next();
            if parts.next().is_some() {
                terminal_notice!("  Usage: /thinking <off|low|medium|high|xhigh|max|default>");
                return Ok(SlashCommandResult::Handled);
            }
            let summary = handle_interactive_thinking_command(config, level)?;
            for line in summary.lines() {
                terminal_notice!("{line}");
            }
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
                    let status = crate::i18n::cli_executor_status_label(match backend.availability {
                        nola_domain::execution_backend::ExecutionBackendAvailability::Available => "available",
                        nola_domain::execution_backend::ExecutionBackendAvailability::NotInstalled => "not installed",
                        nola_domain::execution_backend::ExecutionBackendAvailability::Unauthenticated => "not authenticated",
                        nola_domain::execution_backend::ExecutionBackendAvailability::Incompatible => "incompatible",
                        nola_domain::execution_backend::ExecutionBackendAvailability::Unreachable => "unreachable",
                    });
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
            let previous_profile = config.permission_profile;
            let previous_auto_approve = config.auto_approve;
            match interactive_permission_panel(config) {
                Ok(()) => {
                    if (config.permission_profile != previous_profile
                        || config.auto_approve != previous_auto_approve)
                        && config.save_session
                        && config.session_id.is_some()
                    {
                        if let Err(e) = crate::conversation_invocation::persist(config) {
                            terminal_notice!("  {} {e}", tr(Message::InteractiveAllowPanelErrorPrefix));
                        }
                    }
                }
                Err(e) => {
                    terminal_notice!("  {} {e}", tr(Message::InteractiveAllowPanelErrorPrefix));
                }
            }
            return Ok(SlashCommandResult::Handled);
        }
        "/sources" => {
            handle_data_source_command(trimmed, config)?;
            return Ok(SlashCommandResult::Handled);
        }
        "/attach" => {
            let raw_path = trimmed
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest.trim())
                .unwrap_or("");
            if raw_path.is_empty() {
                let usage = match i18n::current_language() {
                    i18n::Language::ZhHans => "用法：/attach <文件路径> [文件路径 ...]",
                    i18n::Language::ZhHant => "用法：/attach <檔案路徑> [檔案路徑 ...]",
                    i18n::Language::De => "Verwendung: /attach <Dateipfad> [Dateipfad ...]",
                    i18n::Language::Es => "Uso: /attach <ruta> [ruta ...]",
                    i18n::Language::Fr => "Utilisation : /attach <chemin> [chemin ...]",
                    i18n::Language::Ja => "使い方: /attach <ファイルパス> [ファイルパス ...]",
                    i18n::Language::Ko => "사용법: /attach <파일 경로> [파일 경로 ...]",
                    i18n::Language::Ru => "Использование: /attach <путь> [путь ...]",
                    i18n::Language::En => "Usage: /attach <path> [path ...]",
                };
                terminal_notice!("  {usage}");
                return Ok(SlashCommandResult::Handled);
            }
            match crate::composer_media::attach_path(raw_path, &config.project_root) {
                Ok(tokens) => {
                    let attached_label = match i18n::current_language() {
                        i18n::Language::ZhHans => "已附加",
                        i18n::Language::ZhHant => "已附加",
                        i18n::Language::De => "Angehängt",
                        i18n::Language::Es => "Adjuntado",
                        i18n::Language::Fr => "Pièce jointe ajoutée",
                        i18n::Language::Ja => "添付しました",
                        i18n::Language::Ko => "첨부됨",
                        i18n::Language::Ru => "Прикреплено",
                        i18n::Language::En => "Attached",
                    };
                    terminal_notice!("  {attached_label}: {tokens}");
                    return Ok(SlashCommandResult::Draft(format!("{tokens} ")));
                }
                Err(error) => {
                    terminal_notice!("{}", crate::composer_media::paste_path_error(&error));
                    return Ok(SlashCommandResult::Handled);
                }
            }
        }
        "/goal" => {
            let target_raw = trimmed.split_once(char::is_whitespace).map(|(_, rest)| rest.trim()).unwrap_or("");
            let action = target_raw.to_ascii_lowercase();
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
                    if goal.total_requirements > 0 {
                        let req_label = match i18n::current_language() {
                            i18n::Language::ZhHans => "需求进度",
                            i18n::Language::ZhHant => "需求進度",
                            i18n::Language::De => "Anforderungen",
                            i18n::Language::Es => "Requisitos",
                            i18n::Language::Fr => "Exigences",
                            i18n::Language::Ja => "要件進捗",
                            i18n::Language::Ko => "요구 사항",
                            i18n::Language::Ru => "Требования",
                            i18n::Language::En => "Requirements",
                        };
                        terminal_notice!(
                            "  {req_label}: {}/{}",
                            goal.completed_requirements,
                            goal.total_requirements
                        );
                    }
                    if goal.attempt_count > 0 {
                        let attempts_label = match i18n::current_language() {
                            i18n::Language::ZhHans => "执行轮次",
                            i18n::Language::ZhHant => "執行輪次",
                            i18n::Language::De => "Versuche",
                            i18n::Language::Es => "Intentos",
                            i18n::Language::Fr => "Tentatives",
                            i18n::Language::Ja => "試行回数",
                            i18n::Language::Ko => "시도 횟수",
                            i18n::Language::Ru => "Попытки",
                            i18n::Language::En => "Attempts",
                        };
                        terminal_notice!("  {attempts_label}: {}", goal.attempt_count);
                    }
                    if let Some(reason) = goal.stop_reason.as_deref() {
                        let reason_label = match i18n::current_language() {
                            i18n::Language::ZhHans => "暂停/停止原因",
                            i18n::Language::ZhHant => "暫停/停止原因",
                            i18n::Language::De => "Grund",
                            i18n::Language::Es => "Motivo",
                            i18n::Language::Fr => "Raison",
                            i18n::Language::Ja => "理由",
                            i18n::Language::Ko => "사유",
                            i18n::Language::Ru => "Причина",
                            i18n::Language::En => "Reason",
                        };
                        terminal_notice!("  {reason_label}: {reason}");
                    }
                    if let Some(wake) = goal.next_wake_at_epoch_ms {
                        let wake_label = match i18n::current_language() {
                            i18n::Language::ZhHans => "计划恢复",
                            i18n::Language::ZhHant => "排程恢復",
                            i18n::Language::De => "Geplante Fortsetzung",
                            i18n::Language::Es => "Reanudación programada",
                            i18n::Language::Fr => "Reprise prévue",
                            i18n::Language::Ja => "次回再開予定",
                            i18n::Language::Ko => "예정된 재개",
                            i18n::Language::Ru => "Запланированное возобновление",
                            i18n::Language::En => "Next resume",
                        };
                        terminal_notice!("  {wake_label}: {}", format_goal_wake_time(wake));
                    }
                } else {
                    if target_raw.is_empty() {
                        return Ok(SlashCommandResult::GoalEntry);
                    }
                    terminal_notice!("  {} {}", tr(Message::InteractiveGoalStatusPrefix), tr(Message::InteractiveStatusOff));
                }
                terminal_notice!("  {}", tr(Message::InteractiveSlashGoalUsage));
                return Ok(SlashCommandResult::Handled);
            }

            if action == "resume" {
                let project_root = config.project_root.to_string_lossy().to_string();
                task_runtime::reconcile_for_resume_snapshot(&config.host_invocation_request()?);
                let goal = if let Some(snapshot) =
                    if config.session_id.is_none() {
                        session::load_resume_snapshot_for_project(&project_root, goal_now_epoch_ms())?
                    } else { None }
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
                    if !submit_interactive_goal_control(config, nola_application::goal_control_intent::GoalControlAction::Resume)? {
                        return Ok(SlashCommandResult::Handled);
                    }
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

            if matches!(action.as_str(), "pause" | "off" | "cancel") {
                let no_active_goal = match i18n::current_language() {
                    i18n::Language::ZhHans => "当前会话没有进行中的目标。",
                    i18n::Language::ZhHant => "目前工作階段沒有進行中的目標。",
                    i18n::Language::De => "In dieser Sitzung ist kein aktives Ziel vorhanden.",
                    i18n::Language::Es => "No hay ningún objetivo activo en esta sesión.",
                    i18n::Language::Fr => "Aucun objectif actif dans cette session.",
                    i18n::Language::Ja => "このセッションに実行中の目標はありません。",
                    i18n::Language::Ko => "이 세션에 활성 목표가 없습니다.",
                    i18n::Language::Ru => "В этом сеансе нет активной цели.",
                    i18n::Language::En => "No active goal in this session.",
                };
                if let Some(session_id) = config.session_id.as_deref() {
                    if session::load_session_goal_projection(session_id)?.and_then(|p| p.active)
                        .is_some_and(|goal| !goal.status.is_terminal()) {
                        if !submit_interactive_goal_control(config, if action == "pause" {
                            nola_application::goal_control_intent::GoalControlAction::Pause
                        } else { nola_application::goal_control_intent::GoalControlAction::Cancel })? {
                            return Ok(SlashCommandResult::Handled);
                        }
                    } else {
                        *goal_target = None;
                        *goal_constraints = None;
                        terminal_notice!("  {no_active_goal}");
                        return Ok(SlashCommandResult::Handled);
                    }
                } else {
                    *goal_target = None;
                    *goal_constraints = None;
                    terminal_notice!("  {no_active_goal}");
                    return Ok(SlashCommandResult::Handled);
                }

                *goal_target = None;
                *goal_constraints = None;
                terminal_notice!("  {} {}", tr(Message::InteractiveGoalStatePrefix),
                    if action == "pause" { interactive_goal_state_text("paused") }
                    else { tr(Message::InteractiveStatusOff).to_string() });
                return Ok(SlashCommandResult::Handled);
            }

            let target_input = if action == "start" { "" } else {
                target_raw.split_once(char::is_whitespace)
                    .filter(|(head, _)| head.eq_ignore_ascii_case("start"))
                    .map(|(_, rest)| rest).unwrap_or(target_raw)
            };
            if target_input.trim().is_empty() {
                return Ok(SlashCommandResult::GoalEntry);
            }
            let (target, constraints) = match parse_goal_target_and_constraints(target_input) {
                Ok(input) => input,
                Err(error) => {
                    terminal_notice!("{}", format_error(&error));
                    return Ok(SlashCommandResult::Draft(prompt.to_string()));
                }
            };
            if interactive_goal_snapshot(config, goal_target.as_deref(), goal_constraints.as_deref())?.is_some() {
                terminal_notice!("  {}", tr(Message::InteractiveGoalAlreadyActive));
                return Ok(SlashCommandResult::Draft(prompt.to_string()));
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
        "/config" => {
            if trimmed.split_whitespace().count() > 1 {
                terminal_notice!("{}", crate::config_files::usage());
                return Ok(SlashCommandResult::Handled);
            }
            match crate::config_files::overview(&config.project_root, false) {
                Ok(output) => terminal_notice!("{}", output),
                Err(error) => terminal_notice!("{}", format_error(&error)),
            }
            Ok(SlashCommandResult::Handled)
        }
        "/guide" => {
            terminal_notice!("  {} {}", tr(Message::InteractiveModelLabel), config.model);
            terminal_notice!("  {} {}", tr(Message::InteractiveWorkspaceLabel), config.project_root.display());
            print_interactive_shortcuts_guide();
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
            if let Some(session_id) = config.session_id.as_deref() {
                if session::load_session_goal_projection(session_id)?
                    .and_then(|projection| projection.active)
                    .is_some_and(|goal| goal.status == GoalStatus::Running)
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
            apply_workspace_runtime(config, runtime);
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
        "/new" | "/clear" => {
            if command == "/clear" {
                crate::terminal_ui::clear_replayed_history();
                crate::terminal_ui::invalidate_presented_screen();
            }
            if let Some(session_id) = config.session_id.as_deref() {
                if session::load_session_goal_projection(session_id)?
                    .and_then(|projection| projection.active)
                    .is_some_and(|goal| goal.status == GoalStatus::Running)
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
                            _ => "Delegated; view child task progress",
                        };
                        if let Some((_, description)) = task_runtime::available_agent_profiles(&config.host_invocation_request()?)
                            .into_iter().find(|(name, _)| name == agent_type) {
                            terminal_notice!("  {agent_type} · {description}");
                        }
                        terminal_notice!("  {label}\n  /tasks {task_id}");
                    } else {
                        let summary = format_non_delegated_agent_outcome(&outcome);
                        terminal_notice!("  {summary}");
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
                    if control == "rerun" {
                        if item.is_sop() {
                            let rerun = work::rerun_sop(config, work_id)?;
                            format!(
                                "  {} · `{}`",
                                ui_text(UiText::WorkRerunStarted),
                                rerun.record.run_id
                            )
                        } else {
                            format!("  {}", ui_text(UiText::WorkControlUnsupported))
                        }
                    } else if item.is_goal() {
                        let session_id = config
                            .session_id
                            .as_deref()
                            .ok_or_else(|| "Goal control requires an active session".to_string())?;
                        let Some(command) =
                            goal_work_control_command(control, goal_now_epoch_ms())
                        else {
                            emit_work(format!("  {}", ui_text(UiText::WorkControlUnsupported)));
                            return Ok(SlashCommandResult::Handled);
                        };
                        if control == "resume"
                            && session::load_session_goal_projection(session_id)?
                                .and_then(|projection| projection.active)
                                .is_some_and(|goal| goal.status != GoalStatus::Running)
                        {
                            crate::core_agent_runtime::rebind_goal_execution(
                                &config.host_invocation_request()?,
                            )?;
                        }
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
            let body = render_interactive_jobs_view(config, action, task_id);
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
        "/stop" => {
            let task_id = parts.next();
            if let Some(task_id) = task_id {
                let outcome = tools::manage_background_bash_tasks(
                    "stop",
                    Some(task_id),
                    &config.allowed_dirs,
                    config.session_id.as_deref(),
                    &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
                        config.skill_dirs.clone(),
                    ),
                );
                if !outcome.error {
                    terminal_notice!("  {}", outcome.display);
                    return Ok(SlashCommandResult::Handled);
                }
                let runtime_outcome = task_runtime::manage_runtime_tasks(
                    &config.host_invocation_request()?,
                    "stop",
                    Some(task_id),
                );
                if !runtime_outcome.error {
                    terminal_notice!("  {}", runtime_outcome.display);
                    return Ok(SlashCommandResult::Handled);
                }
                if let Some(session_id) = config.session_id.as_deref() {
                    if let Some(goal) = session::load_session_goal_projection(session_id)?
                        .and_then(|projection| projection.active)
                        .filter(|goal| {
                            !goal.status.is_terminal()
                                && (goal.id == task_id || task_id.eq_ignore_ascii_case("goal"))
                        })
                    {
                        if submit_interactive_goal_control(
                            config,
                            nola_application::goal_control_intent::GoalControlAction::Cancel,
                        )? {
                            *goal_target = None;
                            *goal_constraints = None;
                            terminal_notice!(
                                "  {} · `{}`",
                                ui_text(UiText::WorkStopped),
                                goal.id
                            );
                        }
                        return Ok(SlashCommandResult::Handled);
                    }
                }
                terminal_notice!("  {}", format_error(&runtime_outcome.display));
            } else {
                terminal_notice!(
                    "  {}",
                    match current_language() {
                        Language::ZhHans => "当前没有正在运行的前台回合；可用 /stop <task-id> 或 /jobs stop <id> 停止后台任务。",
                        _ => "No active foreground turn; use /stop <task-id> or /jobs stop <id> to stop a background task.",
                    }
                );
            }
            Ok(SlashCommandResult::Handled)
        }
        "/skills" => {
            let action = parts.next().unwrap_or("list");
            let builtins = crate::skill_catalog_port::load_web_builtin_skills()?
                .into_iter()
                .map(|mut item| {
                    if let Some(obj) = item.as_object_mut() {
                        obj.entry("built_in").or_insert(json!(true));
                    }
                    item
                })
                .collect::<Vec<_>>();
            let uploaded = crate::skill_catalog_port::load_web_uploaded_skills()?;
            let mut baseline = Vec::with_capacity(builtins.len() + uploaded.len());
            baseline.extend(builtins);
            baseline.extend(uploaded);

            if action.eq_ignore_ascii_case("list") {
                terminal_notice!("{}", interactive_layer_rule(Some(crate::i18n::cli_section_label("skills"))));
                if baseline.is_empty() {
                    terminal_notice!("  {}", ui_text(UiText::NoAvailableSkills));
                } else {
                    for skill in &baseline {
                        let id = skill["id"].as_str().unwrap_or("");
                        let name = skill["name"].as_str().unwrap_or(id);
                        let summary = skill["description"].as_str().unwrap_or("");
                        let enabled = skill["enabled"].as_bool().unwrap_or(true);
                        let badge = if enabled { "on " } else { "off" };
                        terminal_notice!("  [{badge}] {id} · {name} — {}", preview(summary, 60));
                    }
                }
            } else if matches!(action.to_ascii_lowercase().as_str(), "enable" | "disable") {
                let Some(target) = parts.next() else {
                    terminal_notice!("  Usage: /skills [list|enable <id>|disable <id>]");
                    return Ok(SlashCommandResult::Handled);
                };
                let desired = action.eq_ignore_ascii_case("enable");
                let mut updated = baseline.clone();
                let Some(entry) = updated.iter_mut().find(|item| {
                    item["id"].as_str() == Some(target)
                        || item["name"]
                            .as_str()
                            .is_some_and(|n| n.eq_ignore_ascii_case(target))
                }) else {
                    return Err(format!("Skill not found: {target}"));
                };
                let id = entry["id"].as_str().ok_or("Missing Skill ID")?.to_owned();
                entry["enabled"] = json!(desired);
                crate::skill_catalog_port::save_web_uploaded_skills(&baseline, &updated)?;
                crate::builtin_skills::sync_registered_custom(
                    &mut crate::application_port::CliApplicationPort::persistent_default()?, Some(&id),
                )?;
                terminal_notice!(
                    "  {} -> {}",
                    target,
                    if desired { "enabled" } else { "disabled" }
                );
            } else {
                terminal_notice!("  Usage: /skills [list|enable <id>|disable <id>]");
            }
            Ok(SlashCommandResult::Handled)
        }
        "/mcp" => {
            let action = parts.next().unwrap_or("list");
            let baseline = crate::mcp_catalog_port::load_web_mcp_servers()?;
            if action.eq_ignore_ascii_case("list") {
                terminal_notice!("{}", interactive_layer_rule(Some(crate::i18n::cli_section_label("mcp"))));
                if baseline.is_empty() {
                    terminal_notice!("  {}", ui_text(UiText::NoConfiguredMcp));
                } else {
                    for server in &baseline {
                        let id = server["server_id"].as_str().unwrap_or("");
                        let name = server["name"].as_str().unwrap_or(id);
                        let enabled = server["enabled"].as_bool().unwrap_or(true);
                        let badge = if enabled { "on " } else { "off" };
                        terminal_notice!("  [{badge}] {id} · {name}");
                    }
                }
            } else if matches!(action.to_ascii_lowercase().as_str(), "enable" | "disable") {
                let Some(target) = parts.next() else {
                    terminal_notice!("  Usage: /mcp [list|enable <id>|disable <id>]");
                    return Ok(SlashCommandResult::Handled);
                };
                let desired = action.eq_ignore_ascii_case("enable");
                let mut updated = baseline.clone();
                let Some(entry) = updated.iter_mut().find(|item| {
                    item["server_id"].as_str() == Some(target)
                        || item["name"]
                            .as_str()
                            .is_some_and(|n| n.eq_ignore_ascii_case(target))
                }) else {
                    return Err(format!("MCP server not found: {target}"));
                };
                entry["enabled"] = json!(desired);
                crate::mcp_catalog_port::save_web_mcp_changes(&baseline, &updated)?;
                config.mcp_server_registry = crate::mcp_catalog_port::load_web_mcp_servers()?;
                terminal_notice!(
                    "  {} -> {}",
                    target,
                    if desired { "enabled" } else { "disabled" }
                );
            } else {
                terminal_notice!("  Usage: /mcp [list|enable <id>|disable <id>]");
            }
            Ok(SlashCommandResult::Handled)
        }
        "/memory" => {
            let action = parts.next().unwrap_or("list");
            let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
            let binding = crate::memory_admin::workspace_memory_binding(&config.host_invocation_request()?)?;
            if action.eq_ignore_ascii_case("list") {
                let records = crate::memory_admin::visible_memories(&mut app, &binding, None, 30)?;
                terminal_notice!("{}", interactive_layer_rule(Some(ui_text(UiText::MemoryHeading))));
                if records.is_empty() {
                    terminal_notice!("  {}", ui_text(UiText::NoSavedMemory));
                } else {
                    for item in records {
                        let id = item["id"].as_str().unwrap_or("");
                        let scope = item["scope"].as_str().unwrap_or("");
                        let content = item["content"].as_str().unwrap_or("");
                        terminal_notice!("  [{scope}] {id} · {}", preview(content, 72));
                    }
                }
            } else if action.eq_ignore_ascii_case("search") {
                let query = parts.collect::<Vec<_>>().join(" ");
                if query.trim().is_empty() {
                    terminal_notice!("  {}  /memory search <query>", ui_text(UiText::DataSourcesUsage));
                    return Ok(SlashCommandResult::Handled);
                }
                let records = crate::memory_admin::visible_memories(&mut app, &binding, Some(query.trim()), 20)?;
                terminal_notice!("{}", interactive_layer_rule(Some(ui_text(UiText::MemorySearchHeading))));
                if records.is_empty() {
                    terminal_notice!("  {}", ui_text(UiText::NoMatchingMemory));
                }
                for item in &records {
                    let id = item["id"].as_str().unwrap_or("");
                    let content = item["content"].as_str().unwrap_or("");
                    terminal_notice!("  {id} · {}", preview(content, 72));
                }
            } else if action.eq_ignore_ascii_case("add") {
                let text = parts.collect::<Vec<_>>().join(" ");
                if text.trim().is_empty() {
                    terminal_notice!("  {}  /memory add <text>", ui_text(UiText::DataSourcesUsage));
                    return Ok(SlashCommandResult::Handled);
                }
                let draft: crate::memory_admin::MemoryDraft = serde_json::from_value(json!({
                    "scope": "workspace",
                    "kind": "other",
                    "content": text.trim(),
                }))
                .map_err(|e| e.to_string())?;
                let saved = crate::memory_admin::add_workspace_memory(&mut app, &binding, &draft)?;
                let id = saved["id"].as_str().unwrap_or("");
                terminal_notice!("  {} {id}", ui_text(UiText::MemorySaved));
            } else {
                terminal_notice!("  {}  /memory [list|search <query>|add <text>]", ui_text(UiText::DataSourcesUsage));
            }
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

fn print_interactive_shortcuts_guide() {
    terminal_notice!("{}", interactive_layer_rule(Some(crate::i18n::cli_section_label("guide"))));
    match current_language() {
        Language::ZhHans => {
            terminal_notice!("  换行：Shift+Enter · Alt+Enter · Ctrl+J · 行尾 \\+Enter");
            terminal_notice!("  跳转：Option/Alt+←/→ 或 Alt+B/F 按词跳转 · Ctrl+A/E 行首/行尾 · Alt+Home/End 全文首尾");
            terminal_notice!("  滚动：PgUp/PgDn · Alt+↑/↓ · Ctrl+↑/↓ · Ctrl+Home/End");
            terminal_notice!("  选专家：输入 @，上下键选择，回车插入名字。/edit 编辑已发送消息。");
            terminal_notice!("  补全：↑↓ 浏览 /命令，Tab 补全，Enter 执行，Esc 收起；在 @ 候选中 Enter 插入");
            terminal_notice!("  草稿：连按两次 Esc 清空输入 · /stop 停止当前运行 · /resume <id> 直接切换会话");
            terminal_notice!("  扩展：/model · /thinking · /experts · /skills · /mcp · /memory · /questions · /plan");
        }
        _ => {
            terminal_notice!("  Newline: Shift+Enter · Alt+Enter · Ctrl+J · trailing \\+Enter");
            terminal_notice!("  Word nav: Option/Alt+Left/Right or Alt+B/F · Ctrl+A/E line · Alt+Home/End buffer");
            terminal_notice!("  Scroll: PgUp/PgDn · Alt+Up/Down · Ctrl+Up/Down · Ctrl+Home/End");
            terminal_notice!("  Experts: type @, choose with Up/Down, press Enter to insert. /edit reopens a sent message.");
            terminal_notice!("  Complete: Up/Down browses /commands, Tab completes, Enter runs, Esc closes. In @ suggestions, Enter inserts.");
            terminal_notice!("  Control: Esc x2 clears draft · /stop cancels active run · /resume <id> switches session");
            terminal_notice!("  Catalog: /model · /thinking · /experts · /skills · /mcp · /memory · /questions · /plan");
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
    let resumed = with_cli_execution_control(config, cancellation.clone(), emitter, |control, emitter|
        resume_cli_core_pending(config, emitter, control))?;
    match resumed.map(|outcome| finish_cli_foreground(config, cancellation, emitter, outcome)).transpose()? {
        Some(outcome) if outcome.turn_status == nola_core::agent::AgentTurnStatus::AwaitingInput => Ok(true),
        Some(outcome) if !outcome.is_terminal() => {
            if !config.json_output {
                terminal_notice!("{}", match current_language() {
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

fn format_non_delegated_agent_outcome(
    outcome: &crate::core_agent_runtime::CoreRunOutcome,
) -> String {
    use nola_core::agent::AgentTurnStatus;
    use nola_domain::run_result::RunResultValue;
    let lang = i18n::current_language();
    match outcome.run_result.as_ref() {
        Some(RunResultValue::Failed { error_ref, .. }) => {
            let prefix = match lang {
                i18n::Language::ZhHans => "子代理启动失败",
                i18n::Language::ZhHant => "子代理啟動失敗",
                i18n::Language::De => "Agent-Delegation fehlgeschlagen",
                i18n::Language::Es => "Error al iniciar el subagente",
                i18n::Language::Fr => "Échec du lancement du sous-agent",
                i18n::Language::Ja => "サブエージェントの起動に失敗しました",
                i18n::Language::Ko => "하위 에이전트 시작 실패",
                i18n::Language::Ru => "Не удалось запустить подзадачу агента",
                i18n::Language::En => "Agent delegation failed",
            };
            if error_ref.trim().is_empty() {
                prefix.to_string()
            } else {
                format!("{prefix}: {error_ref}")
            }
        }
        Some(RunResultValue::Blocked {
            resolution_condition,
            ..
        }) => {
            let prefix = match lang {
                i18n::Language::ZhHans => "子代理被阻塞",
                i18n::Language::ZhHant => "子代理被阻擋",
                i18n::Language::De => "Agent-Delegation blockiert",
                i18n::Language::Es => "Delegación del agente bloqueada",
                i18n::Language::Fr => "Délégation bloquée",
                i18n::Language::Ja => "サブエージェントがブロックされました",
                i18n::Language::Ko => "하위 에이전트 위임이 차단되었습니다",
                i18n::Language::Ru => "Делегирование заблокировано",
                i18n::Language::En => "Agent delegation blocked",
            };
            if resolution_condition.trim().is_empty() {
                prefix.to_string()
            } else {
                format!("{prefix}: {resolution_condition}")
            }
        }
        Some(RunResultValue::Cancelled { .. }) => match lang {
            i18n::Language::ZhHans => "子代理委派已取消。".to_string(),
            i18n::Language::ZhHant => "子代理委派已取消。".to_string(),
            i18n::Language::De => "Agent-Delegation abgebrochen.".to_string(),
            i18n::Language::Es => "Delegación del agente cancelada.".to_string(),
            i18n::Language::Fr => "Délégation de l’agent annulée.".to_string(),
            i18n::Language::Ja => "サブエージェントの委任がキャンセルされました。".to_string(),
            i18n::Language::Ko => "하위 에이전트 위임이 취소되었습니다.".to_string(),
            i18n::Language::Ru => "Делегирование агента отменено.".to_string(),
            i18n::Language::En => "Agent delegation cancelled.".to_string(),
        },
        Some(RunResultValue::Incomplete { .. }) => match lang {
            i18n::Language::ZhHans => "子代理未完成委派。".to_string(),
            i18n::Language::ZhHant => "子代理未完成委派。".to_string(),
            i18n::Language::De => "Agent-Delegation unvollständig.".to_string(),
            i18n::Language::Es => "Delegación del agente incompleta.".to_string(),
            i18n::Language::Fr => "Délégation de l’agent incomplète.".to_string(),
            i18n::Language::Ja => "サブエージェントの委任が完了しませんでした。".to_string(),
            i18n::Language::Ko => "하위 에이전트 위임이 완료되지 않았습니다.".to_string(),
            i18n::Language::Ru => "Делегирование агента не завершено.".to_string(),
            i18n::Language::En => "Agent delegation did not complete.".to_string(),
        },
        Some(RunResultValue::Answered { .. } | RunResultValue::ActionCompleted { .. }) => {
            match lang {
                i18n::Language::ZhHans => "已完成。".to_string(),
                i18n::Language::ZhHant => "已完成。".to_string(),
                i18n::Language::De => "Abgeschlossen.".to_string(),
                i18n::Language::Es => "Completado.".to_string(),
                i18n::Language::Fr => "Terminé.".to_string(),
                i18n::Language::Ja => "完了しました。".to_string(),
                i18n::Language::Ko => "완료되었습니다.".to_string(),
                i18n::Language::Ru => "Завершено.".to_string(),
                i18n::Language::En => "Completed.".to_string(),
            }
        }
        _ => match outcome.turn_status {
            AgentTurnStatus::Cancelled => match lang {
                i18n::Language::ZhHans => "子代理委派已取消。".to_string(),
                i18n::Language::ZhHant => "子代理委派已取消。".to_string(),
                i18n::Language::En => "Agent delegation cancelled.".to_string(),
                _ => "Agent delegation cancelled.".to_string(),
            },
            AgentTurnStatus::Failed => match lang {
                i18n::Language::ZhHans => "子代理委派失败。".to_string(),
                i18n::Language::ZhHant => "子代理委派失敗。".to_string(),
                i18n::Language::En => "Agent delegation failed.".to_string(),
                _ => "Agent delegation failed.".to_string(),
            },
            AgentTurnStatus::AwaitingPermission => match lang {
                i18n::Language::ZhHans => "子代理正在等待权限授权。".to_string(),
                i18n::Language::ZhHant => "子代理正在等待權限授權。".to_string(),
                i18n::Language::En => "Agent delegation is awaiting permission.".to_string(),
                _ => "Agent delegation is awaiting permission.".to_string(),
            },
            AgentTurnStatus::AwaitingInput => match lang {
                i18n::Language::ZhHans => "子代理正在等待你的回答（/questions）。".to_string(),
                i18n::Language::ZhHant => "子代理正在等待您的回答（/questions）。".to_string(),
                i18n::Language::En => "Agent delegation is awaiting input (/questions).".to_string(),
                _ => "Agent delegation is awaiting input (/questions).".to_string(),
            },
            _ => match lang {
                i18n::Language::ZhHans => "子代理委派未立即完成，请用 /tasks 查看状态。".to_string(),
                i18n::Language::ZhHant => "子代理委派未立即完成，請用 /tasks 檢視狀態。".to_string(),
                i18n::Language::En => "Agent delegation is pending; check /tasks for status.".to_string(),
                _ => "Agent delegation is pending; check /tasks for status.".to_string(),
            },
        },
    }
}

fn localized_job_state(state: &str) -> &'static str {
    let lang = i18n::current_language();
    match (lang, state) {
        (i18n::Language::ZhHans, "running" | "queued") => "运行中",
        (i18n::Language::ZhHans, "completed") => "已完成",
        (i18n::Language::ZhHans, "failed") => "失败",
        (i18n::Language::ZhHans, "cancelled") => "已取消",
        (i18n::Language::ZhHant, "running" | "queued") => "執行中",
        (i18n::Language::ZhHant, "completed") => "已完成",
        (i18n::Language::ZhHant, "failed") => "失敗",
        (i18n::Language::ZhHant, "cancelled") => "已取消",
        (i18n::Language::De, "running" | "queued") => "läuft",
        (i18n::Language::De, "completed") => "abgeschlossen",
        (i18n::Language::De, "failed") => "fehlgeschlagen",
        (i18n::Language::De, "cancelled") => "abgebrochen",
        (i18n::Language::Es, "running" | "queued") => "en ejecución",
        (i18n::Language::Es, "completed") => "completada",
        (i18n::Language::Es, "failed") => "fallida",
        (i18n::Language::Es, "cancelled") => "cancelada",
        (i18n::Language::Fr, "running" | "queued") => "en cours",
        (i18n::Language::Fr, "completed") => "terminée",
        (i18n::Language::Fr, "failed") => "échec",
        (i18n::Language::Fr, "cancelled") => "annulée",
        (i18n::Language::Ja, "running" | "queued") => "実行中",
        (i18n::Language::Ja, "completed") => "完了",
        (i18n::Language::Ja, "failed") => "失敗",
        (i18n::Language::Ja, "cancelled") => "キャンセル",
        (i18n::Language::Ko, "running" | "queued") => "실행 중",
        (i18n::Language::Ko, "completed") => "완료",
        (i18n::Language::Ko, "failed") => "실패",
        (i18n::Language::Ko, "cancelled") => "취소됨",
        (i18n::Language::Ru, "running" | "queued") => "выполняется",
        (i18n::Language::Ru, "completed") => "завершена",
        (i18n::Language::Ru, "failed") => "ошибка",
        (i18n::Language::Ru, "cancelled") => "отменена",
        (_, "running" | "queued") => "running",
        (_, "completed") => "completed",
        (_, "failed") => "failed",
        (_, "cancelled") => "cancelled",
        _ => "unknown",
    }
}

fn format_job_summary_line(task: &tools::BackgroundTaskSummary, width: usize) -> String {
    let icon = match task.canonical_state.as_str() {
        "running" | "queued" => "●",
        "completed" => "✓",
        _ => "!",
    };
    let state = localized_job_state(&task.canonical_state);
    let elapsed = format_elapsed_duration(Duration::from_secs_f64(task.elapsed_secs.max(0.0)));
    let single_line_cmd = task
        .command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cmd_budget = width.saturating_sub(36).max(20);
    let cmd = truncate_display_width(&single_line_cmd, cmd_budget);
    if interactive_color_enabled() {
        let color = match task.canonical_state.as_str() {
            "running" | "queued" => "\x1b[33m",
            "completed" => "\x1b[32m",
            _ => "\x1b[31m",
        };
        format!(
            "  {color}{icon}  {state}{INTERACTIVE_RESET}  {INTERACTIVE_VALUE}{}{INTERACTIVE_RESET}  {INTERACTIVE_DIM}pid {} · {elapsed}{INTERACTIVE_RESET}  {cmd}",
            task.id, task.pid
        )
    } else {
        format!("  {icon}  {state}  {}  pid {} · {elapsed}  {cmd}", task.id, task.pid)
    }
}

fn extract_clean_job_output_tails(raw: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_output = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## stdout") || trimmed.starts_with("## stderr") || trimmed.starts_with("stdout:") || trimmed.starts_with("stderr:") {
            in_output = true;
            lines.push(format!("  {}", trimmed.trim_start_matches('#').trim()));
            continue;
        }
        if trimmed.starts_with("```") {
            continue;
        }
        if in_output {
            lines.push(format!("    {line}"));
        }
    }
    if lines.len() > 24 {
        let tail_start = lines.len() - 24;
        let mut trimmed = vec!["    ...".to_string()];
        trimmed.extend(lines.into_iter().skip(tail_start));
        return trimmed;
    }
    lines
}

fn render_interactive_jobs_view(
    config: &Config,
    action: &str,
    task_id: Option<&str>,
) -> String {
    let width = terminal_columns().saturating_sub(4).max(40);
    if action == "stop" {
        let outcome = tools::manage_background_bash_tasks(
            action,
            task_id,
            &config.allowed_dirs,
            config.session_id.as_deref(),
            &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
                config.skill_dirs.clone(),
            ),
        );
        return if outcome.error {
            format_error(&outcome.display)
        } else {
            format!("  {}", outcome.display)
        };
    }

    let summaries = match tools::background_bash_task_summaries(
        &config.allowed_dirs,
        config.session_id.as_deref(),
    ) {
        Ok(list) => list,
        Err(err) => return format_error(&err),
    };

    if action == "list" {
        if summaries.is_empty() {
            let empty = match i18n::current_language() {
                i18n::Language::ZhHans => "当前没有后台命令任务。",
                i18n::Language::ZhHant => "目前沒有背景命令任務。",
                i18n::Language::De => "Keine Hintergrundaufgaben vorhanden.",
                i18n::Language::Es => "No hay tareas en segundo plano.",
                i18n::Language::Fr => "Aucune tâche en arrière-plan.",
                i18n::Language::Ja => "バックグラウンドジョブはありません。",
                i18n::Language::Ko => "백그라운드 작업이 없습니다.",
                i18n::Language::Ru => "Нет фоновых задач.",
                i18n::Language::En => "No background jobs.",
            };
            return format!("  {empty}");
        }
        return summaries
            .iter()
            .map(|task| format_job_summary_line(task, width))
            .collect::<Vec<_>>()
            .join("\n");
    }

    // status for a specific task_id
    let Some(target_id) = task_id else {
        return format!("  {}", tr(Message::InteractiveJobsUsage));
    };
    let mut lines = Vec::new();
    if let Some(summary) = summaries.iter().find(|t| t.id == target_id) {
        lines.push(format_job_summary_line(summary, width));
    }
    let outcome = tools::manage_background_bash_tasks(
        "status",
        Some(target_id),
        &config.allowed_dirs,
        config.session_id.as_deref(),
        &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
            config.skill_dirs.clone(),
        ),
    );
    if outcome.error {
        return format_error(&outcome.display);
    }
    let tails = extract_clean_job_output_tails(&outcome.text);
    if !tails.is_empty() {
        lines.extend(tails);
    } else if lines.is_empty() {
        lines.push(format!("  {}", outcome.display));
    }
    lines.join("\n")
}

fn reasoning_tier_display_name(tier: nola_domain::reasoning_tiers::ReasoningTier) -> &'static str {
    match tier {
        nola_domain::reasoning_tiers::ReasoningTier::Concise => "concise",
        nola_domain::reasoning_tiers::ReasoningTier::Balanced => "balanced",
        nola_domain::reasoning_tiers::ReasoningTier::Detailed => "detailed",
    }
}

fn active_model_reasoning_levels(config: &Config) -> Vec<String> {
    if let Some(nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration::Effort { values }) =
        config.runtime.model_capabilities.reasoning_declaration.as_ref()
    {
        if !values.is_empty() {
            return values.clone();
        }
    }
    let active_model = config.model.trim();
    if active_model.is_empty() {
        return Vec::new();
    }
    let Ok(mut app) = crate::application_port::CliApplicationPort::persistent_default() else {
        return Vec::new();
    };
    let Ok(connections) = app.execute::<serde_json::Value>(nola_application::ApplicationCommand::Model {
        command: nola_application::ModelCommand::ListConnections,
    }) else {
        return Vec::new();
    };
    let Some(items) = connections.as_array() else {
        return Vec::new();
    };
    for item in items {
        let Some(models) = item
            .pointer("/connection/document/catalog/models")
            .or_else(|| item.pointer("/document/catalog/models"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for model in models {
            let matches_id = model
                .get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| id == active_model)
                || model
                    .get("model_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| id == active_model);
            if !matches_id {
                continue;
            }
            if let Some(levels) = model
                .get("reasoning_levels")
                .and_then(serde_json::Value::as_array)
            {
                let parsed: Vec<String> = levels
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
                if !parsed.is_empty() {
                    return parsed;
                }
            }
        }
    }
    Vec::new()
}

fn handle_interactive_thinking_command(
    config: &mut Config,
    arg: Option<&str>,
) -> Result<String, String> {
    let Some(raw) = arg.map(str::trim).filter(|value| !value.is_empty()) else {
        let current = config
            .runtime
            .reasoning_effort_display()
            .unwrap_or_else(|| "default".to_string());
        let tier = reasoning_tier_display_name(config.runtime.reasoning_tier);
        let model_levels = active_model_reasoning_levels(config);
        let mut lines = vec![format!(
            "  {}: {current} (tier: {tier})",
            interactive_reasoning_label()
        )];
        if !model_levels.is_empty() {
            lines.push(format!("  Model levels: {}", model_levels.join(", ")));
        }
        lines.push("  Available: off, low, medium, high, xhigh, max, default".to_string());
        lines.push("  Usage: /thinking <off|low|medium|high|xhigh|max|default>".to_string());
        return Ok(lines.join("\n"));
    };

    let selection = crate::runtime_config::parse_reasoning_effort_selection(raw)?;
    config.runtime.reasoning_selection = selection;
    let confirmation = match config.runtime.reasoning_effort_display() {
        Some(level) => format!("  Reasoning effort set to {level}."),
        None => "  Reasoning effort reset to default.".to_string(),
    };
    Ok(confirmation)
}

fn goal_work_control_command(control: &str, now_epoch_ms: i64) -> Option<GoalRuntimeCommand> {
    match control {
        "pause" => Some(GoalRuntimeCommand::Pause {
            status: GoalStatus::Paused,
            wake_at_epoch_ms: None,
            now_epoch_ms,
        }),
        "resume" => Some(GoalRuntimeCommand::Resume { now_epoch_ms }),
        "stop" | "cancel" => Some(GoalRuntimeCommand::Cancel { now_epoch_ms }),
        _ => None,
    }
}

#[cfg(test)]
mod goal_work_control_tests {
    use super::goal_work_control_command;
    use nola_core::agent::GoalRuntimeCommand;

    #[test]
    fn goal_work_control_rejects_rerun_without_panicking() {
        assert!(goal_work_control_command("rerun", 42).is_none());
        assert!(goal_work_control_command("unknown", 42).is_none());
        assert!(matches!(
            goal_work_control_command("pause", 42),
            Some(GoalRuntimeCommand::Pause { now_epoch_ms: 42, .. })
        ));
        assert!(matches!(
            goal_work_control_command("resume", 42),
            Some(GoalRuntimeCommand::Resume { now_epoch_ms: 42 })
        ));
        assert!(matches!(
            goal_work_control_command("stop", 42),
            Some(GoalRuntimeCommand::Cancel { now_epoch_ms: 42 })
        ));
        assert!(matches!(
            goal_work_control_command("cancel", 42),
            Some(GoalRuntimeCommand::Cancel { now_epoch_ms: 42 })
        ));
    }
}

#[cfg(test)]
mod completed_task_digest_tests {
    use super::{
        clear_pending_task_completions, drain_pending_task_completion_prefix,
        inject_completed_task_updates, record_completed_task_digest, CompletedTaskDigest,
        MAX_PENDING_TASK_COMPLETIONS,
    };

    #[test]
    fn completed_task_digest_deduplicates_caps_formats_and_clears() {
        clear_pending_task_completions();

        for i in 1..=14 {
            record_completed_task_digest(CompletedTaskDigest {
                task_id: format!("task-{i}"),
                agent_type: Some("research".to_string()),
                status: "completed".to_string(),
                summary: format!("Finished step {i}"),
            });
        }
        // Updating an existing task_id should deduplicate instead of duplicating.
        record_completed_task_digest(CompletedTaskDigest {
            task_id: "task-14".to_string(),
            agent_type: Some("research".to_string()),
            status: "sealed".to_string(),
            summary: "Final   updated\nresult 14".to_string(),
        });

        let prefix = drain_pending_task_completion_prefix().expect("expected queued updates");
        assert!(prefix.starts_with("[Background task updates]\n"));
        assert!(prefix.ends_with("\n[/Background task updates]"));
        let item_lines: Vec<&str> = prefix
            .lines()
            .filter(|line| line.starts_with("- "))
            .collect();
        assert_eq!(item_lines.len(), MAX_PENDING_TASK_COMPLETIONS);
        assert!(!prefix.contains("- task-1 "));
        assert!(!prefix.contains("- task-2 "));
        assert!(prefix.contains("- task-3 (research, completed): Finished step 3"));
        assert_eq!(
            item_lines
                .iter()
                .filter(|line| line.starts_with("- task-14 "))
                .count(),
            1
        );
        assert!(prefix.contains("- task-14 (research, sealed): Final updated result 14"));

        let effective = inject_completed_task_updates("Continue with the summary", Some(&prefix));
        assert!(effective.starts_with("[Background task updates]\n"));
        assert!(effective.ends_with("\n\nContinue with the summary"));

        // Draining empties the inbox.
        assert!(drain_pending_task_completion_prefix().is_none());

        // Session switch / reset clears queued digests without injecting them.
        record_completed_task_digest(CompletedTaskDigest {
            task_id: "task-stale".to_string(),
            agent_type: None,
            status: "failed".to_string(),
            summary: "Old session failure".to_string(),
        });
        clear_pending_task_completions();
        assert!(drain_pending_task_completion_prefix().is_none());
        assert_eq!(
            inject_completed_task_updates("Fresh prompt", None),
            "Fresh prompt"
        );
    }
}

#[cfg(test)]
mod thinking_command_tests {
    use super::{
        handle_interactive_thinking_command, interactive_prompt_compact_line,
        interactive_prompt_footer_line, normalize_interactive_command, Config,
        INTERACTIVE_SLASH_COMMANDS,
    };
    use nola_core::agent::reasoning::ReasoningSelection;
    use nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration;

    #[test]
    fn thinking_slash_command_is_registered_and_updates_live_state() {
        let _lock = crate::store::test_env_lock();
        assert!(INTERACTIVE_SLASH_COMMANDS
            .iter()
            .any(|cmd| cmd.name == "/thinking"));
        assert_eq!(normalize_interactive_command("/thinking"), Some("/thinking"));
        assert_eq!(normalize_interactive_command("/THINKING"), Some("/thinking"));

        let mut config = Config::for_test().unwrap();
        config.model = "gpt-5".to_string();
        config.runtime.model_capabilities.reasoning_declaration =
            Some(ReasoningCapabilityDeclaration::Effort {
                values: vec!["low".into(), "medium".into(), "high".into()],
            });

        let initial = handle_interactive_thinking_command(&mut config, None).unwrap();
        assert!(initial.contains("default"), "{initial}");
        assert!(initial.contains("tier: balanced"), "{initial}");
        assert!(initial.contains("low, medium, high"), "{initial}");
        assert!(
            initial.contains("/thinking <off|low|medium|high|xhigh|max|default>"),
            "{initial}"
        );

        let set_high = handle_interactive_thinking_command(&mut config, Some("high")).unwrap();
        assert!(set_high.contains("high"), "{set_high}");
        assert_eq!(
            config.runtime.reasoning_selection,
            Some(ReasoningSelection::Effort {
                value: "high".to_string()
            })
        );
        assert_eq!(
            config.runtime.reasoning_effort_display().as_deref(),
            Some("high")
        );

        let footer_high = interactive_prompt_footer_line(&config, None, None).unwrap();
        assert!(
            footer_high.contains("gpt-5 · high"),
            "footer should show active effort next to model: {footer_high}"
        );

        let compact_status = interactive_prompt_compact_line(
            1,
            "gpt-5",
            "high",
            "auto",
            "~/repo",
            "~/repo",
            "openai",
            false,
            120,
        )
        .unwrap();
        assert!(compact_status.contains("high"), "{compact_status}");

        let set_off = handle_interactive_thinking_command(&mut config, Some("off")).unwrap();
        assert!(set_off.contains("off"), "{set_off}");
        assert_eq!(
            config.runtime.reasoning_effort_display().as_deref(),
            Some("off")
        );

        let reset = handle_interactive_thinking_command(&mut config, Some("default")).unwrap();
        assert!(reset.contains("default"), "{reset}");
        assert_eq!(config.runtime.reasoning_selection, None);
        let footer_default = interactive_prompt_footer_line(&config, None, None).unwrap();
        assert!(
            !footer_default.contains("gpt-5 · high") && !footer_default.contains("gpt-5 · off"),
            "default effort should not clutter footer: {footer_default}"
        );

        assert!(handle_interactive_thinking_command(&mut config, Some("invalid")).is_err());
    }
}
