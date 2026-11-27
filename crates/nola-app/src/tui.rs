// SPDX-License-Identifier: Apache-2.0
//! Full interactive crossterm terminal UI for Nola v2 with:
//! - Fullscreen alternate-screen scrollable viewport (with automatic non-TTY line fallback)
//! - Non-blocking background `AgentEngine` worker + live mid-run `/model`, `/thinking`,
//!   `/guide` inbox steering, and instant `Esc`/`Ctrl+C` interruptible cancellation
//! - Clean ANSI Markdown rendering, toggleable dimmed reasoning (`/verbose`),
//!   interactive Approval (with durable session grants) & `ask_user` modals
//! - File/image attachments (`/attach`, `@<path>`, `/paste`), Tab completion,
//!   command history, EN/ZH-Hans i18n, and full 35+ slash command parity.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEventKind,
};
use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::QueueableCommand;
use nola_engine::{
    approve_agent_plan, compact_session_now, is_plan_mode, load_session_plan, load_session_todos,
    persist_permission_grant, register_planning_tools, register_subagent_tools, set_plan_mode,
    AgentEngine, AgentEvent, AgentRunConfig, AgentRunResult, DurablePermissionGrant,
    EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller, ModelCaller, PlanningState,
    SubagentManager, TaskStatus,
};
use nola_llm::{CallParams, LlmClient, LlmError, PrivacyRedactor};
use nola_mcp::{run_interactive_oauth_flow_with_options, McpManager, DEFAULT_OAUTH_CALLBACK_TIMEOUT};
use nola_modules::local_models_plugins::find_binary_on_path;
use nola_modules::{
    built_in_role_templates, index_data_source, save_memory_v2, search_memory_v2,
    ExtendedGoalRecord, GoalWorker, MemoryKind, MemoryMetadata, MemoryOwner, MemoryProvenance,
    MemoryScope, MemorySearchFilter, MemorySensitivity, MemoryWorker, ModulesSuite, RoomRuntime,
};
use nola_store::{Store, Vault};
use nola_tools::{extract_document_text_from_bytes, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    new_id, now_ms, sha256_hex, AgentRoom, DataSourceRecord, ExpertPeer, FrozenSystemBlock,
    GoalStatus, HostCapabilities, MediaAttachment, MemoryItem, ModelProtocol, ModelRouteConfig,
    ModelStreamDelta, NormalizedModelResponse, PermissionProfile, StructuredModelRequest,
    TokenUsage,
};
use serde_json::{json, Value};

use crate::config::{
    build_frozen_system_blocks, connection_has_api_key, default_vault_path, load_active_config,
    resolve_model_route, AppliedConfig, DefaultModelSelection,
};

const MAX_ATTACHMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_DOC_ATTACHMENT_CHARS: usize = 32_000;
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/exit",
    "/quit",
    "/new",
    "/resume",
    "/sessions",
    "/compact",
    "/model",
    "/thinking",
    "/allow",
    "/config",
    "/verbose",
    "/edit",
    "/skills",
    "/tools",
    "/room",
    "/rooms",
    "/plan",
    "/checkpoint",
    "/checkpoints",
    "/undo",
    "/redo",
    "/diff",
    "/worktree",
    "/tasks",
    "/jobs",
    "/stop",
    "/goal",
    "/goals",
    "/sources",
    "/mcp",
    "/memory",
    "/agent",
    "/guide",
    "/questions",
    "/clear",
    "/history",
    "/status",
    "/continue",
    "/export",
    "/attach",
    "/paste",
];

#[derive(Debug, Clone)]
pub struct TuiLaunchOptions {
    pub db_path: PathBuf,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub permission_profile: Option<PermissionProfile>,
    pub yes: bool,
    pub resume: bool,
    pub session_id: Option<String>,
}

// ============================================================================
// i18n (EN / ZH-Hans)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiLocale {
    En,
    ZhHans,
}

impl TuiLocale {
    fn from_env() -> Self {
        for var in ["NOLA_LANG", "LC_ALL", "LANG"] {
            if let Ok(val) = std::env::var(var) {
                if !val.trim().is_empty() {
                    return Self::from_lang_str(&val);
                }
            }
        }
        Self::En
    }

    fn from_lang_str(raw: &str) -> Self {
        let lower = raw.trim().to_ascii_lowercase();
        if lower.starts_with("zh") {
            Self::ZhHans
        } else {
            Self::En
        }
    }

    fn t<'a>(self, en: &'a str, zh: &'a str) -> &'a str {
        match self {
            Self::En => en,
            Self::ZhHans => zh,
        }
    }
}

// ============================================================================
// Live Model Caller (supports live mid-run /model & /thinking + cancel)
// ============================================================================

struct LiveModelCaller {
    inner: HttpModelCaller,
    live_route: Arc<Mutex<ModelRouteConfig>>,
    last_step_usage: Arc<Mutex<Option<TokenUsage>>>,
}

impl LiveModelCaller {
    fn effective_route(&self, fallback: &ModelRouteConfig) -> ModelRouteConfig {
        self.live_route
            .lock()
            .map(|r| r.clone())
            .unwrap_or_else(|_| fallback.clone())
    }

    fn record_usage(&self, usage: &TokenUsage) {
        if usage.input_tokens > 0 || usage.cache_read_tokens > 0 {
            if let Ok(mut guard) = self.last_step_usage.lock() {
                *guard = Some(usage.clone());
            }
        }
    }
}

impl ModelCaller for LiveModelCaller {
    fn call(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        let effective = self.effective_route(route);
        let resp = self.inner.call(&effective, req, stream, on_delta)?;
        self.record_usage(&resp.usage);
        Ok(resp)
    }

    fn call_cancellable(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        call_params: &CallParams,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        let effective = self.effective_route(route);
        let resp = self
            .inner
            .call_cancellable(&effective, req, call_params, stream, on_delta)?;
        self.record_usage(&resp.usage);
        Ok(resp)
    }
}

// ============================================================================
// Terminal Guards
// ============================================================================

struct InteractiveTerminalGuard {
    active: bool,
}

impl InteractiveTerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        out.queue(EnterAlternateScreen)?;
        out.queue(EnableMouseCapture)?;
        out.queue(Hide)?;
        out.flush()?;
        Ok(Self { active: true })
    }

    fn suspend(&mut self) {
        if self.active {
            let mut out = io::stdout();
            let _ = out.queue(Show);
            let _ = out.queue(DisableMouseCapture);
            let _ = out.queue(LeaveAlternateScreen);
            let _ = out.flush();
            let _ = disable_raw_mode();
            self.active = false;
        }
    }

    fn resume(&mut self) -> io::Result<()> {
        if !self.active {
            enable_raw_mode()?;
            let mut out = io::stdout();
            out.queue(EnterAlternateScreen)?;
            out.queue(EnableMouseCapture)?;
            out.queue(Hide)?;
            out.flush()?;
            self.active = true;
        }
        Ok(())
    }
}

impl Drop for InteractiveTerminalGuard {
    fn drop(&mut self) {
        self.suspend();
    }
}

struct RawModeGuard {
    active: bool,
}

impl RawModeGuard {
    fn enter() -> Option<Self> {
        if io::stdin().is_terminal() && enable_raw_mode().is_ok() {
            Some(Self { active: true })
        } else {
            None
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_raw_mode();
            self.active = false;
        }
    }
}

// ============================================================================
// Slash Command Parsing & Tab Completion
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedInput {
    Empty,
    LiteralPrompt(String),
    Slash { cmd: String, arg: String },
}

