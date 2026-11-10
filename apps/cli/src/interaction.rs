use std::cell::RefCell;
use std::io::{self, IsTerminal, Write};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::execution_context::{ExecutionContext, ScopeStack};
use crate::runtime_control::{RuntimeExecutionControl, RuntimeControlScope, InputDisposition, UserInstructionHandoff};
use crate::runtime_input::{RuntimeInput, RuntimeInputMailbox, InputReceipt};

// Only the CLI adapter interprets slash commands. Non-terminal Hosts submit
// the explicit RuntimeInput variant instead of routing by text content.
fn terminal_runtime_input(text: String) -> RuntimeInput {
    if text.trim_start().starts_with('/') { RuntimeInput::ClientControl(text) }
    else { RuntimeInput::UserFollowUp(text) }
}
use crate::i18n::{interaction_text, permission_text, ui_text, UiText};
use crossterm::cursor;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crate::terminal_input::{poll, read};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled, Clear, ClearType};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

static EXECUTION_CANCEL_LISTENER_ACTIVE: AtomicBool = AtomicBool::new(false);
static PERMISSION_PROMPT_PENDING: AtomicUsize = AtomicUsize::new(0);
static PERMISSION_PROMPT_ACTIVE: Mutex<()> = Mutex::new(());
static STDIN_EVENT_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static CURRENT_EXECUTION_CANCELLATION: RefCell<ScopeStack<ExecutionCancellation>> =
        const { RefCell::new(ScopeStack::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionMode {
    Auto,
    Always,
    Never,
}

pub fn stdin_confirmation_available() -> bool {
    match interaction_mode() {
        InteractionMode::Auto => std::io::stdin().is_terminal(),
        InteractionMode::Always => true,
        InteractionMode::Never => false,
    }
}

pub fn confirmation_answer_is_yes(answer: &str) -> bool {
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "是" | "好" | "允许"
    )
}

use crate::runtime_interaction::{PermissionPromptKind, PermissionPrompt, PermissionChoice,
    UserQuestionPrompt, UserQuestionAnswer, RuntimeInteraction};

#[derive(Debug)]
pub struct TerminalInteraction;
impl RuntimeInteraction for TerminalInteraction {
    fn permission_available(&self) -> bool { stdin_confirmation_available() }
    fn request_permission(&self, prompt: &PermissionPrompt) -> PermissionChoice {
        if !stdin_confirmation_available() {
            return PermissionChoice::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
        }
        prompt_permission(prompt)
    }
    fn request_user_question(&self, prompt: &UserQuestionPrompt) -> UserQuestionAnswer { prompt_user_question(prompt) }
}

pub fn prompt_user_question(prompt: &UserQuestionPrompt) -> UserQuestionAnswer {
    if !stdin_confirmation_available() {
        return UserQuestionAnswer::Unavailable;
    }
    let _guard = PermissionPromptActiveGuard::new();
    let _stdin_guard = lock_stdin_events();
    if std::io::stdin().is_terminal() {
        if let Some(answer) = prompt_user_question_keyboard(prompt) {
            return answer;
        }
    }
    prompt_user_question_line(prompt)
}

pub fn prompt_permission(prompt: &PermissionPrompt) -> PermissionChoice {
    crate::llm::log_local_debug_payload(
        "permission_prompt",
        &serde_json::json!({
            "type": "permission_prompt",
            "phase": "presented",
            "kind": format!("{:?}", prompt.kind).to_lowercase(),
            "title": prompt.title,
            "language": format!("{:?}", crate::i18n::current_language()),
        }),
    );
    let _guard = PermissionPromptActiveGuard::new();
    let _stdin_guard = lock_stdin_events();
    let choice =
        prompt_permission_keyboard(prompt).unwrap_or_else(|| prompt_permission_line(prompt));
    let decision = match &choice {
        PermissionChoice::Once => "allow_once",
        PermissionChoice::Session(_) => "allow_session",
        PermissionChoice::Custom(_) | PermissionChoice::ReturnToComposer => "return_to_composer",
        PermissionChoice::TransportInterrupted(_) => "transport_interrupted",
        PermissionChoice::Deny => "deny",
    };
    crate::llm::log_local_debug_payload(
        "permission_prompt",
        &serde_json::json!({
            "type": "permission_prompt",
            "phase": "resolved",
            "kind": format!("{:?}", prompt.kind).to_lowercase(),
            "decision": decision,
        }),
    );
    choice
}

pub fn permission_prompt_active() -> bool {
    PERMISSION_PROMPT_PENDING.load(Ordering::SeqCst) > 0
}

struct PermissionPromptActiveGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
}

/// One modal owns both input and the transient surface. Cleanup also runs on
/// terminal read errors, so a failed picker cannot strand the composer.
pub(crate) struct TerminalModalGuard {
    _input: std::sync::MutexGuard<'static, ()>,
    _prompt: PermissionPromptActiveGuard,
}

impl Drop for TerminalModalGuard {
    fn drop(&mut self) {
        crate::terminal_ui::clear_permission_frame();
    }
}

pub(crate) fn acquire_terminal_modal() -> TerminalModalGuard {
    let prompt = PermissionPromptActiveGuard::new();
    TerminalModalGuard { _input: lock_stdin_events(), _prompt: prompt }
}

impl PermissionPromptActiveGuard {
    fn new() -> Self {
        PERMISSION_PROMPT_PENDING.fetch_add(1, Ordering::SeqCst);
        let guard = match PERMISSION_PROMPT_ACTIVE.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // Queued prompts suspend input, but only the current owner may change
        // the visible surface. Never pause the renderer while waiting for it.
        crate::terminal_ui::pause_for_prompt();
        Self { _guard: guard }
    }
}

impl Drop for PermissionPromptActiveGuard {
    fn drop(&mut self) {
        crate::terminal_ui::clear_permission_frame();
        crate::terminal_ui::resume_after_prompt();
        PERMISSION_PROMPT_PENDING.fetch_sub(1, Ordering::SeqCst);
    }
}

fn lock_stdin_events() -> std::sync::MutexGuard<'static, ()> {
    match STDIN_EVENT_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[allow(dead_code)]
pub struct TerminalPromptInputGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for TerminalPromptInputGuard {
    fn drop(&mut self) {
        PERMISSION_PROMPT_PENDING.fetch_sub(1, Ordering::SeqCst);
    }
}

#[allow(dead_code)]
pub fn acquire_terminal_prompt_input() -> TerminalPromptInputGuard {
    PERMISSION_PROMPT_PENDING.fetch_add(1, Ordering::SeqCst);
    TerminalPromptInputGuard {
        _guard: lock_stdin_events(),
    }
}

pub trait ClientCommandHandler: std::fmt::Debug + Send + Sync {
    fn execute(&self, command: &str);
}

#[derive(Debug, Clone)]
pub struct ExecutionCancellation {
    runtime: RuntimeExecutionControl,
    client_commands: Option<Arc<dyn ClientCommandHandler>>,
    pending_client_command: Arc<Mutex<Option<String>>>,
    listen_to_terminal: bool,
    terminal_listener_active: Arc<AtomicBool>,
    steering: Arc<Mutex<ExecutionSteeringState>>,
}

#[derive(Debug, Default)]
struct ExecutionSteeringState {
    draft: String,
    cursor: usize,
    queued: Arc<Mutex<RuntimeInputMailbox>>,
}

#[derive(Default)]
struct InteractiveComposers {
    current: Arc<Mutex<ExecutionSteeringState>>,
    owner: Option<String>,
    sessions: std::collections::BTreeMap<String, Arc<Mutex<ExecutionSteeringState>>>,
}
thread_local! {
    // The UI thread chooses the owner. Execution listeners and permission
    // callbacks capture its Arc and never follow subsequent selection changes.
    static INTERACTIVE_COMPOSERS: RefCell<InteractiveComposers> = RefCell::new(InteractiveComposers::default());
}

pub(crate) fn select_interactive_composer(owner: Option<String>) {
    INTERACTIVE_COMPOSERS.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.current = match owner.as_ref() {
            Some(owner) => registry.sessions.entry(owner.clone()).or_default().clone(),
            None => Arc::new(Mutex::new(ExecutionSteeringState::default())),
        };
        registry.owner = owner;
    });
}

pub(crate) fn adopt_interactive_composer(owner: String) {
    INTERACTIVE_COMPOSERS.with(|registry| {
        let mut registry = registry.borrow_mut();
        // First submission assigns an ID to the currently anonymous editor.
        // Existing named state is never rebound into another conversation.
        if registry.owner.is_none() && !registry.sessions.contains_key(&owner) {
            let state = registry.current.clone();
            registry.sessions.insert(owner.clone(), state);
            registry.owner = Some(owner);
        }
    });
}

fn interactive_steering_state() -> Arc<Mutex<ExecutionSteeringState>> {
    INTERACTIVE_COMPOSERS.with(|registry| registry.borrow().current.clone())
}

pub(crate) fn clear_interactive_execution_draft() {
    let state = interactive_steering_state();
    let mut state = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    state.draft.clear();
    state.cursor = 0;
}

pub(crate) fn take_interactive_queued_inputs() -> Vec<String> {
    let state = interactive_steering_state();
    let queue = state.lock().unwrap_or_else(|p| p.into_inner()).queued.clone();
    let inputs = queue.lock().unwrap_or_else(|p| p.into_inner()).drain_all();
    inputs
}

