// SPDX-License-Identifier: Apache-2.0
//! One terminal event source shared by the composer and focused modal adapters.
//! Reads are demand-driven: the broker never prefetches while a line-mode or
//! external process owns stdin. The interaction focus guard serializes consumers.
use crossterm::event::Event;
use std::io;
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ModalKey {
    Action,
    Cancel,
    Ignore,
}

/// Modal shortcuts are distinct from text: Ctrl+Y must never grant approval.
pub(crate) fn modal_key(key: &crossterm::event::KeyEvent) -> ModalKey {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    if key.kind != KeyEventKind::Press { return ModalKey::Ignore; }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return if matches!(key.code, KeyCode::Char('c' | 'C')) {
            ModalKey::Cancel
        } else { ModalKey::Ignore };
    }
    if key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META) { return ModalKey::Ignore; }
    if key.code == KeyCode::Esc { ModalKey::Cancel } else { ModalKey::Action }
}

#[derive(Default)]
pub(crate) struct ModalEditor {
    pub(crate) text: String,
    pub(crate) cursor: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EditorAction { Redraw, Submit(String), Cancel, Ignore }

impl ModalEditor {
    const LIMIT: usize = 4096;

    fn insert(&mut self, value: &str) {
        let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
        for ch in normalized.chars() {
            // Preserve intentional line breaks; terminal control sequences and
            // pasted tabs must not gain cursor/terminal ownership.
            let ch = if ch == '\t' { ' ' } else { ch };
            if ch.is_control() && ch != '\n' { continue; }
            if self.text.len() + ch.len_utf8() > Self::LIMIT { break; }
            self.text.insert(self.cursor, ch);
            self.cursor += ch.len_utf8();
        }
    }

    pub(crate) fn apply(&mut self, event: Event) -> EditorAction {
        use crossterm::event::{KeyCode, KeyModifiers};
        let key = match event {
            Event::Resize(_, _) => return EditorAction::Redraw,
            Event::Paste(text) => { self.insert(&text); return EditorAction::Redraw; }
            Event::Key(key) => key,
            _ => return EditorAction::Ignore,
        };
        // Only explicit text-editing keys bypass modal shortcut filtering.
        // Reuse the composer reducer for whole graphemes and line/word movement.
        if key.kind != crossterm::event::KeyEventKind::Release
            && (matches!(key.code, KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End
                | KeyCode::Up | KeyCode::Down | KeyCode::Backspace | KeyCode::Delete)
                || key.modifiers == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char('a' | 'A' | 'e' | 'E' | 'b' | 'B' | 'f' | 'F' | 'u' | 'U' | 'w' | 'W' | 'k' | 'K')))
        {
            return if crate::composer::edit(&mut self.text, &mut self.cursor, &key) { EditorAction::Redraw } else { EditorAction::Ignore };
        }
        if key.kind == crossterm::event::KeyEventKind::Repeat
            && !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META)
        {
            if let KeyCode::Char(ch) = key.code {
                if !ch.is_control() { self.insert(&ch.to_string()); return EditorAction::Redraw; }
            }
        }
        match modal_key(&key) {
            ModalKey::Cancel => return EditorAction::Cancel,
            ModalKey::Ignore => return EditorAction::Ignore,
            ModalKey::Action => {}
        }
        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => self.insert("\n"),
            KeyCode::Enter if !self.text.trim().is_empty() => return EditorAction::Submit(self.text.trim().to_string()),
            KeyCode::Char(ch) if !ch.is_control() => self.insert(&ch.to_string()),
            _ => return EditorAction::Ignore,
        }
        EditorAction::Redraw
    }
}

enum Request {
    Poll(Duration, mpsc::SyncSender<io::Result<bool>>),
    Read(mpsc::SyncSender<io::Result<Event>>),
}

static SOURCE: OnceLock<mpsc::Sender<Request>> = OnceLock::new();

fn source() -> &'static mpsc::Sender<Request> {
    SOURCE.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new().name("nola-terminal-input".into()).spawn(move || {
            serve(receiver, crossterm::event::poll, crossterm::event::read);
        }).expect("start terminal event broker");
        sender
    })
}

fn serve(
    receiver: mpsc::Receiver<Request>,
    mut poll_source: impl FnMut(Duration) -> io::Result<bool>,
    mut read_source: impl FnMut() -> io::Result<Event>,
) {
    while let Ok(request) = receiver.recv() {
        match request {
            Request::Poll(duration, reply) => { let _ = reply.send(poll_source(source_poll_timeout(duration))); }
            Request::Read(reply) => { let _ = reply.send(read_source()); }
        }
    }
}

fn source_poll_timeout(timeout: Duration) -> Duration {
    // Crossterm's level-triggered Unix TTY backend checks its deadline before
    // polling the descriptor. A zero timeout would skip buffered events and
    // ready input entirely. This bounds a readiness probe; it is not a sleep
    // or a timing workaround for the resize race.
    #[cfg(unix)]
    if timeout.is_zero() { return Duration::from_millis(1); }
    timeout
}

fn disconnected() -> io::Error { io::Error::new(io::ErrorKind::BrokenPipe, "terminal event source disconnected") }

pub(crate) fn poll(timeout: Duration) -> io::Result<bool> {
    let (reply, answer) = mpsc::sync_channel(1);
    source().send(Request::Poll(timeout, reply)).map_err(|_| disconnected())?;
    answer.recv().map_err(|_| disconnected())?
}

