// SPDX-License-Identifier: Apache-2.0
//! Phase-independent editor operations. Runtime intent (submit/cancel/authorize)
//! is routed by the focused surface, never interpreted by the text editor.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

/// Paste is authored content. Preserve whitespace and never silently truncate it.
pub(crate) fn normalize_paste(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n").chars()
        .filter(|ch| matches!(ch, '\n' | '\t') || !ch.is_control()).collect()
}

pub(crate) fn is_submit(key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press && match key.code {
        KeyCode::Enter | KeyCode::Char('\r' | '\n') => key.modifiers.is_empty(),
        KeyCode::Char('m' | 'M' | 'j' | 'J') => key.modifiers == KeyModifiers::CONTROL,
        _ => false,
    }
}

/// Returns true when the focused editor consumed this key. All byte offsets
/// remain UTF-8 boundaries; callers use the same reducer in idle and live turns.
pub(crate) fn edit(text: &mut String, cursor: &mut usize, key: &KeyEvent) -> bool {
    if key.kind == KeyEventKind::Release { return false; }
    if key.modifiers.intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META) { return false; }
    *cursor = (*cursor).min(text.len());
    while !text.is_char_boundary(*cursor) { *cursor -= 1; }
    let previous = |text: &str, cursor: usize| {
        text[..cursor].grapheme_indices(true).next_back().map_or(0, |(index, _)| index)
    };
    let next = |text: &str, cursor: usize| {
        text[cursor..].graphemes(true).next().map_or(cursor, |ch| cursor + ch.len())
    };
    let word_left = |text: &str, cursor: usize| {
        text[..cursor].unicode_word_indices().next_back().map_or(0, |(index, _)| index)
    };
    let word_right = |text: &str, cursor: usize| {
        text[cursor..].unicode_word_indices().next().map_or(text.len(), |(index, word)| cursor + index + word.len())
    };
    let by_word = key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    match key.code {
        KeyCode::Backspace if by_word => { let start = word_left(text, *cursor); text.drain(start..*cursor); *cursor = start; }
        KeyCode::Delete if by_word => { text.drain(*cursor..word_right(text, *cursor)); }
        KeyCode::Left if by_word => *cursor = word_left(text, *cursor),
        KeyCode::Right if by_word => *cursor = word_right(text, *cursor),
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => *cursor = 0,
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => *cursor = text.len(),
        KeyCode::Backspace => { let start = previous(text, *cursor); text.drain(start..*cursor); *cursor = start; }
        KeyCode::Delete => { text.drain(*cursor..next(text, *cursor)); }
        KeyCode::Left => *cursor = previous(text, *cursor),
        KeyCode::Right => *cursor = next(text, *cursor),
        KeyCode::Home => *cursor = text[..*cursor].rfind('\n').map_or(0, |i| i + 1),
        KeyCode::End => *cursor += text[*cursor..].find('\n').unwrap_or(text.len() - *cursor),
        KeyCode::Enter if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => {
            text.insert(*cursor, '\n'); *cursor += 1;
        }
        KeyCode::Up | KeyCode::Down if text.contains('\n') => {
            let start = text[..*cursor].rfind('\n').map_or(0, |i| i + 1);
            let column = text[start..*cursor].graphemes(true).count();
            let end = *cursor + text[*cursor..].find('\n').unwrap_or(text.len() - *cursor);
            let target = if key.code == KeyCode::Up {
                if start == 0 { return true; }
                let end = start - 1;
                (text[..end].rfind('\n').map_or(0, |i| i + 1), end)
            } else {
                if end == text.len() { return true; }
                let start = end + 1;
                (start, start + text[start..].find('\n').unwrap_or(text.len() - start))
            };
            *cursor = text[target.0..target.1].grapheme_indices(true).nth(column)
                .map_or(target.1, |(offset, _)| target.0 + offset);
        }
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => match ch {
            'b' | 'B' => *cursor = previous(text, *cursor),
            'f' | 'F' => *cursor = next(text, *cursor),
            'k' | 'K' => { let end = *cursor + text[*cursor..].find('\n').unwrap_or(text.len() - *cursor); text.drain(*cursor..end); }
            'a' | 'A' => *cursor = 0,
            'e' | 'E' => *cursor = text.len(),
            'u' | 'U' => { text.drain(..*cursor); *cursor = 0; }
            'w' | 'W' => {
                let start = text[..*cursor].trim_end_matches(char::is_whitespace)
                    .char_indices().rev().find(|(_, ch)| ch.is_whitespace())
                    .map_or(0, |(index, ch)| index + ch.len_utf8());
                text.drain(start..*cursor); *cursor = start;
            }
            _ => return false,
        },
        KeyCode::Char(ch) if !ch.is_control() && !key.modifiers.contains(KeyModifiers::ALT) => {
            text.insert(*cursor, ch); *cursor += ch.len_utf8();
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn word_editing_and_document_navigation_preserve_unicode_and_other_lines() {
        let mut text = "alpha café\n下一行".to_string();
        let mut cursor = "alpha café".len();
        assert!(edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT)));
        assert_eq!(text, "alpha \n下一行");
        assert_eq!(cursor, 6);
        edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
        edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(cursor, 5);
        edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(text, "alpha\n下一行");
        edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL));
        assert_eq!(cursor, text.len());
        let before = text.clone();
        assert!(!edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::Char('s'), KeyModifiers::SUPER)));
        assert_eq!(text, before);
    }

    #[test]
    fn modified_enter_edits_and_repeat_never_submits() {
        for modifier in [KeyModifiers::SHIFT, KeyModifiers::ALT] {
            let key = KeyEvent::new(KeyCode::Enter, modifier);
            assert!(!is_submit(&key));
            let mut text = "ab".to_owned(); let mut cursor = 1;
            assert!(edit(&mut text, &mut cursor, &key));
            assert_eq!(text, "a\nb");
        }
        assert!(!is_submit(&KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat)));
    }
    #[test]
    fn editing_moves_whole_graphemes_and_multiline_cursor() {
        for cluster in ["e\u{301}", "👨‍👩‍👧‍👦", "🇨🇳"] {
            let mut text = format!("a{cluster}"); let mut cursor = text.len();
            edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
            assert_eq!(text, "a");
        }
        let mut text = "一二\n三四五".to_owned(); let mut cursor = "一二\n三".len();
        edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(cursor, "一".len());
        edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(cursor, "一二\n三".len());
    }
    #[test]
    fn paste_preserves_whitespace_and_large_content() {
        let text = "\tcode\r\n  next\n".repeat(20_000);
        assert_eq!(normalize_paste(&text), "\tcode\n  next\n".repeat(20_000));
    }
    #[test]
    fn composer_reducer_preserves_unicode_and_ignores_key_release() {
        let mut text = "中🙂文".to_owned(); let mut cursor = "中🙂".len();
        assert!(edit(&mut text, &mut cursor, &KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)));
        assert_eq!((&*text, cursor), ("中文", 3));
        let key = KeyEvent::new_with_kind(KeyCode::Char('x'), KeyModifiers::NONE, KeyEventKind::Release);
        assert!(!edit(&mut text, &mut cursor, &key));
        assert_eq!(text, "中文");
    }
    #[test]
    fn composer_submit_contract_is_identical_in_every_phase() {
        for code in [KeyCode::Enter, KeyCode::Char('\r'), KeyCode::Char('\n')] {
            assert!(is_submit(&KeyEvent::new(code, KeyModifiers::NONE)));
            assert!(!is_submit(&KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release)));
        }
        assert!(is_submit(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)));
        assert!(!is_submit(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL | KeyModifiers::ALT)));
    }
}