pub(crate) fn interactive_composer_snapshot() -> ExecutionInputSnapshot {
    let state = interactive_steering_state();
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    let queued_count = state.queued.lock().unwrap_or_else(|p| p.into_inner()).len();
    ExecutionInputSnapshot { draft: state.draft.clone(), cursor: state.cursor, queued_count }
}

pub(crate) fn update_interactive_composer(draft: &str, cursor: usize) {
    let state = interactive_steering_state();
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.draft.clear();
    state.draft.push_str(draft);
    state.cursor = cursor.min(draft.len());
}

#[derive(Debug, Clone)]
pub struct ExecutionInputSnapshot {
    pub draft: String,
    pub cursor: usize,
    pub queued_count: usize,
}

#[derive(Debug)]
struct ComposerInstructionHandoff(Arc<Mutex<ExecutionSteeringState>>);
impl UserInstructionHandoff for ComposerInstructionHandoff {
    fn handoff(&self, text: String) -> Result<(), String> {
        let mut draft = self.0.lock().map_err(|_| "composer draft poisoned")?;
        draft.draft = text;
        draft.cursor = draft.draft.len();
        Ok(())
    }
}
impl Default for ExecutionCancellation {
    fn default() -> Self {
        Self::with_steering(Arc::new(Mutex::new(ExecutionSteeringState::default())), false)
    }
}
impl ExecutionCancellation {
    fn with_steering(steering: Arc<Mutex<ExecutionSteeringState>>, interactive: bool) -> Self {
        let mailbox = steering.lock().unwrap_or_else(|p|p.into_inner()).queued.clone();
        let mut runtime = RuntimeExecutionControl::default().with_interaction(Arc::new(TerminalInteraction)).with_input_adapter(
            mailbox, if interactive {InputDisposition::NextTurn} else {InputDisposition::ActiveBoundary},
            Arc::new(ComposerInstructionHandoff(steering.clone())));
        if interactive {runtime = runtime.with_background_transfer();}
        Self { runtime, client_commands: None, pending_client_command: Default::default(), listen_to_terminal:interactive,
            terminal_listener_active:Arc::new(AtomicBool::new(false)), steering }
    }
    pub fn interactive() -> Self { Self::with_steering(interactive_steering_state(), true) }
    pub fn interactive_deferred_input() -> Self { Self::interactive() }
    pub fn with_client_commands(mut self, handler: Arc<dyn ClientCommandHandler>) -> Self {
        self.client_commands = Some(handler);
        self
    }
    fn dispatch_client_command(&self) {
        let command = self.pending_client_command.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(command) = command {
            if crate::is_exit_command(&command) || crate::is_resume_command(&command) || crate::is_continue_command(&command) {
                // Exit is a local lifecycle command, never an Agent follow-up.
                // Interrupt now, then leave the composer after the normal
                // cancellation handoff has committed its actual outcome.
                if self.queue_runtime_input(RuntimeInput::ClientControl(command.clone())).is_ok() {
                    self.request();
                    crate::terminal_ui::commit_history(if crate::is_exit_command(&command) {
                        "正在停止当前任务并退出；未确认的操作会保留恢复状态。"
                    } else {
                        "正在停止当前前台任务并打开会话选择；待发送输入仍归属原会话。"
                    });
                }
                return;
            }
            if let Some(handler) = &self.client_commands {
                handler.execute(&command);
            } else {
                crate::terminal_ui::commit_history("Command unavailable in this execution context; return to the interactive prompt.");
            }
        }
    }
    pub fn runtime_control(&self) -> RuntimeExecutionControl { self.runtime.clone() }
    pub fn context(&self) -> ExecutionContext { self.runtime.context() }
    pub fn request(&self) { self.runtime.request(); }
    pub fn is_requested(&self) -> bool { self.runtime.is_requested() }
    pub fn execution_id(&self) -> u64 { self.runtime.execution_id() }
    pub fn request_background(&self) -> bool { self.runtime.request_background() }
    pub fn is_background_requested(&self) -> bool { self.runtime.is_background_requested() }
    pub fn take_background_request(&self) -> bool { self.runtime.take_background_request() }
    pub fn supports_background_transfer(&self) -> bool { self.runtime.supports_background_transfer() }
    pub fn child(&self) -> Self { Self { runtime:self.runtime.child(), ..self.clone() } }
    pub fn accepts_terminal_input(&self) -> bool {
        self.listen_to_terminal && self.terminal_listener_active.load(Ordering::SeqCst) && std::io::stdin().is_terminal()
    }
    pub fn wait_for_terminal_listener_release(&self) {
        for _ in 0..200 {
            if !self.terminal_listener_active.load(Ordering::SeqCst) {return}
            thread::sleep(Duration::from_millis(5));
        }
    }
    pub fn input_snapshot(&self) -> ExecutionInputSnapshot {
        let state = self.steering.lock().unwrap_or_else(|p|p.into_inner());
        let queued_count=state.queued.lock().unwrap_or_else(|p|p.into_inner()).len();
        ExecutionInputSnapshot {draft:state.draft.clone(),cursor:state.cursor.min(state.draft.len()),queued_count}
    }
    pub(crate) fn bind_session_input(&self, session: impl Into<String>, retention: crate::runtime_control::InputRetention) -> Result<(), String> {
        self.runtime.bind_session_input(session, retention)
    }
    pub fn take_queued_inputs(&self) -> Vec<String> { self.runtime.take_queued_inputs() }
    pub fn take_boundary_user_inputs(&self) -> Vec<String> { self.runtime.take_boundary_user_inputs() }
    pub(crate) fn queue_runtime_input(&self, input: RuntimeInput) -> Result<InputReceipt,String> {
        self.runtime.queue_runtime_input(input)
    }
    fn has_draft_input(&self) -> bool {
        !self.steering.lock().unwrap_or_else(|p|p.into_inner()).draft.is_empty()
    }
    fn queue_draft_input(&self) {submit_execution_draft(self);}
    fn commit_runtime_input(&self, queued: String) -> Result<InputReceipt,String> {
        match terminal_runtime_input(queued) {
            RuntimeInput::ClientControl(command) => {
                *self.pending_client_command.lock().unwrap_or_else(|p| p.into_inner()) = Some(command);
                Ok(InputReceipt::Local)
            }
            input => self.queue_runtime_input(input),
        }
    }
    pub fn queue_input(&self, text:String) -> bool {
        self.runtime.enqueue_local(terminal_runtime_input(text)).is_ok()
    }
    pub fn set_draft_input(&self, text:String) {
        let mut state=self.steering.lock().unwrap_or_else(|p|p.into_inner());
        state.draft=text;state.cursor=state.draft.len();
    }
    pub fn draft_input(&self) -> Option<String> {
        let state=self.steering.lock().unwrap_or_else(|p|p.into_inner());
        (!state.draft.trim().is_empty()).then(||state.draft.clone())
    }
    pub fn enter(&self) -> ExecutionScopeGuard {
        let context_scope=self.runtime.enter();
        let binding=CURRENT_EXECUTION_CANCELLATION.with(|current|current.borrow_mut().push(self.clone()));
        ExecutionScopeGuard{_context_scope:context_scope,binding}
    }
}

pub struct ExecutionScopeGuard {
    _context_scope: RuntimeControlScope,
    binding: u64,
}

impl Drop for ExecutionScopeGuard {
    fn drop(&mut self) {
        CURRENT_EXECUTION_CANCELLATION.with(|current| {
            current.borrow_mut().remove(self.binding);
        });
    }
}

pub(crate) fn current_execution_cancellation() -> Option<ExecutionCancellation> {
    CURRENT_EXECUTION_CANCELLATION.with(|current| current.borrow().current().cloned())
}

pub struct ExecutionCancelListener {
    stop: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    execution_id: u64,
}

impl Drop for ExecutionCancelListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.active.store(false, Ordering::SeqCst);
        EXECUTION_CANCEL_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
        crate::terminal_ui::end_turn(self.execution_id);
    }
}

