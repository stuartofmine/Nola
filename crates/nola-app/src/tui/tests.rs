// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn slash_command_parsing_handles_commands_args_and_escaped_literal() {
    assert_eq!(parse_user_input("   "), ParsedInput::Empty);
    assert_eq!(
        parse_user_input("hello world"),
        ParsedInput::LiteralPrompt("hello world".into())
    );
    assert_eq!(
        parse_user_input("//not-a-command arg"),
        ParsedInput::LiteralPrompt("/not-a-command arg".into())
    );
    assert_eq!(
        parse_user_input("/PLAN approve"),
        ParsedInput::Slash {
            cmd: "/plan".into(),
            arg: "approve".into(),
        }
    );
    assert_eq!(
        parse_user_input("/thinking   high  "),
        ParsedInput::Slash {
            cmd: "/thinking".into(),
            arg: "high".into(),
        }
    );
}

#[test]
fn tab_completion_completes_slash_commands_subcommands_and_experts() {
    let experts = vec![
        "architect".to_string(),
        "reviewer".to_string(),
        "researcher".to_string(),
    ];

    // Unique slash command completion
    let r1 = complete_composer_input("/workt", 6, &experts);
    assert_eq!(r1.candidates, vec!["/worktree"]);
    assert_eq!(r1.replacement, Some((0, 6, "/worktree ".into())));

    // Multi-match slash command common prefix completion
    let r2 = complete_composer_input("/check", 6, &experts);
    assert_eq!(r2.candidates, vec!["/checkpoint", "/checkpoints"]);
    assert_eq!(r2.replacement, Some((0, 6, "/checkpoint".into())));

    // Subcommand completion for /plan
    let r3 = complete_composer_input("/plan ap", 8, &experts);
    assert_eq!(r3.candidates, vec!["approve"]);
    assert_eq!(r3.replacement, Some((6, 8, "approve ".into())));

    // Subcommand completion for /thinking
    let r4 = complete_composer_input("/thinking m", 11, &experts);
    assert_eq!(r4.candidates, vec!["medium"]);
    assert_eq!(r4.replacement, Some((10, 11, "medium ".into())));

    // @expert mention completion mid-sentence
    let r5 = complete_composer_input("ask @arch", 9, &experts);
    assert_eq!(r5.candidates, vec!["@architect"]);
    assert_eq!(r5.replacement, Some((4, 9, "@architect ".into())));

    // Multi-match @expert
    let r6 = complete_composer_input("ask @re", 7, &experts);
    assert_eq!(r6.candidates, vec!["@researcher", "@reviewer"]);
    assert_eq!(r6.replacement, None);
}

#[test]
fn markdown_ansi_rendering_formats_headings_bold_code_lists_and_fences() {
    let md = "# Title\n\nUse **bold** and `inline`.\n- item one\n```rust\nlet x = 1;\n```\n> note";
    let rendered = render_markdown_ansi(md, 80);
    let joined = rendered.join("\n");
    assert!(joined.contains("\x1b[1;35m# Title\x1b[0m"));
    assert!(joined.contains("\x1b[1mbold\x1b[22m"));
    assert!(joined.contains("\x1b[48;5;237;38;5;222m inline \x1b[0m"));
    assert!(joined.contains("•"));
    assert!(joined.contains("╭─ ") && joined.contains("rust") && joined.contains("╮"));
    assert!(joined.contains("let x = 1;"));
    assert!(joined.contains("╰") && joined.contains("╯"));
    assert!(joined.contains("▎") && joined.contains("note"));
    // Every fenced code box row must be enclosed on all 4 sides at exact width 80
    for line in rendered.iter().filter(|l| l.contains('╭') || l.contains('╰') || l.contains("let x = 1;")) {
        assert_eq!(str_display_width(line), 80, "misaligned box line: {line}");
    }
}

#[test]
fn rounded_box_encloses_all_four_sides_with_exact_cjk_alignment() {
    let body = vec![
        "  \x1b[1m/compact\x1b[0m   压缩模型上下文，同时保留完整历史".to_string(),
        "\x1b[48;5;238;1;36m› * secondary:openrouter/free (ctx=128k)\x1b[0m".to_string(),
        "emoji check 👋 👍 中文混排 ABC".to_string(),
    ];
    let boxed = render_rounded_box(
        "\x1b[36m",
        "选择模型 (1/3)",
        &body,
        Some("↑↓ 选择 · Enter 确认 · Esc 取消"),
        72,
    );
    assert_eq!(boxed.len(), body.len() + 2);
    assert!(boxed.first().unwrap().contains('╭') && boxed.first().unwrap().contains('╮'));
    assert!(boxed.last().unwrap().contains('╰') && boxed.last().unwrap().contains('╯'));
    for row in &boxed {
        assert_eq!(str_display_width(row), 72, "row width mismatch: {row}");
    }
    for middle in &boxed[1..boxed.len() - 1] {
        assert!(middle.starts_with("\x1b[36m│\x1b[0m "));
        assert!(middle.ends_with(" \x1b[36m│\x1b[0m"));
    }
}

