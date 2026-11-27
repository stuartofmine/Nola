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
    assert!(joined.contains("\x1b[33minline\x1b[0m"));
    assert!(joined.contains("•"));
    assert!(joined.contains("┌─ rust"));
    assert!(joined.contains("let x = 1;"));
    assert!(joined.contains("└────"));
    assert!(joined.contains("│ note"));
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