pub fn start_execution_cancel_listener(
    cancellation: &ExecutionCancellation,
) -> Option<ExecutionCancelListener> {
    let stdin_is_terminal = std::io::stdin().is_terminal();
    if !cancellation.listen_to_terminal || !stdin_is_terminal {
        return None;
    }
    if EXECUTION_CANCEL_LISTENER_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return None;
    }
    if enable_raw_mode().is_err() {
        EXECUTION_CANCEL_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
        return None;
    }
    restore_terminal_newline_output();
    cancellation
        .terminal_listener_active
        .store(true, Ordering::SeqCst);
    crate::terminal_ui::begin_turn(cancellation.execution_id());
    crate::terminal_ui::sync_composer(cancellation);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread_cancellation = cancellation.clone();
    let thread_active = Arc::clone(&cancellation.terminal_listener_active);
    let mut view_store = crate::cli_view_state::active_store();
    let handle = thread::spawn(move || {
        let mut terminal_prompt_was_active = false;
        loop {
            // The persistent event source retains unread keys. Ending a turn
            // transfers focus, not stdin ownership; no timed drain is needed.
            if thread_stop.load(Ordering::SeqCst) { break; }
            if permission_prompt_active() {
                terminal_prompt_was_active = true;
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            let stdin_guard = match STDIN_EVENT_LOCK.try_lock() {
                Ok(guard) => guard,
                Err(std::sync::TryLockError::WouldBlock) => {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            };
            if permission_prompt_active() {
                terminal_prompt_was_active = true;
                drop(stdin_guard);
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            if terminal_prompt_was_active {
                if enable_raw_mode().is_err() {
                    break;
                }
                restore_terminal_newline_output();
                terminal_prompt_was_active = false;
            }
            let mut view_changed = false;
            let stop_after_input = match poll(Duration::from_millis(25)) {
                Ok(true) => match read() {
                    // Terminals encode Alt+key as ESC followed by the key. If a
                    // user starts typing immediately after pressing Esc,
                    // crossterm can legitimately coalesce both physical keys
                    // into one Alt-modified event. During execution there are
                    // no Alt editing commands, so preserve the user's intent:
                    // cancel the turn and hand the following character to the
                    // persistent composer instead of silently dropping it.
                    Ok(Event::Key(mut key))
                        if key.kind == KeyEventKind::Press
                            && matches!(key.code, KeyCode::Char(_))
                            && key.modifiers.contains(KeyModifiers::ALT)
                            && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        thread_cancellation.request();
                        crate::terminal_ui::mark_turn_cancelling(
                            thread_cancellation.execution_id(),
                        );
                        view_changed = true;
                        key.modifiers.remove(KeyModifiers::ALT);
                        update_execution_input_from_key(&thread_cancellation, &key);
                        crate::terminal_ui::sync_composer(&thread_cancellation);
                        false
                    }
                    Ok(Event::Key(key)) if crate::composer_media::paste_key(&key) => {
                        match crate::composer_media::paste_image() {
                            Ok(marker) => append_execution_input(&thread_cancellation, &marker),
                            Err(error) => crate::terminal_ui::commit_history(&crate::composer_media::paste_error(&error)),
                        }
                        view_changed = true;
                        crate::terminal_ui::sync_composer(&thread_cancellation);
                        false
                    }
                    Ok(Event::Key(key)) if execution_cancel_key(&key) => {
                        thread_cancellation.request();
                        crate::terminal_ui::mark_turn_cancelling(
                            thread_cancellation.execution_id(),
                        );
                        // Keep the turn-scoped listener alive until the turn owner
                        // drops it. Exiting here races terminal mode restoration
                        // against the next composer and can consume its first key.
                        false
                    }
                    Ok(Event::Key(key)) if execution_background_key(&key) => {
                        thread_cancellation.request_background();
                        false
                    }
                    Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                        view_changed = true;
                        if !crate::terminal_ui::scroll_history_key(&key) {
                            update_execution_input_from_key(&thread_cancellation, &key);
                        }
                        // Preserve both scrolling and editing during execution.

                        crate::terminal_ui::sync_composer(&thread_cancellation);
                        false
                    }
                    Ok(Event::Paste(text)) => {
                        view_changed = true;
                        match crate::composer_media::paste_path(&text) {
                            Ok(marker) => append_execution_input(&thread_cancellation, marker.as_deref().unwrap_or(&text)),
                            Err(error) => crate::terminal_ui::commit_history(&crate::composer_media::paste_error(&error)),
                        }
                        crate::terminal_ui::sync_composer(&thread_cancellation);
                        false
                    }
                    Ok(Event::Resize(_, _)) => {
                        view_changed = true;
                        crate::terminal_ui::sync_composer(&thread_cancellation);
                        crate::terminal_ui::invalidate_presented_screen();
                        false
                    }
                    Ok(_) => false,
                    Err(_) => true,
                },
                Ok(false) => false,
                Err(_) => true,
            };
            drop(stdin_guard);
            if view_changed {
                if let Some(store) = view_store.as_mut() {
                    let snapshot = thread_cancellation.input_snapshot();
                    if let Err(error) = store.save(crate::cli_view_state::ViewState {
                        draft: snapshot.draft, cursor: snapshot.cursor,
                        viewport: crate::terminal_ui::capture_viewport_bookmark(),
                    }) {
                        crate::terminal_ui::commit_history(&format!("无法保存 CLI 显示状态：{error}"));
                        view_store = None;
                    }
                }
            }
            // Modal commands acquire stdin themselves. Never invoke their UI
            // while holding the listener's stdin lock, or enqueue them as chat.
            thread_cancellation.dispatch_client_command();
            if stop_after_input {
                break;
            }
        }
        let _ = disable_raw_mode();
        thread_active.store(false, Ordering::SeqCst);
        EXECUTION_CANCEL_LISTENER_ACTIVE.store(false, Ordering::SeqCst);
    });
    Some(ExecutionCancelListener {
        stop,
        active: Arc::clone(&cancellation.terminal_listener_active),
        handle: Some(handle),
        execution_id: cancellation.execution_id(),
    })
}

#[cfg(test)]
fn process_execution_terminal_bytes(
    cancellation: &ExecutionCancellation,
    bytes: &[u8],
    pending_utf8: &mut Vec<u8>,
) -> bool {
    for byte in bytes.iter().copied() {
        if matches!(byte, 0x1b | 0x03) {
            flush_execution_terminal_utf8(cancellation, pending_utf8);
            cancellation.request();
            return true;
        }
        if byte < 0x20 || byte == 0x7f {
            flush_execution_terminal_utf8(cancellation, pending_utf8);
            if matches!(byte, b'\r' | b'\n') {
                submit_execution_draft(cancellation);
                continue;
            }
            let mut state = cancellation
                .steering
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match byte {
                0x08 | 0x7f => {
                    state.draft.pop();
                }
                0x15 => state.draft.clear(),
                _ => {}
            }
            continue;
        }
        pending_utf8.push(byte);
        flush_execution_terminal_utf8(cancellation, pending_utf8);
    }
    false
}

#[cfg(test)]
fn flush_execution_terminal_utf8(cancellation: &ExecutionCancellation, pending_utf8: &mut Vec<u8>) {
    loop {
        match std::str::from_utf8(pending_utf8) {
            Ok(text) => {
                if !text.is_empty() {
                    append_execution_input(cancellation, text);
                }
                pending_utf8.clear();
                return;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    if let Ok(text) = std::str::from_utf8(&pending_utf8[..valid]) {
                        append_execution_input(cancellation, text);
                    }
                    pending_utf8.drain(..valid);
                    continue;
                }
                let Some(invalid_len) = error.error_len() else {
                    return;
                };
                pending_utf8.drain(..invalid_len.min(pending_utf8.len()));
            }
        }
    }
}

fn execution_cancel_key(key: &crossterm::event::KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && (key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)))
}

fn execution_background_key(key: &crossterm::event::KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && key.code == KeyCode::Char('b')
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn update_execution_input_from_key(
    cancellation: &ExecutionCancellation,
    key: &crossterm::event::KeyEvent,
) {
    if crate::composer::is_submit(key) {
        submit_execution_draft(cancellation);
        return;
    }
    let mut state = cancellation
        .steering
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let ExecutionSteeringState { draft, cursor, .. } = &mut *state;
    crate::composer::edit(draft, cursor, key);
}

fn submit_execution_draft(cancellation: &ExecutionCancellation) {
    let committed = cancellation.draft_input();
    if let Some(committed) = committed {
        let receipt = cancellation.commit_runtime_input(committed.clone());
        if receipt.is_ok() {
            let mut state = cancellation.steering.lock().unwrap_or_else(|p| p.into_inner());
            if state.draft == committed {
                state.draft.clear();
                state.cursor = 0;
            }
        } else if cancellation.listen_to_terminal {
            crate::terminal_ui::commit_history(&format!("\n⚠ {}\n", ui_text(UiText::InputAdmissionFailed)));
        }
        if cancellation.listen_to_terminal && receipt == Ok(InputReceipt::Durable) {
            crate::agent::print_queued_follow_up(&committed);
        }
    }
}

fn append_execution_input(cancellation: &ExecutionCancellation, value: &str) {
    let mut state = cancellation
        .steering
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let value = crate::composer::normalize_paste(value);
    let cursor = state.cursor.min(state.draft.len());
    state.draft.insert_str(cursor, &value);
    state.cursor = cursor + value.len();
}

fn previous_execution_char_boundary(value: &str, cursor: usize) -> Option<usize> {
    value
        .get(..cursor.min(value.len()))?
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
}

fn next_execution_char_boundary(value: &str, cursor: usize) -> Option<usize> {
    let cursor = cursor.min(value.len());
    let ch = value.get(cursor..)?.chars().next()?;
    Some(cursor + ch.len_utf8())
}

fn previous_execution_word_boundary(value: &str, cursor: usize) -> usize {
    let before = value.get(..cursor.min(value.len())).unwrap_or_default();
    let trimmed = before.trim_end_matches(char::is_whitespace);
    trimmed
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0)
}

fn prompt_permission_line(prompt: &PermissionPrompt) -> PermissionChoice {
    print!("{}", render_permission_prompt(prompt));
    if std::io::stdout().flush().is_err() {
        return PermissionChoice::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
        );
    }
    permission_choice_from_reader(prompt, std::io::stdin().lock())
}