#[test]
fn user_tinted_block_omits_you_label_and_fills_width() {
    let lines = render_user_tinted_lines("push下未提交代码", 60);
    assert_eq!(lines.len(), 1);
    assert!(!lines[0].contains("You"));
    assert!(lines[0].contains("\x1b[48;5;236m"));
    assert!(lines[0].contains("▎ "));
    assert_eq!(str_display_width(&lines[0]), 60);
}

#[test]
fn readonly_shell_command_and_prefix_grant_classification() {
    assert!(is_readonly_safe_shell_command("pwd"));
    assert!(is_readonly_safe_shell_command("ls -la /tmp"));
    assert!(is_readonly_safe_shell_command("git status --short"));
    assert!(is_readonly_safe_shell_command("git rev-parse --show-toplevel"));
    assert!(!is_readonly_safe_shell_command("git add ."));
    assert!(!is_readonly_safe_shell_command("pwd && rm -rf /tmp/x"));
    assert!(!is_readonly_safe_shell_command("ls > out.txt"));
    assert!(!is_readonly_safe_shell_command("cat a | sh"));
    assert_eq!(
        extract_session_grant_key("bash", &json!({"command": "git add crates/a.rs"})),
        "prefix:git add"
    );
}

#[test]
fn i18n_locale_selection_supports_en_and_zh_hans() {
    assert_eq!(TuiLocale::from_lang_str("en_US.UTF-8"), TuiLocale::En);
    assert_eq!(TuiLocale::from_lang_str(""), TuiLocale::En);
    assert_eq!(TuiLocale::from_lang_str("zh_CN.UTF-8"), TuiLocale::ZhHans);
    assert_eq!(TuiLocale::from_lang_str("zh-Hans"), TuiLocale::ZhHans);
    assert_eq!(TuiLocale::from_lang_str("ZH"), TuiLocale::ZhHans);
    assert_eq!(TuiLocale::En.t("Hello", "你好"), "Hello");
    assert_eq!(TuiLocale::ZhHans.t("Hello", "你好"), "你好");
    assert!(format_tui_help(TuiLocale::ZhHans).contains("斜杠命令列表"));
    assert!(format_tui_help(TuiLocale::En).contains("Slash commands"));
}

#[test]
fn composer_cursor_utf8_editing_and_history_navigation() {
    let mut c = ComposerState::default();
    c.insert_str("你好");
    c.insert_char('!');
    assert_eq!(c.buffer, "你好!");
    assert_eq!(c.cursor, "你好!".len());

    c.move_left();
    c.insert_char(' ');
    assert_eq!(c.buffer, "你好 !");

    c.move_home();
    c.insert_str("> ");
    assert_eq!(c.buffer, "> 你好 !");

    c.move_end();
    c.backspace();
    assert_eq!(c.buffer, "> 你好 ");

    c.move_home();
    c.delete();
    assert_eq!(c.buffer, " 你好 ");

    let first = c.submit();
    assert_eq!(first, " 你好 ");
    assert!(c.buffer.is_empty());

    c.insert_str("/status");
    let _ = c.submit();

    // Draft preservation across history Up/Down
    c.insert_str("draft");
    c.history_up();
    assert_eq!(c.buffer, "/status");
    c.history_up();
    assert_eq!(c.buffer, "你好");
    c.history_down();
    assert_eq!(c.buffer, "/status");
    c.history_down();
    assert_eq!(c.buffer, "draft");
}

