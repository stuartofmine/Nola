#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn replay_includes_surface_summary_beside_text_without_reading_form_values() {
        let message = json!({
            "content":"Here is the form.",
            "display_ui_surface_summaries":["Travel dates and destination"],
            "data":{"private_input":"not a display summary"}
        });
        assert_eq!(interactive_session_assistant_content_text(&message).as_deref(),
            Some("Here is the form.\n\nTravel dates and destination\n\n[只读界面摘要：CLI 不支持图形表单交互，请在 Web 或原生客户端操作。]"));
        assert_eq!(interactive_session_assistant_content_text(&json!({
            "content":"", "display_ui_surface_summaries":["Form available"]
        })).as_deref(), Some("Form available\n\n[只读界面摘要：CLI 不支持图形表单交互，请在 Web 或原生客户端操作。]"));
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::store::test_env_lock()
    }

    #[test]
    fn background_notification_labels_distinguish_canonical_terminal_results() {
        for language in [
            i18n::Language::ZhHans, i18n::Language::En, i18n::Language::De,
            i18n::Language::Es, i18n::Language::Fr, i18n::Language::Ja, i18n::Language::Ru,
        ] {
            let completed = background_task_notification_label(language, "completed");
            let failed = background_task_notification_label(language, "failed");
            let cancelled = background_task_notification_label(language, "cancelled");
            assert_ne!(completed, failed);
            assert_ne!(completed, cancelled);
            assert_ne!(failed, cancelled);
            for state in ["queued", "waiting_permission", "unknown", "exited(0)"] {
                let unconfirmed = background_task_notification_label(language, state);
                assert_ne!(unconfirmed, completed, "display text is not completion evidence");
                assert_ne!(unconfirmed, failed);
                assert_ne!(unconfirmed, cancelled);
            }
        }
    }

    #[test]
    fn cli_execution_adapter_restores_scope_after_runtime_error() {
        let outer = interaction::ExecutionCancellation::default();
        let _outer_scope = outer.enter();
        let inner = interaction::ExecutionCancellation::default();
        let inner_id = inner.execution_id();
        let mut emitter = Emitter::new(true);
        let result: Result<(), String> = with_cli_execution_control(&Config::for_test().unwrap(), inner, &mut emitter, |control, _| {
            assert_eq!(control.execution_id(), inner_id);
            assert_eq!(interaction::current_execution_cancellation().unwrap().execution_id(), inner_id);
            Err("fixture runtime failure".into())
        });
        assert_eq!(result.unwrap_err(), "fixture runtime failure");
        assert_eq!(interaction::current_execution_cancellation().unwrap().execution_id(), outer.execution_id());
        // The wrapper must have released any listener before returning. This
        // no-terminal fixture exercises scope cleanup, not real keyboard I/O.
        emitter.wait_for_execution_input_release();
    }

    #[test]
    #[ignore = "requires a private recorded SQLite backup; never resumes its runs"]
    fn recorded_long_context_cli_replay_baseline() {
        let _lock = env_lock();
        let source = std::fs::canonicalize(std::env::var("NOLA_LONG_CONTEXT_FIXTURE_DB").unwrap()).unwrap();
        let conversation = std::env::var("NOLA_LONG_CONTEXT_CONVERSATION").unwrap();
        let shared_temporary = std::fs::canonicalize("/tmp").unwrap();
        let temporary = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(source.starts_with(shared_temporary) || source.starts_with(temporary),
            "use a consistent private backup, never the live store");
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("recorded.sqlite");
        std::fs::copy(source, &database).unwrap();
        struct RestoreDatabase(Option<std::ffi::OsString>);
        impl Drop for RestoreDatabase {
            fn drop(&mut self) {
                if let Some(value) = self.0.take() { std::env::set_var("NOLA_DB_PATH", value); }
                else { std::env::remove_var("NOLA_DB_PATH"); }
            }
        }
        let _restore = RestoreDatabase(std::env::var_os("NOLA_DB_PATH"));
        std::env::set_var("NOLA_DB_PATH", &database);
        let started = Instant::now();
        let snapshot = session::load_interactive_session_snapshot(&conversation).unwrap();
        let loaded = started.elapsed();
        let warm_start = Instant::now();
        let repeated = session::load_interactive_session_snapshot(&conversation).unwrap();
        let warm_loaded = warm_start.elapsed();
        assert_eq!(repeated.saved.messages, snapshot.saved.messages);
        #[cfg(target_os = "macos")]
        {
            let before = recorded_peak_rss_bytes();
            let repeat_start = Instant::now();
            for _ in 0..20 {
                let reread = session::load_interactive_session_snapshot(&conversation).unwrap();
                assert_eq!(reread.saved.messages, snapshot.saved.messages);
                let _ = interactive_session_replay_lines(&reread.saved);
            }
            let after = recorded_peak_rss_bytes();
            println!("recorded_cli_memory peak_before_bytes={before} peak_after_bytes={after} repeat_count=20 repeat_elapsed={:?}", repeat_start.elapsed());
        }
        let rendering = Instant::now();
        let commits = interactive_session_replay_commits(&snapshot.saved);
        let render_elapsed = rendering.elapsed();
        let user_rows = commits.iter().filter(|row| matches!(row, InteractiveReplayCommit::User(_))).count();
        let assistant_rows = commits.iter().filter(|row| matches!(row, InteractiveReplayCommit::Assistant(_))).count();
        assert!(snapshot.total_message_count > 0);
        assert!(user_rows > 0);
        assert!(assistant_rows > 0, "recorded assistant content must not disappear in CLI replay");
        assert!(!commits.iter().all(|row| matches!(row, InteractiveReplayCommit::Empty)));
        let layout_start = Instant::now();
        let lines = interactive_session_replay_lines(&snapshot.saved);
        let rendered_bytes: usize = lines.iter().map(String::len).sum();
        println!("recorded_cli total_messages={} retained_messages={} commits={} user_rows={} assistant_rows={assistant_rows} load={loaded:?} warm_load={warm_loaded:?} replay_projection={render_elapsed:?} layout={:?} lines={} rendered_bytes={rendered_bytes}",
            snapshot.total_message_count, snapshot.saved.messages.len(), commits.len(), user_rows, layout_start.elapsed(), lines.len());
        #[cfg(target_os = "macos")]
        recorded_replay_pty(&snapshot.saved);
    }

    #[cfg(target_os = "macos")]
    fn recorded_peak_rss_bytes() -> u64 {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) }, 0);
        // Darwin reports ru_maxrss in bytes (unlike Linux's KiB).
        unsafe { usage.assume_init().ru_maxrss.max(0) as u64 }
    }

    #[cfg(target_os = "macos")]
    fn recorded_replay_pty(saved: &session::SavedSession) {
        use std::io::Write;
        const MARK: &str = "\x1b]0;nola-recorded-frame\x07";
        const SIZES: [(u16, u16); 3] = [(30, 100), (15, 40), (40, 120)];
        if std::env::var_os("NOLA_RECORDED_PTY_CHILD").is_some() {
            assert!(io::stdout().is_terminal());
            let _raw = InteractiveRawModeGuard::new().unwrap();
            for (index, (rows, columns)) in SIZES.into_iter().enumerate() {
                let size = libc::winsize { ws_row: rows, ws_col: columns, ws_xpixel: 0, ws_ypixel: 0 };
                assert_eq!(unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCSWINSZ, &size) }, 0);
                if index == 0 {
                    crate::terminal_ui::begin_interactive_session();
                    print_interactive_session_replay(saved);
                }
                draw_composer_frame("RECORDED-DRAFT", 14, Some("Context 42% · test-model"), None).unwrap();
                // The real event loop paints invalidated frames separately
                // from input reduction; this read-only harness does the same.
                crate::terminal_ui::paint_pending_frame();
                print!("{MARK}");
                io::stdout().flush().unwrap();
            }
            crate::terminal_ui::end_interactive_session();
            return;
        }
        // Run only this test in a real terminal. No normal interactive loop:
        // that loop reconciles tasks and must not run against captured work.
        let output = std::process::Command::new("/usr/bin/script")
            .args(["-q", "/dev/null"])
            .arg(std::env::current_exe().unwrap())
            .args(["tests::recorded_long_context_cli_replay_baseline", "--exact", "--ignored", "--nocapture"])
            .env("NOLA_RECORDED_PTY_CHILD", "1")
            .env("TERM", "xterm-256color")
            .stdin(std::process::Stdio::null())
            .output().unwrap();
        assert!(output.status.success(), "Recorded PTY child failed (private output intentionally omitted)");
        let bytes = String::from_utf8_lossy(&output.stdout);
        let frames: Vec<_> = bytes.split(MARK).collect();
        assert_eq!(frames.len(), 4, "Expected three rendered terminal frames");
        let mut parser = vt100::Parser::new(30, 100, 2000);
        for (index, (rows, columns)) in SIZES.into_iter().enumerate() {
            parser.set_size(rows, columns);
            parser.process(frames[index].as_bytes());
            let screen = parser.screen().contents();
            assert_eq!(screen.matches("Context 42%").count(), 1, "Exactly one visible footer at {columns}x{rows}");
            let draft = screen.find("RECORDED-DRAFT").expect("Draft survives recorded history and resize");
            let footer = screen.find("Context 42%").unwrap();
            assert!(footer > draft, "Footer must stay below the composer");
            let footer_line = screen.lines().find(|line| line.contains("Context 42%")).unwrap();
            assert!(!footer_line.contains("╰─") && !footer_line.contains("├─"),
                "Footer must not have a tree prefix; recorded content may contain trees");
        }
        println!("recorded_cli_pty three_sizes=passed footer=unique_below_draft captured_work=not_resumed");
    }

    #[test]
    fn permission_picker_digits_confirm_while_navigation_only_selects() {
        use PermissionPickerAction::*;
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        for (digit, index) in [('1', 0), ('2', 1)] {
            assert_eq!(permission_picker_action(key(KeyCode::Char(digit)), 0), SelectAndConfirm(index));
            for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
                assert_eq!(permission_picker_action(KeyEvent::new_with_kind(KeyCode::Char(digit), KeyModifiers::NONE, kind), 0), Ignore);
            }
            for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
                assert_eq!(permission_picker_action(KeyEvent::new(KeyCode::Char(digit), modifiers), 0), Ignore);
            }
        }
        assert_eq!(permission_picker_action(key(KeyCode::Char('3')), 0), Ignore);
        for selected in 0..ALL_PERMISSION_LEVELS.len() {
            for code in [KeyCode::Up, KeyCode::Down, KeyCode::Tab, KeyCode::BackTab, KeyCode::Char('j'), KeyCode::Char('k')] {
                let Select(next) = permission_picker_action(key(code), selected) else { panic!("navigation expected"); };
                assert!(next < ALL_PERMISSION_LEVELS.len());
            }
        }
        assert_eq!(permission_picker_action(key(KeyCode::Enter), 2), Confirm);
        assert_eq!(permission_picker_action(key(KeyCode::Esc), 2), Cancel);
        assert_eq!(permission_picker_action(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), 2), Cancel);
        assert_eq!(permission_picker_action(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT), 0), Ignore);
        assert_eq!(permission_picker_action(KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release), 2), Ignore);
        assert_eq!(permission_picker_action(key(KeyCode::Up), 0), Select(1));
        assert_eq!(permission_picker_action(key(KeyCode::Tab), 1), Select(0));
    }


    fn temporary_session_db_root(test_name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nola-cli-main-{test_name}-{}", std::process::id()))
    }

    #[test]
    fn natural_language_never_becomes_a_client_workspace_command() {
        let mut config = Config::for_test().expect("test config");
        let mut goal = None;
        let mut constraints = None;
        let mut previous_workspace = None;
        let mut usage = None;
        for prompt in ["切到上级目录", "切换到上级目录", "返回上级目录",
            "go to the parent directory", "search current files", "继续", "保留原消息"] {
            assert!(matches!(handle_interactive_slash_command(prompt, &mut config,
                &mut goal, &mut constraints, &mut previous_workspace, &mut usage, 0).unwrap(),
                SlashCommandResult::PassThrough));
        }
        assert!(goal.is_none() && constraints.is_none() && previous_workspace.is_none() && usage.is_none());
    }

    #[test]
    fn a_new_goal_does_not_enter_canonical_run_recovery_before_session_allocation() {
        let config = Config::for_test().expect("test config");

        assert!(crate::goal_runtime::pending_goal_run(&config.host_invocation_request().unwrap()).unwrap().is_none());
    }

    #[test]
    fn goal_pause_without_canonical_session_never_reports_local_success() {
        let config = Config::for_test().expect("test config");
        assert!(config.session_id.is_none());
        for cause in [GoalCyclePause::UserPaused, GoalCyclePause::NeedsUser, GoalCyclePause::Incomplete] {
            assert!(matches!(pause_goal(&config, cause), Err(error)
                if error == "Goal runtime requires a durable session id"));
        }
    }

    #[test]
    fn interactive_session_history_shows_all_messages_in_order() {
        let saved = session::SavedSession {
            id: "session-1".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"user","content":"first"}),
                json!({"role":"assistant","content":"ack"}),
                json!({"role":"user","content":"second"}),
                json!({"role":"assistant","content":"ok"}),
                json!({"role":"user","content":"third"}),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        assert_eq!(
            lines,
            vec![
                "  first".to_string(),
                "".to_string(),
                "ack".to_string(),
                "".to_string(),
                "  second".to_string(),
                "".to_string(),
                "ok".to_string(),
                "".to_string(),
                "  third".to_string(),
                "".to_string(),
            ]
        );
    }

    #[test]
    fn interactive_session_user_replay_line_is_plain_when_color_is_disabled() {
        let line = interactive_session_replay_user_line("hello");
        assert_eq!(line, "    › hello");
        assert!(!line.contains('\r'));
        assert!(!line.contains("\x1b["));
    }

    #[test]
    fn cli_polish_user_history_keeps_multiline_cjk_content_and_background() {
        let text = "这是很长的历史输入，需要完整换行显示。\n第二行 🙂";
        let rendered = interactive_user_history_at_width(text, 24, true);
        let mut terminal = vt100::Parser::new(20, 24, 0);
        terminal.process(rendered.replace('\n', "\r\n").as_bytes());
        for row in 0..rendered.lines().count() as u16 {
            assert_eq!(terminal.screen().cell(row, 0).unwrap().bgcolor(), vt100::Color::Idx(236));
            let last = terminal.screen().cell(row, 23).unwrap();
            if !last.is_wide_continuation() {
                assert_eq!(last.bgcolor(), vt100::Color::Idx(236), "row={row} {rendered:?}");
            }
        }
        let plain = interactive_user_history_at_width(text, 24, false);
        let reconstructed = plain.lines().enumerate().map(|(index, row)| {
            row.strip_prefix(if index == 0 { "    › " } else { "      " }).unwrap()
        }).collect::<String>();
        assert_eq!(reconstructed, text.replace('\n', ""));
        terminal.process(b"\r\nassistant");
        assert_eq!(terminal.screen().cell(rendered.lines().count() as u16, 0).unwrap().bgcolor(), vt100::Color::Default);
    }

    #[test]
    fn cli_polish_cache_footer_differentiates_a_real_miss_from_unreported_usage() {
        let config = Config::for_test().unwrap();
        let mut usage = nola_core::agent::context_window::ContextUsageSnapshot {
            cache_usage: Some(nola_domain::PromptCacheUsage::measured(Some(1000), Some(0), Some(997), false)),
            context_window_tokens: 100_000, ..Default::default()
        };
        let footer = interactive_prompt_footer_line(&config, None, Some(usage)).unwrap();
        assert!(footer.contains("0.0%"), "{footer}");
        assert!(footer.contains("Latest request") || footer.contains("本次请求"), "{footer}");
        usage.cache_usage = Some(nola_domain::PromptCacheUsage::measured(Some(1000), None, None, false));
        let unknown = interactive_prompt_footer_line(&config, None, Some(usage)).unwrap();
        assert!(unknown.contains(&format!("{} —", ui_text(UiText::CacheHitRate))), "{unknown}");
    }


    #[test]
    fn interactive_session_history_hides_system_messages() {
        let saved = session::SavedSession {
            id: "session-system-hidden".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"system","content":"system context"}),
                json!({"role":"user","content":"hello"}),
                json!({"role":"assistant","content":"hi"}),
                json!({"role":"system","content":"system end"}),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        assert_eq!(
            lines,
            vec![
                "  hello".to_string(),
                "".to_string(),
                "hi".to_string(),
                "".to_string(),
            ]
        );
    }

    #[test]
    fn rejected_drafts_are_not_replayed_as_assistant_answers() {
        let _guard = crate::store::test_env_lock();
        let message = json!({"role":"assistant", "content":"",
            "display_text_parts":[{"text":"INVALID-DRAFT", "review":"rejected"}],
            "tool_calls":[{"id":"call", "type":"function", "function":{"name":"read_file", "arguments":"{}"}}]});
        assert!(is_replay_tool_only_message(&message));
        let saved = session::SavedSession {
            id: "draft-replay".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            messages: vec![message.clone()],
        };
        let rendered = interactive_session_replay_lines(&saved).join("\n");
        assert!(!rendered.contains("INVALID-DRAFT"));
        assert!(rendered.contains("Read file"));
        let items =
            crate::core_agent_runtime::session_message_to_agent_items(&message, "draft-replay", 0);
        assert!(items.iter().all(|item| !serde_json::to_string(item)
            .unwrap()
            .contains("INVALID-DRAFT")));
    }

    #[test]
    fn deferred_metadata_is_not_visible_or_recalled_as_user_input() {
        let deferred = json!({"role":"assistant", "content":"",
            "display_text_parts":[{"text":"DEFERRED-DRAFT", "review":null,
                "metadata_deferred":true}]});
        assert!(!replay_text_part_is_visible(
            &deferred["display_text_parts"][0]
        ));
        let omitted_provider = json!({"role":"user", "content":"",
            "metadata_deferred":true,"projection_truncated":true});
        assert!(!is_replay_user_input_message(&omitted_provider));
        // Unknown envelope metadata proves neither human input nor a tool result.
        assert!(!is_replay_tool_only_message(&omitted_provider));
        let saved = session::SavedSession {
            id: "deferred-replay".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            messages: vec![
                deferred,
                omitted_provider,
                json!({"role":"user","content":"real input"}),
            ],
        };
        assert!(!interactive_session_replay_lines(&saved)
            .join("\n")
            .contains("DEFERRED-DRAFT"));
        assert_eq!(
            interactive_session_replay_user_inputs(&saved),
            vec!["real input"]
        );
    }

    #[test]
    fn only_approved_reviewed_answer_is_replayed() {
        let saved = session::SavedSession {
            id: "reviewed-answer".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            messages: vec![json!({"role":"assistant", "content":"Final answer",
            "display_text_parts":[
                {"text":"Rejected draft", "review":"rejected"},
                {"text":"Incomplete draft", "review":"pending"},
                {"text":"Final answer", "review":"approved"}
            ]})],
        };
        let rendered = interactive_session_replay_lines(&saved).join("\n");
        assert!(!rendered.contains("Rejected draft"));
        assert!(!rendered.contains("Incomplete draft"));
        assert_eq!(rendered.matches("Final answer").count(), 1);
    }

    #[test]
    fn interactive_session_replay_uses_session_page_style_not_role_text() {
        let saved = session::SavedSession {
            id: "session-page-style".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"system","content":"system prompt"}),
                json!({"role":"user","content":"hello"}),
                json!({"role":"assistant","content":"hi"}),
                json!({"role":"tool","name":"read_file","content":"file contents hidden"}),
            ],
        };

        let rendered = interactive_session_replay_lines(&saved).join("\n");
        assert!(rendered.contains("  hello"));
        assert!(rendered.contains("hi"));

        assert!(rendered.contains("› Read file"));
        assert!(!rendered.contains("read_file"));
        assert!(!rendered.contains("工具 ·"));
        assert!(!rendered.contains("Session "));
        assert!(!rendered.contains("Created "));
        assert!(!rendered.contains("Updated "));
        assert!(!rendered.contains("Messages "));
        assert!(!rendered.contains("user:"));
        assert!(!rendered.contains("assistant:"));
        assert!(!rendered.contains("system prompt"));
        assert!(!rendered.contains("file contents hidden"));
    }

    #[test]
    fn failed_session_reply_remains_visible_without_model_text() {
        let saved = session::SavedSession {
            id: "failed-reply".into(), created_at: String::new(), updated_at: String::new(),
            messages: vec![
                json!({"role":"user","content":"question"}),
                json!({"role":"assistant","display_status":"failed","content":""}),
                json!({"role":"user","content":"follow-up"}),
                json!({"role":"assistant","display_status":"completed","content":"answer"}),
            ],
        };
        let blocks = interactive_session_replay_blocks(&saved);
        assert!(blocks.iter().any(|block| matches!(block,
            InteractiveReplayBlock::Assistant(text) if text == ui_text(UiText::Failed))));
        assert!(blocks.iter().any(|block| matches!(block,
            InteractiveReplayBlock::Assistant(text) if text == "answer")));
    }

    #[test]
    fn interactive_session_replay_blocks_keep_assistant_markdown_intact() {
        let table = "| 文件 | 说明 |\n|---|---|\n| **`nola`** | CLI 二进制 |";
        let saved = session::SavedSession {
            id: "session-markdown-block".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"user","content":"list files"}),
                json!({"role":"assistant","content":table}),
            ],
        };

        let blocks = interactive_session_replay_blocks(&saved);
        assert!(blocks.iter().any(|block| matches!(
            block,
            InteractiveReplayBlock::Assistant(text) if text == table
        )));
    }

    #[test]
    fn interactive_session_history_replays_consecutive_tools() {
        let saved = session::SavedSession {
            id: "session-tool-block".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"user","content":"run"}),
                json!({"role":"tool","name":"read_file","content":"ok"}),
                json!({"role":"tool","name":"edit_file","tool_calls":[{"id":"call_1","function":{"name":"edit_file","arguments":"{}"}}]}),
                json!({"role":"assistant","content":"done"}),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        assert_eq!(lines[0], "  run");
        assert_eq!(lines[2], "    › Read file", "{lines:#?}");
        assert_eq!(lines[3], "    › Edit file", "{lines:#?}");
        assert!(!lines.iter().any(|line| line.contains("工具 ·")));
        assert!(!lines.iter().any(|line| line.contains("[输出已隐藏]")));
        assert!(lines.iter().any(|line| line == "done"));
        assert_eq!(lines.last().map(String::as_str), Some(""));
    }

    #[test]
    fn interactive_session_history_preserves_repeated_calls_to_the_same_tool() {
        let saved = session::SavedSession {
            id: "session-repeated-tools".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({
                    "role":"assistant",
                    "content":"",
                    "tool_calls":[
                        {"id":"call-1","function":{"name":"bash","arguments":"{\"command\":\"cargo test\"}"}},
                        {"id":"call-2","function":{"name":"bash","arguments":"{\"command\":\"cargo build\"}"}}
                    ]
                }),
                json!({"role":"tool","tool_call_id":"call-1","name":"bash","content":"ok"}),
                json!({"role":"tool","tool_call_id":"call-2","name":"bash","content":"ok"}),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        let bash_lines = lines
            .iter()
            .filter(|line| line.contains("› Run command"))
            .collect::<Vec<_>>();
        assert_eq!(bash_lines.len(), 2, "{lines:#?}");
        assert!(bash_lines.iter().any(|line| line.contains("cargo test")));
        assert!(bash_lines.iter().any(|line| line.contains("cargo build")));
    }

    #[test]
    fn interactive_session_history_returns_all_messages() {
        let messages = (0..10)
            .map(|idx| json!({"role":"user","content":format!("msg {idx}")}))
            .collect();

        let saved = session::SavedSession {
            id: "session-1".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages,
        };

        let lines = interactive_session_replay_lines(&saved);
        assert_eq!(lines.len(), 20);
        assert_eq!(lines[0], "  msg 0");
        assert_eq!(lines[18], "  msg 9");
        assert_eq!(lines[19], "");
    }

    #[test]
    fn interactive_session_history_hides_tool_call_arguments() {
        let saved = session::SavedSession {
            id: "session-tool-call-hidden".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({
                    "role":"user",
                    "content":"run",
                }),
                json!({
                    "role":"assistant",
                    "content":"",
                    "tool_calls":[{
                        "id":"call_1",
                        "type":"function",
                        "function":{"name":"edit_file","arguments":"{\"path\":\"/tmp/a\",\"content\":\"big change\"}"}
                    }]
                }),
                json!({
                    "role":"tool",
                    "name":"edit_file",
                    "content":"diff output should be hidden",
                }),
                json!({
                    "role":"assistant",
                    "content":"done"
                }),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        let serialized = lines.join("\n");

        assert!(serialized.contains("›"));
        assert!(!serialized.contains("工具 ·"));
        assert!(serialized.contains("Edit file"));
        assert!(!serialized.contains("edit_file"));
        assert!(serialized.contains("path=/tmp/a"));
        assert!(!serialized.contains("big change"));
        assert!(!serialized.contains("diff output should be hidden"));
    }

    #[test]
    fn interactive_session_history_shows_safe_tool_args_for_grep() {
        let saved = session::SavedSession {
            id: "session-cautious-tool-args".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({
                    "role":"assistant",
                    "content":"",
                    "tool_calls":[{
                        "id":"call_1",
                        "type":"function",
                        "function":{"name":"grep","arguments":"{\"path\":\"cli/src\",\"pattern\":\"resume\",\"context_lines\":3}"}
                    }]
                }),
                json!({"role":"tool","tool_call_id":"call_1","name":"grep","content":"(no matches)"}),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        let serialized = lines.join("\n");

        assert!(serialized.contains("Search files  path=cli/src pattern=resume"));
        assert!(!serialized.contains("› grep"));
        assert!(!serialized.contains("[输出已隐藏]"));
        assert!(!serialized.contains("context_lines"));
        assert!(!serialized.contains("(no matches)"));
    }

    #[test]
    fn interactive_session_history_hides_stringified_tool_payload() {
        let saved = session::SavedSession {
            id: "session-stringified-tool-payload".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"user","content":"inspect"}),
                json!({
                    "role":"assistant",
                    "content":"{\"tool\":\"read_file\",\"tool_use_id\":\"call-1\",\"result\":\"secret\"}",
                }),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        let serialized = lines.join("\n");

        assert!(serialized.contains("›"));
        assert!(!serialized.contains("工具 ·"));
        assert!(!serialized.contains("secret"));
    }

    #[test]
    fn interactive_session_history_hides_tool_result_contents() {
        let saved = session::SavedSession {
            id: "session-tool-result-only".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"user","content":"inspect"}),
                json!({"role":"tool","name":"read_file","content":"very large file content to hide"}),
                json!({"role":"tool","name":"edit_file","content":"diff content should be hidden"}),
                json!({"role":"assistant","content":"updated"}),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        let serialized = lines.join("\n");
        assert!(serialized.contains("  inspect"));
        assert!(serialized.contains("updated"));
        assert!(!serialized.contains("user:"));
        assert!(!serialized.contains("assistant:"));
        assert!(serialized.contains("› Read file"));
        assert!(serialized.contains("› Edit file"));
        assert!(!serialized.contains("工具 ·"));
        assert!(!serialized.contains("read_file"));
        assert!(!serialized.contains("edit_file"));
        assert!(!serialized.contains("very large file content to hide"));
        assert!(!serialized.contains("diff content should be hidden"));
    }

    #[test]
    fn interactive_session_history_treats_tool_like_roles_as_tool_blocks() {
        let saved = session::SavedSession {
            id: "session-tool-result-role".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"user","content":"read"}),
                json!({
                    "role":"tool_result",
                    "tool_use_id":"call-1",
                    "content":"should be hidden",
                    "result":"should also be hidden",
                }),
                json!({"role":"assistant","content":"done"}),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        let serialized = lines.join("\n");

        assert!(serialized.contains("›"));
        assert!(!serialized.contains("工具 ·"));
        assert!(!serialized.contains("should be hidden"));
    }

    #[test]
    fn interactive_session_history_maps_tool_use_id_for_tool_result_role() {
        let saved = session::SavedSession {
            id: "session-tool-use-id".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({
                    "role":"assistant",
                    "content":[],
                    "tool_calls":[{
                        "id":"call-1",
                        "type":"function",
                        "function":{"name":"read_file","arguments":"{\"path\":\"/tmp/x\"}"},
                    }],
                }),
                json!({
                    "role":"tool_result",
                    "tool_use_id":"call-1",
                    "content":"secret content",
                }),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        let serialized = lines.join("\n");

        assert!(serialized.contains("›"));
        assert!(!serialized.contains("工具 ·"));
        assert!(serialized.contains("Read file"));
        assert!(!serialized.contains("read_file"));
        assert!(!serialized.contains("secret content"));
    }

    #[test]
    fn interactive_session_history_shows_assistant_tool_call_with_block_and_no_empty_line_suppressed(
    ) {
        let saved = session::SavedSession {
            id: "session-assistant-tool-call-only".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"assistant","content":"","tool_calls":[{"type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.txt\"}"}}]}),
                json!({"role":"tool","name":"read_file","content":"alpha"}),
            ],
        };

        let lines = interactive_session_replay_lines(&saved);
        let serialized = lines.join("\n");

        assert!(serialized.contains("›"));
        assert!(!serialized.contains("工具 ·"));
        assert!(!serialized.contains("assistant:"));
        assert!(!serialized.contains("alpha"));
    }

    #[test]
    fn normalize_interactive_history_command() {
        assert_eq!(normalize_interactive_command("/history"), Some("/history"));
        assert_eq!(normalize_interactive_command("/hist"), Some("/history"));
        assert_eq!(normalize_interactive_command("/plan"), Some("/plan"));
        assert_eq!(normalize_interactive_command("/p"), Some("/plan"));
        assert_eq!(normalize_interactive_command("/h"), Some("/help"));
        assert_eq!(normalize_interactive_command("/compact"), Some("/compact"));
        assert_eq!(
            normalize_interactive_command("/executor"),
            Some("/executor")
        );
        assert_eq!(normalize_interactive_command("/exec"), Some("/executor"));
    }

    #[test]
    fn executor_completion_lists_stable_backend_ids() {
        let names = matching_slash_commands_inner("/executor cla", None)
            .into_iter()
            .map(|item| item.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["/executor claude_code"]);
    }

    #[test]
    fn tab_inserts_the_same_executor_candidate_that_the_composer_displays() {
        let config = Config::for_test().unwrap();
        for input in ["/executor", "/executor cla", "/exec cod", "/executor\top"] {
            let displayed = matching_slash_commands_with_config(input, &config).into_iter().next().map(|item| item.name);
            let inserted = first_matching_slash_completion(input, &config).map(|item| item.name);
            assert_eq!(displayed, inserted, "{input}");
            assert!(inserted.is_some(), "{input}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn executor_completion_never_starts_installed_backend_processes() {
        use std::os::unix::fs::PermissionsExt;
        let _lock = env_lock();
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("spawned");
        // A deliberately invalid executable still leaves a marker if discovery
        // starts it. Candidate rendering must not run even --version/auth/model.
        for name in ["codex", "claude", "opencode"] {
            let path = directory.path().join(name);
            fs::write(&path, format!("#!/bin/sh\nprintf called >> '{}'\nexit 1\n", marker.display())).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let previous = std::env::var_os("PATH");
        struct RestorePath(Option<std::ffi::OsString>);
        impl Drop for RestorePath {
            fn drop(&mut self) {
                if let Some(value) = &self.0 { std::env::set_var("PATH", value); }
                else { std::env::remove_var("PATH"); }
            }
        }
        let _restore = RestorePath(previous);
        std::env::set_var("PATH", directory.path());
        let candidates = matching_slash_commands_inner("/executor ", None);
        assert_eq!(candidates.len(), nola_domain::execution_backend::ExecutionBackendKind::ALL.len());
        assert!(!marker.exists(), "typing a candidate must not start another Agent CLI");
    }

    #[test]
    fn executor_command_switches_to_builtin_nola_without_model_routing() {
        let mut config = Config::for_test().unwrap();
        config.execution_backend = "codex".to_string();
        let mut goal_target = None;
        let mut goal_constraints = None;
        let mut previous_workspace = None;
        let mut usage = None;

        let outcome = handle_interactive_slash_command(
            "/executor nola",
            &mut config,
            &mut goal_target,
            &mut goal_constraints,
            &mut previous_workspace,
            &mut usage,
            1,
        )
        .unwrap();

        assert!(matches!(outcome, SlashCommandResult::Handled));
        assert_eq!(config.execution_backend, "nola");
    }

    #[test]
    fn prompt_footer_keeps_the_selected_execution_unit_visible() {
        let mut config = Config::for_test().unwrap();
        config.execution_backend = "codex".to_string();
        config.runtime.compaction.context_window_tokens = Some(128_000);

        let footer = interactive_prompt_footer_line(&config, None, None).unwrap();

        assert!(footer.contains(tr(Message::InteractiveExecutorLabel)));
        assert!(footer.contains("Codex"));
    }

    #[test]
    fn goal_completion_exposes_status_resume_and_off_without_rewriting_targets() {
        let names = matching_slash_commands_inner("/goal ", None)
            .into_iter()
            .map(|item| item.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "/goal status".to_string(),
                "/goal resume".to_string(),
                "/goal off".to_string(),
            ]
        );
        assert_eq!(
            first_matching_slash_completion("/goal r", &Config::for_test().unwrap())
                .map(|item| item.name)
                .as_deref(),
            Some("/goal resume")
        );
        assert!(matching_slash_commands_inner("/goal finish parser", None).is_empty());
    }

    #[test]
    fn goal_prompt_uses_runtime_long_task_lifecycle_without_command_classification() {
        let initial = compose_goal_prompt(
            "finish the requested work",
            "run the workflow",
            "verify the final output",
            false,
            None,
            None,
        );
        assert!(initial.contains("execution_mode=foreground"));
        assert!(initial.contains("bash_background_wait"));
        assert!(initial.contains("intentionally long-running"));
        assert!(initial.contains("Current request: run the workflow"));
        assert!(!initial.contains("nola_goal_control"));

        let continuation = compose_goal_prompt(
            "finish the requested work",
            "run the workflow",
            "verify the final output",
            true,
            None,
            Some("continue from existing evidence"),
        );
        assert!(continuation.contains("Autonomous goal continuation mode"));
        assert!(!continuation.contains("Current request: run the workflow"));
        assert!(continuation.contains("continue from existing evidence"));
        assert!(!continuation.contains("nola_goal_control"));
    }

    #[test]
    fn goal_retry_backoff_is_bounded_and_has_no_attempt_limit() {
        assert_eq!(goal_retry_delay_for_attempt(0), Duration::from_secs(60));
        assert_eq!(goal_retry_delay_for_attempt(1), Duration::from_secs(120));
        assert_eq!(goal_retry_delay_for_attempt(6), Duration::from_secs(3_840));
        assert_eq!(
            goal_retry_delay_for_attempt(10_000),
            Duration::from_secs(3_840)
        );
    }

    #[test]
    fn goal_resume_restores_persisted_target_constraints_and_running_state() {
        let _guard = env_lock();
        let db_root = temporary_session_db_root("goal-resume");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));

        let project_root = db_root.join("workspace");
        fs::create_dir_all(&project_root).unwrap();
        let session_id = "goal-resume-session";
        let mut config = Config::from_args(vec![
            "nola".to_string(),
            "--session".to_string(),
            session_id.to_string(),
            "--project".to_string(),
            project_root.to_string_lossy().to_string(),
            "--endpoint".to_string(),
            "http://127.0.0.1:11434/v1".to_string(),
        ])
        .unwrap();
        config.save_session = true;
        submit_goal_command(
            &config,
            "goal-resume-test:start".to_string(),
            GoalRuntimeCommand::Start {
                id: format!("goal-{session_id}"),
                objective: "finish parser".to_string(),
                constraints: vec!["run focused tests".to_string()],
                requirements: goal_requirement_records("finish parser", "run focused tests"),
                now_epoch_ms: goal_now_epoch_ms(),
            },
        )
        .unwrap();
        submit_goal_command(
            &config,
            "goal-resume-test:pause".to_string(),
            GoalRuntimeCommand::Pause {
                status: GoalStatus::Paused,
                wake_at_epoch_ms: None,
                now_epoch_ms: goal_now_epoch_ms(),
            },
        )
        .unwrap();
        let mut goal_target = None;
        let mut goal_constraints = None;
        let mut previous_workspace = None;
        let mut last_context_usage = None;
        let result = handle_interactive_slash_command(
            "/goal resume",
            &mut config,
            &mut goal_target,
            &mut goal_constraints,
            &mut previous_workspace,
            &mut last_context_usage,
            1,
        )
        .unwrap();

        assert!(matches!(
            result,
            SlashCommandResult::RunGoalPrompt(ref prompt) if prompt.is_empty()
        ));
        assert_eq!(goal_target.as_deref(), Some("finish parser"));
        assert_eq!(goal_constraints.as_deref(), Some("run focused tests"));
        let resumed = session::load_session_goal_projection(session_id)
            .unwrap()
            .and_then(|projection| projection.active)
            .expect("resumed canonical goal");
        assert_eq!(resumed.status, GoalStatus::Running);
        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn interactive_exit_requires_a_slash_command() {
        for command in ["/exit", "/quit", "/q", "/e", "/EXIT"] {
            assert!(is_exit_command(command), "{command}");
        }
        for message in ["q", "quit", "exit", ":q", "exit now", "/exit now"] {
            assert!(!is_exit_command(message), "{message}");
        }
    }

    #[test]
    fn interactive_session_history_handles_empty_messages() {
        let saved = session::SavedSession {
            id: "session-1".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: Vec::new(),
        };

        let lines = interactive_session_replay_lines(&saved);
        assert_eq!(lines, vec!["  暂无消息".to_string()]);
    }

    #[test]
    fn interactive_session_replay_user_inputs_are_extracted() {
        let saved = session::SavedSession {
            id: "session-replay".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"user","content":"first"}),
                json!({"role":"assistant","content":"ack"}),
                json!({"role":"user","content":"second"}),
            ],
        };

        let inputs = interactive_session_replay_user_inputs(&saved);
        assert_eq!(inputs, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn interactive_session_replay_user_inputs_handle_structured_payloads() {
        let saved = session::SavedSession {
            id: "session-replay-structured".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"user","content":{"type":"text","text":"line1\nline2"}}),
                json!({"role":"assistant","content":"ack"}),
                json!({"role":"user","content":[{"text":"first part"},{"value":"second part"}]}),
                json!({"role":"assistant","content":"again"}),
                json!({"role":"user","content":{"parts":[{"type":"text","text":"third"}]}}),
            ],
        };

        let inputs = interactive_session_replay_user_inputs(&saved);
        assert_eq!(
            inputs,
            vec![
                "line1".to_string(),
                "line2".to_string(),
                "first part".to_string(),
                "second part".to_string(),
                "third".to_string()
            ]
        );
    }

    #[test]
    fn interactive_session_replay_user_inputs_skips_tool_only_user_messages() {
        let saved = session::SavedSession {
            id: "session-replay-tool-only-user".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            messages: vec![
                json!({"role":"user","content":"ask for read"}),
                json!({"role":"tool","name":"read_file","content":"secret"}),
                json!({"role":"assistant","content":"ok"}),
                json!({"role":"user","content":{"type":"tool_result","content":"should skip"}}),
            ],
        };

        let inputs = interactive_session_replay_user_inputs(&saved);
        assert_eq!(inputs, vec!["ask for read".to_string()]);
    }

    #[test]
    fn load_interactive_session_for_command_prefers_explicit_session_id() {
        let _guard = env_lock();
        let db_root = temporary_session_db_root("explicit-session");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));

        let project_root = Path::new("/tmp").join("nola-cli-session-test-explicit");
        let session_id = "explicit-session".to_string();

        let mut store = session::SessionStore::open(
            Some(&session_id),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        store
            .save_messages(vec![json!({"role":"user","content":"first"})])
            .unwrap();

        let config = Config::from_args(vec![
            "nola".to_string(),
            "--session".to_string(),
            session_id,
            "--project".to_string(),
            project_root.to_string_lossy().to_string(),
            "--endpoint".to_string(),
            "http://127.0.0.1:11434/v1".to_string(),
        ])
        .unwrap();

        let loaded = load_interactive_session_for_command(&config)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, "explicit-session");
        assert_eq!(loaded.messages.len(), 1);

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn load_interactive_session_for_command_allows_missing_explicit_session_id() {
        let _guard = env_lock();
        let db_root = temporary_session_db_root("missing-explicit-session");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));

        let project_root = Path::new("/tmp").join("nola-cli-session-test-missing-explicit");
        let missing_session = "missing-explicit-session-id".to_string();

        let config = Config::from_args(vec![
            "nola".to_string(),
            "--session".to_string(),
            missing_session,
            "--project".to_string(),
            project_root.to_string_lossy().to_string(),
            "--endpoint".to_string(),
            "http://127.0.0.1:11434/v1".to_string(),
        ])
        .unwrap();

        let loaded = load_interactive_session_for_command(&config).unwrap();
        assert!(loaded.is_none());

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn load_interactive_session_for_command_uses_latest_session_for_project_with_continue() {
        let _guard = env_lock();
        let db_root = temporary_session_db_root("continue");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));

        let project_root = Path::new("/tmp").join("nola-cli-session-test-continue");
        let first_session = "continue-first".to_string();
        let second_session = "continue-second".to_string();

        let mut first = session::SessionStore::open(
            Some(&first_session),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        first
            .save_messages(vec![
                json!({"role":"user","content":"first"}),
                json!({"role":"assistant","content":"reply1"}),
            ])
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let mut second = session::SessionStore::open(
            Some(&second_session),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        second
            .save_messages(vec![
                json!({"role":"user","content":"second"}),
                json!({"role":"assistant","content":"reply2"}),
            ])
            .unwrap();

        let config = Config::from_args(vec![
            "nola".to_string(),
            "--continue".to_string(),
            "--project".to_string(),
            project_root.to_string_lossy().to_string(),
            "--endpoint".to_string(),
            "http://127.0.0.1:11434/v1".to_string(),
        ])
        .unwrap();

        let loaded = load_interactive_session_for_command(&config)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, second_session);
        assert_eq!(loaded.messages.len(), 2);

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn restore_interactive_session_messages_returns_saved_session_and_turns() {
        let _guard = env_lock();
        let db_root = temporary_session_db_root("restore-interactive");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));

        let project_root = Path::new("/tmp").join("nola-cli-session-restore");
        let session_id = "restore-session".to_string();
        let mut store = session::SessionStore::open(
            Some(&session_id),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        store
            .save_messages(vec![
                json!({"role":"user","content":"first"}),
                json!({"role":"assistant","content":"ack"}),
                json!({"role":"user","content":"second"}),
            ])
            .unwrap();

        let config = Config::from_args(vec![
            "nola".to_string(),
            "--session".to_string(),
            session_id,
            "--project".to_string(),
            project_root.to_string_lossy().to_string(),
            "--endpoint".to_string(),
            "http://127.0.0.1:11434/v1".to_string(),
        ])
        .unwrap();

        let mut started = false;
        let mut turn = 1usize;
        let mut input_history: Vec<String> = Vec::new();
        let saved = restore_interactive_session_messages(
            &config,
            &mut started,
            &mut turn,
            &mut input_history,
        )
        .unwrap()
        .expect("restore returns saved session");

        assert!(started);
        assert_eq!(turn, 3);
        assert_eq!(
            input_history,
            vec!["first".to_string(), "second".to_string()]
        );
        assert_eq!(saved.id, "restore-session");

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn restore_interactive_session_messages_ignores_tool_only_user_outputs_in_turns() {
        let _guard = env_lock();
        let db_root = temporary_session_db_root("restore-interactive-tool-only");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));

        let project_root = Path::new("/tmp").join("nola-cli-session-restore-tool");
        let session_id = "restore-session-tool-only".to_string();
        let mut store = session::SessionStore::open(
            Some(&session_id),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        store
            .save_messages(vec![
                json!({"role":"user","content":"first"}),
                json!({"role":"assistant","content":"ack"}),
                json!({
                    "role":"user",
                    "content":{"type":"tool_result","tool_use_id":"call-1","content":"hidden"},
                }),
                json!({"role":"user","content":"second"}),
            ])
            .unwrap();

        let config = Config::from_args(vec![
            "nola".to_string(),
            "--session".to_string(),
            session_id,
            "--project".to_string(),
            project_root.to_string_lossy().to_string(),
            "--endpoint".to_string(),
            "http://127.0.0.1:11434/v1".to_string(),
        ])
        .unwrap();

        let mut started = false;
        let mut turn = 1usize;
        let mut input_history: Vec<String> = Vec::new();
        let _ = restore_interactive_session_messages(
            &config,
            &mut started,
            &mut turn,
            &mut input_history,
        )
        .unwrap();

        assert_eq!(turn, 3);
        assert_eq!(
            input_history,
            vec!["first".to_string(), "second".to_string()]
        );

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn resume_session_list_lines_are_carriage_return_safe() {
        let sessions = vec![ResumeSessionOption {
            id: "session-1".to_string(),
            updated_at: "2026-07-10T10:23:40.535939706+08:00".to_string(),
            last_user_message_at: "2026-07-10T10:23:40.535939706+08:00".to_string(),
            first_user_message: "resume".to_string(),
            first_user_message_preview: "resume".to_string(),
        }];

        let rendered = resume_session_list_lines(&sessions, 0, 0, 10);

        assert!(rendered.contains("\r\n"));
        assert!(!rendered.replace("\r\n", "").contains('\n'));
        assert!(rendered.contains("resume"));
        assert!(!rendered.contains("535939706"));
    }

    #[test]
    fn two_ten_million_character_sessions_restore_and_switch_with_bounded_terminal_views() {
        const CONVERSATION_CHARACTERS: usize = 10_000_000;
        const SWITCH_COUNT: usize = 12;
        const DEFAULT_COLD_RESTORE_BUDGET_MS: u64 = 8_000;

        fn fixture(marker: &str) -> Vec<Value> {
            let user =
                format!("Analyze the {marker} fixture and keep the tool and Skill timeline.");
            let tool_result = format!("{marker}: file inspection completed");
            let skill_result = format!("{marker}: analyze-data Skill completed");
            let fixed =
                user.chars().count() + tool_result.chars().count() + skill_result.chars().count();
            let body = format!(
                "{marker}\n{}",
                "x".repeat(CONVERSATION_CHARACTERS - fixed - marker.chars().count() - 1)
            );
            let messages = vec![
                json!({"role":"user","content":user}),
                json!({"role":"assistant","content":body}),
                json!({
                    "role":"assistant",
                    "content":"",
                    "tool_calls":[{
                        "id":format!("{marker}-read"),
                        "type":"function",
                        "function":{"name":"read_file","arguments":"{\"path\":\"report.csv\"}"}
                    }]
                }),
                json!({
                    "role":"tool",
                    "tool_call_id":format!("{marker}-read"),
                    "name":"read_file",
                    "content":tool_result
                }),
                json!({
                    "role":"assistant",
                    "content":"",
                    "tool_calls":[{
                        "id":format!("{marker}-skill"),
                        "type":"function",
                        "function":{"name":"skill_run","arguments":"{\"skill_id\":\"analyze-data\"}"}
                    }]
                }),
                json!({
                    "role":"tool",
                    "tool_call_id":format!("{marker}-skill"),
                    "name":"skill_run",
                    "content":skill_result
                }),
            ];
            let content_characters: usize = messages
                .iter()
                .filter_map(|message| message.get("content").and_then(Value::as_str))
                .map(|text| text.chars().count())
                .sum();
            assert_eq!(content_characters, CONVERSATION_CHARACTERS);
            messages
        }

        fn persist_accumulated_fixture(database: &Path, session_id: &str, messages: &[Value]) {
            let mut connection = rusqlite::Connection::open(database).unwrap();
            connection.busy_timeout(Duration::from_secs(2)).unwrap();
            let transaction = connection.transaction().unwrap();
            for (index, message) in messages.iter().enumerate() {
                let ordinal = index as i64 + 1;
                let message_id = format!("{session_id}-message-{ordinal}");
                let provider_role = message
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("assistant");
                let canonical_role = if provider_role == "user" {
                    "user"
                } else {
                    "assistant"
                };
                let content_text = message
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let content_json = serde_json::to_string(message).unwrap();
                transaction
                    .execute(
                        "INSERT INTO messages(
                            id, conversation_id, run_id, role, status, ordinal,
                            created_at_ms, completed_at_ms
                         ) VALUES(?1, ?2, NULL, ?3, 'completed', ?4, ?5, ?5)",
                        rusqlite::params![
                            message_id,
                            session_id,
                            canonical_role,
                            ordinal,
                            1_800_000_000_000_i64 + ordinal,
                        ],
                    )
                    .unwrap();
                transaction
                    .execute(
                        "INSERT INTO message_parts(
                            id, message_id, kind, ordinal, state, content_text,
                            content_json, content_digest, created_at_ms, updated_at_ms
                         ) VALUES(?1, ?2, 'provider_message', 1, 'completed', ?3,
                                  ?4, NULL, ?5, ?5)",
                        rusqlite::params![
                            format!("{message_id}-part"),
                            message_id,
                            content_text,
                            content_json,
                            1_800_000_000_000_i64 + ordinal,
                        ],
                    )
                    .unwrap();
            }
            transaction
                .execute(
                    "UPDATE conversations
                     SET revision = revision + 1, updated_at_ms = ?2
                     WHERE id = ?1",
                    rusqlite::params![session_id, 1_800_000_000_100_i64],
                )
                .unwrap();
            transaction.commit().unwrap();
        }

        let _guard = env_lock();
        let db_root = temporary_session_db_root("ten-million-switch");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));
        let project_root = db_root.join("workspace");
        fs::create_dir_all(&project_root).unwrap();
        let database = db_root.join("nola.db");

        for (session_id, marker) in [("long-alpha", "ALPHA"), ("long-beta", "BETA")] {
            let store = session::SessionStore::open(
                Some(session_id),
                false,
                project_root.to_string_lossy().as_ref(),
            )
            .unwrap();
            drop(store);
            persist_accumulated_fixture(&database, session_id, &fixture(marker));
        }
        let empty = session::SessionStore::open(
            Some("empty-newest"),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        drop(empty);
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute(
                "UPDATE conversations
                 SET revision = revision + 1, updated_at_ms = ?2
                 WHERE id = ?1",
                rusqlite::params!["empty-newest", i64::MAX - 1],
            )
            .unwrap();
        let work_store = crate::store::NolaStore::open(&database).unwrap();
        for (session_id, marker) in [("long-alpha", "ALPHA"), ("long-beta", "BETA")] {
            // Use the real shell producer so the Work projection receives its
            // complete metadata contract, not an Agent-shaped shell record.
            let task = crate::store::DbBackgroundTask {
        process_sandbox: None,
                terminal: None,
                id: format!("work-{marker}"),
                project_root: project_root.to_string_lossy().to_string(),
                command: format!("printf '{marker}'"),
                cwd: project_root.to_string_lossy().to_string(),
                pid: std::process::id(),
                stdout_path: project_root.join(format!("{marker}.stdout")).to_string_lossy().into_owned(),
                stderr_path: project_root.join(format!("{marker}.stderr")).to_string_lossy().into_owned(),
                status_path: project_root.join(format!("{marker}.status")).to_string_lossy().into_owned(),
                started_at_ms: 1_800_000_000_000_i64,
                owner_session_id: Some(session_id.to_string()),
                output_contract_json: None,
                sop_invocation: None,
            };
            work_store.save_background_task_with_scheduling(&task, nola_application::RuntimeTaskScheduling {
                priority: nola_application::RuntimeTaskPriority::Normal,
                queued_at_ms: task.started_at_ms,
                resource_budget: nola_application::RuntimeTaskResourceBudget::default(),
                output_bytes: 0,
                resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
                budget_exhaustion: None,
            })
            .unwrap();
            assert!(work_store
                .claim_runtime_task(
                    &task.id,
                    &format!("worker-{marker}"),
                    &format!("generation-{marker}"),
                    &format!("owner-{marker}"),
                    1_800_000_000_110_i64,
                    i64::MAX,
                )
                .unwrap()
                .is_some());
        }

        let list_started = Instant::now();
        let listed = session::list_sessions_with_first_user_message_for_project_with_limit(
            project_root.to_string_lossy().as_ref(),
            128,
        )
        .unwrap();
        let list_elapsed = list_started.elapsed();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|entry| entry.id != "empty-newest"));
        assert!(listed
            .iter()
            .all(|entry| entry.first_user_message.len() < 512));
        assert!(
            list_elapsed < Duration::from_secs(1),
            "resume list took {list_elapsed:?}"
        );

        let cold_started = Instant::now();
        let cold = session::load_interactive_session_snapshot("long-alpha").unwrap();
        let cold_elapsed = cold_started.elapsed();
        assert_eq!(cold.total_message_count, 6);
        assert_eq!(cold.total_user_turns, 1);
        let cold_serialized = serde_json::to_vec(&cold.saved.messages).unwrap();
        assert!(
            cold_serialized.len() < 400_000,
            "terminal projection retained {} bytes",
            cold_serialized.len()
        );
        let cold_lines = interactive_session_replay_lines(&cold.saved).join("\n");
        let projected_json = String::from_utf8(cold_serialized.clone()).unwrap();
        assert!(projected_json.contains("read_file"));
        assert!(projected_json.contains("skill_run"));
        assert!(cold_lines.contains("report.csv"));
        assert!(
            cold_lines.to_ascii_lowercase().contains("skill") || cold_lines.contains("技能"),
            "Skill call was not visible in terminal replay: {cold_lines}"
        );
        assert!(cold_lines.len() < 500_000);

        let mut switch_times = Vec::with_capacity(SWITCH_COUNT);
        let mut work_times = Vec::with_capacity(SWITCH_COUNT);
        let mut steering_times = Vec::with_capacity(SWITCH_COUNT);
        for index in 0..SWITCH_COUNT {
            let session_id = if index % 2 == 0 {
                "long-beta"
            } else {
                "long-alpha"
            };
            let started = Instant::now();
            let snapshot = session::load_interactive_session_snapshot(session_id).unwrap();
            let rendered = interactive_session_replay_lines(&snapshot.saved);
            assert!(rendered.iter().any(|line| {
                line.to_ascii_lowercase().contains("skill") || line.contains("技能")
            }));
            switch_times.push(started.elapsed());

            let mut config = Config::from_args(vec![
                "nola".to_string(),
                "--session".to_string(),
                session_id.to_string(),
                "--project".to_string(),
                project_root.to_string_lossy().to_string(),
                "--endpoint".to_string(),
                "http://127.0.0.1:11434/v1".to_string(),
            ])
            .unwrap();
            config.session_id = Some(session_id.to_string());
            let work_started = Instant::now();
            let projected_work = crate::work::collect(&config).unwrap();
            work_times.push(work_started.elapsed());
            assert_eq!(
                projected_work
                    .iter()
                    .filter(|item| item.session_id.as_deref() == Some(session_id))
                    .count(),
                1,
                "session={session_id} root={} work={projected_work:?}",
                config.project_root.display()
            );

            let steering_started = Instant::now();
            let cancellation = crate::interaction::ExecutionCancellation::default();
            cancellation.bind_session_input(session_id, crate::runtime_control::InputRetention::Persistent).unwrap();
            cancellation.queue_runtime_input(crate::runtime_input::RuntimeInput::UserFollowUp(
                format!("steer {session_id} at switch {index}")
            )).unwrap();
            let lease_owner = format!("switch-consumer-{index}");
            let command = crate::agent_command_port::claim_next_agent_command(
                session_id,
                &lease_owner,
                index as i64 + 1,
                i64::MAX,
            )
            .unwrap()
            .expect("durable steering command");
            let nola_core::agent::host::AgentBoundaryInput::Submission { submission } =
                crate::agent_command_port::decode_claimed(&command).unwrap()
            else {
                panic!("expected user submission");
            };
            assert!(matches!(
                submission,
                nola_core::agent::AgentSubmission::UserInput {
                    delivery: nola_core::agent::AgentInputDelivery::NextTurn,
                    ..
                }
            ));
            assert!(crate::agent_command_port::acknowledge_agent_command(
                session_id,
                &command.command_id,
                &lease_owner,
            )
            .unwrap());
            steering_times.push(steering_started.elapsed());
        }
        switch_times.sort_unstable();
        work_times.sort_unstable();
        steering_times.sort_unstable();
        let p95 = switch_times[(SWITCH_COUNT * 95 / 100).min(SWITCH_COUNT - 1)];
        let work_p95 = work_times[(SWITCH_COUNT * 95 / 100).min(SWITCH_COUNT - 1)];
        let steering_p95 = steering_times[(SWITCH_COUNT * 95 / 100).min(SWITCH_COUNT - 1)];
        let average = switch_times.iter().sum::<Duration>() / SWITCH_COUNT as u32;
        eprintln!(
            "CLI 10M switch metrics: list={:?}, cold={:?}, average={:?}, p95={:?}, work_p95={:?}, steering_p95={:?}, projection={} bytes",
            list_elapsed,
            cold_elapsed,
            average,
            p95,
            work_p95,
            steering_p95,
            cold_serialized.len()
        );
        let cold_restore_budget_ms = std::env::var("NOLA_CLI_10M_COLD_RESTORE_BUDGET_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_COLD_RESTORE_BUDGET_MS);
        assert!(
            cold_elapsed < Duration::from_millis(cold_restore_budget_ms),
            "cold restore took {cold_elapsed:?}; budget={cold_restore_budget_ms}ms (set NOLA_CLI_10M_COLD_RESTORE_BUDGET_MS=3000 for the strict performance gate)"
        );
        assert!(p95 < Duration::from_secs(1), "p95 switch took {p95:?}");
        assert!(
            work_p95 < Duration::from_secs(1),
            "p95 Work query took {work_p95:?}"
        );
        assert!(
            steering_p95 < Duration::from_secs(1),
            "p95 durable steering took {steering_p95:?}"
        );

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn five_thousand_conversations_list_without_n_plus_one_queries() {
        let _guard = env_lock();
        let db_root = temporary_session_db_root("five-thousand-resume-list");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        let database = db_root.join("nola.db");
        let project_root = db_root.join("target-project");
        let other_root = db_root.join("other-project");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&other_root).unwrap();
        unsafe { std::env::set_var("NOLA_DB_PATH", &database) };
        let bootstrap = session::SessionStore::open(
            Some("resume-list-bootstrap"),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        drop(bootstrap);

        let mut connection = rusqlite::Connection::open(&database).unwrap();
        let transaction = connection.transaction().unwrap();
        for index in 0..5_000usize {
            let conversation_id = format!("resume-{index:05}");
            let timestamp = 1_800_000_000_000_i64 + index as i64;
            transaction
                .execute(
                    "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms)
                     VALUES(?1,?2,1,?3,?3)",
                    rusqlite::params![conversation_id, format!("Conversation {index}"), timestamp],
                )
                .unwrap();
            let root = if index % 2 == 0 {
                project_root.to_string_lossy().into_owned()
            } else {
                other_root.to_string_lossy().into_owned()
            };
            transaction
                .execute(
                    "INSERT INTO conversation_settings(conversation_id,settings_json,updated_at_ms)
                     VALUES(?1,?2,?3)",
                    rusqlite::params![
                        conversation_id,
                        json!({"project_root":root}).to_string(),
                        timestamp
                    ],
                )
                .unwrap();
            if index % 97 == 0 || index == 4_998 {
                continue;
            }
            let message_id = format!("resume-message-{index:05}");
            transaction
                .execute(
                    "INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms,completed_at_ms)
                     VALUES(?1,?2,NULL,'user','completed',0,?3,?3)",
                    rusqlite::params![message_id, conversation_id, timestamp],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
                     VALUES(?1,?2,'text',0,'completed',?3,NULL,?4,?4)",
                    rusqlite::params![
                        format!("resume-part-{index:05}"),
                        message_id,
                        format!("message {index}"),
                        timestamp
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();

        let started = Instant::now();
        let listed = session::list_sessions_with_first_user_message_for_project_with_limit(
            project_root.to_string_lossy().as_ref(),
            128,
        )
        .unwrap();
        let elapsed = started.elapsed();

        assert_eq!(listed.len(), 128);
        assert!(listed.iter().all(|entry| entry.id != "resume-04998"));
        assert_eq!(listed[0].id, "resume-04996");
        assert!(
            listed
                .windows(2)
                .all(|pair| pair[0].updated_at >= pair[1].updated_at),
            "resume list lost descending update order"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "5k resume list took {elapsed:?}"
        );
        eprintln!(
            "CLI 5k resume-list metrics: elapsed={elapsed:?}, returned={}",
            listed.len()
        );

        unsafe { std::env::remove_var("NOLA_DB_PATH") };
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn ten_thousand_tool_events_restore_through_a_bounded_terminal_window() {
        const TOOL_EVENT_COUNT: usize = 10_000;

        let _guard = env_lock();
        let db_root = temporary_session_db_root("ten-thousand-tool-events");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));
        let project_root = db_root.join("workspace");
        fs::create_dir_all(&project_root).unwrap();
        let database = db_root.join("nola.db");
        let store = session::SessionStore::open(
            Some("tool-event-window"),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        drop(store);

        let mut connection = rusqlite::Connection::open(&database).unwrap();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO messages(
                    id, conversation_id, run_id, role, status, ordinal,
                    created_at_ms, completed_at_ms
                 ) VALUES('tool-window-user', 'tool-event-window', NULL, 'user',
                          'completed', 1, 1800000000001, 1800000000001)",
                [],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO message_parts(
                    id, message_id, kind, ordinal, state, content_text,
                    content_json, content_digest, created_at_ms, updated_at_ms
                 ) VALUES('tool-window-user-part', 'tool-window-user', 'provider_message',
                          1, 'completed', 'inspect many files',
                          '{\"role\":\"user\",\"content\":\"inspect many files\"}',
                          NULL, 1800000000001, 1800000000001)",
                [],
            )
            .unwrap();

        for index in 0..TOOL_EVENT_COUNT {
            let ordinal = index as i64 + 2;
            let message_id = format!("tool-window-message-{index:05}");
            let call_id = format!("window-call-{index:05}");
            let envelope = if index % 2 == 0 {
                json!({
                    "role":"assistant",
                    "content":"",
                    "tool_calls":[{
                        "id":call_id,
                        "type":"function",
                        "function":{
                            "name":"read_file",
                            "arguments":serde_json::to_string(&json!({
                                "path":format!("file-{index:05}.txt")
                            })).unwrap()
                        }
                    }]
                })
            } else {
                json!({
                    "role":"tool",
                    "tool_call_id":format!("window-call-{:05}", index - 1),
                    "name":"read_file",
                    "content":format!("result-{index:05}")
                })
            };
            let content_text = envelope
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default();
            transaction
                .execute(
                    "INSERT INTO messages(
                        id, conversation_id, run_id, role, status, ordinal,
                        created_at_ms, completed_at_ms
                     ) VALUES(?1, 'tool-event-window', NULL, 'assistant', 'completed',
                              ?2, ?3, ?3)",
                    rusqlite::params![message_id, ordinal, 1_800_000_000_000_i64 + ordinal],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO message_parts(
                        id, message_id, kind, ordinal, state, content_text,
                        content_json, content_digest, created_at_ms, updated_at_ms
                     ) VALUES(?1, ?2, 'provider_message', 1, 'completed', ?3, ?4,
                              NULL, ?5, ?5)",
                    rusqlite::params![
                        format!("{message_id}-part"),
                        message_id,
                        content_text,
                        serde_json::to_string(&envelope).unwrap(),
                        1_800_000_000_000_i64 + ordinal,
                    ],
                )
                .unwrap();
        }
        transaction
            .execute(
                "UPDATE conversations
                 SET revision = revision + 1, updated_at_ms = 1800000011000
                 WHERE id = 'tool-event-window'",
                [],
            )
            .unwrap();
        transaction.commit().unwrap();

        let started = Instant::now();
        let snapshot = session::load_interactive_session_snapshot("tool-event-window").unwrap();
        let lines = interactive_session_replay_lines(&snapshot.saved);
        let elapsed = started.elapsed();
        let serialized = serde_json::to_vec(&snapshot.saved.messages).unwrap();
        let serialized_text = String::from_utf8_lossy(&serialized);
        let rendered = lines.join("\n");

        assert_eq!(snapshot.total_message_count, TOOL_EVENT_COUNT + 1);
        assert!(snapshot.saved.messages.len() <= 96);
        assert!(
            serialized.len() < 1_000_000,
            "projection={} bytes",
            serialized.len()
        );
        assert!(rendered.contains("file-09998.txt"), "{rendered}");
        assert!(
            serialized_text.contains("result-09999"),
            "{serialized_text}"
        );
        assert!(
            !rendered.contains("result-09999"),
            "tool results must stay out of compact history"
        );
        assert!(
            !rendered.contains("file-00000.txt"),
            "old tool event escaped the window"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "10k tool-event restore took {elapsed:?}"
        );
        eprintln!(
            "CLI 10k tool-event metrics: restore={elapsed:?}, messages={}, projection={} bytes, rendered_lines={}",
            snapshot.saved.messages.len(),
            serialized.len(),
            lines.len()
        );

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn resume_session_preview_counts_unicode_characters_not_terminal_columns() {
        assert_eq!(
            resume_session_message_preview("第二条较新的历史消息用于恢复", 10),
            "第二条较新的历史消息…"
        );
        assert_eq!(
            resume_session_message_preview("hello   world", 10),
            "hello worl…"
        );
    }

    #[test]
    fn resume_session_recent_timestamp_is_less_than_one_minute() {
        let timestamp = Local::now().to_rfc3339();
        assert_eq!(
            resume_session_relative_time(&timestamp).as_deref(),
            Some("<1m")
        );
    }

    #[test]
    fn composer_input_multiline_tracks_cursor_across_unicode_text() {
        let line = "ab你好cd";
        let cursor = "ab你".len();
        let (lines, cursor_row, cursor_col) = composer_input_multiline(line, cursor, 4, "> ");
        // "ab你好" width=6 > 4, 所以折行: "ab你" | "好cd"
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, "ab你");
        assert_eq!(lines[1].0, "好cd");
        // 光标位于换行后的 "好" 之前，应显示在第二行开头。
        assert_eq!(cursor_row, 1);
        assert_eq!(cursor_col, 2); // prefix "> " width=2

        let (lines, cursor_row, cursor_col) = composer_input_multiline("abcdef", 5, 3, "> ");
        // "abcdef" width=6 > 3, 折行: "abc" | "def"
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, "abc");
        assert_eq!(lines[1].0, "def");
        // 光标在 "def" 中 "de" 之后，第二行
        assert_eq!(cursor_row, 1);
        assert_eq!(cursor_col, 2 + 2); // prefix "> " width=2 + "de" width=2
    }

    #[test]
    fn composer_footer_never_reaches_the_terminal_wrap_column() {
        let rendered = composer_prompt_footer_line(
            "model-with-a-very-long-name · tools=auto · network=allow\n\
             Provider: 127.0.0.1:8080 · Endpoint: http://127.0.0.1:8080/v1 · Working dir: /tmp/a/very/long/workspace",
        );
        let terminal_width = terminal_columns().saturating_sub(1).max(1);
        for line in rendered.lines() {
            let visible = strip_ansi_sequences(line)
                .trim_start_matches('\r')
                .to_string();
            assert!(
                display_width(&visible) <= terminal_width,
                "footer line reached the terminal wrap column: width={} limit={} line={visible:?}",
                display_width(&visible),
                terminal_width,
            );
        }
    }

    #[test]
    fn composer_input_multiline_handles_explicit_newlines() {
        // 模拟粘贴多行文本：包含 \n 的输入
        let line = "hello\nworld\nfoo";
        let cursor = line.len(); // 光标在末尾
        let (lines, cursor_row, _cursor_col) = composer_input_multiline(line, cursor, 20, "> ");
        // 应该有三行，每行对应一个 \n 分隔的片段
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].0, "hello");
        assert_eq!(lines[1].0, "world");
        assert_eq!(lines[2].0, "foo");
        // 光标在最后一行
        assert_eq!(cursor_row, 2);
    }

    #[test]
    fn composer_input_multiline_newline_with_width_wrap() {
        // \n 换行 + 宽度折行混合
        let line = "ab\ncdefgh";
        let cursor = line.len();
        let (lines, cursor_row, _cursor_col) = composer_input_multiline(line, cursor, 4, "> ");
        // "ab" (width=2) -> 第0行
        // "cdefgh" (width=6>4) -> 折为 "cdef" | "gh"
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].0, "ab");
        assert_eq!(lines[1].0, "cdef");
        assert_eq!(lines[2].0, "gh");
        assert_eq!(cursor_row, 2);
    }

    #[test]
    fn composer_word_boundary_preserves_unicode_boundaries() {
        let line = "hello 世界";
        let boundary = previous_word_boundary(line, line.len());

        assert_eq!(&line[..boundary], "hello ");
        assert!(line.is_char_boundary(boundary));
    }

    #[test]
    fn resolve_continue_session_id_prefers_project_latest_session() {
        let _guard = env_lock();
        let db_root = temporary_session_db_root("resolve-continue");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));

        let project_root = Path::new("/tmp").join("nola-cli-session-test-resolve");

        let mut alpha_old = session::SessionStore::open(
            Some("alpha-old"),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        alpha_old
            .save_messages(vec![json!({"role":"user","content":"old alpha"})])
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let mut alpha_new = session::SessionStore::open(
            Some("alpha-new"),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();
        alpha_new
            .save_messages(vec![json!({"role":"user","content":"new alpha"})])
            .unwrap();

        let config = Config::from_args(vec![
            "nola".to_string(),
            "--continue".to_string(),
            "--project".to_string(),
            project_root.to_string_lossy().to_string(),
            "--endpoint".to_string(),
            "http://127.0.0.1:11434/v1".to_string(),
        ])
        .unwrap();
        let resolved = resolve_continue_session_id(&config).unwrap().unwrap();
        assert_eq!(resolved, "alpha-new");

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn resolve_continue_session_id_does_not_fallback_to_global() {
        let _guard = env_lock();
        let db_root = temporary_session_db_root("resolve-continue-no-fallback");
        let _ = fs::remove_dir_all(&db_root);
        fs::create_dir_all(&db_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", db_root.join("nola.db"));

        let project_root = Path::new("/tmp").join("nola-cli-session-test-resolve-no-fallback");

        let mut other_project_session = session::SessionStore::open(
            Some("other-project-session"),
            false,
            "/tmp/another-project",
        )
        .unwrap();
        other_project_session
            .save_messages(vec![json!({"role":"user","content":"other"})])
            .unwrap();

        let _local_empty_session = session::SessionStore::open(
            Some("local-empty"),
            false,
            project_root.to_string_lossy().as_ref(),
        )
        .unwrap();

        let config = Config::from_args(vec![
            "nola".to_string(),
            "--continue".to_string(),
            "--project".to_string(),
            project_root.to_string_lossy().to_string(),
            "--endpoint".to_string(),
            "http://127.0.0.1:11434/v1".to_string(),
        ])
        .unwrap();

        let resolved = resolve_continue_session_id(&config).unwrap();
        assert!(resolved.is_none());

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(db_root);
    }

    #[test]
    fn background_footer_is_visible_and_preserves_debug_lines() {
        let rendered = format_interactive_background_footer(
            "context 12% · model\nworking directory: /tmp/project",
            2,
            120,
        );

        assert_eq!(rendered.lines().next(), Some("context 12% · model"));
        assert!(rendered.lines().nth(1).unwrap().starts_with("● "));
        assert!(rendered.contains(": 2 · /tasks"));
        assert!(rendered.ends_with("working directory: /tmp/project"));
    }

    #[test]
    fn desktop_host_request_hydration_keeps_prompt_and_secret_out_of_arguments() {
        let mut config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--host-request-stdin",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap().host_invocation_request().expect("host request");

        crate::native_host_request::apply_desktop_host_request(
            &mut config,
            r#"{"prompt":"create a private document","submission_id":"turn-42","user_message_id":"user-42","assistant_message_id":"assistant-42","input_origin":"scheduled_action","context_components":[{"kind":"system","id":"document-template","content":"Use the selected document template.","source":{"platform":"macos","title":"Selected document template"}}],"api_key":"test-secret"}"#,
        )
        .unwrap();

        assert_eq!(config.input.message, "create a private document");
        assert_eq!(
            config.input.host_input_origin,
            nola_core::agent::AgentInputOrigin::ScheduledAction
        );
        assert_eq!(config.model_settings.api_key, "test-secret");
        assert_eq!(config.session.host_submission_id.as_deref(), Some("turn-42"));
        assert_eq!(config.session.host_user_message_id.as_deref(), Some("user-42"));
        assert_eq!(
            config.session.host_assistant_message_id.as_deref(),
            Some("assistant-42")
        );
        assert_eq!(config.input.host_context_components.len(), 1);
        assert_eq!(
            config.input.host_context_components[0].content,
            Value::String("Use the selected document template.".to_string())
        );
    }

    #[test]
    fn desktop_host_request_rejects_partial_canonical_identity() {
        let mut config = Config::from_args(
            ["run", "--host-request-stdin"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap().host_invocation_request().expect("host request");

        let error = crate::native_host_request::apply_desktop_host_request(
            &mut config,
            r#"{"prompt":"hello","submission_id":"turn-only"}"#,
        )
        .unwrap_err();

        assert!(error.contains("requires submission_id"));
    }

    #[test]
    fn desktop_host_request_without_api_key_preserves_provider_config() {
        let mut config = Config::from_args(
            ["run", "--host-request-stdin"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap().host_invocation_request().expect("host request");
        config.model_settings.api_key = "configured-secret".to_string();

        crate::native_host_request::apply_desktop_host_request(&mut config, r#"{"prompt":"hello"}"#).unwrap();

        assert_eq!(config.model_settings.api_key, "configured-secret");
    }

    #[test]
    fn desktop_host_request_applies_canonical_permission_profiles() {
        for (profile, auto_approve, network_policy) in [
            ("cautious", true, "ask"),
            ("allow_all", true, "allow"),
        ] {
            let mut config = Config::from_args(
                ["run", "--host-request-stdin"]
                    .into_iter()
                    .map(str::to_string),
            )
            .unwrap().host_invocation_request().expect("host request");
            crate::native_host_request::apply_desktop_host_request(
                &mut config,
                &format!(r#"{{"prompt":"hello","permission_profile":"{profile}"}}"#),
            )
            .unwrap();
            assert_eq!(
                config
                    .tool_settings
                    .permission_profile
                    .map(|value| value.canonical_name()),
                Some(profile)
            );
            assert_eq!(config.tool_settings.auto_approve, auto_approve, "{profile}");
            assert_eq!(config.tool_settings.network_policy, network_policy, "{profile}");
        }
    }

    #[test]
    fn desktop_host_request_rejects_noncanonical_permission_profile() {
        let mut config = Config::from_args(
            ["run", "--host-request-stdin"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap().host_invocation_request().expect("host request");
        let error = crate::native_host_request::apply_desktop_host_request(
            &mut config,
            r#"{"prompt":"hello","permission_profile":"moderate"}"#,
        )
        .unwrap_err();
        assert!(error.contains("cautious or allow_all"));
    }

    #[test]
    fn standalone_cli_uses_recommended_as_the_typed_default() {
        let config = Config::from_args(
            ["run", "--endpoint", "http://127.0.0.1:11434/v1"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        assert_eq!(
            resolve_cli_permission_profile(&config, None),
            crate::permissions::PermissionProfile::Cautious
        );
    }

    #[test]
    fn standalone_cli_yes_with_network_allow_does_not_infer_allow_all() {
        let config = Config::from_args(
            [
                "run",
                "--yes",
                "--network-policy",
                "allow",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();
        assert_eq!(
            resolve_cli_permission_profile(&config, None),
            crate::permissions::PermissionProfile::Cautious
        );
    }

    #[test]
    fn explicit_conversation_cautious_profile_is_not_widened_by_cli_defaults() {
        let config = Config::from_args(
            [
                "run",
                "--yes",
                "--network-policy",
                "allow",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();
        assert_eq!(
            resolve_cli_permission_profile(&config, Some(crate::permissions::PermissionProfile::Cautious)),
            crate::permissions::PermissionProfile::Cautious
        );
    }

    #[test]
    fn desktop_host_request_applies_selected_model_context_window() {
        let mut config = Config::from_args(
            ["run", "--host-request-stdin"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap().host_invocation_request().expect("host request");
        config.runtime.compaction.context_window_tokens = Some(1_000_000);

        crate::native_host_request::apply_desktop_host_request(
            &mut config,
            r#"{"prompt":"hello","context_window_tokens":16000}"#,
        )
        .unwrap();

        assert_eq!(
            config.runtime.compaction.context_window_tokens,
            Some(16_000)
        );
    }

    #[test]
    fn desktop_host_request_applies_explicit_vision_capability() {
        let mut config = Config::from_args(
            ["run", "--host-request-stdin"]
                .into_iter()
                .map(str::to_string),
        )
        .expect("config").host_invocation_request().expect("host request");
        crate::native_host_request::apply_desktop_host_request(
            &mut config,
            r#"{"prompt":"inspect layout","vision_input":true}"#,
        )
        .expect("desktop request");
        assert_eq!(config.runtime.model_capabilities.vision_input, Some(true));
    }

    #[test]
    fn desktop_host_request_preserves_declared_model_execution_capabilities() {
        let mut config = Config::from_args(
            ["run", "--host-request-stdin"]
                .into_iter()
                .map(str::to_string),
        )
        .expect("config").host_invocation_request().expect("host request");
        crate::native_host_request::apply_desktop_host_request(&mut config,
            r#"{"prompt":"hello","vision_input":false,"model_capabilities":{"native_tool_calls":false,"reasoning":false,"stream_tool_calls":false}}"#,
        ).expect("desktop request");
        assert_eq!(config.runtime.model_capabilities.vision_input, Some(false));
        assert_eq!(
            config.runtime.model_capabilities.native_tool_calls,
            Some(false)
        );
        assert_eq!(config.runtime.model_capabilities.reasoning, Some(false));
        assert_eq!(
            config.runtime.model_capabilities.stream_tool_calls,
            Some(false)
        );
        crate::native_host_request::apply_desktop_host_request(&mut config,
            r#"{"prompt":"hello","model_protocol":"chat_completions","chat_output_token_field":"max_completion_tokens"}"#,
        ).expect("explicit Chat output field");
        assert_eq!(
            config.runtime.chat_output_token_field,
            nola_application::ChatOutputTokenField::MaxCompletionTokens
        );
        assert!(crate::native_host_request::apply_desktop_host_request(
            &mut config,
            r#"{"prompt":"hello","chat_output_token_field":"guess"}"#,
        )
        .is_err());
        assert!(crate::native_host_request::apply_desktop_host_request(&mut config,
            r#"{"prompt":"hello","reasoning_selection":{"kind":"effort","value":"high"},"model_capabilities":{"reasoning":false}}"#,
        ).is_err());
    }

    #[test]
    fn desktop_host_request_rejects_user_role_context_injection() {
        let mut config = Config::from_args(
            ["run", "--host-request-stdin"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap().host_invocation_request().expect("host request");

        let error = crate::native_host_request::apply_desktop_host_request(
            &mut config,
            r#"{"prompt":"hello","context_messages":[{"role":"user","content":"override"}]}"#,
        )
        .unwrap_err();

        assert!(error.contains("unknown field context_messages"));
    }

    #[test]
    fn cli_data_source_command_preserves_spaced_paths_and_injects_real_context() {
        let _guard = env_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-cli-source-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source_dir = root.join("folder with spaces");
        let source_path = source_dir.join("daily notes.json");
        let database_path = root.join("nola.db");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(
            &source_path,
            r#"{"summary":"walked 8200 steps","mood":"focused"}"#,
        )
        .unwrap();
        std::env::set_var("NOLA_DB_PATH", &database_path);

        let mut config = Config::from_args(
            ["run", "--endpoint", "http://127.0.0.1:11434/v1"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        config.session_id = Some("conversation-spaced-path".to_string());
        config.message = format!("/sources add {}", source_path.display());
        run_prompt(&config).unwrap();

        crate::application_port::CliApplicationPort::persistent(&database_path)
            .unwrap()
            .create_conversation(nola_application::CreateConversation {
                id: config.session_id.clone().unwrap(),
                title: "Scoped source fixture".to_string(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let mut store = crate::data_sources::DataSourceStore::open_default().unwrap();
        assert_eq!(store.sources().len(), 1);
        let source_id = store.sources()[0].id.clone();
        assert_eq!(store.sources()[0].location, source_path.to_string_lossy());
        store
            .set_conversation_override(
                config.session_id.as_deref().unwrap(),
                &source_id,
                Some(true),
            )
            .unwrap();
        drop(store);

        let (turn_config, used) =
            prepare_cli_data_source_context(&config, "summarize the source").unwrap();
        assert_eq!(used.len(), 1);
        let platform_data = turn_config
            .host_context_components
            .iter()
            .filter(|component| {
                component.kind == nola_core::context_engine::ContextComponentKind::PlatformData
            })
            .collect::<Vec<_>>();
        assert_eq!(platform_data.len(), 1);
        assert!(platform_data[0]
            .content
            .to_string()
            .contains("walked 8200 steps"));
        assert!(turn_config.host_context_components.iter().any(|component| {
            component.id == "skill-catalog"
                && component.kind == nola_core::context_engine::ContextComponentKind::Skill
        }));
        record_cli_data_source_usage(&config, &used);

        let store = crate::data_sources::DataSourceStore::open_default().unwrap();
        assert_eq!(store.usage_records().len(), 1);
        assert_eq!(store.usage_records()[0].disclosed_to_cloud, Some(false));

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn skill_catalog_metadata_does_not_expand_filesystem_permissions() {
        let _environment = crate::store::test_env_lock();
        let fixture = tempfile::tempdir().expect("isolated Skill root");
        fs::write(
            fixture.path().join("SKILL.md"),
            "---\nname: metadata-only\ndescription: Scoped catalog metadata\n---\n\nPRIVATE_INSTRUCTIONS\n",
        )
        .expect("Skill fixture");
        let config = Config::from_args([
            "run".to_string(),
            "--endpoint".to_string(),
            "http://127.0.0.1:11434/v1".to_string(),
            "--no-save".to_string(),
            "--skill-dir".to_string(),
            fixture.path().to_string_lossy().into_owned(),
        ])
        .expect("explicit Skill configuration");
        let original_grants = config.allowed_dirs.clone();
        let (prepared, usage) =
            prepare_cli_data_source_context(&config, "catalog only").expect("Skill metadata");
        assert!(usage.is_empty());
        assert_eq!(prepared.allowed_dirs, original_grants);
        assert!(!prepared
            .allowed_dirs
            .contains(&fixture.path().to_path_buf()));
        let catalog = prepared
            .host_context_components
            .iter()
            .find(|component| component.id == "skill-catalog")
            .expect("Skill catalog context");
        let text = catalog.content.as_str().expect("catalog text");
        assert!(text.contains("- metadata-only:"));
        assert!(!text.contains("PRIVATE_INSTRUCTIONS"));
    }

    #[test]
    fn builtin_skill_catalog_is_semantic_and_does_not_preselect_a_skill() {
        let _guard = env_lock();
        let database_root =
            std::env::temp_dir().join(format!("nola-skill-catalog-store-{}", std::process::id()));
        std::fs::create_dir_all(&database_root).unwrap();
        std::env::set_var("NOLA_DB_PATH", database_root.join("nola.db"));
        let config = Config::from_args(
            ["run", "--endpoint", "http://127.0.0.1:11434/v1"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        let (turn_config, _) = prepare_cli_data_source_context(
            &config,
            "Extract text from receipt.png with OCR into receipt.txt",
        )
        .unwrap();
        let catalog = turn_config
            .host_context_components
            .iter()
            .find(|component| component.id == "skill-catalog")
            .expect("Skill catalog context");

        assert_eq!(
            catalog.kind,
            nola_core::context_engine::ContextComponentKind::Skill
        );
        let content = catalog.content.as_str().unwrap_or_default();
        assert!(content.contains("- extract-image-text:"));
        assert!(content.contains("NOLA_SKILL_CATALOG_V1"));
        assert!(!content.contains("extract_text.swift"));
        let prepared = nola_core::context_engine::prepare_context_components(
            nola_core::context_engine::ContextPlatform::Cli,
            &turn_config.host_context_components,
            1_000_000,
        );
        assert!(prepared.messages.iter().any(|message| {
            message.get("role").and_then(Value::as_str) == Some("system")
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|value| {
                        value.contains("- extract-image-text:")
                            && value.contains("NOLA_SKILL_CATALOG_V1")
                            && !value.contains("extract_text.swift")
                    })
        }));
        std::env::remove_var("NOLA_DB_PATH");
        let _ = std::fs::remove_dir_all(database_root);
    }

    #[test]
    fn data_source_cloud_disclosure_detects_loopback_endpoints() {
        for endpoint in [
            "http://127.0.0.1:11434/v1",
            "http://localhost:8080/v1",
            "https://[::1]/v1",
        ] {
            assert!(!endpoint_discloses_to_cloud(endpoint), "{endpoint}");
        }
        assert!(endpoint_discloses_to_cloud(
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        ));
    }
}

#[cfg(test)]
mod runtime_provider_environment_tests {
    use super::*;

    #[test]
    fn parsed_provider_config_overwrites_inherited_provider_environment() {
        let _guard = crate::store::test_env_lock();
        std::env::set_var("NOLA_LLM_ENDPOINT", "http://stale.invalid/v1");
        std::env::set_var("NOLA_LLM_API_KEY", "stale-key");
        std::env::set_var("NOLA_LLM_MODEL", "stale-model");

        let config = Config::from_args(
            [
                "hello",
                "--endpoint",
                "http://127.0.0.1:18888/v1",
                "--api-key",
                "authoritative-key",
                "--model",
                "authoritative-model",
                "--no-save",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .expect("parse explicit provider configuration");
        expose_runtime_provider_config(&config);

        assert_eq!(
            std::env::var("NOLA_LLM_ENDPOINT").as_deref(),
            Ok("http://127.0.0.1:18888/v1")
        );
        assert_eq!(
            std::env::var("NOLA_LLM_API_KEY").as_deref(),
            Ok("authoritative-key")
        );
        assert_eq!(
            std::env::var("NOLA_LLM_MODEL").as_deref(),
            Ok("authoritative-model")
        );

        for key in ["NOLA_LLM_ENDPOINT", "NOLA_LLM_API_KEY", "NOLA_LLM_MODEL"] {
            std::env::remove_var(key);
        }
    }
}