fn permission_choice_from_reader(
    prompt: &PermissionPrompt,
    reader: impl io::BufRead,
) -> PermissionChoice {
    use nola_host_protocol::PermissionTransportInterruptionReason::{Disconnected, Unavailable};
    let mut bounded = io::Read::take(reader, 4097);
    let mut answer = String::new();
    match io::BufRead::read_line(&mut bounded, &mut answer) {
        Ok(0) | Err(_) => return PermissionChoice::TransportInterrupted(Disconnected),
        Ok(count) if count > 4096 => return PermissionChoice::TransportInterrupted(Unavailable),
        Ok(_) => {}
    }
    let trimmed = answer.trim();
    if let Some(choice) = permission_instruction_choice(prompt.supports_instruction, trimmed) {
        return choice;
    }
    if confirmation_answer_is_yes(trimmed) || trimmed == "1" {
        return PermissionChoice::Once;
    }
    if prompt.allow_conversation && (trimmed.eq_ignore_ascii_case("s") || trimmed == "2") {
        return PermissionChoice::Session(prompt.session_value.clone());
    }
    if trimmed.eq_ignore_ascii_case("c") || trimmed == "3" {
        return return_to_composer_choice();
    }
    PermissionChoice::Deny
}

fn prompt_permission_keyboard(prompt: &PermissionPrompt) -> Option<PermissionChoice> {
    if !stdin_confirmation_available() {
        return Some(PermissionChoice::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
        ));
    }
    if !std::io::stdin().is_terminal() {
        return None;
    }
    let _raw_mode = match PermissionRawModeGuard::new() {
        Ok(mode) => mode,
        Err(_) => {
            return Some(PermissionChoice::TransportInterrupted(
                nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
            ))
        }
    };
    let mut selected = 0usize;
    let mut rendered_lines = 0usize;
    if draw_permission_prompt_frame(prompt, selected, rendered_lines).is_err() {
        return Some(PermissionChoice::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
        ));
    }
    rendered_lines = render_permission_prompt_with_selection(prompt, Some(selected))
        .lines()
        .count();
    loop {
        let event = match read() {
            Ok(event) => event,
            Err(_) => {
                return Some(PermissionChoice::TransportInterrupted(
                    nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
                ))
            }
        };
        if matches!(event, Event::Resize(_, _)) {
            if draw_permission_prompt_after_resize(prompt, selected).is_err() {
                return Some(PermissionChoice::TransportInterrupted(
                    nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
                ));
            }
            rendered_lines = render_permission_prompt_with_selection(prompt, Some(selected))
                .lines()
                .count();
            continue;
        }
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match crate::terminal_input::modal_key(&key) {
            crate::terminal_input::ModalKey::Cancel => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                return Some(PermissionChoice::Deny);
            }
            crate::terminal_input::ModalKey::Ignore => continue,
            crate::terminal_input::ModalKey::Action => {}
        }
        match key.code {
            KeyCode::PageUp | KeyCode::PageDown => {
                crate::terminal_ui::scroll_permission_frame(key.code == KeyCode::PageDown);
            }
            KeyCode::Up | KeyCode::Left => {
                selected = if selected == 0 { 3 } else { selected - 1 };
                if selected == 1 && !prompt.allow_conversation { selected = 0; }
                if draw_permission_prompt_frame(prompt, selected, rendered_lines).is_err() {
                    return Some(PermissionChoice::TransportInterrupted(
                        nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
                    ));
                }
                rendered_lines = render_permission_prompt_with_selection(prompt, Some(selected))
                    .lines()
                    .count();
            }
            KeyCode::Down | KeyCode::Right => {
                selected = (selected + 1) % 4;
                if selected == 1 && !prompt.allow_conversation { selected = 2; }
                if draw_permission_prompt_frame(prompt, selected, rendered_lines).is_err() {
                    return Some(PermissionChoice::TransportInterrupted(
                        nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
                    ));
                }
                rendered_lines = render_permission_prompt_with_selection(prompt, Some(selected))
                    .lines()
                    .count();
            }
            KeyCode::Enter => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                let choice = selected_permission_choice(prompt, selected);
                return Some(choice);
            }
            KeyCode::Esc => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                return Some(PermissionChoice::Deny);
            }
            KeyCode::Char('1') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                return Some(PermissionChoice::Once);
            }
            KeyCode::Char('2') | KeyCode::Char('s') | KeyCode::Char('S') if prompt.allow_conversation => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                return Some(PermissionChoice::Session(prompt.session_value.clone()));
            }
            KeyCode::Char('3') | KeyCode::Char('c') | KeyCode::Char('C') => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                return Some(return_to_composer_choice());
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                return Some(PermissionChoice::Deny);
            }
            KeyCode::Char('i') | KeyCode::Char('I') if prompt.supports_instruction => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                match read_permission_instruction() {
                    Ok(Some(instruction)) => return Some(PermissionChoice::Custom(instruction)),
                    Ok(None) => {}
                    Err(reason) => return Some(PermissionChoice::TransportInterrupted(reason)),
                }
                if draw_permission_prompt_frame(prompt, selected, 0).is_err() {
                    return Some(PermissionChoice::TransportInterrupted(
                        nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
                    ));
                }
            }
            _ => {}
        }
    }
}

fn read_permission_instruction(
) -> Result<Option<String>, nola_host_protocol::PermissionTransportInterruptionReason> {
    read_modal_answer()
}

fn render_modal_answer(editor: &crate::terminal_input::ModalEditor, columns: usize) -> String {
    let width = columns.saturating_sub(4).max(1);
    let mut draft = editor.text.clone();
    draft.insert(editor.cursor, '▏');
    let mut rows = vec![ui_text(UiText::Answer).to_string()];
    for logical in draft.split('\n') {
        for line in wrap_display_width(logical, width) {
            rows.push(format!("{} {line}", if line.contains('▏') { "›" } else { " " }));
        }
    }
    rows.push(format!("Enter ✓ · Esc / Ctrl+C × · Shift+Enter ↵ · ←/→ · {}/4096", editor.text.len()));
    rows.join("\n")
}

fn read_modal_answer(
) -> Result<Option<String>, nola_host_protocol::PermissionTransportInterruptionReason> {
    use nola_host_protocol::PermissionTransportInterruptionReason::Disconnected;
    use crate::terminal_input::{EditorAction, ModalEditor};
    let mut editor = ModalEditor::default();
    loop {
        crate::terminal_ui::present_permission_frame(
            &render_modal_answer(&editor, terminal_columns()), true,
        ).map_err(|_| Disconnected)?;
        match editor.apply(read().map_err(|_| Disconnected)?) {
            EditorAction::Cancel => {
                crate::terminal_ui::clear_permission_frame();
                return Ok(None);
            }
            EditorAction::Submit(answer) => {
                crate::terminal_ui::clear_permission_frame();
                return Ok(Some(answer));
            }
            EditorAction::Redraw | EditorAction::Ignore => {}
        }
    }
}

fn permission_instruction_choice(enabled: bool, answer: &str) -> Option<PermissionChoice> {
    if !enabled {
        return None;
    }
    let instruction = answer
        .strip_prefix("i ")
        .or_else(|| answer.strip_prefix("I "))?
        .trim();
    (!instruction.is_empty() && instruction.len() <= 4096)
        .then(|| PermissionChoice::Custom(instruction.to_string()))
}

#[cfg(test)]
mod permission_instruction_input_tests {
    use super::*;

    #[test]
    fn multiline_permission_review_keeps_every_line_inside_panel() {
        let prompt=PermissionPrompt {allow_conversation:false,kind:PermissionPromptKind::Tool,
            title:"Room permission".into(),rows:vec![("Operation".into(),"target\nMode: replace\r\nBytes: 47\nSHA: fixture".into())],
            session_label:"Session".into(),session_value:"context".into(),custom_label:"Instruction".into(),supports_instruction:true};
        let rendered=render_permission_prompt(&prompt);
        for text in ["target","Mode: replace","Bytes: 47","SHA: fixture"] {
            let line=rendered.lines().find(|line|line.contains(text)).unwrap();
            assert!(line.contains('│') && line.matches('│').count()==2,"{line:?}");
            assert!(!line.contains('\r'));
        }
    }

    #[test]
    fn once_only_prompt_explains_disabled_session_without_offering_its_shortcut() {
        let prompt = PermissionPrompt {allow_conversation:false,kind:PermissionPromptKind::Tool,
            title:"Opaque invocation".into(),rows:vec![],session_label:"FORBIDDEN_SESSION_OPTION".into(),
            session_value:"scope".into(),custom_label:"Return".into(),supports_instruction:true};
        let rendered = render_permission_prompt(&prompt);
        assert!(!rendered.contains("FORBIDDEN_SESSION_OPTION"));
        assert!(rendered.contains("2."));
        assert!(!rendered.contains("1/2/3/N"));
        assert!(rendered.contains("1/3/N"));
        for selected in [0, 2, 3] {
            let frame = render_permission_prompt_with_selection(&prompt, Some(selected));
            let option = ["1.", "2.", "3.", "N."][selected];
            assert!(frame.contains(&format!("› {option}")), "{frame}");
            assert!(!frame.contains("› 2."));
        }
        assert!(!render_permission_prompt_with_selection(&prompt, Some(1)).contains("› 2."));
        let enabled = PermissionPrompt { allow_conversation:true, ..prompt.clone() };
        assert!(render_permission_prompt_with_selection(&enabled,Some(1)).contains("› 2."));
        assert!(render_permission_prompt(&enabled).contains("1/2/3/N"));
        assert_eq!(permission_choice_from_reader(&enabled,io::Cursor::new(b"2\n")), PermissionChoice::Session("scope".into()));
        assert_eq!(permission_choice_from_reader(&prompt,io::Cursor::new(b"1\n")),PermissionChoice::Once);
        for answer in [b"2\n",b"s\n"] {
            assert_eq!(permission_choice_from_reader(&prompt,io::Cursor::new(answer)),PermissionChoice::Deny);
        }
        assert_eq!(selected_permission_choice(&prompt,1),PermissionChoice::Deny);
    }