#[test]
fn base64_and_image_sniffing_work_for_attachments() {
    assert_eq!(encode_base64(b"nola"), "bm9sYQ==");
    assert_eq!(encode_base64(b"abc"), "YWJj");
    assert_eq!(
        sniff_image_mime("pic.bin", b"\x89PNG\r\n\x1a\n1234"),
        Some("image/png")
    );
    assert_eq!(
        sniff_image_mime("photo.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]),
        Some("image/jpeg")
    );
    assert_eq!(sniff_image_mime("doc.txt", b"hello"), None);
}

#[test]
fn picker_state_filters_navigates_wraps_and_clamps_selection() {
    let items = vec![
        PickerItem {
            value: "local:qwen3.8-flash".into(),
            label: "local:qwen3.8-flash".into(),
            detail: "http://127.0.0.1:8081/v1 · ctx=128k".into(),
            is_current: false,
        },
        PickerItem {
            value: "openai:gpt-5".into(),
            label: "openai:gpt-5".into(),
            detail: "https://api.openai.com/v1 · ctx=256k".into(),
            is_current: true,
        },
        PickerItem {
            value: "anthropic:claude-sonnet".into(),
            label: "anthropic:claude-sonnet".into(),
            detail: "https://api.anthropic.com · ctx=200k".into(),
            is_current: false,
        },
    ];

    let mut picker = PickerState::new(PickerKind::Model, "Select Model".into(), items);
    // Pre-selects the currently active model
    assert_eq!(picker.selected, 1);
    assert_eq!(
        picker.selected_item().map(|i| i.value.as_str()),
        Some("openai:gpt-5")
    );

    // Move down wraps around cleanly
    picker.move_down(1);
    assert_eq!(
        picker.selected_item().map(|i| i.value.as_str()),
        Some("anthropic:claude-sonnet")
    );
    picker.move_down(1);
    assert_eq!(
        picker.selected_item().map(|i| i.value.as_str()),
        Some("local:qwen3.8-flash")
    );
    // Move up from 0 wraps to last
    picker.move_up(1);
    assert_eq!(
        picker.selected_item().map(|i| i.value.as_str()),
        Some("anthropic:claude-sonnet")
    );

    // Live keyword filtering narrows results and clamps selection
    picker.filter.insert_str("qwen");
    picker.selected = 0;
    picker.clamp_selection(10);
    assert_eq!(picker.filtered_indices(), vec![0]);
    assert_eq!(
        picker.selected_item().map(|i| i.value.as_str()),
        Some("local:qwen3.8-flash")
    );

    // Unmatched query yields empty selection without panicking
    picker.filter.insert_str("zzz-nonexistent");
    picker.selected = 0;
    picker.clamp_selection(10);
    assert!(picker.filtered_indices().is_empty());
    assert!(picker.selected_item().is_none());
}

#[test]
fn markdown_table_links_hr_ordered_list_path_ellipsis_and_composer_shortcuts() {
    let md = "\
| 平台 | 状态 |
| --- | --- |
| CLI | `通过` |
| Web | [Docs](https://nola.local) |

---
1. 第一步 *斜体*
2. 第二步 **加粗**";
    let rendered = render_markdown_ansi(md, 60);
    let joined = rendered.join("\n");
    assert!(joined.contains('┌') && joined.contains('┐') && joined.contains('└') && joined.contains('┘'));
    assert!(joined.contains("1.") && joined.contains("2."));
    assert!(joined.contains("https://nola.local"));
    assert!(joined.contains("\x1b[3m斜体\x1b[23m"));
    assert!(joined.contains("────────"));
    // Every table line has identical visual display width
    let table_widths: Vec<usize> = rendered
        .iter()
        .filter(|l| l.contains('┌') || l.contains('├') || l.contains('└') || l.contains('│'))
        .map(|l| str_display_width(l))
        .collect();
    assert!(table_widths.len() >= 5);
    assert!(table_widths.windows(2).all(|w| w[0] == w[1]), "misaligned table widths: {table_widths:?}");

    // Path middle-truncation with ellipsis
    let long_path = Path::new("/private/var/folders/very/deep/nested/workspace/NolaDemo");
    let abbr = abbreviate_path_with_ellipsis(long_path, 30);
    assert!(str_display_width(&abbr) <= 30, "abbreviated path too wide: {abbr}");
    assert!(abbr.contains('…') && abbr.ends_with("NolaDemo"), "unexpected abbr: {abbr}");

    // Ctrl+W & Ctrl+K composer editing shortcuts
    let mut c = ComposerState::default();
    c.insert_str("hello world foo bar");
    c.delete_word_backward();
    assert_eq!(c.buffer, "hello world foo ");
    c.move_home();
    for _ in 0..5 {
        c.move_right();
    }
    c.kill_to_end();
    assert_eq!(c.buffer, "hello");
}