fn parse_user_input(raw: &str) -> ParsedInput {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ParsedInput::Empty;
    }
    if let Some(escaped) = trimmed.strip_prefix("//") {
        return ParsedInput::LiteralPrompt(format!("/{escaped}"));
    }
    if trimmed.starts_with('/') {
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("").to_ascii_lowercase();
        let arg = parts.next().unwrap_or("").trim().to_string();
        return ParsedInput::Slash { cmd, arg };
    }
    ParsedInput::LiteralPrompt(trimmed.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletionResult {
    replacement: Option<(usize, usize, String)>,
    candidates: Vec<String>,
}

fn longest_common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut prefix = first.clone();
    for item in &items[1..] {
        let mut matched_bytes = 0usize;
        for (a, b) in prefix.chars().zip(item.chars()) {
            if a == b {
                matched_bytes += a.len_utf8();
            } else {
                break;
            }
        }
        prefix.truncate(matched_bytes);
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

fn complete_composer_input(
    buffer: &str,
    cursor: usize,
    expert_names: &[String],
) -> CompletionResult {
    let safe_cursor = cursor.min(buffer.len());
    let before = &buffer[..safe_cursor];

    // Find start of current whitespace-delimited token
    let token_start = before
        .rfind(char::is_whitespace)
        .map(|idx| idx + before[idx..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(0);
    let token = &before[token_start..];

    // 1. `@expert` mention completion anywhere on the line
    if let Some(prefix_name) = token.strip_prefix('@') {
        let lower = prefix_name.to_ascii_lowercase();
        let mut matches: Vec<String> = expert_names
            .iter()
            .filter(|name| name.to_ascii_lowercase().starts_with(&lower))
            .map(|name| format!("@{name}"))
            .collect();
        matches.sort();
        matches.dedup();
        return build_completion(token_start, safe_cursor, token, matches);
    }

    // 2. Leading slash command completion
    if token_start == 0 && token.starts_with('/') {
        let lower = token.to_ascii_lowercase();
        let matches: Vec<String> = SLASH_COMMANDS
            .iter()
            .filter(|c| c.starts_with(&lower))
            .map(|c| (*c).to_string())
            .collect();
        return build_completion(token_start, safe_cursor, token, matches);
    }

    // 3. Subcommand completions for common slash commands
    let trimmed_before = before.trim_start();
    if trimmed_before.starts_with('/') {
        let cmd = trimmed_before
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let sub_options: Vec<String> = match cmd.as_str() {
            "/plan" => ["on", "off", "show", "approve"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "/allow" => ["cautious", "accept_edits", "allow_all"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "/thinking" => ["none", "low", "medium", "high", "automatic"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "/goal" | "/goals" => ["list", "create", "run", "pause", "cancel", "status"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "/worktree" => ["list", "create", "remove", "integrate"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "/sources" => ["list", "add", "index", "search"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "/mcp" => ["list", "tools", "auth"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "/memory" => ["list", "search", "save"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "/attach" => ["list", "clear"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "/agent" => {
                let mut opts = vec!["clear".to_string()];
                opts.extend(expert_names.iter().cloned());
                opts
            }
            _ => Vec::new(),
        };
        if !sub_options.is_empty() {
            let lower = token.to_ascii_lowercase();
            let mut matches: Vec<String> = sub_options
                .into_iter()
                .filter(|opt| opt.to_ascii_lowercase().starts_with(&lower))
                .collect();
            matches.sort();
            matches.dedup();
            return build_completion(token_start, safe_cursor, token, matches);
        }
    }

    CompletionResult {
        replacement: None,
        candidates: Vec::new(),
    }
}

fn build_completion(
    token_start: usize,
    cursor: usize,
    current_token: &str,
    matches: Vec<String>,
) -> CompletionResult {
    if matches.is_empty() {
        return CompletionResult {
            replacement: None,
            candidates: Vec::new(),
        };
    }
    if matches.len() == 1 {
        let completed = format!("{} ", matches[0]);
        return CompletionResult {
            replacement: Some((token_start, cursor, completed)),
            candidates: matches,
        };
    }
    let lcp = longest_common_prefix(&matches);
    let replacement = if lcp.len() > current_token.len() {
        Some((token_start, cursor, lcp))
    } else {
        None
    };
    CompletionResult {
        replacement,
        candidates: matches,
    }
}

// ============================================================================
// Composer State (cursor movement, UTF-8 editing, history, Tab completion)
// ============================================================================

#[derive(Debug, Default)]
struct ComposerState {
    buffer: String,
    cursor: usize,
    history: Vec<String>,
    history_idx: Option<usize>,
    draft: String,
}

impl ComposerState {
    fn insert_char(&mut self, c: char) {
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.history_idx = None;
    }

    fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.buffer.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.history_idx = None;
    }

    fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if let Some((idx, _)) = self.buffer[..self.cursor].char_indices().next_back() {
            self.cursor = idx;
        }
    }

    fn move_right(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        if let Some(ch) = self.buffer[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
        }
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.buffer.len();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if let Some((idx, _)) = self.buffer[..self.cursor].char_indices().next_back() {
            self.buffer.drain(idx..self.cursor);
            self.cursor = idx;
            self.history_idx = None;
        }
    }

    fn delete(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        if let Some(ch) = self.buffer[self.cursor..].chars().next() {
            self.buffer.drain(self.cursor..self.cursor + ch.len_utf8());
            self.history_idx = None;
        }
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.history_idx = None;
    }

    fn submit(&mut self) -> String {
        let text = self.buffer.clone();
        let trimmed = text.trim();
        if !trimmed.is_empty()
            && self
                .history
                .last()
                .map(|last| last != trimmed)
                .unwrap_or(true)
        {
            self.history.push(trimmed.to_string());
            if self.history.len() > 200 {
                self.history.remove(0);
            }
        }
        self.buffer.clear();
        self.cursor = 0;
        self.history_idx = None;
        self.draft.clear();
        text
    }

    fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next_idx = match self.history_idx {
            None => {
                self.draft = self.buffer.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.history_idx = Some(next_idx);
        self.buffer = self.history[next_idx].clone();
        self.cursor = self.buffer.len();
    }

    fn history_down(&mut self) {
        let Some(idx) = self.history_idx else {
            return;
        };
        if idx + 1 >= self.history.len() {
            self.history_idx = None;
            self.buffer = self.draft.clone();
            self.cursor = self.buffer.len();
        } else {
            let next_idx = idx + 1;
            self.history_idx = Some(next_idx);
            self.buffer = self.history[next_idx].clone();
            self.cursor = self.buffer.len();
        }
    }

    fn apply_tab_completion(&mut self, expert_names: &[String]) -> Vec<String> {
        let res = complete_composer_input(&self.buffer, self.cursor, expert_names);
        if let Some((start, end, repl)) = res.replacement {
            self.buffer.replace_range(start..end, &repl);
            self.cursor = start + repl.len();
        }
        res.candidates
    }
}

// ============================================================================
// ANSI Markdown Renderer & Display-Width Wrapping
// ============================================================================

fn char_display_width(c: char) -> usize {
    if c.is_control() {
        return 0;
    }
    let cp = c as u32;
    // Wide CJK, fullwidth, and emoji ranges
    if (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE10..=0xFE6F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
        || (0x1F300..=0x1FAFF).contains(&cp)
        || (0x20000..=0x2FFFD).contains(&cp)
    {
        2
    } else {
        1
    }
}

fn str_display_width(s: &str) -> usize {
    let mut w = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for esc in chars.by_ref() {
                if ('@'..='~').contains(&esc) {
                    break;
                }
            }
            continue;
        }
        w += char_display_width(c);
    }
    w
}

fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for c in s.chars() {
        let cw = char_display_width(c);
        if w + cw > max_width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}

fn wrap_ansi_line(line: &str, max_width: usize) -> Vec<String> {
    let width = max_width.max(8);
    if str_display_width(line) <= width {
        return vec![line.to_string()];
    }
    let mut result = Vec::new();
    let mut current = String::new();
    let mut cur_width = 0usize;
    let mut active_sgr = String::new();

    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            let mut seq = String::from("\x1b[");
            chars.next();
            for esc in chars.by_ref() {
                seq.push(esc);
                if ('@'..='~').contains(&esc) {
                    break;
                }
            }
            if seq == "\x1b[0m" {
                active_sgr.clear();
            } else if seq.ends_with('m') {
                active_sgr.push_str(&seq);
            }
            current.push_str(&seq);
            continue;
        }
        let cw = char_display_width(c);
        if cur_width + cw > width && cur_width > 0 {
            if !active_sgr.is_empty() {
                current.push_str("\x1b[0m");
            }
            result.push(std::mem::take(&mut current));
            if !active_sgr.is_empty() {
                current.push_str(&active_sgr);
            }
            cur_width = 0;
        }
        current.push(c);
        cur_width += cw;
    }
    if !current.is_empty() || result.is_empty() {
        if !active_sgr.is_empty() && !current.ends_with("\x1b[0m") {
            current.push_str("\x1b[0m");
        }
        result.push(current);
    }
    result
}

fn render_inline_markdown_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;
    let mut in_bold = false;
    let mut in_code = false;

    while i < chars.len() {
        if chars[i] == '`' {
            in_code = !in_code;
            if in_code {
                out.push_str("\x1b[33m");
            } else {
                out.push_str("\x1b[0m");
                if in_bold {
                    out.push_str("\x1b[1m");
                }
            }
            i += 1;
            continue;
        }
        if !in_code && i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            in_bold = !in_bold;
            if in_bold {
                out.push_str("\x1b[1m");
            } else {
                out.push_str("\x1b[22m");
            }
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    if in_bold || in_code {
        out.push_str("\x1b[0m");
    }
    out
}

fn render_markdown_ansi(markdown: &str, width: usize) -> Vec<String> {
    let safe_w = width.max(20);
    let mut lines = Vec::new();
    let mut in_code_block = false;

    for raw_line in markdown.lines() {
        let trimmed = raw_line.trim_start();
        if let Some(fence_rest) = trimmed.strip_prefix("```") {
            if !in_code_block {
                in_code_block = true;
                let lang = fence_rest.trim();
                let header = if lang.is_empty() {
                    "┌─ code ─────────────────".to_string()
                } else {
                    format!("┌─ {lang} ─────────────────")
                };
                lines.push(format!("\x1b[36m{}\x1b[0m", truncate_to_width(&header, safe_w)));
            } else {
                in_code_block = false;
                lines.push("\x1b[36m└────────────────────────\x1b[0m".to_string());
            }
            continue;
        }

        if in_code_block {
            let styled = format!("\x1b[36m│ \x1b[37m{raw_line}\x1b[0m");
            lines.extend(wrap_ansi_line(&styled, safe_w));
            continue;
        }

        if let Some(h) = trimmed.strip_prefix("### ") {
            let styled = format!("\x1b[1;36m### {}\x1b[0m", render_inline_markdown_ansi(h));
            lines.extend(wrap_ansi_line(&styled, safe_w));
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            let styled = format!("\x1b[1;34m## {}\x1b[0m", render_inline_markdown_ansi(h));
            lines.extend(wrap_ansi_line(&styled, safe_w));
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            let styled = format!("\x1b[1;35m# {}\x1b[0m", render_inline_markdown_ansi(h));
            lines.extend(wrap_ansi_line(&styled, safe_w));
        } else if let Some(q) = trimmed.strip_prefix("> ") {
            let styled = format!("\x1b[2m│ {}\x1b[0m", render_inline_markdown_ansi(q));
            lines.extend(wrap_ansi_line(&styled, safe_w));
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let indent_len = raw_line.len().saturating_sub(trimmed.len());
            let indent = &raw_line[..indent_len];
            let styled = format!(
                "{indent}\x1b[36m•\x1b[0m {}",
                render_inline_markdown_ansi(item)
            );
            lines.extend(wrap_ansi_line(&styled, safe_w));
        } else {
            let styled = render_inline_markdown_ansi(raw_line);
            lines.extend(wrap_ansi_line(&styled, safe_w));
        }
    }

    if in_code_block {
        lines.push("\x1b[36m└────────────────────────\x1b[0m".to_string());
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

// ============================================================================
// Base64, Image Sniffing, File Attachments & Clipboard Helpers
// ============================================================================

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        let triple = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn sniff_image_mime(filename: &str, bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("image/png");
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some("image/jpeg");
    }
    if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    let ext = Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum AttachmentKind {
    Image(MediaAttachment),
    Document {
        filename: String,
        text: String,
    },
}

#[derive(Debug, Clone)]
struct PendingAttachment {
    display_name: String,
    kind: AttachmentKind,
}

fn resolve_attachment_path(workspace: &Path, raw: &str) -> PathBuf {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    let p = PathBuf::from(trimmed);
    if p.is_absolute() {
        p
    } else {
        workspace.join(p)
    }
}

fn load_file_attachment(workspace: &Path, raw_path: &str) -> Result<PendingAttachment, String> {
    let path = resolve_attachment_path(workspace, raw_path);
    if !path.is_file() {
        return Err(format!("file not found: {}", path.display()));
    }
    let meta = fs::metadata(&path).map_err(|e| format!("failed to stat '{}': {e}", path.display()))?;
    if meta.len() as usize > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "file '{}' exceeds 16 MiB attachment limit",
            path.display()
        ));
    }
    let bytes =
        fs::read(&path).map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("attachment")
        .to_string();

    if let Some(mime) = sniff_image_mime(&filename, &bytes) {
        let media = MediaAttachment {
            mime_type: mime.to_string(),
            filename: filename.clone(),
            bytes_base64: encode_base64(&bytes),
            sha256: sha256_hex(&bytes),
            raw_bytes_len: bytes.len(),
        };
        return Ok(PendingAttachment {
            display_name: format!("{filename} ({mime}, {} KB)", bytes.len().div_ceil(1024)),
            kind: AttachmentKind::Image(media),
        });
    }

    let extracted = extract_document_text_from_bytes(&filename, "", &bytes).ok_or_else(|| {
        format!(
            "unsupported binary file '{}' (supported: PNG/JPEG/GIF/WEBP images, UTF-8 text, PDF, DOCX, XLSX)",
            filename
        )
    })?;
    let truncated: String = extracted.chars().take(MAX_DOC_ATTACHMENT_CHARS).collect();
    Ok(PendingAttachment {
        display_name: format!("{filename} ({} chars)", truncated.chars().count()),
        kind: AttachmentKind::Document {
            filename,
            text: truncated,
        },
    })
}

fn extract_inline_path_attachments(
    workspace: &Path,
    prompt: &str,
    expert_names: &[String],
) -> (Vec<PendingAttachment>, Vec<String>) {
    let mut attachments = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = BTreeSet::new();

    for raw_tok in prompt.split_whitespace() {
        let Some(after_at) = raw_tok.strip_prefix('@') else {
            continue;
        };
        let cleaned = after_at.trim_end_matches([',', ';', ')', ']', '.']);
        if cleaned.is_empty() {
            continue;
        }
        if expert_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(cleaned))
        {
            continue;
        }
        let candidate = resolve_attachment_path(workspace, cleaned);
        if candidate.is_file() && seen.insert(candidate.clone()) {
            match load_file_attachment(workspace, cleaned) {
                Ok(att) => attachments.push(att),
                Err(e) => warnings.push(format!("@{cleaned}: {e}")),
            }
        }
    }
    (attachments, warnings)
}

enum ClipboardPayload {
    Image(PendingAttachment),
    Text(String),
}

fn read_clipboard_payload() -> Result<ClipboardPayload, String> {
    // 1. On macOS, check for image on clipboard via osascript -> temp PNG
    if cfg!(target_os = "macos") && find_binary_on_path("osascript").is_some() {
        let tmp_png = std::env::temp_dir().join(format!("nola-clip-{}.png", new_id()));
        let _cleanup = TempFileCleanupGuard(tmp_png.clone());
        if let Ok(out) = Command::new("osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "try",
                "-e",
                "set theData to the clipboard as «class PNGf»",
                "-e",
                "set p to POSIX file (item 1 of argv)",
                "-e",
                "set outFile to open for access p with write permission",
                "-e",
                "write theData to outFile",
                "-e",
                "close access outFile",
                "-e",
                "return \"ok\"",
                "-e",
                "on error",
                "-e",
                "try",
                "-e",
                "close access outFile",
                "-e",
                "end try",
                "-e",
                "return \"none\"",
                "-e",
                "end try",
                "-e",
                "end run",
                "--",
            ])
            .arg(&tmp_png)
            .output()
        {
            if out.status.success()
                && String::from_utf8_lossy(&out.stdout).trim() == "ok"
                && tmp_png.is_file()
            {
                if let Ok(bytes) = fs::read(&tmp_png) {
                    if !bytes.is_empty() && bytes.len() <= MAX_ATTACHMENT_BYTES {
                        let media = MediaAttachment {
                            mime_type: "image/png".to_string(),
                            filename: "clipboard.png".to_string(),
                            bytes_base64: encode_base64(&bytes),
                            sha256: sha256_hex(&bytes),
                            raw_bytes_len: bytes.len(),
                        };
                        return Ok(ClipboardPayload::Image(PendingAttachment {
                            display_name: format!(
                                "clipboard.png (image/png, {} KB)",
                                bytes.len().div_ceil(1024)
                            ),
                            kind: AttachmentKind::Image(media),
                        }));
                    }
                }
            }
        }
    }

    // 2. Fallback to text clipboard (`pbpaste`, `wl-paste`, or `xclip`)
    let text_cmds: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else {
        &[
            ("wl-paste", &["-n"]),
            ("xclip", &["-o", "-selection", "clipboard"]),
        ]
    };
    for (bin, args) in text_cmds {
        if find_binary_on_path(bin).is_none() {
            continue;
        }
        if let Ok(out) = Command::new(bin).args(*args).output() {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                if !text.trim().is_empty() {
                    return Ok(ClipboardPayload::Text(text));
                }
                return Err("clipboard is empty".to_string());
            }
        }
    }

    Err("no supported clipboard helper found (expected pbpaste/osascript or wl-paste/xclip)".into())
}

// ============================================================================
// Worker Messages, Modals & Transcript Entries
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalChoice {
    AllowOnce,
    AllowForTargetInSession,
    ReturnToInput,
    Deny,
}

#[derive(Debug, Clone)]
struct PendingQuestionRecord {
    id: String,
    question: String,
    options: Vec<String>,
}

enum WorkerMsg {
    AgentEvent(AgentEvent),
    ApprovalRequest {
        tool_name: String,
        target_key: String,
        summary: String,
        reason: String,
        reply: mpsc::Sender<ApprovalChoice>,
    },
    UserQuestion {
        id: String,
        question: String,
        options: Vec<String>,
        reply: mpsc::Sender<Option<String>>,
    },
    TurnFinished {
        prompt: String,
        result: Result<AgentRunResult, String>,
    },
    Notice {
        is_error: bool,
        text: String,
    },
}

enum ActiveModal {
    Approval {
        tool_name: String,
        target_key: String,
        summary: String,
        reason: String,
        reply: mpsc::Sender<ApprovalChoice>,
    },
    AskUser {
        id: String,
        question: String,
        options: Vec<String>,
        input: ComposerState,
        reply: mpsc::Sender<Option<String>>,
    },
}

#[derive(Debug, Clone)]
enum TranscriptKind {
    User,
    Assistant,
    Reasoning,
    Tool { is_error: bool },
    System,
    Error,
}

#[derive(Debug, Clone)]
struct TranscriptEntry {
    kind: TranscriptKind,
    title: String,
    body: String,
}

#[derive(Debug, Clone)]
struct ActiveExpert {
    name: String,
    role: String,
    system_prompt: String,
}

// ============================================================================
// Main TUI Application State
// ============================================================================

struct TuiApp {
    store: Store,
    suite: Arc<ModulesSuite>,
    workspace: PathBuf,
    db_path: PathBuf,
    cfg: AppliedConfig,
    live_route: Arc<Mutex<ModelRouteConfig>>,
    last_step_usage: Arc<Mutex<Option<TokenUsage>>>,
    raw_caller: Arc<dyn ModelCaller>,
    llm_client: Option<LlmClient>,
    registry: Arc<ToolRegistry>,
    engine: Arc<AgentEngine>,
    subagents: Arc<SubagentManager>,
    ctx: ToolContext,
    permission_profile: PermissionProfile,
    base_policy: EffectivePolicy,
    network_allowed: bool,
    workspace_trusted: bool,
    session_id: String,
    first_turn: bool,
    cancel: Arc<AtomicBool>,
    session_grants: Arc<Mutex<BTreeSet<(String, String)>>>,
    pending_questions: Arc<Mutex<Vec<PendingQuestionRecord>>>,
    active_inbox: Option<Arc<Mutex<Vec<String>>>>,
    active_expert: Option<ActiveExpert>,
    pending_attachments: Vec<PendingAttachment>,
    verbose_tools: bool,
    locale: TuiLocale,
    running: bool,
    spinner_idx: usize,
    transcript: Vec<TranscriptEntry>,
    streaming_text: String,
    streaming_reasoning: String,
    scroll_offset: usize,
    auto_follow: bool,
    composer: ComposerState,
    completion_hint: Option<String>,
    active_modal: Option<ActiveModal>,
    worker_tx: mpsc::Sender<WorkerMsg>,
    worker_rx: mpsc::Receiver<WorkerMsg>,
    last_usage: Option<TokenUsage>,
    should_exit: bool,
    pending_editor_prompt: bool,
    interactive: bool,
}

impl TuiApp {
    fn route_snapshot(&self) -> ModelRouteConfig {
        self.live_route.lock().unwrap().clone()
    }

    fn update_route(&self, next: ModelRouteConfig) {
        *self.live_route.lock().unwrap() = next.clone();
        self.subagents
            .configure_parent(next, self.base_policy.clone());
    }

    fn push_entry(&mut self, kind: TranscriptKind, title: impl Into<String>, body: impl Into<String>) {
        let entry = TranscriptEntry {
            kind,
            title: title.into(),
            body: body.into(),
        };
        if !self.interactive {
            print_entry_line_mode(&entry);
        }
        self.transcript.push(entry);
        if self.auto_follow {
            self.scroll_offset = 0;
        }
    }

    fn push_system(&mut self, msg: impl Into<String>) {
        self.push_entry(TranscriptKind::System, "", msg);
    }

    fn push_error(&mut self, msg: impl Into<String>) {
        self.push_entry(TranscriptKind::Error, "ERR", msg);
    }

    fn known_expert_names(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for tpl in built_in_role_templates() {
            if let Some(id) = tpl.get("id").and_then(|v| v.as_str()) {
                names.insert(id.to_string());
            }
        }
        if let Ok(peers) = self.suite.list_records::<ExpertPeer>("expert_peer") {
            for p in peers {
                if !p.name.trim().is_empty() {
                    names.insert(p.name.replace(' ', "_"));
                }
            }
        }
        names.into_iter().collect()
    }

    fn active_background_count(&self) -> usize {
        let active_subagents = self
            .subagents
            .list()
            .into_iter()
            .filter(|s| s.status == TaskStatus::Running)
            .count();
        let active_jobs = self
            .ctx
            .shell
            .list_jobs()
            .into_iter()
            .filter(|j| j.get("running").and_then(|v| v.as_bool()).unwrap_or(false))
            .count();
        active_subagents + active_jobs
    }

    fn current_plan_mode(&self) -> bool {
        is_plan_mode(&self.store, &self.session_id)
    }

    fn load_session_preview(&mut self, sid: &str) {
        if let Ok(events) = self.store.read_event_range(sid, 1, 200) {
            let mut shown = 0usize;
            for ev in events.iter().rev().take(20).rev() {
                if ev.kind == "turn_completed" {
                    if let Some(txt) = ev.payload.get("final_text").and_then(|v| v.as_str()) {
                        let preview: String = txt.chars().take(200).collect();
                        if !preview.trim().is_empty() {
                            self.push_entry(
                                TranscriptKind::Assistant,
                                format!("history #{}", ev.seq),
                                preview,
                            );
                            shown += 1;
                        }
                    }
                }
            }
            if shown > 0 {
                self.push_system(format!(
                    "{} ({shown})",
                    self.locale
                        .t("Loaded recent session turns", "已加载最近会话轮次")
                ));
            }
        }
    }

    fn start_turn(&mut self, raw_prompt: String) {
        if self.running {
            self.push_error(self.locale.t(
                "A turn is already running. Use /guide <msg> to steer, /model or /thinking to switch live, or Esc/Ctrl+C to cancel.",
                "当前已有任务正在执行。可使用 /guide <消息> 实时引导、/model 或 /thinking 实时切换，或按 Esc/Ctrl+C 中断。",
            ));
            return;
        }

        let expert_names = self.known_expert_names();
        let (inline_atts, warnings) =
            extract_inline_path_attachments(&self.workspace, &raw_prompt, &expert_names);
        for w in warnings {
            self.push_error(w);
        }

        let mut all_attachments = std::mem::take(&mut self.pending_attachments);
        all_attachments.extend(inline_atts);

        let mut final_prompt = raw_prompt.clone();
        let mut media_attachments = Vec::new();
        let mut attached_labels = Vec::new();

        for att in all_attachments {
            attached_labels.push(att.display_name.clone());
            match att.kind {
                AttachmentKind::Image(m) => media_attachments.push(m),
                AttachmentKind::Document { filename, text } => {
                    final_prompt.push_str(&format!(
                        "\n\n<attached_file name=\"{filename}\">\n{text}\n</attached_file>"
                    ));
                }
            }
        }

        let user_header = if attached_labels.is_empty() {
            "you".to_string()
        } else {
            format!("you [{}]", attached_labels.join(", "))
        };
        self.push_entry(TranscriptKind::User, user_header, raw_prompt.clone());

        self.cancel.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = self.last_step_usage.lock() {
            *guard = None;
        }
        self.streaming_text.clear();
        self.streaming_reasoning.clear();
        self.running = true;

        let inbox = Arc::new(Mutex::new(Vec::<String>::new()));
        self.active_inbox = Some(Arc::clone(&inbox));
        self.subagents.set_parent_inbox(Some(Arc::clone(&inbox)));

        let hooks = HookRunner::load(
            &self.workspace,
            self.suite.user_home.as_deref(),
            self.workspace_trusted,
        );
        self.subagents
            .configure_runtime(hooks.clone(), Some(Arc::clone(&self.cancel)));

        let mut frozen_blocks = build_frozen_system_blocks(&self.workspace, &self.cfg);
        if let Some(ref expert) = self.active_expert {
            frozen_blocks.push(FrozenSystemBlock {
                id: format!("expert:{}", expert.role),
                text: format!(
                    "Active expert persona: {} ({})\n{}",
                    expert.name, expert.role, expert.system_prompt
                ),
            });
        }

        let route = self.route_snapshot();
        let grants = Arc::clone(&self.session_grants);
        let store_for_grant = self.store.clone();
        let ws_for_grant = self.workspace.clone();
        let sid_for_grant = self.session_id.clone();
        let cancel_for_prompt = Arc::clone(&self.cancel);
        let tx_for_approval = self.worker_tx.clone();
        let interactive_mode = self.interactive;

        let approval_cb = Arc::new(move |spec: &nola_types::ToolSpec, args: &Value, reason: &str| {
            let target_key = extract_target_key(&spec.schema.name, args);
            let grant_key = (spec.schema.name.clone(), target_key.clone());
            if grants.lock().unwrap().contains(&grant_key) {
                return true;
            }
            let summary = format_permission_summary(&spec.schema.name, &target_key, args);

            let choice = if interactive_mode {
                let (reply_tx, reply_rx) = mpsc::channel();
                if tx_for_approval
                    .send(WorkerMsg::ApprovalRequest {
                        tool_name: spec.schema.name.clone(),
                        target_key: target_key.clone(),
                        summary,
                        reason: reason.to_string(),
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    return false;
                }
                reply_rx.recv().unwrap_or(ApprovalChoice::Deny)
            } else {
                prompt_approval_line_mode(&spec.schema.name, &target_key, &summary, reason)
            };

            match choice {
                ApprovalChoice::AllowOnce => true,
                ApprovalChoice::AllowForTargetInSession => {
                    grants.lock().unwrap().insert(grant_key);
                    let resource_pat = extract_resource_pattern(&spec.schema.name, args);
                    let durable = DurablePermissionGrant::for_approval(
                        Some(sid_for_grant.clone()),
                        &ws_for_grant,
                        &spec.schema.name,
                        resource_pat,
                    );
                    let _ = persist_permission_grant(&store_for_grant, &durable);
                    true
                }
                ApprovalChoice::ReturnToInput => {
                    cancel_for_prompt.store(true, Ordering::SeqCst);
                    false
                }
                ApprovalChoice::Deny => false,
            }
        });

        let run_cfg = AgentRunConfig {
            session_id: Some(self.session_id.clone()),
            resume: !self.first_turn,
            route,
            frozen_system_blocks: frozen_blocks,
            host_capabilities: HostCapabilities {
                interactive_user_input: true,
                network_allowed: self.network_allowed,
            },
            policy: self.base_policy.clone(),
            hooks,
            approval: Some(approval_cb),
            max_steps: self.cfg.max_iterations.unwrap_or(0),
            force_tool_set: ForceToolSet::Auto,
            stream: true,
            json_schema: None,
            cancel: Arc::clone(&self.cancel),
            auto_recall_memory: true,
            inbox: Some(inbox),
        };

        if self.interactive {
            let engine = Arc::clone(&self.engine);
            let ctx = self.ctx.clone();
            let tx = self.worker_tx.clone();
            let prompt_copy = raw_prompt;
            thread::spawn(move || {
                let tx_ev = tx.clone();
                let res = engine.run_with_media(
                    &ctx,
                    &final_prompt,
                    media_attachments,
                    run_cfg,
                    move |ev| {
                        let _ = tx_ev.send(WorkerMsg::AgentEvent(ev));
                    },
                );
                let _ = tx.send(WorkerMsg::TurnFinished {
                    prompt: prompt_copy,
                    result: res.map_err(|e| e.to_string()),
                });
            });
        } else {
            // Non-TTY synchronous line mode
            let verbose = self.verbose_tools;
            let res = self.engine.run_with_media(
                &self.ctx,
                &final_prompt,
                media_attachments,
                run_cfg,
                |ev| match ev {
                    AgentEvent::ReasoningDelta { delta } if verbose => {
                        let mut out = io::stdout().lock();
                        let _ = out.queue(SetAttribute(Attribute::Dim));
                        let _ = write!(out, "{delta}");
                        let _ = out.queue(SetAttribute(Attribute::NormalIntensity));
                        let _ = out.flush();
                    }
                    AgentEvent::TextDelta { delta } => {
                        let mut out = io::stdout().lock();
                        let _ = write!(out, "{delta}");
                        let _ = out.flush();
                    }
                    AgentEvent::ToolStarted {
                        name, arguments, ..
                    } if verbose => {
                        let safe_args = PrivacyRedactor::redact_text(
                            &serde_json::to_string(&arguments).unwrap_or_default(),
                        );
                        eprintln!("\n[RUN {name}] {safe_args}");
                    }
                    AgentEvent::ToolFinished {
                        name,
                        is_error,
                        summary,
                        ..
                    } => {
                        let tag = if is_error { "ERR" } else { "OK" };
                        eprintln!("\n[{tag} {name}] {summary}");
                    }
                    _ => {}
                },
            );
            self.finish_turn(raw_prompt, res.map_err(|e| e.to_string()));
        }
    }

    fn finish_turn(&mut self, prompt: String, result: Result<AgentRunResult, String>) {
        self.running = false;
        self.active_inbox = None;
        self.subagents.set_parent_inbox(None);

        if !self.streaming_reasoning.is_empty() {
            let reasoning = std::mem::take(&mut self.streaming_reasoning);
            self.transcript.push(TranscriptEntry {
                kind: TranscriptKind::Reasoning,
                title: "reasoning".to_string(),
                body: reasoning,
            });
        }
        let streamed_text = std::mem::take(&mut self.streaming_text);

        match result {
            Ok(res) => {
                self.first_turn = false;
                let final_body = if !streamed_text.is_empty() {
                    streamed_text
                } else {
                    res.final_text.clone()
                };
                if !final_body.is_empty() {
                    let route = self.route_snapshot();
                    self.transcript.push(TranscriptEntry {
                        kind: TranscriptKind::Assistant,
                        title: route.model.clone(),
                        body: final_body,
                    });
                }
                let _ = MemoryWorker::distill_turn(&self.suite, "tui", &prompt, &res.final_text);
                let step_usage = self
                    .last_step_usage
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .unwrap_or(res.usage);
                self.last_usage = Some(step_usage.clone());
                if res.cancelled {
                    self.push_system(self.locale.t("Turn cancelled.", "当前轮次已中断。"));
                }
                if !self.interactive {
                    let route = self.route_snapshot();
                    println!();
                    print_context_usage_footer(&route, &step_usage);
                    println!();
                }
            }
            Err(e) => {
                if self.store.get_session(&self.session_id).ok().flatten().is_some() {
                    self.first_turn = false;
                }
                self.push_error(e);
            }
        }
        if self.auto_follow {
            self.scroll_offset = 0;
        }
    }

    fn drain_worker_messages(&mut self) -> bool {
        let mut dirty = false;
        while let Ok(msg) = self.worker_rx.try_recv() {
            dirty = true;
            match msg {
                WorkerMsg::AgentEvent(ev) => match ev {
                    AgentEvent::ReasoningDelta { delta } => {
                        self.streaming_reasoning.push_str(&delta);
                    }
                    AgentEvent::TextDelta { delta } => {
                        self.streaming_text.push_str(&delta);
                    }
                    AgentEvent::ToolStarted {
                        name, arguments, ..
                    } => {
                        if !self.streaming_reasoning.is_empty() {
                            let r = std::mem::take(&mut self.streaming_reasoning);
                            self.transcript.push(TranscriptEntry {
                                kind: TranscriptKind::Reasoning,
                                title: "reasoning".to_string(),
                                body: r,
                            });
                        }
                        if !self.streaming_text.is_empty() {
                            let t = std::mem::take(&mut self.streaming_text);
                            let model = self.route_snapshot().model;
                            self.transcript.push(TranscriptEntry {
                                kind: TranscriptKind::Assistant,
                                title: model,
                                body: t,
                            });
                        }
                        if self.verbose_tools {
                            let safe_args = PrivacyRedactor::redact_text(
                                &serde_json::to_string(&arguments).unwrap_or_default(),
                            );
                            self.transcript.push(TranscriptEntry {
                                kind: TranscriptKind::Tool { is_error: false },
                                title: format!("RUN {name}"),
                                body: safe_args,
                            });
                        }
                    }
                    AgentEvent::ToolFinished {
                        name,
                        is_error,
                        summary,
                        ..
                    } => {
                        let tag = if is_error { "ERR" } else { "OK" };
                        self.transcript.push(TranscriptEntry {
                            kind: TranscriptKind::Tool { is_error },
                            title: format!("{tag} {name}"),
                            body: summary,
                        });
                    }
                    AgentEvent::Compacted {
                        tokens_before,
                        tokens_after,
                        ..
                    } => {
                        self.push_system(format!(
                            "Compacted context: {tokens_before} -> {tokens_after} tokens"
                        ));
                    }
                    _ => {}
                },
                WorkerMsg::ApprovalRequest {
                    tool_name,
                    target_key,
                    summary,
                    reason,
                    reply,
                } => {
                    self.active_modal = Some(ActiveModal::Approval {
                        tool_name,
                        target_key,
                        summary,
                        reason,
                        reply,
                    });
                }
                WorkerMsg::UserQuestion {
                    id,
                    question,
                    options,
                    reply,
                } => {
                    self.active_modal = Some(ActiveModal::AskUser {
                        id,
                        question,
                        options,
                        input: ComposerState::default(),
                        reply,
                    });
                }
                WorkerMsg::TurnFinished { prompt, result } => {
                    self.finish_turn(prompt, result);
                }
                WorkerMsg::Notice { is_error, text } => {
                    if is_error {
                        self.push_error(text);
                    } else {
                        self.push_system(text);
                    }
                }
            }
            if self.auto_follow {
                self.scroll_offset = 0;
            }
        }
        dirty
    }

    fn execute_input(&mut self, raw: &str) {
        match parse_user_input(raw) {
            ParsedInput::Empty => {}
            ParsedInput::LiteralPrompt(prompt) => self.start_turn(prompt),
            ParsedInput::Slash { cmd, arg } => self.execute_slash_command(&cmd, &arg),
        }
    }

    fn execute_slash_command(&mut self, cmd: &str, rest: &str) {
        // Commands allowed live while a turn is actively running
        if self.running {
            let allowed_mid_run = matches!(
                cmd,
                "/help"
                    | "/model"
                    | "/thinking"
                    | "/guide"
                    | "/stop"
                    | "/tasks"
                    | "/jobs"
                    | "/status"
                    | "/questions"
                    | "/verbose"
                    | "/clear"
                    | "/plan"
                    | "/exit"
                    | "/quit"
            );
            if !allowed_mid_run {
                self.push_error(self.locale.t(
                    "Command unavailable while a turn is running. Live mid-run commands: /model, /thinking, /guide, /stop, /tasks, /jobs, /status, /questions, /verbose, /plan show, /clear.",
                    "当前轮次运行中不可执行该命令。运行中可用命令：/model、/thinking、/guide、/stop、/tasks、/jobs、/status、/questions、/verbose、/plan show、/clear。",
                ));
                return;
            }
        }

        match cmd {
            "/exit" | "/quit" => {
                if self.running {
                    self.cancel.store(true, Ordering::SeqCst);
                }
                self.should_exit = true;
            }
            "/help" => {
                self.push_system(format_tui_help(self.locale));
            }
            "/clear" => {
                self.transcript.clear();
                self.scroll_offset = 0;
                self.auto_follow = true;
            }
            "/verbose" => {
                self.verbose_tools = !self.verbose_tools;
                let state = if self.verbose_tools {
                    self.locale.t("expanded", "已展开")
                } else {
                    self.locale.t("compact", "已折叠")
                };
                self.push_system(format!(
                    "{}: {state}",
                    self.locale
                        .t("Reasoning & tool detail display", "思考过程与工具详情显示")
                ));
            }
            "/edit" => {
                if self.interactive {
                    self.pending_editor_prompt = true;
                } else {
                    match compose_prompt_in_editor() {
                        Ok(Some(composed)) => {
                            self.push_system(format!(
                                "Submitting composed prompt ({} chars)...",
                                composed.chars().count()
                            ));
                            self.start_turn(composed);
                        }
                        Ok(None) => self.push_system("Editor closed with empty prompt; nothing sent."),
                        Err(e) => self.push_error(format!("edit error: {e}")),
                    }
                }
            }
            "/new" => {
                self.session_id = new_id();
                self.first_turn = true;
                self.session_grants.lock().unwrap().clear();
                self.last_usage = None;
                self.transcript.clear();
                self.push_system(format!(
                    "{} {}",
                    self.locale
                        .t("Started new session (cleared session grants):", "已开启新会话（已清空会话授权）："),
                    self.session_id
                ));
            }
            "/resume" => {
                if rest.is_empty() {
                    self.push_error("Usage: /resume <session-id>");
                    return;
                }
                match self.store.get_session(rest) {
                    Ok(Some(sess)) => {
                        self.session_id = sess.id.clone();
                        self.first_turn = false;
                        self.session_grants.lock().unwrap().clear();
                        self.push_system(format!(
                            "Resumed session {} ({})",
                            self.session_id, sess.title
                        ));
                        self.load_session_preview(&sess.id);
                    }
                    Ok(None) => {
                        self.push_error(format!("Session '{rest}' not found. Run /sessions to list."));
                    }
                    Err(e) => self.push_error(format!("resume error: {e}")),
                }
            }
            "/sessions" => {
                let limit = rest.parse::<usize>().unwrap_or(15).clamp(1, 50);
                match self.store.list_sessions(limit) {
                    Ok(list) if !list.is_empty() => {
                        let lines: Vec<String> = list
                            .into_iter()
                            .map(|s| {
                                let active = if s.id == self.session_id { "*" } else { " " };
                                let m = s.model_id.as_deref().unwrap_or("-");
                                format!("{active} {}  [{m}]  {}", s.id, s.title)
                            })
                            .collect();
                        self.push_system(lines.join("\n"));
                    }
                    Ok(_) => self.push_system("No sessions recorded yet."),
                    Err(e) => self.push_error(format!("sessions error: {e}")),
                }
            }
            "/compact" => {
                let route = self.route_snapshot();
                match compact_session_now(
                    &self.store,
                    self.llm_client.as_ref(),
                    Some(&route),
                    &self.session_id,
                ) {
                    Ok(val) => {
                        let before = val["before_messages"].as_u64().unwrap_or(0);
                        let after = val["after_messages"].as_u64().unwrap_or(0);
                        let summary = val["summary"].as_str().unwrap_or("");
                        self.push_system(format!(
                            "Compacted session ({before} -> {after} messages): {summary}"
                        ));
                    }
                    Err(e) => self.push_error(format!("compact error: {e}")),
                }
            }
            "/model" => self.cmd_model(rest),
            "/thinking" => self.cmd_thinking(rest),
            "/allow" => self.cmd_allow(rest),
            "/config" => {
                let route = self.route_snapshot();
                self.push_system(format_redacted_tui_config(
                    &self.cfg,
                    &route,
                    self.permission_profile,
                    &self.workspace,
                    &self.db_path,
                ));
            }
            "/plan" => self.cmd_plan(rest),
            "/checkpoint" => {
                let label = if rest.is_empty() { "tui" } else { rest };
                match self.ctx.checkpoints.create(&self.workspace, label) {
                    Ok((id, files)) => {
                        self.push_system(format!("Created checkpoint {id} ({files} files)"));
                    }
                    Err(e) => self.push_error(format!("checkpoint error: {e}")),
                }
            }
            "/checkpoints" => {
                let list = self.ctx.checkpoints.list_in(&self.workspace);
                let (undo_d, redo_d) = self.ctx.checkpoints.stack_depths(&self.workspace);
                if list.is_empty() {
                    self.push_system(format!(
                        "No checkpoints found. (undo_depth={undo_d}, redo_depth={redo_d})"
                    ));
                } else {
                    let mut lines = vec![format!(
                        "Checkpoints (undo_depth={undo_d}, redo_depth={redo_d}):"
                    )];
                    for cp in list {
                        let id = cp["id"].as_str().unwrap_or("");
                        let label = cp["label"].as_str().unwrap_or("");
                        let files = cp["files"].as_u64().unwrap_or(0);
                        lines.push(format!("  {id}  [{label}]  ({files} files)"));
                    }
                    self.push_system(lines.join("\n"));
                }
            }
            "/undo" => match self.ctx.checkpoints.undo(&self.ctx) {
                Ok((id, effects)) => {
                    self.push_system(format!(
                        "Undo applied from {id} ({} file change(s)).",
                        effects.len()
                    ));
                }
                Err(e) => self.push_error(format!("undo error: {e}")),
            },
            "/redo" => match self.ctx.checkpoints.redo(&self.ctx) {
                Ok((id, effects)) => {
                    self.push_system(format!(
                        "Redo applied from {id} ({} file change(s)).",
                        effects.len()
                    ));
                }
                Err(e) => self.push_error(format!("redo error: {e}")),
            },
            "/diff" => self.cmd_diff(rest),
            "/worktree" => self.cmd_worktree(rest),
            "/tasks" | "/jobs" => self.cmd_tasks(),
            "/stop" => self.cmd_stop(rest),
            "/goal" | "/goals" => self.cmd_goal(rest),
            "/sources" => self.cmd_sources(rest),
            "/mcp" => self.cmd_mcp(rest),
            "/memory" => self.cmd_memory(rest),
            "/agent" => self.cmd_agent(rest),
            "/guide" => self.cmd_guide(rest),
            "/questions" => self.cmd_questions(),
            "/history" => self.cmd_history(rest),
            "/status" => self.cmd_status(),
            "/continue" => {
                let msg = if rest.is_empty() {
                    "Continue from where you left off.".to_string()
                } else {
                    format!("Continue: {rest}")
                };
                self.start_turn(msg);
            }
            "/export" => self.cmd_export(rest),
            "/attach" => self.cmd_attach(rest),
            "/paste" => self.cmd_paste(),
            "/skills" => {
                let skills = nola_modules::skills::discover_skills_detailed(
                    Some(&self.store),
                    &self.workspace,
                    self.suite.user_home.as_deref(),
                );
                if skills.is_empty() {
                    self.push_system("No skills discovered.");
                } else {
                    let lines: Vec<String> = skills
                        .into_iter()
                        .map(|s| {
                            let name = s["name"].as_str().unwrap_or("");
                            let desc = s["description"].as_str().unwrap_or("");
                            let enabled = s["enabled"].as_bool().unwrap_or(true);
                            let state = if enabled { "on" } else { "off" };
                            format!("  [{state}] {name} — {desc}")
                        })
                        .collect();
                    self.push_system(lines.join("\n"));
                }
            }
            "/tools" => {
                let specs = if rest.is_empty() {
                    self.registry.all_specs()
                } else {
                    self.registry.search_specs(rest)
                };
                let lines: Vec<String> = specs
                    .into_iter()
                    .map(|spec| {
                        format!(
                            "  [{:?}] {} — {}",
                            spec.tier, spec.schema.name, spec.schema.description
                        )
                    })
                    .collect();
                self.push_system(if lines.is_empty() {
                    "No matching tools.".to_string()
                } else {
                    lines.join("\n")
                });
            }
            "/rooms" => self.cmd_list_rooms(),
            "/room" => self.cmd_room(rest),
            other => {
                self.push_error(format!(
                    "Unknown slash command '{other}'. Type /help for available commands (use // to send a literal leading '/')."
                ));
            }
        }
    }

    fn cmd_model(&mut self, arg: &str) {
        let mut route = self.route_snapshot();
        if arg.is_empty() {
            let mut lines = vec![format!(
                "Active model: {}:{} ({}, ctx={}k, thinking={})",
                route.connection_id,
                route.model,
                route.protocol.as_str(),
                route.context_window / 1000,
                route.reasoning_effort.as_deref().unwrap_or("automatic")
            )];
            for conn in &self.cfg.llm.connections {
                for m in &conn.models {
                    let marker = if conn.id == route.connection_id && m.id == route.model {
                        "*"
                    } else {
                        " "
                    };
                    lines.push(format!("  {marker} {}:{}  ({})", conn.id, m.id, conn.endpoint));
                }
            }
            self.push_system(lines.join("\n"));
            return;
        }

        let prev_effort = route.reasoning_effort.clone();
        let mut matched = false;
        for conn in &self.cfg.llm.connections {
            for m in &conn.models {
                let full_id = format!("{}:{}", conn.id, m.id);
                if m.id == arg || full_id == arg {
                    let mut next_cfg = self.cfg.clone();
                    next_cfg.llm.default_model = Some(DefaultModelSelection {
                        connection: conn.id.clone(),
                        model: m.id.clone(),
                    });
                    if let Ok(mut resolved) = resolve_model_route(&next_cfg) {
                        if resolved.reasoning_effort.is_none() {
                            resolved.reasoning_effort = prev_effort.clone();
                        }
                        route = resolved;
                        matched = true;
                        break;
                    }
                }
            }
            if matched {
                break;
            }
        }
        if !matched {
            route.model = arg.to_string();
        }
        let msg = if self.running {
            format!(
                "Live-switched active model to {}:{} (effective next step)",
                route.connection_id, route.model
            )
        } else {
            format!("Switched active model to {}:{}", route.connection_id, route.model)
        };
        self.update_route(route);
        self.push_system(msg);
    }

    fn cmd_thinking(&mut self, arg: &str) {
        let mut route = self.route_snapshot();
        if arg.is_empty() {
            let cur = route.reasoning_effort.as_deref().unwrap_or("automatic");
            self.push_system(format!(
                "Reasoning effort: {cur} (options: none | low | medium | high | automatic)"
            ));
            return;
        }
        let norm = arg.trim().to_ascii_lowercase();
        match norm.as_str() {
            "none" | "off" => route.reasoning_effort = Some("none".to_string()),
            "low" => route.reasoning_effort = Some("low".to_string()),
            "medium" => route.reasoning_effort = Some("medium".to_string()),
            "high" => route.reasoning_effort = Some("high".to_string()),
            "auto" | "automatic" | "default" => route.reasoning_effort = None,
            other => {
                self.push_error(format!(
                    "Invalid thinking level '{other}'. Expected: none | low | medium | high | automatic"
                ));
                return;
            }
        }
        let label = route
            .reasoning_effort
            .clone()
            .unwrap_or_else(|| "automatic".to_string());
        self.update_route(route);
        if self.running {
            self.push_system(format!(
                "Live-switched reasoning effort to '{label}' (effective next step)"
            ));
        } else {
            self.push_system(format!("Switched reasoning effort to '{label}'"));
        }
    }

    fn cmd_allow(&mut self, rest: &str) {
        if rest.is_empty() {
            self.push_system(format!(
                "Active permission profile: {} (use `/allow cautious`, `/allow accept_edits`, or `/allow allow_all`)",
                self.permission_profile.as_canonical_str()
            ));
            return;
        }
        match PermissionProfile::parse_strict(rest) {
            Ok(next) => {
                self.permission_profile = next;
                self.base_policy = EffectivePolicy::load(
                    &self.workspace,
                    self.suite.user_home.as_deref(),
                    self.workspace_trusted,
                    Some(self.permission_profile),
                    self.network_allowed,
                );
                self.ctx.permission_profile = self.permission_profile;
                self.subagents
                    .configure_parent(self.route_snapshot(), self.base_policy.clone());
                self.push_system(format!(
                    "Switched permission profile to '{}'",
                    self.permission_profile.as_canonical_str()
                ));
            }
            Err(e) => self.push_error(e),
        }
    }

    fn cmd_plan(&mut self, rest: &str) {
        let sub = rest.trim().to_ascii_lowercase();
        match sub.as_str() {
            "" => {
                let next = !self.current_plan_mode();
                match set_plan_mode(&self.store, &self.session_id, next) {
                    Ok(()) => {
                        self.base_policy.plan_mode = next;
                        self.push_system(format!(
                            "Plan Mode: {}",
                            if next { "ON [PLAN]" } else { "OFF" }
                        ));
                    }
                    Err(e) => self.push_error(format!("plan error: {e}")),
                }
            }
            "on" | "enable" => match set_plan_mode(&self.store, &self.session_id, true) {
                Ok(()) => {
                    self.base_policy.plan_mode = true;
                    self.push_system("Plan Mode enabled [PLAN]. Mutating tools are gated until plan approval.");
                }
                Err(e) => self.push_error(format!("plan error: {e}")),
            },
            "off" | "disable" => match set_plan_mode(&self.store, &self.session_id, false) {
                Ok(()) => {
                    self.base_policy.plan_mode = false;
                    self.push_system("Plan Mode disabled.");
                }
                Err(e) => self.push_error(format!("plan error: {e}")),
            },
            "show" => {
                let active = self.current_plan_mode();
                let plan = load_session_plan(&self.store, &self.session_id);
                let todos = load_session_todos(&self.store, &self.session_id);
                let mut out = format!("Plan Mode: {}\n", if active { "ON" } else { "OFF" });
                if plan.is_null() {
                    out.push_str("No proposed plan yet.");
                } else {
                    let rev = plan["revision"].as_u64().unwrap_or(1);
                    let status = plan["status"].as_str().unwrap_or("proposed");
                    let title = plan["title"].as_str().unwrap_or("Plan");
                    let content = plan["content"].as_str().unwrap_or("");
                    out.push_str(&format!("Plan rev {rev} [{status}]: {title}\n{content}"));
                }
                if let Some(arr) = todos.get("todos").and_then(|v| v.as_array()) {
                    if !arr.is_empty() {
                        out.push_str("\n\nTodos:");
                        for t in arr {
                            let st = t["status"].as_str().unwrap_or("pending");
                            let title = t["title"].as_str().unwrap_or("");
                            out.push_str(&format!("\n  [{st}] {title}"));
                        }
                    }
                }
                self.push_system(out);
            }
            "approve" => match approve_agent_plan(&self.store, &self.session_id) {
                Ok(plan) => {
                    self.base_policy.plan_mode = false;
                    let rev = plan["revision"].as_u64().unwrap_or(1);
                    let title = plan["title"].as_str().unwrap_or("Plan");
                    self.push_system(format!(
                        "Approved plan rev {rev} ({title}) and exited Plan Mode."
                    ));
                }
                Err(e) => self.push_error(format!("plan approve error: {e}")),
            },
            other => {
                self.push_error(format!(
                    "Unknown /plan option '{other}'. Usage: /plan [on|off|show|approve]"
                ));
            }
        }
    }

    fn cmd_diff(&mut self, rest: &str) {
        let staged = rest.contains("--staged") || rest.contains("--cached");
        let status_res = self.registry.execute("git", &self.ctx, json!({"action": "status"}));
        let diff_res = self.registry.execute(
            "git",
            &self.ctx,
            json!({"action": "diff", "staged": staged, "stat_only": true}),
        );
        let status_txt = extract_git_stdout(&status_res);
        let diff_txt = extract_git_stdout(&diff_res);
        let mut out = String::new();
        out.push_str("=== git status ===\n");
        out.push_str(if status_txt.trim().is_empty() {
            "(clean)\n"
        } else {
            &status_txt
        });
        out.push_str("\n=== git diff --stat ===\n");
        out.push_str(if diff_txt.trim().is_empty() {
            "(no diff)"
        } else {
            &diff_txt
        });
        self.push_system(out);
    }

    fn cmd_worktree(&mut self, rest: &str) {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        let sub = parts.first().copied().unwrap_or("list");
        let args = match sub {
            "list" => json!({"action": "worktree_list"}),
            "create" => {
                let Some(target) = parts.get(1) else {
                    self.push_error("Usage: /worktree create <path> [branch]");
                    return;
                };
                if let Some(branch) = parts.get(2) {
                    json!({"action": "worktree_create", "target": target, "branch": branch})
                } else {
                    json!({"action": "worktree_create", "target": target})
                }
            }
            "remove" => {
                let Some(target) = parts.get(1) else {
                    self.push_error("Usage: /worktree remove <path>");
                    return;
                };
                json!({"action": "worktree_remove", "target": target})
            }
            "integrate" => {
                let Some(target) = parts.get(1) else {
                    self.push_error("Usage: /worktree integrate <path> [merge|cherry_pick]");
                    return;
                };
                let strategy = parts.get(2).copied().unwrap_or("merge");
                json!({"action": "worktree_integrate", "target": target, "strategy": strategy})
            }
            other => {
                self.push_error(format!(
                    "Unknown /worktree action '{other}'. Usage: /worktree [list|create|remove|integrate]"
                ));
                return;
            }
        };
        let res = self.registry.execute("git", &self.ctx, args);
        let body = extract_git_stdout(&res);
        if res.is_error {
            self.push_error(body);
        } else {
            self.push_system(if body.trim().is_empty() {
                res.payload.to_model_string()
            } else {
                body
            });
        }
    }

    fn cmd_tasks(&mut self) {
        let mut lines = Vec::new();
        let subs = self.subagents.list();
        lines.push(format!("Subagents ({}):", subs.len()));
        if subs.is_empty() {
            lines.push("  (none)".to_string());
        } else {
            for s in subs {
                let wt = s
                    .branch
                    .as_deref()
                    .map(|b| format!(" [branch:{b}]"))
                    .unwrap_or_default();
                lines.push(format!(
                    "  {}  [{:?}]  role={}  steps={}{wt}",
                    s.task_id, s.status, s.role, s.steps
                ));
            }
        }
        let jobs = self.ctx.shell.list_jobs();
        lines.push(format!("Background shell jobs ({}):", jobs.len()));
        if jobs.is_empty() {
            lines.push("  (none)".to_string());
        } else {
            for j in jobs {
                let id = j["job_id"].as_str().unwrap_or("");
                let running = j["running"].as_bool().unwrap_or(false);
                let cmd = j["command"].as_str().unwrap_or("");
                let state = if running { "running" } else { "exited" };
                lines.push(format!("  {id}  [{state}]  {cmd}"));
            }
        }
        self.push_system(lines.join("\n"));
    }

    fn cmd_stop(&mut self, rest: &str) {
        let target = rest.trim();
        if target.is_empty() {
            if self.running {
                self.cancel.store(true, Ordering::SeqCst);
                self.push_system(self.locale.t(
                    "Cancelling active turn...",
                    "正在中断当前执行轮次...",
                ));
            } else {
                self.push_system("No active turn running. Usage: /stop <task_id|job_id>");
            }
            return;
        }

        if target.starts_with("job_") {
            let res = self.registry.execute(
                "bash_control",
                &self.ctx,
                json!({"action": "stop", "job_id": target}),
            );
            if res.is_error {
                self.push_error(res.payload.to_model_string());
            } else {
                self.push_system(format!("Stopped shell job {target}."));
            }
            return;
        }

        match self.subagents.stop(target) {
            Ok(snap) => {
                self.push_system(format!(
                    "Stopped subagent {} (status={:?}, steps={})",
                    snap.task_id, snap.status, snap.steps
                ));
            }
            Err(e) => self.push_error(format!("stop error: {e}")),
        }
    }

    fn cmd_goal(&mut self, rest: &str) {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let sub = parts.next().unwrap_or("").trim();
        let body = parts.next().unwrap_or("").trim();

        match sub {
            "" | "list" => match self.suite.list_records::<ExtendedGoalRecord>("goal") {
                Ok(list) if !list.is_empty() => {
                    let lines: Vec<String> = list
                        .into_iter()
                        .map(|g| {
                            let needs = g
                                .needs_user_question
                                .as_deref()
                                .map(|q| format!(" [NEEDS_USER: {q}]"))
                                .unwrap_or_default();
                            format!(
                                "  {} [{:?}] {} — {}{needs}",
                                g.id, g.status, g.title, g.objective
                            )
                        })
                        .collect();
                    self.push_system(lines.join("\n"));
                }
                Ok(_) => self.push_system(
                    "No goals found. Create one with `/goal create <title> | <objective>`.",
                ),
                Err(e) => self.push_error(format!("goal list error: {e}")),
            },
            "create" => {
                if body.is_empty() {
                    self.push_error("Usage: /goal create <title> | <objective>");
                    return;
                }
                let (title, objective) = match body.split_once('|') {
                    Some((t, o)) => (t.trim(), o.trim()),
                    None => (body, body),
                };
                if title.is_empty() || objective.is_empty() {
                    self.push_error("Goal title and objective must not be empty.");
                    return;
                }
                let ts = now_ms();
                let rec = ExtendedGoalRecord {
                    id: new_id(),
                    title: title.to_string(),
                    objective: objective.to_string(),
                    workspace_root: self.workspace.display().to_string(),
                    status: GoalStatus::Pending,
                    progress_notes: Vec::new(),
                    created_at_ms: ts,
                    updated_at_ms: ts,
                    lease_until_ms: None,
                    retry_count: 0,
                    next_retry_at_ms: None,
                    needs_user_question: None,
                    acceptance_criteria: Vec::new(),
                    accepted: false,
                };
                match self.suite.upsert_record("goal", &rec.id, &rec) {
                    Ok(()) => self.push_system(format!("Created goal {} — {}", rec.id, rec.title)),
                    Err(e) => self.push_error(format!("goal create error: {e}")),
                }
            }
            "status" => {
                if body.is_empty() {
                    self.push_error("Usage: /goal status <goal-id>");
                    return;
                }
                match self.suite.get_record::<ExtendedGoalRecord>("goal", body) {
                    Ok(Some(g)) => {
                        let notes = if g.progress_notes.is_empty() {
                            "(none)".to_string()
                        } else {
                            g.progress_notes.join("\n  - ")
                        };
                        self.push_system(format!(
                            "Goal {} [{:?}]\nTitle: {}\nObjective: {}\nRetries: {}\nAccepted: {}\nNotes:\n  - {}",
                            g.id, g.status, g.title, g.objective, g.retry_count, g.accepted, notes
                        ));
                    }
                    Ok(None) => self.push_error(format!("Goal '{body}' not found.")),
                    Err(e) => self.push_error(format!("goal status error: {e}")),
                }
            }
            "pause" => {
                if body.is_empty() {
                    self.push_error("Usage: /goal pause <goal-id>");
                    return;
                }
                match self.suite.get_record::<ExtendedGoalRecord>("goal", body) {
                    Ok(Some(mut g)) => {
                        g.status = GoalStatus::Paused;
                        g.lease_until_ms = None;
                        g.updated_at_ms = now_ms();
                        match self.suite.upsert_record("goal", &g.id, &g) {
                            Ok(()) => self.push_system(format!("Paused goal {}", g.id)),
                            Err(e) => self.push_error(format!("goal pause error: {e}")),
                        }
                    }
                    Ok(None) => self.push_error(format!("Goal '{body}' not found.")),
                    Err(e) => self.push_error(format!("goal pause error: {e}")),
                }
            }
            "cancel" => {
                if body.is_empty() {
                    self.push_error("Usage: /goal cancel <goal-id>");
                    return;
                }
                match self.suite.get_record::<ExtendedGoalRecord>("goal", body) {
                    Ok(Some(mut g)) => {
                        g.status = GoalStatus::Failed;
                        g.lease_until_ms = None;
                        g.progress_notes.push("[cancelled by user in TUI]".into());
                        g.updated_at_ms = now_ms();
                        match self.suite.upsert_record("goal", &g.id, &g) {
                            Ok(()) => self.push_system(format!("Cancelled goal {}", g.id)),
                            Err(e) => self.push_error(format!("goal cancel error: {e}")),
                        }
                    }
                    Ok(None) => self.push_error(format!("Goal '{body}' not found.")),
                    Err(e) => self.push_error(format!("goal cancel error: {e}")),
                }
            }
            "run" => {
                if body.is_empty() {
                    self.push_error("Usage: /goal run <goal-id>");
                    return;
                }
                let suite = Arc::clone(&self.suite);
                let caller = Arc::clone(&self.raw_caller);
                let route = self.route_snapshot();
                let policy = self.base_policy.clone();
                let goal_id = body.to_string();
                let max_steps = self.cfg.max_iterations.unwrap_or(24);
                let cancel = Arc::new(AtomicBool::new(false));
                if self.interactive {
                    let tx = self.worker_tx.clone();
                    self.push_system(format!("Started autonomous goal {goal_id} in background..."));
                    thread::spawn(move || {
                        let msg = match GoalWorker::execute_goal_ext(
                            &suite, caller, route, policy, &goal_id, max_steps, cancel,
                        ) {
                            Ok(g) => WorkerMsg::Notice {
                                is_error: false,
                                text: format!("Goal {} finished with status {:?}", g.id, g.status),
                            },
                            Err(e) => WorkerMsg::Notice {
                                is_error: true,
                                text: format!("Goal {goal_id} error: {e}"),
                            },
                        };
                        let _ = tx.send(msg);
                    });
                } else {
                    match GoalWorker::execute_goal_ext(
                        &suite, caller, route, policy, &goal_id, max_steps, cancel,
                    ) {
                        Ok(g) => {
                            self.push_system(format!("Goal {} status: {:?}", g.id, g.status));
                        }
                        Err(e) => self.push_error(format!("goal run error: {e}")),
                    }
                }
            }
            _ => self.push_error(
                "Usage: /goal [list|create <title> | <objective>|run <id>|pause <id>|cancel <id>|status <id>]",
            ),
        }
    }

    fn cmd_sources(&mut self, rest: &str) {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let sub = parts.next().unwrap_or("").trim();
        let body = parts.next().unwrap_or("").trim();

        match sub {
            "" | "list" => match self.suite.list_records::<DataSourceRecord>("data_source") {
                Ok(list) if !list.is_empty() => {
                    let lines: Vec<String> = list
                        .into_iter()
                        .map(|ds| {
                            let chunks = ds
                                .metadata
                                .get("indexed_chunks")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            format!(
                                "  {}  [{}]  {}  -> {} (chunks={chunks})",
                                ds.id, ds.kind, ds.name, ds.location
                            )
                        })
                        .collect();
                    self.push_system(lines.join("\n"));
                }
                Ok(_) => self.push_system(
                    "No data sources registered. Use `/sources add <name> <dir|file> <path>`.",
                ),
                Err(e) => self.push_error(format!("sources list error: {e}")),
            },
            "add" => {
                let toks: Vec<&str> = body.splitn(3, char::is_whitespace).collect();
                if toks.len() < 3 {
                    self.push_error("Usage: /sources add <name> <dir|file|url> <location>");
                    return;
                }
                let rec = DataSourceRecord {
                    id: new_id(),
                    name: toks[0].trim().to_string(),
                    kind: toks[1].trim().to_string(),
                    location: toks[2].trim().to_string(),
                    metadata: json!({}),
                    usage_count: 0,
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record("data_source", &rec.id, &rec) {
                    Ok(()) => self.push_system(format!(
                        "Added data source {} ({} -> {}). Run `/sources index {}` to index.",
                        rec.id, rec.name, rec.location, rec.id
                    )),
                    Err(e) => self.push_error(format!("sources add error: {e}")),
                }
            }
            "index" => {
                if body.is_empty() {
                    self.push_error("Usage: /sources index <source-id>");
                    return;
                }
                match index_data_source(&self.store, body) {
                    Ok(chunks) => {
                        self.push_system(format!("Indexed data source {body}: {chunks} chunk(s)."));
                    }
                    Err(e) => self.push_error(format!("sources index error: {e}")),
                }
            }
            "search" => {
                if body.is_empty() {
                    self.push_error("Usage: /sources search <query>");
                    return;
                }
                match self.store.search_doc_chunks(None, body, 8) {
                    Ok(hits) if !hits.is_empty() => {
                        let lines: Vec<String> = hits
                            .into_iter()
                            .map(|h| {
                                let snippet: String =
                                    h.content.replace('\n', " ").chars().take(160).collect();
                                format!("  [{}#{}] {}: {snippet}", h.source_id, h.chunk_index, h.path)
                            })
                            .collect();
                        self.push_system(lines.join("\n"));
                    }
                    Ok(_) => self.push_system(format!("No document chunks matched '{body}'.")),
                    Err(e) => self.push_error(format!("sources search error: {e}")),
                }
            }
            _ => self.push_error("Usage: /sources [list|add <name> <kind> <location>|index <id>|search <query>]"),
        }
    }

    fn cmd_mcp(&mut self, rest: &str) {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let sub = parts.next().unwrap_or("").trim();
        let body = parts.next().unwrap_or("").trim();

        match sub {
            "" | "list" => match self.store.list_mcp_servers() {
                Ok(servers) if !servers.is_empty() => {
                    let lines: Vec<String> = servers
                        .into_iter()
                        .map(|s| {
                            let state = if s.enabled { "on" } else { "off" };
                            let target = s
                                .url
                                .or(s.command)
                                .unwrap_or_else(|| "-".to_string());
                            format!("  [{state}] {} ({:?}) — {} ({target})", s.id, s.transport, s.name)
                        })
                        .collect();
                    self.push_system(lines.join("\n"));
                }
                Ok(_) => self.push_system("No MCP servers configured."),
                Err(e) => self.push_error(format!("mcp list error: {e}")),
            },
            "tools" => {
                let prefix = if body.is_empty() {
                    "mcp__".to_string()
                } else {
                    format!("mcp__{body}__")
                };
                let matched: Vec<String> = self
                    .registry
                    .all_specs()
                    .into_iter()
                    .filter(|s| s.schema.name.starts_with(&prefix))
                    .map(|s| format!("  {} — {}", s.schema.name, s.schema.description))
                    .collect();
                if matched.is_empty() {
                    self.push_system("No discovered MCP tools matched.");
                } else {
                    self.push_system(matched.join("\n"));
                }
            }
            "auth" => {
                if body.is_empty() {
                    self.push_error("Usage: /mcp auth <server-id>");
                    return;
                }
                let servers = match self.store.list_mcp_servers() {
                    Ok(s) => s,
                    Err(e) => {
                        self.push_error(format!("mcp auth error: {e}"));
                        return;
                    }
                };
                let Some(srv) = servers.into_iter().find(|s| s.id == body) else {
                    self.push_error(format!("MCP server '{body}' not found."));
                    return;
                };
                let Some(url) = srv.url.clone() else {
                    self.push_error(format!("MCP server '{body}' has no HTTP/SSE URL for OAuth."));
                    return;
                };
                let vault = match Vault::open(default_vault_path(&self.db_path)) {
                    Ok(v) => Arc::new(v),
                    Err(e) => {
                        self.push_error(format!("failed to open vault: {e}"));
                        return;
                    }
                };
                if self.interactive {
                    let tx = self.worker_tx.clone();
                    let sid = srv.id.clone();
                    let client_id = srv.oauth_client_id.clone();
                    let scopes = srv.oauth_scopes.clone();
                    self.push_system(format!("Starting MCP OAuth 2.1 flow for '{sid}'..."));
                    thread::spawn(move || {
                        let tx_url = tx.clone();
                        let sid_for_url = sid.clone();
                        let res = run_interactive_oauth_flow_with_options(
                            &sid,
                            &url,
                            vault,
                            client_id.as_deref(),
                            &scopes,
                            DEFAULT_OAUTH_CALLBACK_TIMEOUT,
                            move |auth_url| {
                                let _ = tx_url.send(WorkerMsg::Notice {
                                    is_error: false,
                                    text: format!(
                                        "Open this URL to authorize MCP '{sid_for_url}':\n{auth_url}"
                                    ),
                                });
                            },
                        );
                        let notice = match res {
                            Ok(_) => WorkerMsg::Notice {
                                is_error: false,
                                text: format!("MCP OAuth succeeded for '{sid}' and saved tokens to Vault."),
                            },
                            Err(e) => WorkerMsg::Notice {
                                is_error: true,
                                text: format!("MCP OAuth failed for '{sid}': {e}"),
                            },
                        };
                        let _ = tx.send(notice);
                    });
                } else {
                    let res = run_interactive_oauth_flow_with_options(
                        &srv.id,
                        &url,
                        vault,
                        srv.oauth_client_id.as_deref(),
                        &srv.oauth_scopes,
                        DEFAULT_OAUTH_CALLBACK_TIMEOUT,
                        |auth_url| {
                            println!("Open this URL to authorize MCP '{}':\n{auth_url}", srv.id);
                        },
                    );
                    match res {
                        Ok(_) => self.push_system(format!(
                            "MCP OAuth succeeded for '{}' and saved tokens to Vault.",
                            srv.id
                        )),
                        Err(e) => self.push_error(format!("MCP OAuth failed: {e}")),
                    }
                }
            }
            _ => self.push_error("Usage: /mcp [list|tools [server-id]|auth <server-id>]"),
        }
    }

    fn cmd_memory(&mut self, rest: &str) {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let sub = parts.next().unwrap_or("").trim();
        let body = parts.next().unwrap_or("").trim();

        match sub {
            "" | "list" => {
                match search_memory_v2(&self.store, "", &MemorySearchFilter::default(), 15) {
                    Ok(items) if !items.is_empty() => {
                        let lines: Vec<String> = items
                            .into_iter()
                            .map(|r| {
                                let preview: String = r.item.content.chars().take(120).collect();
                                format!(
                                    "  [{}|{}|{}] {}: {preview}",
                                    r.kind.as_str(),
                                    r.owner.as_str(),
                                    r.scope.as_string(),
                                    r.item.title
                                )
                            })
                            .collect();
                        self.push_system(lines.join("\n"));
                    }
                    Ok(_) => self.push_system("No memories saved yet."),
                    Err(e) => self.push_error(format!("memory list error: {e}")),
                }
            }
            "search" => {
                if body.is_empty() {
                    self.push_error("Usage: /memory search <query>");
                    return;
                }
                match search_memory_v2(&self.store, body, &MemorySearchFilter::default(), 10) {
                    Ok(items) if !items.is_empty() => {
                        let lines: Vec<String> = items
                            .into_iter()
                            .map(|r| {
                                let preview: String = r.item.content.chars().take(160).collect();
                                format!("  [{}] {}: {preview}", r.kind.as_str(), r.item.title)
                            })
                            .collect();
                        self.push_system(lines.join("\n"));
                    }
                    Ok(_) => self.push_system(format!("No memory records matched '{body}'.")),
                    Err(e) => self.push_error(format!("memory search error: {e}")),
                }
            }
            "save" => {
                if body.is_empty() {
                    self.push_error("Usage: /memory save <title> | <content>");
                    return;
                }
                let (title, content) = match body.split_once('|') {
                    Some((t, c)) => (t.trim().to_string(), c.trim().to_string()),
                    None => (body.chars().take(48).collect::<String>(), body.to_string()),
                };
                let ts = now_ms();
                let item = MemoryItem {
                    id: new_id(),
                    namespace: "tui".to_string(),
                    title,
                    content,
                    tags: vec!["tui".to_string()],
                    created_at_ms: ts,
                    updated_at_ms: ts,
                };
                let meta = MemoryMetadata {
                    owner: MemoryOwner::User,
                    scope: MemoryScope::Workspace(self.workspace.display().to_string()),
                    kind: MemoryKind::Fact,
                    confidence: 0.95,
                    importance: 0.75,
                    sensitivity: MemorySensitivity::Normal,
                    provenance: MemoryProvenance {
                        source: "tui_manual".to_string(),
                        session_id: Some(self.session_id.clone()),
                        turn_id: None,
                        supersedes: Vec::new(),
                    },
                };
                match save_memory_v2(&self.store, item, meta) {
                    Ok(saved) => self.push_system(format!(
                        "Saved Memory v2 record {} ({})",
                        saved.item.id, saved.item.title
                    )),
                    Err(e) => self.push_error(format!("memory save error: {e}")),
                }
            }
            _ => self.push_error("Usage: /memory [list|search <query>|save <title> | <content>]"),
        }
    }

    fn cmd_agent(&mut self, rest: &str) {
        let arg = rest.trim().trim_start_matches('@');
        if arg.is_empty() {
            let mut lines = Vec::new();
            if let Some(ref active) = self.active_expert {
                lines.push(format!("Active expert: @{} ({})", active.name, active.role));
            } else {
                lines.push("Active expert: (default assistant)".to_string());
            }
            lines.push("Built-in roles:".to_string());
            for tpl in built_in_role_templates() {
                let id = tpl["id"].as_str().unwrap_or("");
                let desc = tpl["description"].as_str().unwrap_or("");
                lines.push(format!("  @{id} — {desc}"));
            }
            if let Ok(peers) = self.suite.list_records::<ExpertPeer>("expert_peer") {
                if !peers.is_empty() {
                    lines.push("Custom ExpertPeers:".to_string());
                    for p in peers {
                        lines.push(format!("  @{} ({})", p.name, p.role));
                    }
                }
            }
            self.push_system(lines.join("\n"));
            return;
        }

        if arg.eq_ignore_ascii_case("clear") || arg.eq_ignore_ascii_case("none") {
            self.active_expert = None;
            self.push_system("Cleared active expert role (back to default assistant).");
            return;
        }

        // 1. Check custom ExpertPeers first
        if let Ok(peers) = self.suite.list_records::<ExpertPeer>("expert_peer") {
            if let Some(p) = peers.into_iter().find(|p| {
                p.id.eq_ignore_ascii_case(arg)
                    || p.name.eq_ignore_ascii_case(arg)
                    || p.name.replace(' ', "_").eq_ignore_ascii_case(arg)
            }) {
                let ov = p.model_override.clone();
                self.active_expert = Some(ActiveExpert {
                    name: p.name.clone(),
                    role: p.role.clone(),
                    system_prompt: p.system_prompt,
                });
                if let Some(ref m) = ov {
                    self.cmd_model(m);
                }
                self.push_system(format!("Selected expert @{} ({})", p.name, p.role));
                return;
            }
        }

        // 2. Check built-in role templates
        for tpl in built_in_role_templates() {
            let id = tpl["id"].as_str().unwrap_or("");
            let name = tpl["name"].as_str().unwrap_or(id);
            if id.eq_ignore_ascii_case(arg) || name.eq_ignore_ascii_case(arg) {
                self.active_expert = Some(ActiveExpert {
                    name: name.to_string(),
                    role: id.to_string(),
                    system_prompt: tpl["system_prompt"].as_str().unwrap_or("").to_string(),
                });
                self.push_system(format!("Selected built-in expert @{id} ({name})"));
                return;
            }
        }

        self.push_error(format!(
            "Expert '{arg}' not found. Run `/agent` to list available experts."
        ));
    }

    fn cmd_guide(&mut self, rest: &str) {
        if rest.is_empty() {
            self.push_error("Usage: /guide <steering message>");
            return;
        }
        let pushed = if let Some(ref inbox) = self.active_inbox {
            if let Ok(mut q) = inbox.lock() {
                q.push(format!("[Live user guidance]: {rest}"));
                true
            } else {
                false
            }
        } else {
            false
        };
        if pushed {
            self.push_system(format!(
                "{}: {rest}",
                self.locale
                    .t("Queued live guidance into active turn", "已将实时引导注入当前轮次")
            ));
            return;
        }
        self.push_system(self.locale.t(
            "No turn currently running — starting turn with your guidance.",
            "当前无运行中轮次，将直接发送该引导消息。",
        ));
        self.start_turn(rest.to_string());
    }

    fn cmd_questions(&mut self) {
        let mut lines = Vec::new();
        let pending = self.pending_questions.lock().unwrap().clone();
        if !pending.is_empty() {
            lines.push("Pending interactive questions:".to_string());
            for q in pending {
                lines.push(format!("  [{}] {}", q.id, q.question));
                for (i, opt) in q.options.iter().enumerate() {
                    lines.push(format!("    {}. {opt}", i + 1));
                }
            }
        }
        if let Ok(goals) = self.suite.list_records::<ExtendedGoalRecord>("goal") {
            let goal_qs: Vec<_> = goals.into_iter().filter(|g| g.is_needs_user()).collect();
            if !goal_qs.is_empty() {
                lines.push("Goals awaiting user input:".to_string());
                for g in goal_qs {
                    lines.push(format!(
                        "  [goal:{}] {}: {}",
                        g.id,
                        g.title,
                        g.needs_user_question.as_deref().unwrap_or("")
                    ));
                }
            }
        }
        if lines.is_empty() {
            self.push_system("No pending user questions.");
        } else {
            self.push_system(lines.join("\n"));
        }
    }

    fn cmd_history(&mut self, rest: &str) {
        let limit = rest.parse::<usize>().unwrap_or(20).clamp(1, 100);
        match self.store.read_event_range(&self.session_id, 1, limit) {
            Ok(events) if !events.is_empty() => {
                let lines: Vec<String> = events
                    .into_iter()
                    .map(|e| {
                        let snippet: String = e
                            .payload
                            .get("final_text")
                            .or_else(|| e.payload.get("summary"))
                            .or_else(|| e.payload.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .replace('\n', " ")
                            .chars()
                            .take(100)
                            .collect();
                        format!("  #{} [{}] {snippet}", e.seq, e.kind)
                    })
                    .collect();
                self.push_system(lines.join("\n"));
            }
            Ok(_) => self.push_system("No persisted events in current session yet."),
            Err(e) => self.push_error(format!("history error: {e}")),
        }
    }

    fn cmd_status(&mut self) {
        let route = self.route_snapshot();
        let (undo_d, redo_d) = self.ctx.checkpoints.stack_depths(&self.workspace);
        let plan_on = self.current_plan_mode();
        let expert = self
            .active_expert
            .as_ref()
            .map(|e| format!("@{} ({})", e.name, e.role))
            .unwrap_or_else(|| "default".to_string());
        let bg = self.active_background_count();
        let usage_line = self
            .last_usage
            .as_ref()
            .map(|u| format_context_usage_line(&route, u))
            .unwrap_or_else(|| "Context: n/a".to_string());
        self.push_system(format!(
            "Session:     {}\n\
             Model:       {}:{} ({})\n\
             Thinking:    {}\n\
             Permission:  {}\n\
             Plan Mode:   {}\n\
             Expert:      {}\n\
             Running:     {}\n\
             Tasks/Jobs:  {} active\n\
             Checkpoints: undo={undo_d}, redo={redo_d}\n\
             Attachments: {} pending\n\
             {usage_line}",
            self.session_id,
            route.connection_id,
            route.model,
            route.protocol.as_str(),
            route.reasoning_effort.as_deref().unwrap_or("automatic"),
            self.permission_profile.as_canonical_str(),
            if plan_on { "ON [PLAN]" } else { "OFF" },
            expert,
            if self.running { "yes" } else { "no" },
            bg,
            self.pending_attachments.len()
        ));
    }

    fn cmd_export(&mut self, rest: &str) {
        if rest.is_empty() {
            self.push_error("Usage: /export <output-path.md>");
            return;
        }
        let target = resolve_attachment_path(&self.workspace, rest);
        let mut md = format!(
            "# Nola Session Export ({})\n\n- Workspace: `{}`\n- Model: `{}`\n\n---\n\n",
            self.session_id,
            self.workspace.display(),
            self.route_snapshot().model
        );
        for entry in &self.transcript {
            let role = match entry.kind {
                TranscriptKind::User => "## User",
                TranscriptKind::Assistant => "## Assistant",
                TranscriptKind::Reasoning => "### Reasoning",
                TranscriptKind::Tool { .. } => "### Tool",
                TranscriptKind::System => "> System",
                TranscriptKind::Error => "> Error",
            };
            if entry.title.is_empty() {
                md.push_str(&format!("{role}\n\n{}\n\n", entry.body));
            } else {
                md.push_str(&format!("{role} ({})\n\n{}\n\n", entry.title, entry.body));
            }
        }
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::write(&target, md) {
            Ok(()) => self.push_system(format!("Exported transcript to {}", target.display())),
            Err(e) => self.push_error(format!("export error: {e}")),
        }
    }

    fn cmd_attach(&mut self, rest: &str) {
        let arg = rest.trim();
        if arg.is_empty() || arg == "list" {
            if self.pending_attachments.is_empty() {
                self.push_system("No pending attachments. Use `/attach <path>` or `@<path>` in a prompt.");
            } else {
                let lines: Vec<String> = self
                    .pending_attachments
                    .iter()
                    .enumerate()
                    .map(|(i, a)| format!("  {}. {}", i + 1, a.display_name))
                    .collect();
                self.push_system(format!("Pending attachments:\n{}", lines.join("\n")));
            }
            return;
        }
        if arg == "clear" {
            let n = self.pending_attachments.len();
            self.pending_attachments.clear();
            self.push_system(format!("Cleared {n} pending attachment(s)."));
            return;
        }
        match load_file_attachment(&self.workspace, arg) {
            Ok(att) => {
                let label = att.display_name.clone();
                self.pending_attachments.push(att);
                self.push_system(format!("Attached: {label}"));
            }
            Err(e) => self.push_error(format!("attach error: {e}")),
        }
    }

    fn cmd_paste(&mut self) {
        match read_clipboard_payload() {
            Ok(ClipboardPayload::Image(att)) => {
                let label = att.display_name.clone();
                self.pending_attachments.push(att);
                self.push_system(format!("Pasted clipboard image: {label}"));
            }
            Ok(ClipboardPayload::Text(text)) => {
                let preview_len = text.chars().count();
                if self.interactive && !text.contains('\n') && preview_len <= 400 {
                    self.composer.insert_str(&text);
                    self.push_system(format!("Pasted {preview_len} chars into composer."));
                } else {
                    self.pending_attachments.push(PendingAttachment {
                        display_name: format!("clipboard.txt ({preview_len} chars)"),
                        kind: AttachmentKind::Document {
                            filename: "clipboard.txt".to_string(),
                            text: text.chars().take(MAX_DOC_ATTACHMENT_CHARS).collect(),
                        },
                    });
                    self.push_system(format!(
                        "Attached pasted clipboard text ({preview_len} chars) to next prompt."
                    ));
                }
            }
            Err(e) => self.push_error(format!("paste error: {e}")),
        }
    }

    fn cmd_list_rooms(&mut self) {
        match self.suite.list_records::<AgentRoom>("agent_room") {
            Ok(list) if !list.is_empty() => {
                let lines: Vec<String> = list
                    .into_iter()
                    .map(|r| format!("  {} — {}", r.id, r.name))
                    .collect();
                self.push_system(lines.join("\n"));
            }
            Ok(_) => self.push_system("No agent rooms found."),
            Err(e) => self.push_error(format!("room list error: {e}")),
        }
    }

    fn cmd_room(&mut self, rest: &str) {
        if rest.is_empty() || rest == "list" {
            self.cmd_list_rooms();
            return;
        }
        let mut parts = rest.splitn(2, char::is_whitespace);
        let room_id = parts.next().unwrap_or("").trim();
        let tail = parts.next().unwrap_or("").trim();
        if room_id.is_empty() || tail.is_empty() {
            self.push_error("Usage: /room <room-id> [@recipient] <message>");
            return;
        }
        let (recipient, message) = if let Some(after_at) = tail.strip_prefix('@') {
            let mut rparts = after_at.splitn(2, char::is_whitespace);
            let rec = rparts.next().unwrap_or("").trim();
            let msg = rparts.next().unwrap_or("").trim();
            if rec.is_empty() || msg.is_empty() {
                self.push_error("Usage: /room <room-id> [@recipient] <message>");
                return;
            }
            (Some(rec), msg)
        } else {
            (None, tail)
        };
        let route = self.route_snapshot();
        match RoomRuntime::send_message_to(
            &self.suite,
            Arc::clone(&self.raw_caller),
            route,
            self.base_policy.clone(),
            room_id,
            message,
            recipient,
        ) {
            Ok(replies) => {
                for r in replies {
                    let name = r["expert_name"].as_str().unwrap_or("Expert");
                    let content = r["content"].as_str().unwrap_or("");
                    self.push_entry(TranscriptKind::Assistant, format!("room:{name}"), content);
                }
            }
            Err(e) => self.push_error(format!("room error: {e}")),
        }
    }

    // ========================================================================
    // Fullscreen Viewport Rendering & Input Handling
    // ========================================================================

    fn render_transcript_lines(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for entry in &self.transcript {
            match entry.kind {
                TranscriptKind::Reasoning if !self.verbose_tools => continue,
                TranscriptKind::User => {
                    lines.push(format!("\x1b[1;32m▸ {}\x1b[0m", entry.title));
                    for l in entry.body.lines() {
                        lines.extend(wrap_ansi_line(&format!("  {l}"), width));
                    }
                    lines.push(String::new());
                }
                TranscriptKind::Assistant => {
                    lines.push(format!("\x1b[1;36m◆ {}\x1b[0m", entry.title));
                    lines.extend(render_markdown_ansi(&entry.body, width));
                    lines.push(String::new());
                }
                TranscriptKind::Reasoning => {
                    lines.push("\x1b[2m┄ reasoning ┄\x1b[0m".to_string());
                    for l in entry.body.lines() {
                        lines.extend(wrap_ansi_line(&format!("\x1b[2m  {l}\x1b[0m"), width));
                    }
                }
                TranscriptKind::Tool { is_error } => {
                    let color = if is_error { "\x1b[31m" } else { "\x1b[33m" };
                    let header = format!("{color}[{}]\x1b[0m {}", entry.title, entry.body);
                    lines.extend(wrap_ansi_line(&header, width));
                }
                TranscriptKind::System => {
                    for l in entry.body.lines() {
                        lines.extend(wrap_ansi_line(&format!("\x1b[36m{l}\x1b[0m"), width));
                    }
                    lines.push(String::new());
                }
                TranscriptKind::Error => {
                    for l in entry.body.lines() {
                        lines.extend(wrap_ansi_line(&format!("\x1b[1;31m{l}\x1b[0m"), width));
                    }
                    lines.push(String::new());
                }
            }
        }

        if self.verbose_tools && !self.streaming_reasoning.is_empty() {
            lines.push("\x1b[2m┄ thinking... ┄\x1b[0m".to_string());
            for l in self.streaming_reasoning.lines() {
                lines.extend(wrap_ansi_line(&format!("\x1b[2m  {l}\x1b[0m"), width));
            }
        }
        if !self.streaming_text.is_empty() {
            let model = self.route_snapshot().model;
            lines.push(format!("\x1b[1;36m◆ {model} ...\x1b[0m"));
            lines.extend(render_markdown_ansi(&self.streaming_text, width));
        }

        lines
    }

    fn draw_screen(&mut self) -> io::Result<()> {
        let (cols_u16, rows_u16) = terminal::size().unwrap_or((100, 30));
        let cols = cols_u16.max(40) as usize;
        let rows = rows_u16.max(12) as usize;

        let route = self.route_snapshot();
        let plan_badge = if self.current_plan_mode() {
            " [PLAN]"
        } else {
            ""
        };
        let expert_badge = self
            .active_expert
            .as_ref()
            .map(|e| format!(" @{}", e.role))
            .unwrap_or_default();
        let tasks_count = self.active_background_count();
        let spinner = if self.running {
            format!(" {}", SPINNER_FRAMES[self.spinner_idx % SPINNER_FRAMES.len()])
        } else {
            String::new()
        };
        let short_sid: String = self.session_id.chars().take(8).collect();
        let header_text = format!(
            " Nola v2{spinner} | {} | think:{} | perm:{}{plan_badge}{expert_badge} | sid:{short_sid} | tasks:{tasks_count}",
            route.model,
            route.reasoning_effort.as_deref().unwrap_or("auto"),
            self.permission_profile.as_canonical_str()
        );

        let modal_lines = self.build_modal_lines(cols);
        let modal_height = modal_lines.len();
        let footer_height = 3 + modal_height; // status line + separator + composer (+ hint/modal)
        let header_height = 2usize;
        let viewport_height = rows.saturating_sub(header_height + footer_height).max(2);

        let all_lines = self.render_transcript_lines(cols.saturating_sub(1));
        let max_scroll = all_lines.len().saturating_sub(viewport_height);
        if self.auto_follow {
            self.scroll_offset = 0;
        } else if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
        let end_idx = all_lines.len().saturating_sub(self.scroll_offset);
        let start_idx = end_idx.saturating_sub(viewport_height);

        let mut out = io::stdout().lock();
        out.queue(Hide)?;
        out.queue(MoveTo(0, 0))?;

        // Row 0: Header status bar
        out.queue(SetBackgroundColor(Color::DarkBlue))?;
        out.queue(SetForegroundColor(Color::White))?;
        out.queue(SetAttribute(Attribute::Bold))?;
        let clipped_header = truncate_to_width(&header_text, cols);
        let pad = cols.saturating_sub(str_display_width(&clipped_header));
        write!(out, "{clipped_header}{}", " ".repeat(pad))?;
        out.queue(SetAttribute(Attribute::Reset))?;
        out.queue(ResetColor)?;

        // Row 1: Separator
        out.queue(MoveTo(0, 1))?;
        out.queue(SetForegroundColor(Color::DarkGrey))?;
        write!(out, "{}", "─".repeat(cols))?;
        out.queue(ResetColor)?;

        // Viewport rows
        for row_idx in 0..viewport_height {
            let y = (header_height + row_idx) as u16;
            out.queue(MoveTo(0, y))?;
            out.queue(Clear(ClearType::CurrentLine))?;
            if let Some(line) = all_lines.get(start_idx + row_idx) {
                write!(out, "{line}\x1b[0m")?;
            }
        }

        // Optional modal overlay above footer
        let mut cursor_y = (header_height + viewport_height) as u16;
        for mline in &modal_lines {
            out.queue(MoveTo(0, cursor_y))?;
            out.queue(Clear(ClearType::CurrentLine))?;
            write!(out, "{mline}\x1b[0m")?;
            cursor_y += 1;
        }

        // Status/context footer line
        out.queue(MoveTo(0, cursor_y))?;
        out.queue(Clear(ClearType::CurrentLine))?;
        out.queue(SetAttribute(Attribute::Dim))?;
        let footer_text = if let Some(ref hint) = self.completion_hint {
            hint.clone()
        } else if self.running {
            self.locale
                .t(
                    "Running... Esc/Ctrl+C cancel · /guide <msg> · /model <id> · /thinking <lvl> · PgUp/PgDn scroll",
                    "运行中... Esc/Ctrl+C 中断 · /guide <消息> · /model <模型> · /thinking <等级> · PgUp/PgDn 翻页",
                )
                .to_string()
        } else {
            let usage_part = self
                .last_usage
                .as_ref()
                .map(|u| format_context_usage_line(&route, u))
                .unwrap_or_else(|| {
                    self.locale
                        .t("Type /help for commands · Tab to complete · @path to attach", "输入 /help 查看命令 · Tab 自动补全 · @路径 附加文件")
                        .to_string()
                });
            let att_part = if self.pending_attachments.is_empty() {
                String::new()
            } else {
                format!(" · files:{}", self.pending_attachments.len())
            };
            let scroll_part = if self.scroll_offset > 0 {
                format!(" · ↑{}", self.scroll_offset)
            } else {
                String::new()
            };
            format!("{usage_part}{att_part}{scroll_part}")
        };
        write!(out, "{}", truncate_to_width(&footer_text, cols))?;
        out.queue(SetAttribute(Attribute::Reset))?;
        cursor_y += 1;

        // Bottom separator
        out.queue(MoveTo(0, cursor_y))?;
        out.queue(SetForegroundColor(Color::DarkGrey))?;
        write!(out, "{}", "─".repeat(cols))?;
        out.queue(ResetColor)?;
        cursor_y += 1;

        // Composer line
        out.queue(MoveTo(0, cursor_y))?;
        out.queue(Clear(ClearType::CurrentLine))?;
        let (prompt_prefix, active_composer) = match self.active_modal.as_ref() {
            Some(ActiveModal::AskUser { input, .. }) => ("answer> ", input),
            _ => ("nola> ", &self.composer),
        };
        out.queue(SetForegroundColor(Color::Green))?;
        out.queue(SetAttribute(Attribute::Bold))?;
        write!(out, "{prompt_prefix}")?;
        out.queue(SetAttribute(Attribute::Reset))?;
        out.queue(ResetColor)?;

        let avail = cols.saturating_sub(prompt_prefix.len() + 1).max(10);
        let cursor_col_in_buf = str_display_width(&active_composer.buffer[..active_composer.cursor]);
        let visible_buf = if str_display_width(&active_composer.buffer) <= avail {
            active_composer.buffer.clone()
        } else {
            // Simple tail window keeping cursor visible
            let mut slice_start = 0usize;
            while slice_start < active_composer.cursor
                && str_display_width(&active_composer.buffer[slice_start..active_composer.cursor])
                    > avail.saturating_sub(2)
            {
                if let Some(ch) = active_composer.buffer[slice_start..].chars().next() {
                    slice_start += ch.len_utf8();
                } else {
                    break;
                }
            }
            truncate_to_width(&active_composer.buffer[slice_start..], avail)
        };
        write!(out, "{visible_buf}")?;

        if !matches!(self.active_modal, Some(ActiveModal::Approval { .. })) {
            let cursor_x = (prompt_prefix.len() + cursor_col_in_buf.min(avail)) as u16;
            out.queue(MoveTo(cursor_x, cursor_y))?;
            out.queue(Show)?;
        }

        out.flush()
    }

    fn build_modal_lines(&self, cols: usize) -> Vec<String> {
        let Some(modal) = self.active_modal.as_ref() else {
            return Vec::new();
        };
        let inner_w = cols.saturating_sub(4).max(20);
        match modal {
            ActiveModal::Approval {
                tool_name,
                target_key,
                summary,
                reason,
                ..
            } => {
                let title = self.locale.t("Permission Approval", "权限审批");
                let prompt = self.locale.t(
                    "[1/y] Allow once  [2/a] Allow in session (durable)  [3/r/Esc] Return to input  [n/Enter] Deny",
                    "[1/y] 允许一次  [2/a] 本会话持久化允许  [3/r/Esc] 返回输入  [n/Enter] 拒绝",
                );
                vec![
                    format!("\x1b[1;33m┌─ {title} ─────────────────────────────\x1b[0m"),
                    format!(
                        "\x1b[33m│\x1b[0m Tool:    {}",
                        truncate_to_width(tool_name, inner_w)
                    ),
                    format!(
                        "\x1b[33m│\x1b[0m Target:  {}",
                        truncate_to_width(target_key, inner_w)
                    ),
                    format!(
                        "\x1b[33m│\x1b[0m Summary: {}",
                        truncate_to_width(summary, inner_w)
                    ),
                    format!(
                        "\x1b[33m│\x1b[0m Reason:  {}",
                        truncate_to_width(reason, inner_w)
                    ),
                    format!(
                        "\x1b[1;33m└─ {}\x1b[0m",
                        truncate_to_width(prompt, inner_w)
                    ),
                ]
            }
            ActiveModal::AskUser {
                question, options, ..
            } => {
                let title = self.locale.t("Agent Question", "助手提问");
                let mut lines = vec![
                    format!("\x1b[1;35m┌─ {title} ─────────────────────────────\x1b[0m"),
                    format!(
                        "\x1b[35m│\x1b[0m {}",
                        truncate_to_width(question, inner_w)
                    ),
                ];
                for (i, opt) in options.iter().enumerate() {
                    lines.push(format!(
                        "\x1b[35m│\x1b[0m   \x1b[1m{}.\x1b[0m {}",
                        i + 1,
                        truncate_to_width(opt, inner_w.saturating_sub(5))
                    ));
                }
                let hint = self.locale.t(
                    "Type option number or free-text answer and press Enter (Esc to cancel)",
                    "输入选项编号或自由文本并按 Enter 提交（Esc 取消）",
                );
                lines.push(format!(
                    "\x1b[1;35m└─ {}\x1b[0m",
                    truncate_to_width(hint, inner_w)
                ));
                lines
            }
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        self.completion_hint = None;

        // 1. Route keys to active modal first
        if let Some(modal) = self.active_modal.take() {
            match modal {
                ActiveModal::Approval {
                    tool_name,
                    target_key,
                    summary,
                    reason,
                    reply,
                } => {
                    let choice = if key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
                    {
                        Some(ApprovalChoice::ReturnToInput)
                    } else if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
                        match key.code {
                            KeyCode::Char('1') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                                Some(ApprovalChoice::AllowOnce)
                            }
                            KeyCode::Char('2') | KeyCode::Char('a') | KeyCode::Char('A') => {
                                Some(ApprovalChoice::AllowForTargetInSession)
                            }
                            KeyCode::Char('3')
                            | KeyCode::Char('r')
                            | KeyCode::Char('R')
                            | KeyCode::Esc => Some(ApprovalChoice::ReturnToInput),
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter => {
                                Some(ApprovalChoice::Deny)
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if let Some(c) = choice {
                        let _ = reply.send(c);
                    } else {
                        self.active_modal = Some(ActiveModal::Approval {
                            tool_name,
                            target_key,
                            summary,
                            reason,
                            reply,
                        });
                    }
                    return;
                }
                ActiveModal::AskUser {
                    id,
                    question,
                    options,
                    mut input,
                    reply,
                } => {
                    if key.code == KeyCode::Esc
                        || (key.modifiers.contains(KeyModifiers::CONTROL)
                            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')))
                    {
                        self.pending_questions
                            .lock()
                            .unwrap()
                            .retain(|q| q.id != id);
                        let _ = reply.send(None);
                        return;
                    }
                    match key.code {
                        KeyCode::Enter => {
                            let raw = input.submit();
                            let trimmed = raw.trim();
                            let answer = if trimmed.is_empty() {
                                None
                            } else if let Ok(num) = trimmed.parse::<usize>() {
                                if num >= 1 && num <= options.len() {
                                    Some(options[num - 1].clone())
                                } else {
                                    Some(trimmed.to_string())
                                }
                            } else {
                                Some(trimmed.to_string())
                            };
                            self.pending_questions
                                .lock()
                                .unwrap()
                                .retain(|q| q.id != id);
                            let _ = reply.send(answer);
                            return;
                        }
                        KeyCode::Left => input.move_left(),
                        KeyCode::Right => input.move_right(),
                        KeyCode::Home => input.move_home(),
                        KeyCode::End => input.move_end(),
                        KeyCode::Backspace => input.backspace(),
                        KeyCode::Delete => input.delete(),
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
                                && !key.modifiers.contains(KeyModifiers::ALT) =>
                        {
                            input.insert_char(c);
                        }
                        _ => {}
                    }
                    self.active_modal = Some(ActiveModal::AskUser {
                        id,
                        question,
                        options,
                        input,
                        reply,
                    });
                    return;
                }
            }
        }

        // 2. Global shortcuts & scrolling
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    if self.running {
                        self.cancel.store(true, Ordering::SeqCst);
                        self.push_system(self.locale.t(
                            "Cancelling active turn...",
                            "正在中断当前执行轮次...",
                        ));
                    } else if !self.composer.buffer.is_empty() {
                        self.composer.clear();
                    } else {
                        self.should_exit = true;
                    }
                    return;
                }
                KeyCode::Char('u') | KeyCode::Char('U') => {
                    self.auto_follow = false;
                    self.scroll_offset = self.scroll_offset.saturating_add(10);
                    return;
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(10);
                    if self.scroll_offset == 0 {
                        self.auto_follow = true;
                    }
                    return;
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    self.composer.move_home();
                    return;
                }
                KeyCode::Char('e') | KeyCode::Char('E') => {
                    self.composer.move_end();
                    return;
                }
                KeyCode::Char('l') | KeyCode::Char('L') => {
                    self.transcript.clear();
                    self.scroll_offset = 0;
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc => {
                if self.running {
                    self.cancel.store(true, Ordering::SeqCst);
                    self.push_system(self.locale.t(
                        "Cancelling active turn...",
                        "正在中断当前执行轮次...",
                    ));
                } else {
                    self.composer.clear();
                }
            }
            KeyCode::PageUp => {
                self.auto_follow = false;
                self.scroll_offset = self.scroll_offset.saturating_add(12);
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(12);
                if self.scroll_offset == 0 {
                    self.auto_follow = true;
                }
            }
            KeyCode::Left => self.composer.move_left(),
            KeyCode::Right => self.composer.move_right(),
            KeyCode::Home => self.composer.move_home(),
            KeyCode::End => self.composer.move_end(),
            KeyCode::Up => self.composer.history_up(),
            KeyCode::Down => self.composer.history_down(),
            KeyCode::Backspace => self.composer.backspace(),
            KeyCode::Delete => self.composer.delete(),
            KeyCode::Tab => {
                let experts = self.known_expert_names();
                let candidates = self.composer.apply_tab_completion(&experts);
                if candidates.len() > 1 {
                    self.completion_hint = Some(format!("Tab: {}", candidates.join("  ")));
                }
            }
            KeyCode::Enter => {
                let line = self.composer.submit();
                self.execute_input(&line);
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.composer.insert_char(c);
            }
            _ => {}
        }
    }
}

// ============================================================================
// Public Entrypoint
// ============================================================================

pub fn run_tui(
    store: Store,
    suite: Arc<ModulesSuite>,
    workspace: &Path,
    launch: TuiLaunchOptions,
) -> Result<u8, String> {
    let cfg = load_active_config(&store);
    let mut route = resolve_model_route(&cfg)?;
    if let Some(ref model_override) = launch.model {
        apply_tui_model_override(&cfg, &mut route, model_override);
    }
    if let Some(ref ep_override) = launch.endpoint {
        route.endpoint = ep_override.clone();
    }

    let live_route = Arc::new(Mutex::new(route.clone()));
    let last_step_usage = Arc::new(Mutex::new(None::<TokenUsage>));
    let caller: Arc<dyn ModelCaller> = Arc::new(LiveModelCaller {
        inner: HttpModelCaller::new(300).map_err(|e| e.to_string())?,
        live_route: Arc::clone(&live_route),
        last_step_usage: Arc::clone(&last_step_usage),
    });
    let raw_caller: Arc<dyn ModelCaller> =
        Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
    let llm_client = LlmClient::new(120).ok();

    let network_allowed = !cfg
        .network_policy
        .as_deref()
        .unwrap_or("allow")
        .eq_ignore_ascii_case("deny");

    // Never implicitly trust repository `.nola/settings.json` or `.nola/hooks.json` in TUI!
    let workspace_trusted = false;
    let permission_profile = launch
        .permission_profile
        .or_else(|| {
            std::env::var("NOLA_PERMISSION_PROFILE")
                .ok()
                .and_then(|s| PermissionProfile::parse(&s))
        })
        .or_else(|| launch.yes.then_some(PermissionProfile::AllowAll))
        .unwrap_or(PermissionProfile::Cautious);
    let base_policy = EffectivePolicy::load(
        workspace,
        suite.user_home.as_deref(),
        workspace_trusted,
        Some(permission_profile),
        network_allowed,
    );

    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);

    // Wire enabled MCP servers into TUI tool registry using the active db's isolated vault.
    if let Ok(servers) = store.list_mcp_servers() {
        let vault = Vault::open(default_vault_path(&launch.db_path))
            .ok()
            .map(Arc::new);
        let mcp = Arc::new(McpManager::new(vault));
        for srv in servers.into_iter().filter(|s| s.enabled) {
            if let Ok(tools) = mcp.discover_tools(&srv) {
                for t in tools {
                    registry.register(t);
                }
            }
        }
    }

    let subagents = Arc::new(SubagentManager::new(store.clone(), Arc::clone(&raw_caller)));
    subagents.configure_parent(route.clone(), base_policy.clone());
    register_subagent_tools(&subagents, &mut registry);

    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let (worker_tx, worker_rx) = mpsc::channel::<WorkerMsg>();
    let pending_questions: Arc<Mutex<Vec<PendingQuestionRecord>>> =
        Arc::new(Mutex::new(Vec::new()));

    let planning = Arc::new(PlanningState::default());
    {
        let tx_q = worker_tx.clone();
        let pq = Arc::clone(&pending_questions);
        planning.set_user_input_callback(Some(Arc::new(move |question, options| {
            let qid = new_id();
            pq.lock().unwrap().push(PendingQuestionRecord {
                id: qid.clone(),
                question: question.to_string(),
                options: options.to_vec(),
            });
            let ans = if interactive {
                let (reply_tx, reply_rx) = mpsc::channel();
                if tx_q
                    .send(WorkerMsg::UserQuestion {
                        id: qid.clone(),
                        question: question.to_string(),
                        options: options.to_vec(),
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    None
                } else {
                    reply_rx.recv().ok().flatten()
                }
            } else {
                prompt_question_line_mode(question, options)
            };
            pq.lock().unwrap().retain(|q| q.id != qid);
            ans
        })));
    }
    register_planning_tools(store.clone(), &planning, &mut registry);

    let registry = Arc::new(registry);
    let engine = Arc::new(AgentEngine::new(
        store.clone(),
        Arc::clone(&registry),
        Arc::clone(&caller),
    ));

    let mut ctx = ToolContext::new(workspace);
    ctx.interactive_user_input = true;
    ctx.network_allowed = network_allowed;
    ctx.permission_profile = permission_profile;
    ctx.sandbox_mode = SandboxMode::from_env();

    let (session_id, first_turn) = if let Some(sid) = launch.session_id.clone() {
        let exists = store.get_session(&sid).ok().flatten().is_some();
        (sid, !(launch.resume || exists))
    } else if launch.resume {
        if let Ok(list) = store.list_sessions(1) {
            if let Some(latest) = list.into_iter().next() {
                (latest.id, false)
            } else {
                (new_id(), true)
            }
        } else {
            (new_id(), true)
        }
    } else {
        (new_id(), true)
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&cancel));
    let session_grants = Arc::new(Mutex::new(BTreeSet::<(String, String)>::new()));
    let locale = TuiLocale::from_env();

    let mut app = TuiApp {
        store,
        suite,
        workspace: workspace.to_path_buf(),
        db_path: launch.db_path,
        cfg,
        live_route,
        last_step_usage,
        raw_caller,
        llm_client,
        registry,
        engine,
        subagents,
        ctx,
        permission_profile,
        base_policy,
        network_allowed,
        workspace_trusted,
        session_id: session_id.clone(),
        first_turn,
        cancel,
        session_grants,
        pending_questions,
        active_inbox: None,
        active_expert: None,
        pending_attachments: Vec::new(),
        verbose_tools: false,
        locale,
        running: false,
        spinner_idx: 0,
        transcript: Vec::new(),
        streaming_text: String::new(),
        streaming_reasoning: String::new(),
        scroll_offset: 0,
        auto_follow: true,
        composer: ComposerState::default(),
        completion_hint: None,
        active_modal: None,
        worker_tx,
        worker_rx,
        last_usage: None,
        should_exit: false,
        pending_editor_prompt: false,
        interactive,
    };

    let welcome = format!(
        "{} | model={} | permission={} | workspace={}",
        locale.t("Nola v2 Interactive TUI", "Nola v2 交互式终端"),
        route.model,
        permission_profile.as_canonical_str(),
        workspace.display()
    );
    app.push_system(welcome);
    if !first_turn {
        app.push_system(format!("Resuming session {session_id}"));
        app.load_session_preview(&session_id);
    } else {
        app.push_system(locale.t(
            "Type /help for 35+ slash commands (/plan /checkpoint /undo /diff /worktree /tasks /goal /sources /mcp /memory /agent /attach /paste).",
            "输入 /help 查看 35+ 斜杠命令（/plan /checkpoint /undo /diff /worktree /tasks /goal /sources /mcp /memory /agent /attach /paste）。",
        ));
    }

    if interactive {
        run_interactive_loop(&mut app)?;
    } else {
        run_line_fallback_loop(&mut app)?;
    }

    app.ctx.pty.close_all();
    Ok(0)
}

fn run_interactive_loop(app: &mut TuiApp) -> Result<(), String> {
    let mut guard = InteractiveTerminalGuard::enter().map_err(|e| e.to_string())?;
    let mut last_tick = Instant::now();
    let mut dirty = true;

    while !app.should_exit {
        if app.pending_editor_prompt {
            app.pending_editor_prompt = false;
            guard.suspend();
            let edit_res = compose_prompt_in_editor();
            guard.resume().map_err(|e| e.to_string())?;
            match edit_res {
                Ok(Some(composed)) => app.start_turn(composed),
                Ok(None) => app.push_system("Editor closed with empty prompt; nothing sent."),
                Err(e) => app.push_error(format!("edit error: {e}")),
            }
            dirty = true;
        }

        if app.drain_worker_messages() {
            dirty = true;
        }

        if app.running && last_tick.elapsed() >= Duration::from_millis(120) {
            app.spinner_idx = app.spinner_idx.wrapping_add(1);
            last_tick = Instant::now();
            dirty = true;
        }

        if dirty {
            app.draw_screen().map_err(|e| e.to_string())?;
            dirty = false;
        }

        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => {
                    app.handle_key_event(key);
                    dirty = true;
                }
                Ok(Event::Mouse(mouse)) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        app.auto_follow = false;
                        app.scroll_offset = app.scroll_offset.saturating_add(3);
                        dirty = true;
                    }
                    MouseEventKind::ScrollDown => {
                        app.scroll_offset = app.scroll_offset.saturating_sub(3);
                        if app.scroll_offset == 0 {
                            app.auto_follow = true;
                        }
                        dirty = true;
                    }
                    _ => {}
                },
                Ok(Event::Resize(_, _)) => {
                    dirty = true;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }

    drop(guard);
    Ok(())
}

fn run_line_fallback_loop(app: &mut TuiApp) -> Result<(), String> {
    let stdin = io::stdin();
    let mut input = String::new();
    while !app.should_exit {
        print!("nola> ");
        let _ = io::stdout().flush();
        input.clear();
        let n = match stdin.read_line(&mut input) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                app.cancel.store(false, Ordering::Relaxed);
                println!();
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        if n == 0 {
            break;
        }
        app.execute_input(&input);
    }
    Ok(())
}

// ============================================================================
// Helper Functions
// ============================================================================

fn print_entry_line_mode(entry: &TranscriptEntry) {
    match entry.kind {
        TranscriptKind::User => {}
        TranscriptKind::Assistant => {
            if !entry.title.is_empty() {
                println!("\n[{}] {}", entry.title, entry.body);
            } else {
                println!("{}", entry.body);
            }
        }
        TranscriptKind::Reasoning => {
            println!("[reasoning] {}", entry.body);
        }
        TranscriptKind::Tool { .. } => {
            eprintln!("[{}] {}", entry.title, entry.body);
        }
        TranscriptKind::System => {
            println!("{}", entry.body);
        }
        TranscriptKind::Error => {
            eprintln!("error: {}", entry.body);
        }
    }
}

fn extract_git_stdout(res: &nola_types::ToolExecutionResult) -> String {
    let val = res.payload.to_model_value();
    if let Some(s) = val.get("stdout").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            return s.to_string();
        }
    }
    if let Some(s) = val.get("stderr").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            return s.to_string();
        }
    }
    if let Some(s) = val.get("message").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    res.payload.to_model_string()
}

fn apply_tui_model_override(cfg: &AppliedConfig, route: &mut ModelRouteConfig, arg: &str) {
    for conn in &cfg.llm.connections {
        for m in &conn.models {
            let full_id = format!("{}:{}", conn.id, m.id);
            if m.id == arg || full_id == arg {
                let mut next_cfg = cfg.clone();
                next_cfg.llm.default_model = Some(DefaultModelSelection {
                    connection: conn.id.clone(),
                    model: m.id.clone(),
                });
                if let Ok(resolved) = resolve_model_route(&next_cfg) {
                    *route = resolved;
                    return;
                }
            }
        }
    }
    route.model = arg.to_string();
}

struct TempFileCleanupGuard(PathBuf);

impl Drop for TempFileCleanupGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn compose_prompt_in_editor() -> Result<Option<String>, String> {
    let editor_cmd = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            ["vi", "nano"]
                .iter()
                .find(|bin| find_binary_on_path(bin).is_some())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| {
            "no editor found: set $VISUAL or $EDITOR (or install vi/nano)".to_string()
        })?;

    let mut parts = editor_cmd.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| "empty $VISUAL/$EDITOR command".to_string())?;
    let extra_args: Vec<&str> = parts.collect();

    let tmp_path = std::env::temp_dir().join(format!("nola-prompt-{}.txt", new_id()));
    let mut open_opts = fs::OpenOptions::new();
    open_opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open_opts.mode(0o600);
    }
    let file = open_opts.open(&tmp_path).map_err(|e| {
        format!(
            "failed to create temp prompt file '{}': {e}",
            tmp_path.display()
        )
    })?;
    let _cleanup_guard = TempFileCleanupGuard(tmp_path.clone());
    drop(file);

    let status = Command::new(program)
        .args(&extra_args)
        .arg(&tmp_path)
        .status()
        .map_err(|e| format!("failed to launch editor '{program}': {e}"))?;

    let content = fs::read_to_string(&tmp_path).unwrap_or_default();

    if !status.success() {
        return Err(format!("editor '{program}' exited with status {status}"));
    }

    let trimmed = content.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn prompt_question_line_mode(question: &str, options: &[String]) -> Option<String> {
    eprintln!("\n[Question] {question}");
    for (i, opt) in options.iter().enumerate() {
        eprintln!("  {}. {opt}", i + 1);
    }
    eprint!("Answer> ");
    let _ = io::stderr().flush();
    let mut ans = String::new();
    io::stdin().read_line(&mut ans).ok()?;
    let t = ans.trim();
    if t.is_empty() {
        None
    } else if let Ok(num) = t.parse::<usize>() {
        if num >= 1 && num <= options.len() {
            Some(options[num - 1].clone())
        } else {
            Some(t.to_string())
        }
    } else {
        Some(t.to_string())
    }
}

fn prompt_approval_line_mode(
    tool_name: &str,
    target_key: &str,
    summary: &str,
    reason: &str,
) -> ApprovalChoice {
    eprintln!("\n┌─ Permission Approval ─────────────────────────");
    eprintln!("│ Tool:    {tool_name}");
    eprintln!("│ Target:  {target_key}");
    eprintln!("│ Summary: {summary}");
    eprintln!("│ Reason:  {reason}");
    eprint!("└─ [1] Allow once / [2] Allow in session (durable) / [3] Return to input / [n] Deny > ");
    let _ = io::stderr().flush();

    if let Some(_raw_guard) = RawModeGuard::enter() {
        let choice = loop {
            match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                    if k.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(
                            k.code,
                            KeyCode::Char('c')
                                | KeyCode::Char('C')
                                | KeyCode::Char('d')
                                | KeyCode::Char('D')
                        )
                    {
                        break ApprovalChoice::ReturnToInput;
                    }
                    if k.modifiers.is_empty() || k.modifiers == KeyModifiers::SHIFT {
                        match k.code {
                            KeyCode::Char('1') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                                break ApprovalChoice::AllowOnce;
                            }
                            KeyCode::Char('2') | KeyCode::Char('a') | KeyCode::Char('A') => {
                                break ApprovalChoice::AllowForTargetInSession;
                            }
                            KeyCode::Char('3')
                            | KeyCode::Char('r')
                            | KeyCode::Char('R')
                            | KeyCode::Esc => {
                                break ApprovalChoice::ReturnToInput;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter => {
                                break ApprovalChoice::Deny;
                            }
                            _ => {}
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => break ApprovalChoice::Deny,
            }
        };
        drop(_raw_guard);
        eprintln!();
        return choice;
    }

    let mut ans = String::new();
    if io::stdin().read_line(&mut ans).is_err() {
        return ApprovalChoice::Deny;
    }
    match ans.trim().to_ascii_lowercase().as_str() {
        "1" | "y" | "yes" => ApprovalChoice::AllowOnce,
        "2" | "a" | "always" => ApprovalChoice::AllowForTargetInSession,
        "3" | "r" | "return" => ApprovalChoice::ReturnToInput,
        _ => ApprovalChoice::Deny,
    }
}

fn extract_resource_pattern(tool_name: &str, args: &Value) -> Option<String> {
    if matches!(tool_name, "bash" | "pty_exec") {
        return args
            .get("command")
            .and_then(|v| v.as_str())
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty());
    }
    for key in ["path", "file_path", "target_path", "destination", "url"] {
        if let Some(val) = args
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(val.to_string());
        }
    }
    None
}

fn extract_target_key(tool_name: &str, args: &Value) -> String {
    if matches!(tool_name, "bash" | "pty_exec") {
        if let Some(cmd) = args
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let normalized: String = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
            let hash = sha256_hex(normalized.as_bytes());
            let short_hash = &hash[..16];
            let redacted = PrivacyRedactor::redact_text(&normalized);
            let preview: String = redacted.chars().take(80).collect();
            return format!("{preview} [sha256:{short_hash}]");
        }
    }
    for key in [
        "path",
        "file_path",
        "target_path",
        "destination",
        "url",
        "endpoint",
        "server_id",
        "repo",
    ] {
        if let Some(val) = args
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return PrivacyRedactor::redact_text(val)
                .chars()
                .take(120)
                .collect();
        }
    }
    if let Some(cmd) = args
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let normalized: String = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
        let hash = sha256_hex(normalized.as_bytes());
        let short_hash = &hash[..16];
        let redacted = PrivacyRedactor::redact_text(&normalized);
        let preview: String = redacted.chars().take(80).collect();
        return format!("{preview} [sha256:{short_hash}]");
    }
    tool_name.to_string()
}