    #[test]
    fn permission_transport_line_reader_distinguishes_eof_from_explicit_deny() {
        use nola_host_protocol::PermissionTransportInterruptionReason::{
            Disconnected, Unavailable,
        };
        let prompt = PermissionPrompt {
            allow_conversation: true,
            kind: PermissionPromptKind::Tool,
            title: "Test".into(),
            rows: vec![],
            session_label: "Session".into(),
            session_value: "scope".into(),
            custom_label: "Custom".into(),
            supports_instruction: true,
        };
        assert_eq!(
            permission_choice_from_reader(&prompt, io::Cursor::new(b"")),
            PermissionChoice::TransportInterrupted(Disconnected)
        );
        assert_eq!(
            permission_choice_from_reader(&prompt, io::Cursor::new(b"n\n")),
            PermissionChoice::Deny
        );
        assert_eq!(
            permission_choice_from_reader(&prompt, io::Cursor::new(b"1\n")),
            PermissionChoice::Once
        );
        let mut reader = io::Cursor::new(vec![b'x'; 1_000_000]);
        assert_eq!(
            permission_choice_from_reader(&prompt, &mut reader),
            PermissionChoice::TransportInterrupted(Unavailable)
        );
        assert_eq!(reader.position(), 4097);
    }

    #[test]
    fn explicit_instruction_input_never_grants_or_cancels() {
        for instruction in [
            "请保留原文件",
            "Please keep the original",
            "允许 once",
            "search current",
        ] {
            assert_eq!(
                permission_instruction_choice(true, &format!("i {instruction}  ")),
                Some(PermissionChoice::Custom(instruction.to_string()))
            );
        }
        assert_eq!(
            permission_instruction_choice(false, "i preserve original"),
            None
        );
        assert_eq!(permission_instruction_choice(true, "i   "), None);
        assert_eq!(
            permission_instruction_choice(true, &format!("i {}", "字".repeat(1366))),
            None
        );
        assert_eq!(permission_instruction_choice(true, "1"), None);
    }
}

fn selected_permission_choice(
    prompt: &PermissionPrompt,
    selected: usize,
) -> PermissionChoice {
    match selected {
        0 => PermissionChoice::Once,
        1 if prompt.allow_conversation => PermissionChoice::Session(prompt.session_value.clone()),
        2 => return_to_composer_choice(),
        _ => PermissionChoice::Deny,
    }
}

fn return_to_composer_choice() -> PermissionChoice {
    if let Some(cancellation) = current_execution_cancellation() {
        cancellation.request();
    }
    PermissionChoice::ReturnToComposer
}

fn draw_permission_prompt_frame(
    prompt: &PermissionPrompt,
    selected: usize,
    previous_lines: usize,
) -> io::Result<()> {
    crate::terminal_ui::present_permission_frame(&render_permission_prompt_with_selection(
        prompt,
        Some(selected),
    ), previous_lines > 0)
}

fn draw_permission_prompt_after_resize(
    prompt: &PermissionPrompt,
    selected: usize,
) -> io::Result<()> {
    draw_permission_prompt_frame(prompt, selected, 0)
}

fn clear_permission_prompt_frame(rendered_lines: usize) -> io::Result<()> {
    if crate::terminal_ui::clear_permission_frame() {
        return Ok(());
    }
    if rendered_lines == 0 {
        return Ok(());
    }
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        cursor::RestorePosition,
        Clear(ClearType::FromCursorDown)
    )?;
    stdout.flush()
}

fn prompt_user_question_line(prompt: &UserQuestionPrompt) -> UserQuestionAnswer {
    let localized = interaction_text();
    print!("{}", render_user_question_prompt(prompt, None));
    let _ = std::io::stdout().flush();
    loop {
        print!(
            "{} 1-{}{}: ",
            localized.question_input_prefix,
            prompt.options.len(),
            if prompt.allow_freeform {
                localized.question_or_direct_answer
            } else {
                ""
            }
        );
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        let Ok(read) = std::io::stdin().read_line(&mut answer) else {
            return UserQuestionAnswer::Cancelled;
        };
        if read == 0 {
            return UserQuestionAnswer::Cancelled;
        }
        let answer = answer.trim();
        if answer == "/decline" { return UserQuestionAnswer::Declined; }
        if answer.is_empty() || answer.eq_ignore_ascii_case("q") {
            return UserQuestionAnswer::Cancelled;
        }
        if let Ok(number) = answer.parse::<usize>() {
            if let Some(index) = number
                .checked_sub(1)
                .filter(|index| *index < prompt.options.len())
            {
                return UserQuestionAnswer::Selected {
                    index,
                    label: prompt.options[index].0.clone(),
                };
            }
            println!(
                "{}{}",
                localized.question_invalid_range_prefix,
                prompt.options.len()
            );
            continue;
        }
        if prompt.allow_freeform {
            return UserQuestionAnswer::Freeform(answer.to_string());
        }
        println!("{}", localized.question_invalid);
    }
}

fn prompt_user_question_keyboard(prompt: &UserQuestionPrompt) -> Option<UserQuestionAnswer> {
    let _raw_mode = PermissionRawModeGuard::new().ok()?;
    let option_count = prompt.options.len() + if prompt.allow_freeform { 1 } else { 0 };
    if option_count == 0 {
        return Some(UserQuestionAnswer::Unavailable);
    }
    let mut selected = 0usize;
    let mut rendered_lines = 0usize;
    if draw_user_question_frame(prompt, selected, rendered_lines).is_err() {
        return Some(UserQuestionAnswer::Cancelled);
    }
    rendered_lines = render_user_question_prompt(prompt, Some(selected))
        .lines()
        .count();
    loop {
        let event = match read() {
            Ok(event) => event,
            Err(_) => return Some(UserQuestionAnswer::Cancelled),
        };
        if matches!(event, Event::Resize(_, _)) {
            if draw_user_question_frame(prompt, selected, rendered_lines).is_err() {
                return Some(UserQuestionAnswer::Cancelled);
            }
            rendered_lines = render_user_question_prompt(prompt, Some(selected))
                .lines()
                .count();
            continue;
        }
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match crate::terminal_input::modal_key(&key) {
            crate::terminal_input::ModalKey::Cancel => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                return Some(UserQuestionAnswer::Cancelled);
            }
            crate::terminal_input::ModalKey::Ignore => continue,
            crate::terminal_input::ModalKey::Action => {}
        }
        match key.code {
            KeyCode::Char('d') => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                return Some(UserQuestionAnswer::Declined);
            }
            KeyCode::Up | KeyCode::Left => {
                selected = if selected == 0 {
                    option_count - 1
                } else {
                    selected - 1
                };
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Tab => {
                selected = (selected + 1) % option_count;
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                crate::terminal_ui::scroll_permission_frame(key.code == KeyCode::PageDown);
                continue;
            }
            KeyCode::Enter => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                if selected < prompt.options.len() {
                    return Some(UserQuestionAnswer::Selected {
                        index: selected,
                        label: prompt.options[selected].0.clone(),
                    });
                }
                return Some(read_user_question_freeform());
            }
            KeyCode::Esc => {
                let _ = clear_permission_prompt_frame(rendered_lines);
                return Some(UserQuestionAnswer::Cancelled);
            }
            KeyCode::Char(value) if value.is_ascii_digit() => {
                let index = value.to_digit(10).unwrap_or_default() as usize;
                if let Some(index) = index
                    .checked_sub(1)
                    .filter(|index| *index < prompt.options.len())
                {
                    let _ = clear_permission_prompt_frame(rendered_lines);
                    return Some(UserQuestionAnswer::Selected {
                        index,
                        label: prompt.options[index].0.clone(),
                    });
                }
                continue;
            }
            _ => continue,
        }
        if draw_user_question_frame(prompt, selected, rendered_lines).is_err() {
            return Some(UserQuestionAnswer::Cancelled);
        }
        rendered_lines = render_user_question_prompt(prompt, Some(selected))
            .lines()
            .count();
    }
}

fn read_user_question_freeform() -> UserQuestionAnswer {
    match read_modal_answer() {
        Ok(Some(answer)) => UserQuestionAnswer::Freeform(answer),
        Ok(None) | Err(_) => UserQuestionAnswer::Cancelled,
    }
}

fn draw_user_question_frame(
    prompt: &UserQuestionPrompt,
    selected: usize,
    previous_lines: usize,
) -> io::Result<()> {
    crate::terminal_ui::present_permission_frame(
        &render_user_question_prompt(prompt, Some(selected)),
        previous_lines > 0,
    )
}