pub(crate) fn read() -> io::Result<Event> {
    let (reply, answer) = mpsc::sync_channel(1);
    source().send(Request::Read(reply)).map_err(|_| disconnected())?;
    answer.recv().map_err(|_| disconnected())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_probe_reaches_tty_without_changing_positive_deadlines() {
        #[cfg(unix)]
        assert_eq!(source_poll_timeout(Duration::ZERO), Duration::from_millis(1));
        #[cfg(not(unix))]
        assert_eq!(source_poll_timeout(Duration::ZERO), Duration::ZERO);
        assert_eq!(source_poll_timeout(Duration::from_millis(250)), Duration::from_millis(250));
        let (sender, receiver) = mpsc::channel();
        let (reply, answer) = mpsc::sync_channel(1);
        sender.send(Request::Poll(Duration::ZERO, reply)).unwrap();
        drop(sender);
        serve(receiver, |timeout| {
            assert_eq!(timeout, source_poll_timeout(Duration::ZERO));
            Ok(true)
        }, || panic!("poll cannot consume focused input"));
        assert!(answer.recv().unwrap().unwrap());
    }
    #[test]
    fn modal_editor_preserves_cjk_multiline_and_utf8_boundaries() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut editor = ModalEditor::default();
        let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        editor.apply(Event::Paste("你好\r\n世界\t!\u{7}\u{1b}".into()));
        assert_eq!(editor.text, "你好\n世界 !");
        editor.apply(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL)));
        editor.apply(key(KeyCode::Right));
        editor.apply(key(KeyCode::Delete));
        assert_eq!(editor.text, "你\n世界 !");
        editor.apply(key(KeyCode::Backspace));
        assert_eq!(editor.text, "\n世界 !");
        editor.apply(Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL)));
        editor.apply(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)));
        assert_eq!(editor.apply(key(KeyCode::Enter)), EditorAction::Submit("世界 !".into()));
        assert_eq!(editor.apply(Event::Resize(20, 5)), EditorAction::Redraw);
    }

    #[test]
    fn modal_editing_keeps_emoji_whole_and_supports_repeated_navigation() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let mut editor = ModalEditor::default();
        editor.apply(Event::Paste("first\ne\u{301}👨‍👩‍👧‍👦".into()));
        editor.apply(Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)));
        assert_eq!(editor.text, "first\ne\u{301}");
        editor.apply(Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)));
        assert_eq!(editor.text, "first\n");
        editor.apply(Event::Paste("last".into()));
        editor.apply(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)));
        assert_eq!(editor.cursor, 6);
        editor.apply(Event::Key(KeyEvent::new_with_kind(KeyCode::Right, KeyModifiers::NONE, KeyEventKind::Repeat)));
        assert_eq!(editor.cursor, 7);
        editor.apply(Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)));
        assert_eq!(editor.cursor, 0);
        editor.apply(Event::Key(KeyEvent::new_with_kind(KeyCode::Char('x'), KeyModifiers::NONE, KeyEventKind::Repeat)));
        assert!(editor.text.starts_with("xfirst"));
        assert_eq!(editor.apply(Event::Key(KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat))), EditorAction::Ignore);
        assert_eq!(editor.apply(Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::SUPER))), EditorAction::Ignore);
    }

    #[test]
    fn modal_editor_limits_paste_and_never_submits_from_control_keys() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut editor = ModalEditor::default();
        editor.apply(Event::Paste("字".repeat(2000)));
        assert_eq!(editor.text.len(), 4095);
        assert!(editor.text.is_char_boundary(editor.cursor));
        assert_eq!(editor.apply(Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL))), EditorAction::Ignore);
        assert_eq!(editor.apply(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))), EditorAction::Cancel);
        assert_eq!(editor.apply(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))), EditorAction::Cancel);
    }
    #[test]
    fn modal_shortcuts_cannot_accidentally_approve_or_submit() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        for code in ['y', 's', '1', '2'] {
            for modifier in [KeyModifiers::CONTROL, KeyModifiers::ALT, KeyModifiers::SUPER, KeyModifiers::HYPER, KeyModifiers::META] {
                assert_eq!(modal_key(&KeyEvent::new(KeyCode::Char(code), modifier)), ModalKey::Ignore);
            }
        }
        assert_eq!(modal_key(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), ModalKey::Cancel);
        assert_eq!(modal_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)), ModalKey::Cancel);
        assert_eq!(modal_key(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)), ModalKey::Action);
        assert_eq!(modal_key(&KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release)), ModalKey::Ignore);
    }
    #[test]
    fn terminal_source_reads_only_on_demand_and_propagates_disconnects() {
        let (sender, receiver) = mpsc::channel();
        let (answer, response) = mpsc::sync_channel(1);
        sender.send(Request::Poll(Duration::ZERO, answer)).unwrap();
        drop(sender);
        serve(receiver, |_| Ok(true), || panic!("poll must not consume an event"));
        assert!(response.recv().unwrap().unwrap());
        let (sender, receiver) = mpsc::channel();
        let (answer, response) = mpsc::sync_channel(1);
        sender.send(Request::Read(answer)).unwrap(); drop(sender);
        serve(receiver, |_| panic!("read is already focused"), || Err(disconnected()));
        assert_eq!(response.recv().unwrap().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }
}