fn format_permission_summary(tool_name: &str, target_key: &str, args: &Value) -> String {
    let raw = match tool_name {
        "bash" | "pty_exec" => format!("Run shell command: {target_key}"),
        "write_file" | "edit_file" | "apply_patch" | "delete_file" | "move_file" => {
            format!("Modify workspace file: {target_key}")
        }
        "web_fetch" | "web_search" => format!("Network request to: {target_key}"),
        _ => {
            let preview: String = serde_json::to_string(args)
                .unwrap_or_default()
                .chars()
                .take(140)
                .collect();
            format!("{tool_name} on {target_key} ({preview})")
        }
    };
    PrivacyRedactor::redact_text(&raw)
}

fn format_context_usage_line(route: &ModelRouteConfig, latest_usage: &TokenUsage) -> String {
    let excl_cache = route.protocol == ModelProtocol::AnthropicMessages;
    let used_tokens = if excl_cache {
        latest_usage
            .input_tokens
            .saturating_add(latest_usage.cache_read_tokens)
            .saturating_add(latest_usage.cache_write_tokens)
    } else {
        latest_usage.input_tokens
    };
    let window = route.context_window.max(1) as u64;
    let pct = ((used_tokens as f64 / window as f64) * 100.0).min(100.0);
    let used_k = used_tokens as f64 / 1000.0;
    let win_k = window as f64 / 1000.0;
    let cache_bps = latest_usage
        .cache_hit_rate_bps_normalized(excl_cache)
        .unwrap_or_else(|| latest_usage.cache_hit_rate_bps());
    let cache_pct = cache_bps as f64 / 100.0;
    format!("Context {pct:.0}% · {used_k:.1}k/{win_k:.0}k · cache hit {cache_pct:.0}%")
}