fn adaptive_panel_inner_width(rows: &[String], columns: usize) -> usize {
    // Keep a real margin on narrow terminals and a readable line length on
    // wide displays. Content wider than this is wrapped, never truncated.
    let available = columns.saturating_sub(4).max(8);
    let minimum = available.min(32);
    let readable_max = if columns >= 128 {
        available.min(100)
    } else if columns >= 80 {
        available.min(columns.saturating_mul(4) / 5)
    } else {
        available
    }
    .max(minimum);
    rows.iter()
        .map(|row| display_width(row))
        .max()
        .unwrap_or(minimum)
        .clamp(minimum, readable_max)
}

fn render_user_question_prompt(prompt: &UserQuestionPrompt, selected: Option<usize>) -> String {
    let localized = interaction_text();
    let color = permission_color_enabled();
    let accent = "\x1b[38;5;117m";
    let mut rows = vec![
        format!("◆ {}", prompt.header),
        prompt.question.clone(),
        String::new(),
    ];
    let option_start = rows.len();
    for (index, (label, description)) in prompt.options.iter().enumerate() {
        let marker = if selected == Some(index) { "›" } else { " " };
        let description = if description.is_empty() {
            String::new()
        } else {
            format!("  · {description}")
        };
        rows.push(format!("{marker} {}. {label}{description}", index + 1));
    }
    if prompt.allow_freeform {
        let index = prompt.options.len();
        let marker = if selected == Some(index) { "›" } else { " " };
        rows.push(format!("{marker} {}", localized.custom_answer));
    }
    rows.push(localized.selection_hint.to_string());
    rows.push(match crate::i18n::current_language() {
        crate::i18n::Language::ZhHans => "Esc: 保留问题稍后回答 · d / /decline: 明确拒绝回答",
        _ => "Esc: keep open for later · d / /decline: decline to answer",
    }.to_string());

    let inner = adaptive_panel_inner_width(&rows, terminal_columns());
    let mut out = String::new();
    out.push('\n');
    push_permission_border(&mut out, "╭", "╮", inner, accent, color);
    for (row_index, row) in rows.iter().enumerate() {
        let selected_row = selected
            .map(|selected| row_index == option_start + selected)
            .unwrap_or(false);
        let row_color = if row_index == 0 {
            accent
        } else if selected_row {
            "\x1b[38;5;255m"
        } else {
            "\x1b[38;5;250m"
        };
        for rendered in wrap_display_width(row, inner) {
            let padded = pad_right_display_width(&rendered, inner);
            push_permission_line(&mut out, &padded, row_color, color, selected_row);
        }
    }
    push_permission_border(&mut out, "╰", "╯", inner, accent, color);
    out
}

pub fn render_permission_prompt(prompt: &PermissionPrompt) -> String {
    render_permission_prompt_with_selection(prompt, None)
}

fn render_permission_prompt_with_selection(
    prompt: &PermissionPrompt,
    selected: Option<usize>,
) -> String {
    let color = permission_color_enabled();
    let (accent, icon) = permission_kind_style(prompt.kind);
    let localized = permission_text();
    let option_once = format!("1. {}", localized.allow_once_option);
    let option_session = format!("2. {}", if prompt.allow_conversation {
        prompt.session_label.as_str()
    } else { localized.session_unavailable_option });
    let option_custom = format!("3. {}", prompt.custom_label);
    let option_deny = format!("N. {}", localized.deny_option);
    let choose = if prompt.allow_conversation { localized.keyboard_hint } else { localized.once_only_keyboard_hint };
    let mut plain_rows = Vec::new();
    plain_rows.push(format!("{icon} {}", prompt.title));
    for (label, value) in &prompt.rows {
        for (index,line) in value.split('\n').enumerate() {
            plain_rows.push(format!("{:<9} {}",if index==0 {label.as_str()} else {""},line.trim_end_matches('\r')));
        }
    }
    plain_rows.push(String::new());
    let option_start = plain_rows.len();
    for (index, option) in [option_once, option_session, option_custom, option_deny]
        .into_iter()
        .enumerate()
    {
        let marker = if selected == Some(index) && (index != 1 || prompt.allow_conversation) { "›" } else { " " };
        plain_rows.push(format!("{marker} {option}"));
    }
    plain_rows.push(choose.to_string());
    if prompt.supports_instruction {
        plain_rows.push(
            "I. Add instruction; invocation stays paused until explicitly approved".to_string(),
        );
        if selected.is_none() {
            plain_rows.push(
                "Type i <instruction> to save guidance without granting permission".to_string(),
            );
        }
    }

    let inner = adaptive_panel_inner_width(&plain_rows, terminal_columns());
    let mut out = String::new();
    out.push('\n');
    push_permission_border(&mut out, "╭", "╮", inner, accent, color);
    for (index, row) in plain_rows.iter().enumerate() {
        let selected_row = selected
            .filter(|selected| *selected != 1 || prompt.allow_conversation)
            .map(|selected| index == option_start + selected)
            .unwrap_or(false);
        let option_index = index.checked_sub(option_start).filter(|item| *item < 4);
        let row_color = if index == 0 {
            accent
        } else if option_index == Some(1) && !prompt.allow_conversation {
            "\x1b[38;5;244m"
        } else if let Some(option_index) = option_index {
            permission_option_color(option_index)
        } else {
            "\x1b[38;5;250m"
        };
        for rendered in wrap_display_width(row, inner) {
            let padded = pad_right_display_width(&rendered, inner);
            push_permission_line(&mut out, &padded, row_color, color, selected_row);
        }
    }
    push_permission_border(&mut out, "╰", "╯", inner, accent, color);
    out
}

struct PermissionRawModeGuard {
    initially_enabled: bool,
    enabled: bool,
}

impl PermissionRawModeGuard {
    fn new() -> io::Result<Self> {
        let initially_enabled = is_raw_mode_enabled()?;
        enable_raw_mode()?;
        restore_terminal_newline_output();
        Ok(Self { initially_enabled, enabled: true })
    }

    fn restore_with(&mut self, set_mode: impl FnOnce(bool) -> io::Result<()>) {
        // A modal borrows terminal mode from its caller. Restore that mode for
        // every exit path, not just the special "return to composer" choice.
        if self.enabled != self.initially_enabled && set_mode(self.initially_enabled).is_ok() {
            self.enabled = self.initially_enabled;
        }
    }
}

impl Drop for PermissionRawModeGuard {
    fn drop(&mut self) {
        self.restore_with(|enabled| {
            if enabled {
                enable_raw_mode()?;
                restore_terminal_newline_output();
                Ok(())
            } else {
                disable_raw_mode()
            }
        });
    }
}

#[cfg(test)]
mod modal_terminal_mode_tests {
    use super::*;

    #[test]
    fn modal_answer_frame_wraps_cjk_and_tracks_cursor_after_resize() {
        let mut editor = crate::terminal_input::ModalEditor::default();
        editor.apply(Event::Paste("第一行\n第二行很长，需要换行".into()));
        let frame = render_modal_answer(&editor, 16);
        assert!(frame.contains("第一行"));
        assert!(frame.contains("›"));
        assert!(frame.contains("▏"));
        assert!(!frame.contains('\x1b'));
        let contents = frame.lines().skip(1).take(frame.lines().count() - 2)
            .map(|line| line.trim_start_matches([' ', '›']).replace('▏', ""))
            .collect::<String>();
        assert_eq!(contents, "第一行第二行很长，需要换行");
        assert!(render_modal_answer(&editor, 80).lines().count() < frame.lines().count());
    }

    #[test]
    fn modal_restores_its_callers_mode_on_every_exit_path() {
        for (initially_enabled, enabled, expected) in [
            (true, true, None),
            (false, true, Some(false)),
            (true, false, Some(true)),
            (false, false, None),
        ] {
            let mut guard = PermissionRawModeGuard { initially_enabled, enabled };
            let mut restored = None;
            guard.restore_with(|mode| { restored = Some(mode); Ok(()) });
            assert_eq!(restored, expected);
            assert_eq!(guard.enabled, initially_enabled);
            // Restoring again (including Drop) must not touch the terminal.
            guard.restore_with(|_| panic!("mode restored twice"));
        }
    }
}

fn restore_terminal_newline_output() {
    restore_terminal_newline_output_impl();
}

#[cfg(unix)]
fn restore_terminal_newline_output_impl() {
    let _ = std::process::Command::new("stty")
        .args(["opost", "onlcr"])
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(unix))]
fn restore_terminal_newline_output_impl() {}

fn permission_kind_style(kind: PermissionPromptKind) -> (&'static str, &'static str) {
    match kind {
        PermissionPromptKind::Command => ("\x1b[38;5;179m", "◆"),
        PermissionPromptKind::Tool => ("\x1b[38;5;117m", "◆"),
        PermissionPromptKind::Network => ("\x1b[38;5;117m", "◆"),
        PermissionPromptKind::Agent => ("\x1b[38;5;117m", "◆"),
    }
}

fn permission_option_color(index: usize) -> &'static str {
    match index {
        0 => "\x1b[38;5;108m",
        1 => "\x1b[38;5;110m",
        2 => "\x1b[38;5;179m",
        _ => "\x1b[38;5;203m",
    }
}

fn permission_option_label(raw: &str, fallback: &str) -> String {
    let mut label = raw
        .replace("[y/N]:", "")
        .replace("[y/N]", "")
        .replace("[Y/n]:", "")
        .replace("[Y/n]", "")
        .trim()
        .to_string();
    while permission_label_has_trailing_prompt_char(&label) {
        label.pop();
    }
    if label.is_empty() {
        fallback.to_string()
    } else {
        label
    }
}

fn permission_label_has_trailing_prompt_char(value: &str) -> bool {
    matches!(value.chars().last(), Some('?' | '？' | ':' | '：' | ' '))
}

fn push_permission_border(
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

fn push_permission_line(
    out: &mut String,
    text: &str,
    row_color: &str,
    color: bool,
    selected: bool,
) {
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

fn permission_color_enabled() -> bool {
    (std::io::stdout().is_terminal() || std::io::stderr().is_terminal())
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM")
            .map(|value| value != "dumb")
            .unwrap_or(true)
}

fn terminal_columns() -> usize {
    crossterm::terminal::size()
        .ok()
        .map(|(columns, _)| usize::from(columns))
        .filter(|value| *value > 0)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
        })
        .unwrap_or(100)
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn unicode_char_width(ch: char) -> usize {
    if ch == '\t' {
        return 4;
    }
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

fn truncate_display_width(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = unicode_char_width(ch);
        if width.saturating_add(ch_width) > max_width.saturating_sub(1) {
            break;
        }
        width += ch_width;
        out.push(ch);
    }
    out.push('…');
    out
}

fn wrap_display_width(text: &str, max_width: usize) -> Vec<String> {
    if text.is_empty() || max_width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = unicode_char_width(ch);
        if !line.is_empty() && width.saturating_add(ch_width) > max_width {
            if let Some((break_index, break_char)) = line
                .char_indices()
                .rev()
                .find(|(_, character)| character.is_whitespace())
            {
                let split_index = break_index + break_char.len_utf8();
                let remainder = line.split_off(split_index);
                lines.push(line);
                line = remainder;
                width = display_width(&line);
            } else {
                lines.push(line);
                line = String::new();
                width = 0;
            }
        }
        line.push(ch);
        width = width.saturating_add(ch_width);
    }
    lines.push(line);
    lines
}

fn pad_right_display_width(text: &str, width: usize) -> String {
    let current = display_width(text);
    if current >= width {
        return text.to_string();
    }
    format!("{}{}", text, " ".repeat(width - current))
}