fn print_context_usage_footer(route: &ModelRouteConfig, latest_usage: &TokenUsage) {
    let line = format_context_usage_line(route, latest_usage);
    let mut out = io::stdout().lock();
    let _ = out.queue(SetAttribute(Attribute::Dim));
    let _ = write!(out, "{line}");
    let _ = out.queue(SetAttribute(Attribute::NormalIntensity));
    let _ = out.flush();
}

fn format_redacted_tui_config(
    cfg: &AppliedConfig,
    route: &ModelRouteConfig,
    profile: PermissionProfile,
    workspace: &Path,
    db_path: &Path,
) -> String {
    let connections: Vec<Value> = cfg
        .llm
        .connections
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "endpoint": c.endpoint,
                "protocol": c.protocol,
                "has_api_key": connection_has_api_key(c),
                "models": c.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({
        "workspace": workspace.display().to_string(),
        "db_path": db_path.display().to_string(),
        "permission_profile": profile.as_canonical_str(),
        "active_route": {
            "connection_id": route.connection_id,
            "model": route.model,
            "protocol": route.protocol.as_str(),
            "endpoint": route.endpoint,
            "context_window": route.context_window,
            "reasoning_effort": route.reasoning_effort,
            "has_api_key": !route.api_key.is_empty()
        },
        "network_policy": cfg.network_policy,
        "max_iterations": cfg.max_iterations,
        "connections": connections
    }))
    .unwrap_or_else(|_| "{}".into())
}