fn interaction_mode() -> InteractionMode {
    match std::env::var("NOLA_CLI_INTERACTION_MODE")
        .ok()
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("always" | "force" | "on" | "interactive") => InteractionMode::Always,
        Some("never" | "off" | "non-interactive" | "noninteractive") => InteractionMode::Never,
        _ => InteractionMode::Auto,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn non_lifo_adapter_drop_does_not_restore_expired_composer() {
        let first=super::ExecutionCancellation::default();
        let second=super::ExecutionCancellation::default();
        let outer=first.enter();
        let inner=second.enter();
        drop(outer);
        assert_eq!(super::current_execution_cancellation().unwrap().execution_id(),second.execution_id());
        drop(inner);
        assert!(super::current_execution_cancellation().is_none());
        assert!(crate::runtime_control::current_execution_control().is_none());
        assert!(crate::execution_context::current_execution_id().is_none());
    }
    use crate::runtime_input::RuntimeInput;
    use super::{
        adaptive_panel_inner_width, append_execution_input, confirmation_answer_is_yes,
        display_width, render_user_question_prompt, update_execution_input_from_key,
        wrap_display_width, ExecutionCancellation, UserQuestionPrompt,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn runtime_input_does_not_mutate_composer_or_interpret_slash_text() {
        let cancellation = ExecutionCancellation::default();
        cancellation.set_draft_input("unsent draft".into());
        cancellation.queue_runtime_input(RuntimeInput::UserFollowUp("/literal follow-up".into())).unwrap();
        assert_eq!(cancellation.draft_input().as_deref(), Some("unsent draft"));
        assert_eq!(cancellation.take_boundary_user_inputs(), vec!["/literal follow-up"]);
    }

    #[test]
    fn full_queue_retains_draft_until_submission_is_accepted() {
        let cancellation = ExecutionCancellation::default();
        for index in 0..32 { assert!(cancellation.queue_input(format!("queued {index}"))); }
        cancellation.set_draft_input("  keep\n\tall of this  ".to_owned());
        cancellation.queue_draft_input();
        assert_eq!(cancellation.input_snapshot().draft, "  keep\n\tall of this  ");
        assert_eq!(cancellation.take_queued_inputs().len(), 32);
        cancellation.queue_draft_input();
        assert!(cancellation.input_snapshot().draft.is_empty());
        assert_eq!(cancellation.take_queued_inputs(), vec!["  keep\n\tall of this  "]);
    }

    #[test]
    fn live_paste_preserves_multiline_content_beyond_old_limit() {
        let cancellation = ExecutionCancellation::default();
        let text = "\t代码\n".repeat(3000);
        append_execution_input(&cancellation, &text);
        assert_eq!(cancellation.input_snapshot().draft, text);
    }

    #[cfg(unix)]
    use super::process_execution_terminal_bytes;

    #[test]
    fn execution_input_is_queued_only_after_enter() {
        let cancellation = ExecutionCancellation::default();
        append_execution_input(&cancellation, "补充要求");
        let draft = cancellation.input_snapshot();
        assert_eq!(draft.draft, "补充要求");
        assert_eq!(draft.queued_count, 0);

        update_execution_input_from_key(
            &cancellation,
            &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        let submitted = cancellation.input_snapshot();
        assert!(submitted.draft.is_empty());
        assert_eq!(submitted.queued_count, 1);
        assert_eq!(cancellation.take_queued_inputs(), vec!["补充要求"]);
    }

    #[test]
    fn execution_input_control_u_clears_only_the_draft() {
        let cancellation = ExecutionCancellation::default();
        append_execution_input(&cancellation, "discard me");
        update_execution_input_from_key(
            &cancellation,
            &KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert!(cancellation.input_snapshot().draft.is_empty());
    }

    #[test]
    fn execution_composer_preserves_cursor_and_unicode_editing() {
        let cancellation = ExecutionCancellation::default();
        append_execution_input(&cancellation, "你好ab");
        update_execution_input_from_key(
            &cancellation,
            &KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        );
        update_execution_input_from_key(
            &cancellation,
            &KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );
        update_execution_input_from_key(
            &cancellation,
            &KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE),
        );

        let snapshot = cancellation.input_snapshot();
        assert_eq!(snapshot.draft, "你好中b");
        assert_eq!(snapshot.cursor, "你好中".len());
    }

    #[test]
    fn session_switch_isolates_drafts_queue_and_late_callbacks() {
        super::select_interactive_composer(Some("db-one/session-a".into()));
        let first = ExecutionCancellation::interactive();
        first.set_draft_input("甲会话草稿".into());
        first.queue_runtime_input(RuntimeInput::UserFollowUp("甲会话待发送".into())).unwrap();
        super::select_interactive_composer(Some("db-one/session-b".into()));
        let second = ExecutionCancellation::interactive();
        assert!(second.input_snapshot().draft.is_empty());
        assert!(super::take_interactive_queued_inputs().is_empty());
        first.runtime.handoff_user_instruction("甲会话晚到审批回复".into()).unwrap();
        assert!(second.input_snapshot().draft.is_empty());
        super::select_interactive_composer(Some("db-two/session-a".into()));
        assert!(super::interactive_composer_snapshot().draft.is_empty());
        super::select_interactive_composer(Some("db-one/session-a".into()));
        assert_eq!(super::interactive_composer_snapshot().draft, "甲会话晚到审批回复");
        assert_eq!(super::take_interactive_queued_inputs(), vec!["甲会话待发送"]);
        assert!(super::take_interactive_queued_inputs().is_empty());
        super::select_interactive_composer(None);
        let unnamed = ExecutionCancellation::interactive();
        unnamed.set_draft_input("首轮执行草稿".into());
        super::adopt_interactive_composer("db-one/new".into());
        super::select_interactive_composer(Some("db-one/new".into()));
        assert_eq!(super::interactive_composer_snapshot().draft, "首轮执行草稿");
        super::select_interactive_composer(None);
        assert!(super::interactive_composer_snapshot().draft.is_empty());
    }

    #[test]
    fn interactive_execution_phases_share_one_composer() {
        let first = ExecutionCancellation::interactive();
        let second = ExecutionCancellation::interactive_deferred_input();
        first.set_draft_input("跨阶段草稿".to_string());

        assert_eq!(second.input_snapshot().draft, "跨阶段草稿");
        super::update_interactive_composer("跨阶段草稿", 3);
        assert_eq!(first.input_snapshot().cursor, 3);
        assert_eq!(super::interactive_composer_snapshot().cursor, 3);
        super::clear_interactive_execution_draft();
        assert!(first.input_snapshot().draft.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn execution_terminal_bytes_preserve_utf8_queue_and_cancel() {
        let cancellation = ExecutionCancellation::default();
        let mut pending_utf8 = Vec::new();

        assert!(!process_execution_terminal_bytes(
            &cancellation,
            "补充要求\r".as_bytes(),
            &mut pending_utf8,
        ));
        assert!(pending_utf8.is_empty());
        assert_eq!(cancellation.take_queued_inputs(), vec!["补充要求"]);

        assert!(process_execution_terminal_bytes(
            &cancellation,
            &[0x1b],
            &mut pending_utf8,
        ));
        assert!(cancellation.is_requested());
    }

    #[test]
    fn cancellation_tokens_are_isolated_between_runs() {
        let first = ExecutionCancellation::default();
        let first_clone = first.clone();
        let second = ExecutionCancellation::default();

        first_clone.request();

        assert_eq!(first.execution_id(), first_clone.execution_id());
        assert_ne!(first.execution_id(), second.execution_id());
        assert!(first.is_requested());
        assert!(!second.is_requested());
    }

    #[test]
    fn transport_context_shares_cancellation_without_steering_state() {
        let execution = ExecutionCancellation::default();
        let transport = execution.context();
        let child = execution.child();
        assert_eq!(execution.execution_id(), transport.execution_id());
        transport.request();
        assert!(execution.is_requested());
        assert!(child.context().is_requested());
    }

    #[test]
    fn interaction_scope_restores_both_identity_and_terminal_context() {
        let execution = ExecutionCancellation::default();
        let child = execution.child();
        let original_id = crate::execution_context::current_execution_id();
        {
            let _outer = execution.enter();
            {
                let _inner = child.enter();
                assert_eq!(
                    crate::execution_context::current_execution_id(),
                    Some(child.execution_id())
                );
                assert_eq!(
                    super::current_execution_cancellation().unwrap().execution_id(),
                    child.execution_id()
                );
            }
            assert_eq!(
                crate::execution_context::current_execution_id(),
                Some(execution.execution_id())
            );
            assert_eq!(
                super::current_execution_cancellation().unwrap().execution_id(),
                execution.execution_id()
            );
        }
        assert_eq!(crate::execution_context::current_execution_id(), original_id);
        assert!(super::current_execution_cancellation().is_none());
    }

    #[test]
    fn background_request_is_independent_from_cancellation() {
        let execution = ExecutionCancellation::interactive();
        execution.request_background();

        assert!(execution.is_background_requested());
        assert!(!execution.is_requested());
    }

    #[test]
    fn call_local_child_observes_and_consumes_the_turn_background_request() {
        let execution = ExecutionCancellation::interactive();
        let child = execution.child();

        execution.request_background();

        assert!(child.is_background_requested());
        assert!(child.take_background_request());
        assert!(!execution.is_background_requested());
        assert!(!child.take_background_request());
    }

    #[test]
    fn submitted_runtime_input_is_durable_before_the_next_core_boundary() {
        let _environment = crate::store::test_env_lock();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nola-durable-steering-{}-{stamp}",
            std::process::id()
        ));
        let db_path = root.join("nola.db");
        unsafe {
            std::env::set_var("NOLA_DB_PATH", &db_path);
        }
        let cancellation = ExecutionCancellation::default();
        cancellation.bind_session_input("session-steering-test", crate::runtime_control::InputRetention::Persistent).unwrap();
        cancellation.set_draft_input("继续分析失败原因".to_string());
        cancellation.queue_draft_input();

        assert!(cancellation.take_boundary_user_inputs().is_empty());
        let command = crate::agent_command_port::claim_next_agent_command(
            "session-steering-test",
            "test-consumer",
            1,
            i64::MAX,
        )
        .unwrap()
        .expect("submitted input must be present in the durable FIFO");
        let nola_core::agent::host::AgentBoundaryInput::Submission {submission} =
            crate::agent_command_port::decode_claimed(&command).unwrap() else {panic!("expected user submission");};
        let nola_core::agent::AgentSubmission::UserInput {
            items, delivery, ..
        } = submission
        else {
            panic!("durable steering must remain a user input submission");
        };
        assert_eq!(delivery, nola_core::agent::AgentInputDelivery::NextTurn);
        assert!(items.iter().any(|item| matches!(
            item,
            nola_core::agent::AgentItem::Message { content, .. }
                if content == "继续分析失败原因"
        )));

        unsafe {
            std::env::remove_var("NOLA_DB_PATH");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn composer_commands_dispatch_before_turn_completion_without_followup_admission() {
        #[derive(Debug, Default)]
        struct Handler(std::sync::Mutex<Vec<String>>);
        impl super::ClientCommandHandler for Handler {
            fn execute(&self, command: &str) { self.0.lock().unwrap().push(command.into()); }
        }
        let handler = std::sync::Arc::new(Handler::default());
        let cancellation = ExecutionCancellation::default().with_client_commands(handler.clone());
        for command in ["/allow", " /a", "/help", "/unknown"] {
            cancellation.set_draft_input(command.into());
            super::submit_execution_draft(&cancellation);
            assert!(cancellation.take_queued_inputs().is_empty());
            assert!(cancellation.take_boundary_user_inputs().is_empty());
            cancellation.dispatch_client_command();
            assert_eq!(handler.0.lock().unwrap().last().unwrap(), command);
            assert!(!cancellation.is_requested());
        }
        cancellation.dispatch_client_command();
        assert_eq!(handler.0.lock().unwrap().len(), 4);
    }

    #[test]
    fn session_navigation_during_execution_interrupts_without_becoming_user_input() {
        for command in ["/resume", "/r", "/continue"] {
            let cancellation = ExecutionCancellation::default();
            cancellation.commit_runtime_input(command.into()).unwrap();
            cancellation.dispatch_client_command();
            assert!(cancellation.is_requested(), "{command}");
            assert!(cancellation.take_boundary_user_inputs().is_empty());
            assert_eq!(cancellation.take_queued_inputs(), vec![command]);
            cancellation.dispatch_client_command();
            assert!(cancellation.take_queued_inputs().is_empty());
        }
    }

    #[test]
    fn submitted_runtime_slash_command_stays_in_the_cli_control_queue() {
        let cancellation = ExecutionCancellation::default();
        cancellation.bind_session_input("session-control-command-test", crate::runtime_control::InputRetention::Persistent).unwrap();

        cancellation.queue_runtime_input(RuntimeInput::ClientControl("/exit".into())).unwrap();

        assert!(cancellation.take_boundary_user_inputs().is_empty());
        assert_eq!(cancellation.take_queued_inputs(), vec!["/exit"]);
    }

    #[test]
    fn exit_during_execution_interrupts_and_hands_off_only_local_exit() {
        let cancellation = ExecutionCancellation::default();
        cancellation.commit_runtime_input("/exit".into()).unwrap();
        cancellation.dispatch_client_command();
        assert!(cancellation.is_requested());
        assert!(cancellation.take_boundary_user_inputs().is_empty());
        assert_eq!(cancellation.take_queued_inputs(), vec!["/exit"]);
        cancellation.dispatch_client_command();
        assert!(cancellation.take_queued_inputs().is_empty());
    }

    #[test]
    fn boundary_input_atomically_separates_agent_follow_up_from_cli_control() {
        let cancellation = ExecutionCancellation::default();
        cancellation.queue_runtime_input(RuntimeInput::UserFollowUp("继续分析失败原因".into())).unwrap();
        cancellation.queue_runtime_input(RuntimeInput::ClientControl("/exit".into())).unwrap();

        assert_eq!(
            cancellation.take_boundary_user_inputs(),
            vec!["继续分析失败原因"]
        );
        assert_eq!(cancellation.take_queued_inputs(), vec!["/exit"]);
    }

    #[test]
    fn confirmation_answer_accepts_supported_yes_values() {
        for value in ["y", "Y", "yes", "YES", " 是 ", "好", "允许"] {
            assert!(confirmation_answer_is_yes(value), "{value}");
        }
    }

    #[test]
    fn confirmation_answer_rejects_default_or_negative_values() {
        for value in ["", "n", "no", "否", "random"] {
            assert!(!confirmation_answer_is_yes(value), "{value}");
        }
    }

    #[test]
    fn permission_rows_wrap_without_losing_long_command_text() {
        let command = "command 运行一个很长的命令 --with-option=value --and-another=value";
        let rows = wrap_display_width(command, 16);
        assert!(rows.len() > 1);
        assert!(rows.iter().all(|row| display_width(row) <= 16));
        assert_eq!(rows.concat(), command);
    }

    #[test]
    fn question_panel_adapts_to_screen_and_keeps_complete_descriptions() {
        let measurement = vec!["x".repeat(240)];
        assert_eq!(adaptive_panel_inner_width(&measurement, 48), 44);
        assert_eq!(adaptive_panel_inner_width(&measurement, 100), 80);
        assert_eq!(adaptive_panel_inner_width(&measurement, 180), 100);

        let prompt = UserQuestionPrompt {
            header: "Choose a direction".to_string(),
            question: "Which approach should be used for this change?".to_string(),
            options: vec![(
                "Complete repair".to_string(),
                "Preserve every detail instead of truncating important content".to_string(),
            )],
            allow_freeform: true,
        };
        let rendered = render_user_question_prompt(&prompt, Some(0));
        for word in [
            "Preserve",
            "every",
            "detail",
            "truncating",
            "important",
            "content",
        ] {
            assert!(rendered.contains(word), "missing {word}: {rendered}");
        }
    }
}