fn format_tui_help(locale: TuiLocale) -> String {
    match locale {
        TuiLocale::ZhHans => "\
斜杠命令列表（支持 Tab 自动补全与运行中实时切换）：
  会话与编辑：
    /new | /resume <id> | /sessions [n]    新建/恢复/列出会话
    /compact | /clear | /history [n]       压缩上下文 / 清屏 / 查看历史
    /continue [msg] | /export <path>       继续对话 / 导出 Markdown
    /edit | /attach <path|list|clear>      外部编辑器 / 附加图片或文档（也支持 @<路径>）
    /paste | /status | /config | /verbose  粘贴剪贴板 / 状态总览 / 配置 / 思考详情开关
  模型与实时控制（支持轮次运行中直接执行）：
    /model [id]                            列出或实时切换模型
    /thinking [none|low|medium|high|auto]  查看或实时切换推理深度
    /guide <text>                          向运行中的轮次实时注入引导指令
    /allow [cautious|accept_edits|allow_all] 切换权限模式
    /plan [on|off|show|approve]            计划模式开关、查看或批准计划
  代码检查点与 Git：
    /checkpoint [label] | /checkpoints     创建/列出持久化工作区检查点
    /undo | /redo                          撤销/重做工作区检查点修改
    /diff [--staged]                       查看 git status 与 diff --stat
    /worktree [list|create|remove|integrate] 管理与集成 Git worktree
  多智能体、目标与知识库：
    /tasks | /jobs | /stop [id]            查看或停止子智能体/后台任务
    /goal [list|create|run|pause|cancel|status] 自主目标全生命周期管理
    /sources [list|add|index|search]       RAG 数据源管理与检索
    /mcp [list|tools|auth <id>]            查看 MCP 服务/工具并触发 OAuth
    /memory [list|search|save]             查看/检索/保存 Memory v2 记录
    /agent [name|clear]                    选择专家角色（支持 @专家名 补全）
    /room <id> [@name] <msg> | /rooms      多智能体协作房间
    /skills | /tools [filter] | /questions 查看技能 / 工具 / 待答问题
    /quit | /exit | //<text>               退出 TUI / 发送以 '/' 开头的原文"
            .to_string(),
        TuiLocale::En => "\
Slash commands (Tab-completable; live commands work mid-run):
  Session & Composer:
    /new | /resume <id> | /sessions [n]    Start, resume, or list sessions
    /compact | /clear | /history [n]       Compact context / clear screen / event history
    /continue [msg] | /export <path>       Continue turn / export transcript to Markdown
    /edit | /attach <path|list|clear>      External $EDITOR / attach image or doc (or @<path>)
    /paste | /status | /config | /verbose  Paste clipboard / status / config / toggle reasoning
  Model & Live Controls (available while streaming):
    /model [id]                            List or live-switch active model
    /thinking [none|low|medium|high|auto]  View or live-switch reasoning effort
    /guide <text>                          Push steering guidance into active turn inbox
    /allow [cautious|accept_edits|allow_all] Switch strict permission profile
    /plan [on|off|show|approve]            Toggle Plan Mode, view plan, or approve plan
  Checkpoints & Git:
    /checkpoint [label] | /checkpoints     Create or list persistent workspace checkpoints
    /undo | /redo                          Undo / redo workspace checkpoint changes
    /diff [--staged]                       Show git status & git diff --stat
    /worktree [list|create|remove|integrate] Manage & integrate Git worktrees
  Agents, Goals, RAG & Integrations:
    /tasks | /jobs | /stop [id]            Inspect or stop subagents & background jobs
    /goal [list|create|run|pause|cancel|status] Autonomous goal control
    /sources [list|add|index|search]       Manage & search RAG data sources
    /mcp [list|tools|auth <id>]            Inspect MCP servers/tools & trigger OAuth 2.1
    /memory [list|search|save]             Inspect, search, or save Memory v2 records
    /agent [name|clear]                    Select active expert persona (@name completion)
    /room <id> [@name] <msg> | /rooms      Interact with AgentRooms
    /skills | /tools [filter] | /questions Inspect skills, tools, or pending user questions
    /quit | /exit | //<text>               Exit TUI / send a literal leading '/'"
            .to_string(),
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests;
