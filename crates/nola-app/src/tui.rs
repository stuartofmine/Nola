// SPDX-License-Identifier: Apache-2.0
//! Full interactive crossterm terminal UI for Nola v2 with:
//! - Fullscreen alternate-screen scrollable viewport (with automatic non-TTY line fallback)
//! - Non-blocking background `AgentEngine` worker + live mid-run `/model`, `/thinking`,
//!   `/guide` inbox steering, and instant `Esc`/`Ctrl+C` interruptible cancellation
//! - Clean ANSI Markdown rendering, toggleable dimmed reasoning (`/verbose`),
//!   interactive Approval (with durable session grants) & `ask_user` modals
//! - File/image attachments (`/attach`, `@<path>`, `/paste`), Tab completion,
//!   command history, EN/ZH-Hans i18n, and full 35+ slash command parity.

use std::collections::{BTreeSet, HashMap};
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
use crossterm::style::{Attribute, SetAttribute};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::QueueableCommand;
use nola_engine::{
    approve_agent_plan, compact_session_now, is_plan_mode, load_session_plan, load_session_todos,
    register_planning_tools, register_subagent_tools, set_plan_mode,
    AgentEngine, AgentEvent, AgentRunConfig, AgentRunResult,
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
    new_id, now_ms, sha256_hex, AgentRoom, ExpertPeer, FrozenSystemBlock,
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

const SLASH_COMMAND_DESCS: &[(&str, &str, &str)] = &[
    ("/help", "Show all slash commands", "查看所有斜杠命令"),
    ("/model", "Open interactive model picker or live-switch model", "打开交互面板选择或切换模型"),
    ("/thinking", "Open reasoning effort picker (auto/none/low/medium/high)", "选择推理思考深度"),
    ("/allow", "Switch permission mode (cautious / allow_all)", "切换权限模式（谨慎 / 全部允许）"),
    ("/new", "Start a fresh conversation", "开启新会话"),
    ("/resume", "Resume a previous session", "恢复历史会话"),
    ("/sessions", "List recent sessions", "列出最近会话"),
    ("/compact", "Compact model context while preserving full history", "压缩模型上下文，同时保留完整历史"),
    ("/config", "Inspect active configuration and model sources", "查看当前配置与模型来源"),
    ("/continue", "Continue the current session", "继续当前会话"),
    ("/verbose", "Toggle full reasoning & detailed tool arguments", "开关完整思考过程与工具详情"),
    ("/plan", "Toggle, inspect, or approve Plan Mode", "开关、查看或批准计划模式"),
    ("/diff", "Show git status and diff summary", "查看 Git 变更状态与差异"),
    ("/checkpoint", "Create a persistent workspace checkpoint", "创建持久化工作区检查点"),
    ("/checkpoints", "List saved workspace checkpoints", "列出工作区检查点"),
    ("/undo", "Undo last workspace checkpoint changes", "撤销上次检查点修改"),
    ("/redo", "Redo undone workspace checkpoint changes", "重做已撤销的检查点修改"),
    ("/worktree", "Manage and integrate Git worktrees", "管理与集成 Git worktree"),
    ("/tasks", "Inspect active subagent tasks", "查看运行中的子智能体任务"),
    ("/jobs", "Inspect background shell jobs", "查看后台任务"),
    ("/stop", "Stop active turn or background job", "停止当前任务或后台作业"),
    ("/guide", "Inject live steering guidance mid-run", "运行中实时注入引导指令"),
    ("/goal", "Manage autonomous goals", "管理自主目标"),
    ("/goals", "List autonomous goals", "列出自主目标"),
    ("/sources", "Manage and search RAG data sources", "管理与检索 RAG 数据源"),
    ("/mcp", "Inspect MCP servers, tools, and OAuth", "查看 MCP 服务、工具与 OAuth"),
    ("/memory", "List, search, or save persistent memory", "查看、检索或保存持久化记忆"),
    ("/agent", "Select or clear active expert persona", "切换或清除专家角色"),
    ("/room", "Send a message in an AgentRoom", "在多智能体协作房间发消息"),
    ("/rooms", "List AgentRooms", "列出协作房间"),
    ("/skills", "List available skills", "查看可用技能列表"),
    ("/tools", "Inspect registered tools", "查看已注册工具"),
    ("/questions", "Inspect pending user questions", "查看待回答提问"),
    ("/attach", "Attach local file or image", "附加本地文件或图片"),
    ("/paste", "Attach image or text from clipboard", "粘贴剪贴板图片或文本"),
    ("/edit", "Compose prompt in external $EDITOR", "调用外部编辑器输入提示词"),
    ("/export", "Export conversation to Markdown", "导出当前会话为 Markdown"),
    ("/history", "Inspect raw session event history", "查看会话底层事件历史"),
    ("/status", "Show session, model, permission, and usage overview", "查看会话、模型、权限与用量总览"),
    ("/clear", "Clear current screen transcript", "清空当前屏幕显示"),
    ("/exit", "Exit interactive CLI", "退出交互式终端"),
    ("/quit", "Exit interactive CLI", "退出交互式终端"),
];

fn slash_command_description(cmd: &str, locale: TuiLocale) -> &'static str {
    SLASH_COMMAND_DESCS
        .iter()
        .find(|(c, _, _)| *c == cmd)
        .map(|(_, en, zh)| locale.t(en, zh))
        .unwrap_or("")
}

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
            "/allow" => ["cautious", "allow_all"]
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

#[derive(Debug, Default, Clone)]
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

    fn delete_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.buffer[..self.cursor];
        let trimmed_end = before.trim_end_matches(char::is_whitespace).len();
        let new_cursor = if trimmed_end == 0 {
            0
        } else {
            before[..trimmed_end]
                .rfind(char::is_whitespace)
                .map(|idx| idx + before[idx..].chars().next().map_or(1, char::len_utf8))
                .unwrap_or(0)
        };
        self.buffer.drain(new_cursor..self.cursor);
        self.cursor = new_cursor;
        self.history_idx = None;
    }

    fn kill_to_end(&mut self) {
        if self.cursor < self.buffer.len() {
            self.buffer.truncate(self.cursor);
            self.history_idx = None;
        }
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
    // Zero-width combining marks, variation selectors, ZWJ/ZWNJ, BOM
    if (0x0300..=0x036F).contains(&cp)
        || (0x1AB0..=0x1AFF).contains(&cp)
        || (0x1DC0..=0x1DFF).contains(&cp)
        || (0x200B..=0x200F).contains(&cp)
        || cp == 0x2060
        || (0x20D0..=0x20FF).contains(&cp)
        || (0xFE00..=0xFE0F).contains(&cp)
        || (0xFE20..=0xFE2F).contains(&cp)
        || cp == 0xFEFF
        || (0xE0100..=0xE01EF).contains(&cp)
    {
        return 0;
    }
    // Wide CJK, fullwidth, and common wide emoji ranges
    if (0x1100..=0x115F).contains(&cp)
        || (0x231A..=0x231B).contains(&cp)
        || (0x23E9..=0x23F3).contains(&cp)
        || (0x23F8..=0x23FA).contains(&cp)
        || (0x2600..=0x26FF).contains(&cp)
        || (0x2700..=0x27BF).contains(&cp)
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

/// ANSI-aware truncation to exact terminal display columns.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0usize;
    let mut has_sgr = false;
    let mut chars = s.chars().peekable();
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
                has_sgr = false;
            } else if seq.ends_with('m') {
                has_sgr = true;
            }
            out.push_str(&seq);
            continue;
        }
        let cw = char_display_width(c);
        if w + cw > max_width {
            break;
        }
        out.push(c);
        w += cw;
    }
    if has_sgr && !out.ends_with("\x1b[0m") {
        out.push_str("\x1b[0m");
    }
    out
}

/// Render a full 4-sided rounded box (`╭─...─╮`, `│ ... │`, `╰─...─╯`) with exact column alignment.
fn render_rounded_box(
    border_sgr: &str,
    title: &str,
    body_lines: &[String],
    footer_hint: Option<&str>,
    box_width: usize,
) -> Vec<String> {
    let total_w = box_width.max(20);
    let inner_w = total_w.saturating_sub(4); // "│ " + content + " │"
    let mut out = Vec::with_capacity(body_lines.len() + 2);

    // Top border: ╭─ <title> ──────╮
    let top = if title.is_empty() {
        format!("{border_sgr}╭{}╮\x1b[0m", "─".repeat(total_w - 2))
    } else {
        let max_title_w = total_w.saturating_sub(6);
        let clipped_title = truncate_to_width(title, max_title_w);
        let tw = str_display_width(&clipped_title);
        let fill = total_w.saturating_sub(5 + tw); // "╭─ " (3) + title + " " (1) + "╮" (1)
        format!(
            "{border_sgr}╭─ \x1b[1m{clipped_title}\x1b[0m{border_sgr} {}╮\x1b[0m",
            "─".repeat(fill)
        )
    };
    out.push(top);

    // Body rows: │ <content padded to inner_w> │
    for row in body_lines {
        let clipped = truncate_to_width(row, inner_w);
        let vw = str_display_width(&clipped);
        let pad = inner_w.saturating_sub(vw);
        out.push(format!(
            "{border_sgr}│\x1b[0m {clipped}{} {border_sgr}│\x1b[0m",
            " ".repeat(pad)
        ));
    }

    // Bottom border: ╰─ <footer_hint> ──────╯
    let bottom = match footer_hint.filter(|h| !h.is_empty()) {
        Some(hint) => {
            let max_hint_w = total_w.saturating_sub(6);
            let clipped_hint = truncate_to_width(hint, max_hint_w);
            let hw = str_display_width(&clipped_hint);
            let fill = total_w.saturating_sub(5 + hw);
            format!(
                "{border_sgr}╰─ \x1b[2m{clipped_hint}\x1b[0m{border_sgr} {}╯\x1b[0m",
                "─".repeat(fill)
            )
        }
        None => format!("{border_sgr}╰{}╯\x1b[0m", "─".repeat(total_w - 2)),
    };
    out.push(bottom);
    out
}

/// Render a user prompt block with a subtle full-width background tint and left accent bar (no "You" label).
fn render_user_tinted_lines(body: &str, width: usize) -> Vec<String> {
    let total_w = width.max(20);
    let text_w = total_w.saturating_sub(4).max(12); // "▎ " (2) + text + " " (1) + right margin (1)
    let mut wrapped: Vec<String> = Vec::new();
    for raw_line in body.lines() {
        if raw_line.is_empty() {
            wrapped.push(String::new());
        } else {
            wrapped.extend(wrap_ansi_line(raw_line, text_w));
        }
    }
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }
    let bg = "\x1b[48;5;236m";
    let accent = "\x1b[38;5;75m";
    let fg = "\x1b[38;5;255m";
    let mut out = Vec::with_capacity(wrapped.len());
    for line in wrapped {
        let clipped = truncate_to_width(&line, text_w);
        let vw = str_display_width(&clipped);
        let pad = total_w.saturating_sub(2 + vw);
        out.push(format!(
            "{bg}{accent}▎ {fg}{clipped}{}\x1b[0m",
            " ".repeat(pad)
        ));
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

/// ANSI-aware truncation to exact terminal display columns, with trailing `…` if clipped.
fn truncate_with_ellipsis(s: &str, max_width: usize) -> String {
    if str_display_width(s) <= max_width {
        return s.to_string();
    }
    if max_width <= 1 {
        return truncate_to_width(s, max_width);
    }
    let clipped = truncate_to_width(s, max_width - 1);
    format!("{clipped}…")
}

/// Abbreviate a filesystem path by collapsing `$HOME` to `~` and middle-truncating with `…`.
fn abbreviate_path_with_ellipsis(path: &Path, max_width: usize) -> String {
    let raw = path.display().to_string();
    let display = if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() && raw.starts_with(&home) {
            let rest = &raw[home.len()..];
            if rest.is_empty() || rest.starts_with('/') {
                format!("~{rest}")
            } else {
                raw
            }
        } else {
            raw
        }
    } else {
        raw
    };
    if str_display_width(&display) <= max_width || max_width < 8 {
        return truncate_with_ellipsis(&display, max_width);
    }
    let keep_tail = (max_width / 2).max(4);
    let keep_head = max_width.saturating_sub(keep_tail + 1).max(3);
    let head = truncate_to_width(&display, keep_head);
    // Collect trailing characters fitting within `keep_tail` columns
    let mut tail_chars = Vec::new();
    let mut tw = 0usize;
    for ch in display.chars().rev() {
        let cw = char_display_width(ch);
        if tw + cw > keep_tail {
            break;
        }
        tail_chars.push(ch);
        tw += cw;
    }
    tail_chars.reverse();
    let tail: String = tail_chars.into_iter().collect();
    format!("{head}…{tail}")
}

fn render_inline_markdown_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 24);
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;
    let mut in_bold = false;
    let mut in_italic = false;
    let mut in_code = false;

    while i < chars.len() {
        if chars[i] == '`' {
            in_code = !in_code;
            if in_code {
                out.push_str("\x1b[48;5;237;38;5;222m ");
            } else {
                out.push_str(" \x1b[0m");
                if in_bold {
                    out.push_str("\x1b[1m");
                }
                if in_italic {
                    out.push_str("\x1b[3m");
                }
            }
            i += 1;
            continue;
        }
        if !in_code && chars[i] == '[' {
            // Markdown link: [label](url)
            if let Some(close_bracket) = chars[i + 1..].iter().position(|&c| c == ']') {
                let cb_idx = i + 1 + close_bracket;
                if cb_idx + 1 < chars.len() && chars[cb_idx + 1] == '(' {
                    if let Some(close_paren) = chars[cb_idx + 2..].iter().position(|&c| c == ')') {
                        let cp_idx = cb_idx + 2 + close_paren;
                        let label: String = chars[i + 1..cb_idx].iter().collect();
                        let url: String = chars[cb_idx + 2..cp_idx].iter().collect();
                        if !label.is_empty() && !url.is_empty() && !url.contains(char::is_whitespace)
                        {
                            out.push_str(&format!("\x1b[4;36m{label}\x1b[0m \x1b[2m({url})\x1b[0m"));
                            if in_bold {
                                out.push_str("\x1b[1m");
                            }
                            if in_italic {
                                out.push_str("\x1b[3m");
                            }
                            i = cp_idx + 1;
                            continue;
                        }
                    }
                }
            }
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
        if !in_code && chars[i] == '*' {
            let prev_is_space = i == 0 || chars[i - 1].is_whitespace();
            let next_is_space = i + 1 >= chars.len() || chars[i + 1].is_whitespace();
            if (!in_italic && !next_is_space) || (in_italic && !prev_is_space) {
                in_italic = !in_italic;
                if in_italic {
                    out.push_str("\x1b[3m");
                } else {
                    out.push_str("\x1b[23m");
                }
                i += 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    if in_bold || in_italic || in_code {
        if in_code {
            out.push(' ');
        }
        out.push_str("\x1b[0m");
    }
    out
}

fn is_markdown_hr(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
}

fn split_table_cells(line: &str) -> Option<Vec<String>> {
    let t = line.trim();
    if !t.contains('|') {
        return None;
    }
    let stripped = t.strip_prefix('|').unwrap_or(t);
    let stripped = stripped.strip_suffix('|').unwrap_or(stripped);
    let cells: Vec<String> = stripped.split('|').map(|c| c.trim().to_string()).collect();
    if cells.len() >= 2 {
        Some(cells)
    } else {
        None
    }
}

fn is_table_separator_row(line: &str) -> bool {
    let Some(cells) = split_table_cells(line) else {
        return false;
    };
    cells.iter().all(|cell| {
        let c = cell.trim();
        !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')) && c.contains('-')
    })
}

fn render_markdown_table_ansi(rows: &[Vec<String>], max_width: usize) -> Vec<String> {
    let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if col_count == 0 {
        return Vec::new();
    }
    let border = "\x1b[38;5;240m";
    let overhead = 3 * col_count + 1;
    let avail = max_width.saturating_sub(overhead).max(col_count * 3);
    let mut col_widths = vec![3usize; col_count];
    for row in rows {
        for (idx, cell) in row.iter().enumerate() {
            let styled = render_inline_markdown_ansi(cell);
            col_widths[idx] = col_widths[idx].max(str_display_width(&styled));
        }
    }
    let sum_w: usize = col_widths.iter().sum();
    if sum_w > avail {
        let mut excess = sum_w - avail;
        while excess > 0 {
            if let Some((max_i, &max_val)) = col_widths.iter().enumerate().max_by_key(|&(_, w)| w) {
                if max_val <= 4 {
                    break;
                }
                col_widths[max_i] -= 1;
                excess -= 1;
            } else {
                break;
            }
        }
    }

    let make_rule = |left: &str, mid: &str, right: &str| -> String {
        let segs: Vec<String> = col_widths.iter().map(|&w| "─".repeat(w + 2)).collect();
        format!("{border}{left}{}{right}\x1b[0m", segs.join(mid))
    };

    let mut out = Vec::with_capacity(rows.len() + 3);
    out.push(make_rule("┌", "┬", "┐"));
    for (r_idx, row) in rows.iter().enumerate() {
        let mut line = format!("{border}│\x1b[0m");
        for (c_idx, &cw) in col_widths.iter().enumerate() {
            let raw_cell = row.get(c_idx).map(|s| s.as_str()).unwrap_or("");
            let styled = render_inline_markdown_ansi(raw_cell);
            let clipped = truncate_with_ellipsis(&styled, cw);
            let vw = str_display_width(&clipped);
            let pad = cw.saturating_sub(vw);
            if r_idx == 0 {
                line.push_str(&format!(" \x1b[1;38;5;255m{clipped}\x1b[0m{} {border}│\x1b[0m", " ".repeat(pad)));
            } else {
                line.push_str(&format!(" {clipped}{} {border}│\x1b[0m", " ".repeat(pad)));
            }
        }
        out.push(line);
        if r_idx == 0 && rows.len() > 1 {
            out.push(make_rule("├", "┼", "┤"));
        }
    }
    out.push(make_rule("└", "┴", "┘"));
    out
}

fn parse_ordered_list_prefix(trimmed: &str) -> Option<(&str, &str)> {
    let dot_idx = trimmed.find(". ")?;
    if dot_idx == 0 || dot_idx > 4 {
        return None;
    }
    let num = &trimmed[..dot_idx];
    if num.chars().all(|c| c.is_ascii_digit()) {
        Some((num, &trimmed[dot_idx + 2..]))
    } else {
        None
    }
}

fn render_markdown_ansi(markdown: &str, width: usize) -> Vec<String> {
    let safe_w = width.max(20);
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_lines: Vec<String> = Vec::new();
    let raw_lines: Vec<&str> = markdown.lines().collect();
    let mut idx = 0usize;

    while idx < raw_lines.len() {
        let raw_line = raw_lines[idx];
        let trimmed = raw_line.trim_start();
        if let Some(fence_rest) = trimmed.strip_prefix("```") {
            if !in_code_block {
                in_code_block = true;
                let lang = fence_rest.trim();
                code_lang = if lang.is_empty() {
                    "code".to_string()
                } else {
                    lang.to_string()
                };
                code_lines.clear();
            } else {
                in_code_block = false;
                let inner_w = safe_w.saturating_sub(4).max(16);
                let mut wrapped_code = Vec::new();
                for cl in &code_lines {
                    for wline in wrap_ansi_line(&format!("\x1b[38;5;252m{cl}\x1b[0m"), inner_w) {
                        wrapped_code.push(wline);
                    }
                }
                if wrapped_code.is_empty() {
                    wrapped_code.push(String::new());
                }
                let header = if code_lines.is_empty() {
                    code_lang.clone()
                } else {
                    format!("{code_lang} · {} lines", code_lines.len())
                };
                lines.extend(render_rounded_box(
                    "\x1b[38;5;240m",
                    &header,
                    &wrapped_code,
                    None,
                    safe_w,
                ));
                code_lines.clear();
            }
            idx += 1;
            continue;
        }

        if in_code_block {
            code_lines.push(raw_line.to_string());
            idx += 1;
            continue;
        }

        // GFM Table block: header row followed by |---|---| separator
        if idx + 1 < raw_lines.len() && is_table_separator_row(raw_lines[idx + 1]) {
            if let Some(header_cells) = split_table_cells(raw_line) {
                let mut table_rows = vec![header_cells];
                idx += 2;
                while idx < raw_lines.len() {
                    if let Some(row_cells) = split_table_cells(raw_lines[idx]) {
                        table_rows.push(row_cells);
                        idx += 1;
                    } else {
                        break;
                    }
                }
                lines.extend(render_markdown_table_ansi(&table_rows, safe_w));
                continue;
            }
        }

        if is_markdown_hr(trimmed) {
            lines.push(format!("\x1b[38;5;238m{}\x1b[0m", "─".repeat(safe_w)));
        } else if let Some(h) = trimmed.strip_prefix("### ") {
            let styled = format!("\x1b[1;36m### {}\x1b[0m", render_inline_markdown_ansi(h));
            lines.extend(wrap_ansi_line(&styled, safe_w));
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            let styled = format!("\x1b[1;34m## {}\x1b[0m", render_inline_markdown_ansi(h));
            lines.extend(wrap_ansi_line(&styled, safe_w));
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            let styled = format!("\x1b[1;35m# {}\x1b[0m", render_inline_markdown_ansi(h));
            lines.extend(wrap_ansi_line(&styled, safe_w));
        } else if let Some(q) = trimmed.strip_prefix("> ") {
            let styled = format!("\x1b[38;5;242m▎\x1b[0m \x1b[2m{}\x1b[0m", render_inline_markdown_ansi(q));
            lines.extend(wrap_ansi_line(&styled, safe_w));
        } else if let Some((num, item)) = parse_ordered_list_prefix(trimmed) {
            let indent_len = raw_line.len().saturating_sub(trimmed.len());
            let indent = &raw_line[..indent_len];
            let styled = format!(
                "{indent}\x1b[36m{num}.\x1b[0m {}",
                render_inline_markdown_ansi(item)
            );
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
        idx += 1;
    }

    if in_code_block {
        let inner_w = safe_w.saturating_sub(4).max(16);
        let mut wrapped_code = Vec::new();
        for cl in &code_lines {
            for wline in wrap_ansi_line(&format!("\x1b[38;5;252m{cl}\x1b[0m"), inner_w) {
                wrapped_code.push(wline);
            }
        }
        if wrapped_code.is_empty() {
            wrapped_code.push(String::new());
        }
        lines.extend(render_rounded_box(
            "\x1b[38;5;240m",
            &code_lang,
            &wrapped_code,
            None,
            safe_w,
        ));
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
    AssistantEntry {
        title: String,
        body: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    Model,
    Thinking,
    Permission,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PickerItem {
    value: String,
    label: String,
    detail: String,
    is_current: bool,
}

#[derive(Debug, Clone)]
struct PickerState {
    kind: PickerKind,
    title: String,
    items: Vec<PickerItem>,
    selected: usize,
    scroll: usize,
    filter: ComposerState,
}

impl PickerState {
    fn new(kind: PickerKind, title: String, items: Vec<PickerItem>) -> Self {
        let initial_selected = items.iter().position(|it| it.is_current).unwrap_or(0);
        Self {
            kind,
            title,
            items,
            selected: initial_selected,
            scroll: 0,
            filter: ComposerState::default(),
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        let q = self.filter.buffer.trim().to_ascii_lowercase();
        if q.is_empty() {
            return (0..self.items.len()).collect();
        }
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.label.to_ascii_lowercase().contains(&q)
                    || item.value.to_ascii_lowercase().contains(&q)
                    || item.detail.to_ascii_lowercase().contains(&q)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    fn clamp_selection(&mut self, max_visible: usize) {
        let count = self.filtered_indices().len();
        if count == 0 {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        if self.selected >= count {
            self.selected = count - 1;
        }
        let window = max_visible.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + window {
            self.scroll = self.selected + 1 - window;
        }
        let max_scroll = count.saturating_sub(window);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
    }

    fn move_up(&mut self, step: usize) {
        let count = self.filtered_indices().len();
        if count == 0 {
            return;
        }
        if step == 1 && self.selected == 0 {
            self.selected = count - 1;
        } else {
            self.selected = self.selected.saturating_sub(step);
        }
        self.clamp_selection(10);
    }

    fn move_down(&mut self, step: usize) {
        let count = self.filtered_indices().len();
        if count == 0 {
            return;
        }
        if step == 1 && self.selected + 1 >= count {
            self.selected = 0;
        } else {
            self.selected = (self.selected + step).min(count - 1);
        }
        self.clamp_selection(10);
    }

    fn selected_item(&self) -> Option<&PickerItem> {
        let filtered = self.filtered_indices();
        let idx = *filtered.get(self.selected)?;
        self.items.get(idx)
    }
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
    Picker(PickerState),
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
    git_root: Option<PathBuf>,
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
    turn_started_at: Option<Instant>,
    active_tool: Option<(String, String, Instant)>,
    tool_call_previews: HashMap<String, String>,
    spinner_idx: usize,
    transcript: Vec<TranscriptEntry>,
    streaming_text: String,
    streaming_reasoning: String,
    scroll_offset: usize,
    auto_follow: bool,
    composer: ComposerState,
    slash_menu_selected: usize,
    slash_menu_dismissed: bool,
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

    fn read_tail_events(&self, sid: &str, limit: usize) -> Result<Vec<nola_store::StoredEvent>, nola_store::StoreError> {
        let mut tail = std::collections::VecDeque::with_capacity(limit.max(1));
        let mut next_seq = 1i64;
        loop {
            let batch = self.store.read_event_range(sid, next_seq, 500)?;
            if batch.is_empty() {
                break;
            }
            let count = batch.len();
            let last_seq = batch.last().map(|e| e.seq).unwrap_or(next_seq);
            for ev in batch {
                if tail.len() >= limit.max(1) {
                    tail.pop_front();
                }
                tail.push_back(ev);
            }
            if count < 500 || last_seq < next_seq {
                break;
            }
            next_seq = last_seq.saturating_add(1);
        }
        Ok(tail.into_iter().collect())
    }

    fn load_session_preview(&mut self, sid: &str) {
        if let Ok(events) = self.read_tail_events(sid, 200) {
            let mut shown = 0usize;
            let messages: Vec<_> = events.iter().filter(|ev|
                matches!(ev.kind.as_str(), "turn_completed" | "chat_user" | "user_message")
            ).collect();
            for ev in messages.iter().rev().take(20).rev() {
                let (kind, text) = if ev.kind == "turn_completed" {
                    (TranscriptKind::Assistant, ev.payload["final_text"].as_str())
                } else {
                    (TranscriptKind::User, ev.payload["content"].as_str().or_else(|| ev.payload["text"].as_str()))
                };
                if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
                    self.push_entry(kind, format!("history #{}", ev.seq), text.chars().take(200).collect::<String>());
                    shown += 1;
                }
            }
            if shown > 0 {
                self.push_system(format!(
                    "{} ({shown})",
                    self.locale
                        .t("Loaded recent messages", "已加载最近消息")
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

        if let Err(err) = self.store.append_event(
            &self.session_id, "chat_user", json!({"content": raw_prompt}), nola_store::DEFAULT_MAX_EVENTS_PER_SCOPE,
        ) {
            self.push_error(err.to_string());
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
        self.turn_started_at = Some(Instant::now());
        self.active_tool = None;
        self.tool_call_previews.clear();

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
            if matches!(spec.schema.name.as_str(), "bash" | "pty_exec")
                && args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .is_some_and(is_readonly_safe_shell_command)
            {
                return true;
            }
            let target_key = extract_target_key(&spec.schema.name, args);
            let grant_prefix = extract_session_grant_key(&spec.schema.name, args);
            let exact_key = (spec.schema.name.clone(), target_key.clone());
            let prefix_key = (spec.schema.name.clone(), grant_prefix.clone());
            {
                let guard = grants.lock().unwrap();
                if guard.contains(&exact_key) || guard.contains(&prefix_key) {
                    return true;
                }
            }
            let display_target = format_approval_target(&spec.schema.name, args);
            let summary = format_permission_summary(&spec.schema.name, &display_target, args);

            let choice = if interactive_mode {
                let (reply_tx, reply_rx) = mpsc::channel();
                if tx_for_approval
                    .send(WorkerMsg::ApprovalRequest {
                        tool_name: spec.schema.name.clone(),
                        target_key: display_target.clone(),
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
                prompt_approval_line_mode(&spec.schema.name, &display_target, &summary, reason)
            };

            match choice {
                ApprovalChoice::AllowOnce => true,
                ApprovalChoice::AllowForTargetInSession => {
                    {
                        let mut guard = grants.lock().unwrap();
                        guard.insert(exact_key);
                        guard.insert(prefix_key);
                    }
                    let _ = nola_engine::policy::persist_session_target_approval(
                        &store_for_grant, &sid_for_grant, &ws_for_grant, &spec.schema.name, args,
                    );
                    true
                }
                ApprovalChoice::ReturnToInput => {
                    cancel_for_prompt.store(true, Ordering::SeqCst);
                    let _ = tx_for_approval.send(WorkerMsg::Notice {
                        is_error: false,
                        text: format!(
                            "Paused before `{}` ({}) — returned to input.",
                            spec.schema.name, display_target
                        ),
                    });
                    false
                }
                ApprovalChoice::Deny => {
                    let _ = tx_for_approval.send(WorkerMsg::Notice {
                        is_error: false,
                        text: format!("Denied `{}` ({}).", spec.schema.name, display_target),
                    });
                    false
                }
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
        self.turn_started_at = None;
        self.active_tool = None;
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
                } else if !res.cancelled {
                    self.push_system(self.locale.t(
                        "Turn ended without a final reply (a tool call was blocked or declined).",
                        "本轮未生成最终回复（工具调用被拦截或已取消）。",
                    ));
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
                let _ = self.store.append_event(
                    &self.session_id,
                    "turn_failed",
                    json!({
                        "type": "turn_failed",
                        "session_id": self.session_id,
                        "error": e,
                    }),
                    now_ms(),
                );
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
                        call_id,
                        name,
                        arguments,
                        ..
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
                        let preview = summarize_tool_args(&name, &arguments);
                        self.tool_call_previews
                            .insert(call_id.clone(), preview.clone());
                        self.active_tool = Some((name.clone(), preview.clone(), Instant::now()));
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
                        call_id,
                        name,
                        is_error,
                        summary,
                        ..
                    } => {
                        self.active_tool = None;
                        let arg_preview = self
                            .tool_call_previews
                            .remove(&call_id)
                            .unwrap_or_default();
                        let body = if arg_preview.is_empty() {
                            summary
                        } else if is_error {
                            format!("{arg_preview} — {summary}")
                        } else {
                            format!("{arg_preview} · {summary}")
                        };
                        self.transcript.push(TranscriptEntry {
                            kind: TranscriptKind::Tool { is_error },
                            title: name,
                            body,
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
                WorkerMsg::AssistantEntry { title, body } => {
                    self.push_entry(TranscriptKind::Assistant, title, body);
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

    fn collect_model_picker_items(&mut self, route: &ModelRouteConfig) -> Vec<PickerItem> {
        // Refresh active config from SQLite so models added via CLI/Web/Desktop appear immediately.
        let fresh = load_active_config(&self.store);
        if !fresh.llm.connections.is_empty() {
            self.cfg = fresh;
        }

        let mut items: Vec<PickerItem> = Vec::new();
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();

        for conn in &self.cfg.llm.connections {
            for m in &conn.models {
                if seen.insert((conn.id.clone(), m.id.clone())) {
                    let is_current = conn.id == route.connection_id && m.id == route.model;
                    let value = format!("{}:{}", conn.id, m.id);
                    let mut detail_parts = vec![conn.endpoint.clone()];
                    if let Some(ctx) = m.context_window {
                        detail_parts.push(format!("ctx={}k", ctx / 1000));
                    }
                    items.push(PickerItem {
                        value: value.clone(),
                        label: value,
                        detail: detail_parts.join(" · "),
                        is_current,
                    });
                }
            }
        }

        // Also include enabled models from canonical `model_connection` records in SQLite
        // (e.g. configured via macOS/iOS/Android/Web Model Center).
        if let Ok(conns) = self.store.list_module_records::<Value>("model_connection") {
            for c in conns {
                let enabled = c
                    .pointer("/connection/enabled")
                    .or_else(|| c.get("enabled"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                if !enabled {
                    continue;
                }
                let conn_id = c
                    .pointer("/connection/id")
                    .or_else(|| c.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let endpoint = c
                    .pointer("/connection/base_url")
                    .or_else(|| c.pointer("/connection/endpoint"))
                    .or_else(|| c.get("base_url"))
                    .or_else(|| c.get("endpoint"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if conn_id.is_empty() {
                    continue;
                }
                if let Some(models) = c.pointer("/catalog/models").and_then(|v| v.as_array()) {
                    for m in models {
                        let mid = m
                            .get("model_id")
                            .or_else(|| m.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if mid.is_empty() || !seen.insert((conn_id.clone(), mid.clone())) {
                            continue;
                        }
                        let is_current = conn_id == route.connection_id && mid == route.model;
                        let value = format!("{conn_id}:{mid}");
                        items.push(PickerItem {
                            value: value.clone(),
                            label: value,
                            detail: endpoint.clone(),
                            is_current,
                        });
                    }
                }
            }
        }

        // Ensure the currently active route is always selectable/visible even if launched with ad-hoc `--model`.
        if !route.model.is_empty()
            && seen.insert((route.connection_id.clone(), route.model.clone()))
        {
            let value = if route.connection_id.is_empty() {
                route.model.clone()
            } else {
                format!("{}:{}", route.connection_id, route.model)
            };
            items.insert(
                0,
                PickerItem {
                    value: value.clone(),
                    label: value,
                    detail: format!(
                        "{} · ctx={}k",
                        route.endpoint,
                        route.context_window / 1000
                    ),
                    is_current: true,
                },
            );
        }

        items
    }

    fn cmd_model(&mut self, arg: &str) {
        let mut route = self.route_snapshot();
        if arg.is_empty() {
            let items = self.collect_model_picker_items(&route);
            if self.interactive && !items.is_empty() {
                let mut picker = PickerState::new(
                    PickerKind::Model,
                    self.locale
                        .t("Select Model", "选择模型")
                        .to_string(),
                    items,
                );
                picker.clamp_selection(10);
                self.active_modal = Some(ActiveModal::Picker(picker));
                return;
            }
            let mut lines = vec![format!(
                "Active model: {}:{} ({}, ctx={}k, thinking={})",
                route.connection_id,
                route.model,
                route.protocol.as_str(),
                route.context_window / 1000,
                route.reasoning_effort.as_deref().unwrap_or("automatic")
            )];
            for item in &items {
                let marker = if item.is_current { "*" } else { " " };
                lines.push(format!("  {marker} {}  ({})", item.label, item.detail));
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
            if let Some((conn_id, model_id)) = arg.split_once(':') {
                let conn_id = conn_id.trim();
                let model_id = model_id.trim();
                if !conn_id.is_empty()
                    && !model_id.is_empty()
                    && self.cfg.llm.connections.iter().any(|c| c.id == conn_id)
                {
                    let mut next_cfg = self.cfg.clone();
                    if let Some(conn) = next_cfg
                        .llm
                        .connections
                        .iter_mut()
                        .find(|c| c.id == conn_id)
                    {
                        if !conn.models.iter().any(|m| m.id == model_id) {
                            conn.models.push(crate::config::ModelEntry {
                                id: model_id.to_string(),
                                context_window: None,
                                max_output_tokens: None,
                                temperature: None,
                                top_p: None,
                                reasoning_effort: None,
                                capabilities: None,
                            });
                        }
                    }
                    next_cfg.llm.default_model = Some(DefaultModelSelection {
                        connection: conn_id.to_string(),
                        model: model_id.to_string(),
                    });
                    if let Ok(mut resolved) = resolve_model_route(&next_cfg) {
                        if resolved.reasoning_effort.is_none() {
                            resolved.reasoning_effort = prev_effort.clone();
                        }
                        resolved.model = model_id.to_string();
                        route = resolved;
                        matched = true;
                    }
                }
            }
            if !matched {
                if let Some((conn_id, model_id)) = arg.split_once(':') {
                    let conn_id = conn_id.trim();
                    let model_id = model_id.trim();
                    if let Ok(conns) = self.store.list_module_records::<Value>("model_connection") {
                        if let Some(c) = conns.into_iter().find(|c| {
                            c.pointer("/connection/id")
                                .or_else(|| c.get("id"))
                                .and_then(|v| v.as_str())
                                == Some(conn_id)
                        }) {
                            let endpoint = c
                                .pointer("/connection/base_url")
                                .or_else(|| c.pointer("/connection/endpoint"))
                                .or_else(|| c.get("base_url"))
                                .or_else(|| c.get("endpoint"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            let proto_raw = c
                                .pointer("/connection/wire_protocol")
                                .or_else(|| c.pointer("/connection/protocol"))
                                .or_else(|| c.get("wire_protocol"))
                                .or_else(|| c.get("protocol"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("openai_chat");
                            if !endpoint.is_empty() && !model_id.is_empty() {
                                let protocol = match proto_raw {
                                    "responses" | "openai_responses" | "open_ai_responses" => {
                                        ModelProtocol::OpenAiResponses
                                    }
                                    "anthropic" | "anthropic_messages" => {
                                        ModelProtocol::AnthropicMessages
                                    }
                                    "siwc" | "siwc_responses" | "chatgpt" => {
                                        ModelProtocol::SiwcResponses
                                    }
                                    "gemini" | "google_gemini" | "google" => ModelProtocol::Gemini,
                                    _ => ModelProtocol::OpenAiChat,
                                };
                                let vault_key = Vault::open(default_vault_path(&self.db_path))
                                    .ok()
                                    .and_then(|v| {
                                        v.get_secret(&format!("llm.connection.{conn_id}.api_key"))
                                            .ok()
                                            .flatten()
                                            .or_else(|| {
                                                v.get_secret(&format!("model_connection:{conn_id}"))
                                                    .ok()
                                                    .flatten()
                                            })
                                    })
                                    .unwrap_or_default();
                                route.connection_id = conn_id.to_string();
                                route.model = model_id.to_string();
                                route.endpoint = endpoint;
                                route.protocol = protocol;
                                if !vault_key.is_empty() {
                                    route.api_key = vault_key;
                                }
                                if route.reasoning_effort.is_none() {
                                    route.reasoning_effort = prev_effort.clone();
                                }
                                matched = true;
                            }
                        }
                    }
                }
            }
            if !matched {
                route.model = arg.to_string();
            }
        }
        let msg = if self.running {
            format!(
                "Live-switched active model to {}:{} (effective next step)",
                route.connection_id, route.model
            )
        } else {
            format!("Switched active model to {}:{}", route.connection_id, route.model)
        };
        if !route.connection_id.is_empty() && !route.model.is_empty() {
            self.cfg.llm.default_model = Some(DefaultModelSelection {
                connection: route.connection_id.clone(),
                model: route.model.clone(),
            });
            let _ = crate::config::save_applied_config(&self.store, &self.cfg);
        }
        self.update_route(route);
        self.push_system(msg);
    }

    fn cmd_thinking(&mut self, arg: &str) {
        let mut route = self.route_snapshot();
        if arg.is_empty() {
            let cur = route.reasoning_effort.as_deref().unwrap_or("automatic");
            if self.interactive {
                let options = [
                    (
                        "automatic",
                        self.locale.t("automatic", "automatic (自动)"),
                        self.locale.t("Follow model default", "跟随模型默认推理策略"),
                    ),
                    (
                        "none",
                        self.locale.t("none", "none (关闭)"),
                        self.locale.t("Disable extended reasoning", "关闭深度思考"),
                    ),
                    (
                        "low",
                        self.locale.t("low", "low (轻度)"),
                        self.locale.t("Fast brief reasoning", "快速轻量思考"),
                    ),
                    (
                        "medium",
                        self.locale.t("medium", "medium (中等)"),
                        self.locale.t("Balanced depth and speed", "兼顾速度与思考深度"),
                    ),
                    (
                        "high",
                        self.locale.t("high", "high (深度)"),
                        self.locale.t("Maximum reasoning budget", "最大深度推理预算"),
                    ),
                ];
                let items = options
                    .into_iter()
                    .map(|(val, label, detail)| PickerItem {
                        value: val.to_string(),
                        label: label.to_string(),
                        detail: detail.to_string(),
                        is_current: cur.eq_ignore_ascii_case(val),
                    })
                    .collect();
                let mut picker = PickerState::new(
                    PickerKind::Thinking,
                    self.locale
                        .t("Select Reasoning Effort", "选择推理深度")
                        .to_string(),
                    items,
                );
                picker.clamp_selection(10);
                self.active_modal = Some(ActiveModal::Picker(picker));
                return;
            }
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
            if self.interactive {
                let cur = self.permission_profile.as_canonical_str();
                let items = vec![
                    PickerItem {
                        value: "cautious".to_string(),
                        label: self.locale.t("cautious", "cautious (谨慎模式)").to_string(),
                        detail: self
                            .locale
                            .t(
                                "Ask before write/exec/network actions",
                                "写文件/执行命令/联网操作前需确认",
                            )
                            .to_string(),
                        is_current: cur == "cautious",
                    },
                    PickerItem {
                        value: "allow_all".to_string(),
                        label: self.locale.t("allow_all", "allow_all (全部允许)").to_string(),
                        detail: self
                            .locale
                            .t(
                                "Auto-approve workspace tools without prompts",
                                "自动放行工作区工具调用（免确认）",
                            )
                            .to_string(),
                        is_current: cur == "allow_all",
                    },
                ];
                let mut picker = PickerState::new(
                    PickerKind::Permission,
                    self.locale
                        .t("Select Permission Mode", "选择权限模式")
                        .to_string(),
                    items,
                );
                picker.clamp_selection(10);
                self.active_modal = Some(ActiveModal::Picker(picker));
                return;
            }
            self.push_system(format!(
                "Active permission profile: {} (use `/allow cautious` or `/allow allow_all`)",
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
        let staged = match rest.trim() {
            "" => false,
            "--staged" | "--cached" => true,
            _ => {
                self.push_error(self.locale.t("Usage: /diff [--staged]", "用法：/diff [--staged]"));
                return;
            }
        };
        let status_res = self.registry.execute("git", &self.ctx, json!({"action": "status"}));
        if status_res.is_error {
            let hint = self.locale.t(
                "Open a Git working tree to view changes with /diff.",
                "请在 Git 工作目录中打开 Nola，再用 /diff 查看改动。",
            );
            self.push_error(format!("{}\n{hint}", extract_git_stdout(&status_res)));
            return;
        }
        let diff_res = self.registry.execute(
            "git",
            &self.ctx,
            json!({"action": "diff", "staged": staged, "stat_only": true}),
        );
        if diff_res.is_error {
            self.push_error(extract_git_stdout(&diff_res));
            return;
        }
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
            "" | "list" => match self.suite.list_records::<Value>("data_source") {
                Ok(raw_list) => {
                    let list: Vec<Value> = raw_list
                        .into_iter()
                        .filter(|v| v.get("deleted_at_ms").is_none_or(|d| d.is_null()))
                        .collect();
                    if !list.is_empty() {
                        let lines: Vec<String> = list
                            .into_iter()
                            .map(|ds| {
                                let id = ds.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                let kind = ds.get("kind").and_then(|v| v.as_str()).unwrap_or("unknown");
                                let name = ds.get("name").and_then(|v| v.as_str()).unwrap_or(id);
                                let location = ds
                                    .pointer("/configuration/location")
                                    .or_else(|| ds.get("location"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let chunks = ds
                                    .pointer("/metadata/indexed_chunks")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                format!("  {id}  [{kind}]  {name}  -> {location} (chunks={chunks})")
                            })
                            .collect();
                        self.push_system(lines.join("\n"));
                    } else {
                        self.push_system(
                            "No data sources registered. Use `/sources add <name> <dir|file> <path>`.",
                        );
                    }
                }
                Err(e) => self.push_error(format!("sources list error: {e}")),
            },
            "add" => {
                let toks: Vec<&str> = body.splitn(3, char::is_whitespace).collect();
                if toks.len() < 3 {
                    self.push_error("Usage: /sources add <name> <dir|file|url> <location>");
                    return;
                }
                let id = new_id();
                let name = toks[0].trim().to_string();
                let kind = toks[1].trim().to_string();
                let location = toks[2].trim().to_string();
                let ts = now_ms();
                let (connector_id, location_kind) = match kind.to_ascii_lowercase().as_str() {
                    "dir" | "folder" | "localfolder" | "local_folder" => ("local_folder", "local_path"),
                    "file" | "localfile" | "local_file" => ("local_file", "local_path"),
                    _ => ("generic", "http_url"),
                };
                let rec = json!({
                    "id": id,
                    "name": name,
                    "kind": kind,
                    "location": location,
                    "configuration": {
                        "connector_id": connector_id,
                        "location_kind": location_kind,
                        "location": location,
                        "parameters": {},
                        "secret_references": []
                    },
                    "access_policy": "read_only",
                    "default_enabled": true,
                    "lifecycle": "ready",
                    "revision": 1,
                    "metadata": {},
                    "usage_count": 0,
                    "created_at_ms": ts,
                    "updated_at_ms": ts,
                    "deleted_at_ms": Value::Null
                });
                match self.suite.upsert_record("data_source", &id, &rec) {
                    Ok(()) => self.push_system(format!(
                        "Added data source {id} ({name} -> {location}). Run `/sources index {id}` to index."
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
        match self.read_tail_events(&self.session_id, limit) {
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
        if self.interactive {
            let suite = Arc::clone(&self.suite);
            let caller = Arc::clone(&self.raw_caller);
            let policy = self.base_policy.clone();
            let room_id_owned = room_id.to_string();
            let message_owned = message.to_string();
            let recipient_owned = recipient.map(ToString::to_string);
            let tx = self.worker_tx.clone();
            self.push_system(format!("Sending message to room {room_id} in background..."));
            thread::spawn(move || {
                match RoomRuntime::send_message_to(
                    &suite,
                    caller,
                    route,
                    policy,
                    &room_id_owned,
                    &message_owned,
                    recipient_owned.as_deref(),
                ) {
                    Ok(replies) => {
                        for r in replies {
                            let name = r["expert_name"].as_str().unwrap_or("Expert").to_string();
                            let content = r["content"].as_str().unwrap_or("").to_string();
                            let _ = tx.send(WorkerMsg::AssistantEntry {
                                title: format!("room:{name}"),
                                body: content,
                            });
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(WorkerMsg::Notice {
                            is_error: true,
                            text: format!("room error: {e}"),
                        });
                    }
                }
            });
        } else {
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
    }

    // ========================================================================
    // Fullscreen Viewport Rendering & Input Handling
    // ========================================================================

    fn slash_menu_candidates(&self) -> Vec<&'static str> {
        if self.active_modal.is_some() || self.slash_menu_dismissed {
            return Vec::new();
        }
        let buf = self.composer.buffer.as_str();
        if !buf.starts_with('/') || buf.starts_with("//") || buf.contains(char::is_whitespace) {
            return Vec::new();
        }
        let lower = buf.to_ascii_lowercase();
        SLASH_COMMANDS
            .iter()
            .copied()
            .filter(|cmd| cmd.starts_with(&lower))
            .collect()
    }

    fn build_slash_menu_lines(&self, cols: usize) -> Vec<String> {
        let candidates = self.slash_menu_candidates();
        if candidates.is_empty() {
            return Vec::new();
        }
        let max_visible = 7usize;
        let total = candidates.len();
        let selected = self.slash_menu_selected.min(total - 1);
        let scroll = if selected >= max_visible {
            selected + 1 - max_visible
        } else {
            0
        };
        let end = (scroll + max_visible).min(total);
        let box_w = cols.saturating_sub(2).max(24);
        let inner_w = box_w.saturating_sub(4);
        let mut rows = Vec::with_capacity(end - scroll + 1);
        let cmd_col_w = if inner_w < 54 { 12usize } else { 15usize };
        for (offset, cmd) in candidates[scroll..end].iter().enumerate() {
            let idx = scroll + offset;
            let is_sel = idx == selected;
            let desc = slash_command_description(cmd, self.locale);
            let pad = cmd_col_w.saturating_sub(cmd.len()).max(1);
            let desc_w = inner_w.saturating_sub(2 + cmd.len() + pad);
            let clipped_desc = truncate_with_ellipsis(desc, desc_w);
            if is_sel {
                let used = 2 + cmd.len() + pad + str_display_width(&clipped_desc);
                let fill = inner_w.saturating_sub(used);
                rows.push(format!(
                    "\x1b[48;5;237m\x1b[36m› \x1b[1;38;5;255m{cmd}\x1b[0;48;5;237m{}\x1b[38;5;250m{clipped_desc}{}\x1b[0m",
                    " ".repeat(pad),
                    " ".repeat(fill)
                ));
            } else {
                rows.push(format!(
                    "  \x1b[38;5;250m{cmd}\x1b[0m{}\x1b[2m{clipped_desc}\x1b[0m",
                    " ".repeat(pad)
                ));
            }
        }
        if total > max_visible {
            rows.push(format!(
                "\x1b[2m  … {}-{end} / {total}\x1b[0m",
                scroll + 1
            ));
        }
        let hint = self.locale.t(
            "↑↓ Select · Tab Complete · Enter Run · Esc Dismiss",
            "↑↓ 选择 · Tab 补全 · Enter 执行 · Esc 收起",
        );
        let title = format!("{} ({total})", self.composer.buffer);
        let raw_box = render_rounded_box(
            "\x1b[38;5;242m",
            &title,
            &rows,
            Some(hint),
            box_w,
        );
        raw_box.into_iter().map(|l| format!(" {l}")).collect()
    }

    fn render_welcome_lines(&self, width: usize) -> Vec<String> {
        let content_w = width.saturating_sub(4).max(24);
        let route = self.route_snapshot();
        let perm_label = match self.permission_profile {
            PermissionProfile::Cautious => self.locale.t("cautious", "谨慎"),
            PermissionProfile::AllowAll => self.locale.t("allow_all", "全部允许"),
        };
        let mut lines = Vec::new();
        lines.push(String::new());
        lines.push(format!(
            "  \x1b[1;38;5;255mNola\x1b[0m  \x1b[2m{}\x1b[0m",
            self.locale
                .t("Local-first Personal AI Assistant", "本地优先个人助手")
        ));
        lines.push(format!(
            "  \x1b[38;5;238m{}\x1b[0m",
            "─".repeat(content_w)
        ));
        lines.push(format!(
            "  \x1b[2m{}\x1b[0m  {}",
            self.locale.t("Workspace", "工作目录"),
            abbreviate_path_with_ellipsis(&self.workspace, content_w.saturating_sub(12))
        ));
        lines.push(format!(
            "  \x1b[2m{}\x1b[0m  \x1b[36m{}\x1b[0m \x1b[2m· {perm_label}\x1b[0m",
            self.locale.t("Active   ", "当前配置"),
            truncate_with_ellipsis(&route.model, content_w.saturating_sub(24))
        ));
        if let Some(ref root) = self.git_root {
            if root != &self.workspace {
                let root_disp = abbreviate_path_with_ellipsis(root, content_w.saturating_sub(24).max(16));
                lines.push(format!(
                    "  \x1b[33m{}\x1b[0m  \x1b[2m{}\x1b[0m",
                    self.locale.t("Git Root ", "Git 仓库根"),
                    truncate_with_ellipsis(
                        &format!(
                            "{root_disp} ({})",
                            self.locale.t(
                                "launch from repo root for full Git writes",
                                "如需写 .git 建议从仓库根目录启动"
                            )
                        ),
                        content_w.saturating_sub(13)
                    )
                ));
            }
        }
        lines.push(String::new());
        lines.push(format!(
            "  \x1b[2m/help · /model · /thinking · /allow · /resume · /attach · @file\x1b[0m"
        ));
        lines
    }

    fn render_transcript_lines(&self, width: usize) -> Vec<String> {
        if self.transcript.is_empty()
            && self.streaming_reasoning.is_empty()
            && self.streaming_text.is_empty()
            && self.active_tool.is_none()
        {
            return self.render_welcome_lines(width);
        }

        let mut lines = vec![String::new()];
        for entry in &self.transcript {
            match entry.kind {
                TranscriptKind::User => {
                    lines.extend(render_user_tinted_lines(&entry.body, width));
                    lines.push(String::new());
                }
                TranscriptKind::Assistant => {
                    lines.push(format!("\x1b[1;36m◆ {}\x1b[0m", entry.title));
                    lines.extend(render_markdown_ansi(&entry.body, width));
                    lines.push(String::new());
                }
                TranscriptKind::Reasoning => {
                    if self.verbose_tools {
                        lines.push(format!(
                            "\x1b[2m◦ {}\x1b[0m",
                            self.locale.t("Reasoning", "思考过程")
                        ));
                        for l in entry.body.lines() {
                            lines.extend(wrap_ansi_line(
                                &format!("\x1b[38;5;244m│ {l}\x1b[0m"),
                                width,
                            ));
                        }
                    } else {
                        let chars = entry.body.chars().count();
                        lines.push(format!(
                            "\x1b[2m  ◦ {} ({chars} {}, /verbose)\x1b[0m",
                            self.locale.t("Thought", "思考完成"),
                            self.locale.t("chars", "字")
                        ));
                    }
                }
                TranscriptKind::Tool { is_error } => {
                    let line = if is_error {
                        format!(
                            "  \x1b[1;31m✗ {}\x1b[0m  \x1b[31m{}\x1b[0m",
                            entry.title, entry.body
                        )
                    } else {
                        format!(
                            "  \x1b[32m✓\x1b[0m \x1b[1m{}\x1b[0m  \x1b[2m{}\x1b[0m",
                            entry.title, entry.body
                        )
                    };
                    lines.extend(wrap_ansi_line(&line, width));
                }
                TranscriptKind::System => {
                    for l in entry.body.lines() {
                        lines.extend(wrap_ansi_line(
                            &format!("\x1b[38;5;240m▎\x1b[0m \x1b[38;5;110m{l}\x1b[0m"),
                            width,
                        ));
                    }
                    lines.push(String::new());
                }
                TranscriptKind::Error => {
                    for (idx, l) in entry.body.lines().enumerate() {
                        let formatted = if idx == 0 {
                            format!("\x1b[31m▎\x1b[0m \x1b[1;31m✗ {l}\x1b[0m")
                        } else {
                            format!("\x1b[31m▎\x1b[0m   \x1b[31m{l}\x1b[0m")
                        };
                        lines.extend(wrap_ansi_line(&formatted, width));
                    }
                    lines.push(String::new());
                }
            }
        }

        if !self.streaming_reasoning.is_empty() {
            let spinner = SPINNER_FRAMES[self.spinner_idx % SPINNER_FRAMES.len()];
            lines.push(format!(
                "\x1b[2m  {spinner} {}\x1b[0m",
                self.locale.t("Thinking...", "思考中...")
            ));
            let mut wrapped = Vec::new();
            for l in self.streaming_reasoning.lines() {
                wrapped.extend(wrap_ansi_line(
                    &format!("\x1b[38;5;244m  │ {l}\x1b[0m"),
                    width,
                ));
            }
            if self.verbose_tools {
                lines.extend(wrapped);
            } else {
                let tail_start = wrapped.len().saturating_sub(5);
                lines.extend(wrapped.into_iter().skip(tail_start));
            }
        }
        if let Some((ref name, ref preview, started)) = self.active_tool {
            let spinner = SPINNER_FRAMES[self.spinner_idx % SPINNER_FRAMES.len()];
            let secs = started.elapsed().as_secs_f32();
            let detail = if preview.is_empty() {
                format!("{secs:.1}s")
            } else {
                format!("{preview} · {secs:.1}s")
            };
            let live_line = format!(
                "  \x1b[36m{spinner}\x1b[0m \x1b[1m{name}\x1b[0m  \x1b[2m{detail}\x1b[0m"
            );
            lines.extend(wrap_ansi_line(&live_line, width));
        }
        if !self.streaming_text.is_empty() {
            let model = self.route_snapshot().model;
            lines.push(format!("\x1b[1;36m◆ {model}\x1b[0m"));
            let mut rendered = render_markdown_ansi(&self.streaming_text, width);
            if let Some(last) = rendered.last_mut() {
                if !last.ends_with('╯') && !last.ends_with('┘') {
                    last.push_str("\x1b[36m▍\x1b[0m");
                }
            }
            lines.extend(rendered);
        }

        lines
    }

    fn draw_screen(&mut self) -> io::Result<()> {
        let (cols_u16, rows_u16) = terminal::size().unwrap_or((100, 30));
        let cols = cols_u16.max(40) as usize;
        let rows = rows_u16.max(12) as usize;

        let route = self.route_snapshot();
        let ws_name = self
            .workspace
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("workspace");
        let plan_badge = if self.current_plan_mode() {
            " [PLAN]"
        } else {
            ""
        };
        let expert_badge = self
            .active_expert
            .as_ref()
            .map(|e| format!(" · @{}", e.role))
            .unwrap_or_default();
        let tasks_count = self.active_background_count();

        let overlay_lines = if self.active_modal.is_some() {
            self.build_modal_lines(cols)
        } else {
            self.build_slash_menu_lines(cols)
        };
        let show_attach_bar = !self.pending_attachments.is_empty() && self.active_modal.is_none();
        let overlay_height = overlay_lines.len() + usize::from(show_attach_bar);
        // Bottom stack: overlay + optional attach bar + top sep + composer + bottom sep + status bar
        let footer_height = 4 + overlay_height;
        let header_height = 2usize;
        let viewport_height = rows.saturating_sub(header_height + footer_height).max(2);

        let all_lines = self.render_transcript_lines(cols.saturating_sub(3));
        let max_scroll = all_lines.len().saturating_sub(viewport_height);
        if self.auto_follow {
            self.scroll_offset = 0;
        } else if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
        let end_idx = all_lines.len().saturating_sub(self.scroll_offset);
        let start_idx = end_idx.saturating_sub(viewport_height);

        let mut out = io::stdout().lock();
        // Synchronized update reduces flicker in modern terminals (ignored safely by older terminals)
        let _ = write!(out, "\x1b[?2026h");
        out.queue(Hide)?;

        // Row 0: Minimalist dark top bar (no harsh solid blue banner)
        out.queue(MoveTo(0, 0))?;
        out.queue(Clear(ClearType::CurrentLine))?;
        let left_header = format!("  \x1b[1mNola\x1b[0m \x1b[2m· {ws_name}{plan_badge}{expert_badge}\x1b[0m");
        let right_header = if self.running {
            let spinner = SPINNER_FRAMES[self.spinner_idx % SPINNER_FRAMES.len()];
            let elapsed = self
                .turn_started_at
                .map(|t| format!("{:.1}s", t.elapsed().as_secs_f32()))
                .unwrap_or_else(|| "RUN".to_string());
            format!("\x1b[36m{spinner}\x1b[0m \x1b[2m{elapsed}\x1b[0m  ")
        } else {
            "\x1b[32m●\x1b[0m \x1b[2mLIVE\x1b[0m  ".to_string()
        };
        let rw = str_display_width(&right_header);
        let clipped_left = truncate_to_width(&left_header, cols.saturating_sub(rw + 1));
        let lw = str_display_width(&clipped_left);
        let hpad = cols.saturating_sub(lw + rw);
        write!(out, "{clipped_left}{}{right_header}", " ".repeat(hpad))?;

        // Row 1: Subtle divider
        out.queue(MoveTo(0, 1))?;
        out.queue(Clear(ClearType::CurrentLine))?;
        write!(out, "\x1b[38;5;238m{}\x1b[0m", "─".repeat(cols))?;

        // Viewport rows + subtle right-edge scrollbar when content overflows
        let needs_scrollbar = all_lines.len() > viewport_height && viewport_height >= 3;
        let thumb_len = if needs_scrollbar {
            ((viewport_height * viewport_height) / all_lines.len().max(1)).clamp(1, viewport_height)
        } else {
            0
        };
        let thumb_top = if needs_scrollbar && max_scroll > 0 {
            let progress = max_scroll.saturating_sub(self.scroll_offset);
            (progress * viewport_height.saturating_sub(thumb_len)) / max_scroll
        } else {
            0
        };

        for row_idx in 0..viewport_height {
            let y = (header_height + row_idx) as u16;
            out.queue(MoveTo(0, y))?;
            out.queue(Clear(ClearType::CurrentLine))?;
            if let Some(line) = all_lines.get(start_idx + row_idx) {
                write!(out, " {line}\x1b[0m")?;
            }
            if needs_scrollbar {
                let in_thumb = row_idx >= thumb_top && row_idx < thumb_top + thumb_len;
                out.queue(MoveTo((cols - 1) as u16, y))?;
                if in_thumb {
                    write!(out, "\x1b[38;5;244m┃\x1b[0m")?;
                } else {
                    write!(out, "\x1b[38;5;236m│\x1b[0m")?;
                }
            }
        }

        // Optional full-enclosed modal or live slash menu overlay above composer
        let mut cursor_y = (header_height + viewport_height) as u16;
        for oline in &overlay_lines {
            out.queue(MoveTo(0, cursor_y))?;
            out.queue(Clear(ClearType::CurrentLine))?;
            write!(out, "{oline}\x1b[0m")?;
            cursor_y += 1;
        }

        // Visible pending-attachments pill bar above composer
        if show_attach_bar {
            out.queue(MoveTo(0, cursor_y))?;
            out.queue(Clear(ClearType::CurrentLine))?;
            let names: Vec<&str> = self
                .pending_attachments
                .iter()
                .map(|a| a.display_name.as_str())
                .collect();
            let prefix = self.locale.t("Attached:", "已附加:");
            let clear_hint = "/attach clear";
            let bar_text = format!(
                "  \x1b[36m+\x1b[0m \x1b[2m{prefix}\x1b[0m {}  \x1b[2m({clear_hint})\x1b[0m",
                names.join(" · ")
            );
            write!(out, "{}\x1b[0m", truncate_with_ellipsis(&bar_text, cols.saturating_sub(1)))?;
            cursor_y += 1;
        }

        // Top composer divider (with scrollback jump pill when scrolled up)
        out.queue(MoveTo(0, cursor_y))?;
        out.queue(Clear(ClearType::CurrentLine))?;
        if self.scroll_offset > 0 {
            let pill = if self.locale == TuiLocale::ZhHans {
                format!(" ↑ 向上 {} 行 · PgDn/End 回到底部 ", self.scroll_offset)
            } else {
                format!(" ↑ {} lines above · PgDn/End to follow ", self.scroll_offset)
            };
            let pill_clipped = truncate_to_width(&pill, cols.saturating_sub(6));
            let pw = str_display_width(&pill_clipped);
            let left_d = cols.saturating_sub(pw) / 2;
            let right_d = cols.saturating_sub(pw + left_d);
            write!(
                out,
                "\x1b[38;5;238m{}\x1b[33m{pill_clipped}\x1b[38;5;238m{}\x1b[0m",
                "─".repeat(left_d),
                "─".repeat(right_d)
            )?;
        } else {
            write!(out, "\x1b[38;5;238m{}\x1b[0m", "─".repeat(cols))?;
        }
        cursor_y += 1;

        // Composer line
        let composer_y = cursor_y;
        out.queue(MoveTo(0, composer_y))?;
        out.queue(Clear(ClearType::CurrentLine))?;
        let (prompt_prefix, active_composer) = match self.active_modal.as_ref() {
            Some(ActiveModal::AskUser { input, .. }) => ("  answer› ", input),
            Some(ActiveModal::Picker(picker)) => ("  filter› ", &picker.filter),
            _ => ("  › ", &self.composer),
        };
        let prompt_w = str_display_width(prompt_prefix);
        write!(out, "\x1b[1;36m{prompt_prefix}\x1b[0m")?;

        let avail = cols.saturating_sub(prompt_w + 2).max(10);
        let mut slice_start = 0usize;
        if active_composer.buffer.is_empty() {
            match self.active_modal.as_ref() {
                None => {
                    let placeholder = if self.running {
                        self.locale.t(
                            "Running... /guide <msg> or Esc to cancel",
                            "执行中... 输入 /guide 引导或按 Esc 中断",
                        )
                    } else {
                        self.locale.t("Type a message...", "输入消息...")
                    };
                    let right_hint = if cols >= 56 { "/help · @file  " } else { "" };
                    let rh_w = str_display_width(right_hint);
                    let clipped_ph = truncate_to_width(placeholder, avail.saturating_sub(rh_w + 1));
                    let ph_w = str_display_width(&clipped_ph);
                    let gap = cols.saturating_sub(prompt_w + ph_w + rh_w);
                    write!(out, "\x1b[2m{clipped_ph}{}{right_hint}\x1b[0m", " ".repeat(gap))?;
                }
                Some(ActiveModal::Picker(_)) => {
                    let ph = self.locale.t("Type to filter...", "输入关键字过滤...");
                    write!(out, "\x1b[2m{}\x1b[0m", truncate_to_width(ph, avail))?;
                }
                Some(ActiveModal::AskUser { .. }) => {
                    let ph = self.locale.t("Type option number or answer...", "输入选项编号或回答...");
                    write!(out, "\x1b[2m{}\x1b[0m", truncate_to_width(ph, avail))?;
                }
                Some(ActiveModal::Approval { .. }) => {
                    let ph = self.locale.t("Press 1 / 2 / 3 / n...", "按 1 / 2 / 3 / n 响应审批...");
                    write!(out, "\x1b[2m{}\x1b[0m", truncate_to_width(ph, avail))?;
                }
            }
        } else {
            let total_buf_w = str_display_width(&active_composer.buffer);
            let visible_buf = if total_buf_w <= avail {
                active_composer.buffer.clone()
            } else {
                let inner_avail = avail.saturating_sub(2).max(6);
                while slice_start < active_composer.cursor
                    && str_display_width(
                        &active_composer.buffer[slice_start..active_composer.cursor],
                    ) > inner_avail.saturating_sub(1)
                {
                    if let Some(ch) = active_composer.buffer[slice_start..].chars().next() {
                        slice_start += ch.len_utf8();
                    } else {
                        break;
                    }
                }
                let sliced = truncate_to_width(&active_composer.buffer[slice_start..], inner_avail);
                let has_left = slice_start > 0;
                let has_right = slice_start + sliced.len() < active_composer.buffer.len();
                format!(
                    "{}{sliced}{}",
                    if has_left { "\x1b[2m‹\x1b[0m" } else { "" },
                    if has_right { "\x1b[2m›\x1b[0m" } else { "" }
                )
            };
            write!(out, "{visible_buf}")?;
        }
        cursor_y += 1;

        // Bottom composer divider
        out.queue(MoveTo(0, cursor_y))?;
        out.queue(Clear(ClearType::CurrentLine))?;
        write!(out, "\x1b[38;5;238m{}\x1b[0m", "─".repeat(cols))?;
        cursor_y += 1;

        // Minimalist bottom status bar with dynamic context usage color & mini gauge
        out.queue(MoveTo(0, cursor_y))?;
        out.queue(Clear(ClearType::CurrentLine))?;
        let perm_label = match self.permission_profile {
            PermissionProfile::Cautious => self.locale.t("cautious", "谨慎"),
            PermissionProfile::AllowAll => self.locale.t("allow_all", "全部允许"),
        };
        let think_label = route.reasoning_effort.as_deref().unwrap_or("auto");
        let mut left_parts = vec![
            route.model.clone(),
            perm_label.to_string(),
            format!("think:{think_label}"),
        ];
        if tasks_count > 0 {
            left_parts.push(format!("tasks:{tasks_count}"));
        }
        if !self.pending_attachments.is_empty() {
            left_parts.push(format!("files:{}", self.pending_attachments.len()));
        }
        if self.scroll_offset > 0 {
            left_parts.push(format!("↑{}", self.scroll_offset));
        }
        let left_status = if let Some(ref hint) = self.completion_hint {
            format!("  {hint}")
        } else {
            format!("  {}", left_parts.join(" · "))
        };
        let ctx_pct = self
            .last_usage
            .as_ref()
            .map(|u| {
                let used = u.input_tokens.saturating_add(u.cache_read_tokens);
                let win = route.context_window.max(1);
                ((used as f64 / win as f64) * 100.0).min(100.0)
            })
            .unwrap_or(0.0);
        let ctx_sgr = if ctx_pct >= 85.0 {
            "\x1b[1;31m"
        } else if ctx_pct >= 65.0 {
            "\x1b[33m"
        } else {
            "\x1b[2m"
        };
        let gauge = if cols >= 74 {
            let filled = ((ctx_pct / 20.0).round() as usize).min(5);
            format!("{}{} ", "▪".repeat(filled), "▫".repeat(5 - filled))
        } else {
            String::new()
        };
        let right_status_plain = format!(
            "{} {gauge}{:.0}%  ",
            self.locale.t("Context", "上下文"),
            ctx_pct
        );
        let rsw = str_display_width(&right_status_plain);
        let clipped_ls = truncate_with_ellipsis(&left_status, cols.saturating_sub(rsw + 1));
        let lsw = str_display_width(&clipped_ls);
        let spad = cols.saturating_sub(lsw + rsw);
        write!(
            out,
            "\x1b[2m{clipped_ls}\x1b[0m{}{ctx_sgr}{right_status_plain}\x1b[0m",
            " ".repeat(spad)
        )?;

        // Clear any leftover rows below status bar after terminal shrink or modal close
        if (cursor_y as usize + 1) < rows {
            out.queue(MoveTo(0, cursor_y + 1))?;
            out.queue(Clear(ClearType::FromCursorDown))?;
        }

        if !matches!(self.active_modal, Some(ActiveModal::Approval { .. })) {
            let left_marker_col = usize::from(slice_start > 0);
            let cursor_col_in_view = left_marker_col
                + str_display_width(&active_composer.buffer[slice_start..active_composer.cursor]);
            let cursor_x = (prompt_w + cursor_col_in_view.min(avail)) as u16;
            out.queue(MoveTo(cursor_x, composer_y))?;
            out.queue(Show)?;
        }

        let _ = write!(out, "\x1b[?2026l");
        out.flush()
    }

    fn build_modal_lines(&self, cols: usize) -> Vec<String> {
        let Some(modal) = self.active_modal.as_ref() else {
            return Vec::new();
        };
        let box_w = cols.saturating_sub(2).max(24);
        let inner_w = box_w.saturating_sub(4);
        let raw_box = match modal {
            ActiveModal::Approval {
                tool_name,
                target_key,
                summary,
                reason,
                ..
            } => {
                let title = self.locale.t("Permission Approval", "权限审批");
                let prompt = self.locale.t(
                    "[1/y] Allow once · [2/a] Allow in session · [3/r/Esc] Pause · [n/Enter] Deny",
                    "[1/y] 允许一次 · [2/a] 本会话允许 · [3/r/Esc] 暂停返回 · [n/Enter] 拒绝",
                );
                let body = vec![
                    format!(
                        "\x1b[2m{}:\x1b[0m  \x1b[1m{}\x1b[0m",
                        self.locale.t("Tool", "工具"),
                        truncate_to_width(tool_name, inner_w.saturating_sub(10))
                    ),
                    format!(
                        "\x1b[2m{}:\x1b[0m  {}",
                        self.locale.t("Target", "目标"),
                        truncate_to_width(target_key, inner_w.saturating_sub(10))
                    ),
                    format!(
                        "\x1b[2m{}:\x1b[0m  {}",
                        self.locale.t("Detail", "详情"),
                        truncate_to_width(summary, inner_w.saturating_sub(10))
                    ),
                    format!(
                        "\x1b[2m{}:\x1b[0m  {}",
                        self.locale.t("Reason", "原因"),
                        truncate_to_width(reason, inner_w.saturating_sub(10))
                    ),
                ];
                render_rounded_box("\x1b[33m", title, &body, Some(prompt), box_w)
            }
            ActiveModal::AskUser {
                question, options, ..
            } => {
                let title = self.locale.t("Agent Question", "助手提问");
                let mut body = vec![truncate_to_width(question, inner_w)];
                for (i, opt) in options.iter().enumerate() {
                    body.push(format!(
                        "  \x1b[1;35m{}.\x1b[0m {}",
                        i + 1,
                        truncate_to_width(opt, inner_w.saturating_sub(5))
                    ));
                }
                let hint = self.locale.t(
                    "Type option number or answer · Enter Submit · Esc Cancel",
                    "输入选项编号或内容 · Enter 提交 · Esc 取消",
                );
                render_rounded_box("\x1b[35m", title, &body, Some(hint), box_w)
            }
            ActiveModal::Picker(picker) => {
                let filtered = picker.filtered_indices();
                let total = filtered.len();
                let max_visible = 10usize;
                let scroll = picker
                    .scroll
                    .min(total.saturating_sub(max_visible.min(total)));
                let end = (scroll + max_visible).min(total);
                let count_badge = if total == 0 {
                    format!("0/{}", picker.items.len())
                } else {
                    format!("{}/{total}", picker.selected.min(total - 1) + 1)
                };
                let title = format!("{} ({count_badge})", picker.title);
                let mut body = Vec::new();
                if total == 0 {
                    body.push(format!(
                        "  \x1b[2m{}\x1b[0m",
                        self.locale.t("No matching items", "没有匹配项")
                    ));
                } else {
                    for (pos, &item_idx) in filtered[scroll..end].iter().enumerate() {
                        let abs_pos = scroll + pos;
                        let item = &picker.items[item_idx];
                        let is_sel = abs_pos == picker.selected;
                        let label_part = truncate_with_ellipsis(&item.label, inner_w.saturating_sub(6));
                        let detail_budget = inner_w
                            .saturating_sub(6 + str_display_width(&label_part));
                        let detail_part = if !item.detail.is_empty() && detail_budget >= 6 {
                            format!(
                                "  ({})",
                                truncate_with_ellipsis(&item.detail, detail_budget.saturating_sub(4))
                            )
                        } else {
                            String::new()
                        };
                        let total_text_w =
                            4 + str_display_width(&label_part) + str_display_width(&detail_part);
                        if is_sel {
                            let pad = inner_w.saturating_sub(total_text_w);
                            let mark_sgr = if item.is_current {
                                "\x1b[32m●\x1b[38;5;255m"
                            } else {
                                " "
                            };
                            body.push(format!(
                                "\x1b[48;5;238m\x1b[1;36m› {mark_sgr} \x1b[1;38;5;255m{label_part}\x1b[0;48;5;238;38;5;250m{detail_part}{}\x1b[0m",
                                " ".repeat(pad)
                            ));
                        } else {
                            let mark_sgr = if item.is_current {
                                "\x1b[32m●\x1b[0m"
                            } else {
                                " "
                            };
                            body.push(format!(
                                "  {mark_sgr} {label_part}\x1b[2m{detail_part}\x1b[0m"
                            ));
                        }
                    }
                    if scroll > 0 || end < total {
                        body.push(format!("\x1b[2m  … {}-{end} / {total}\x1b[0m", scroll + 1));
                    }
                }
                let hint = self.locale.t(
                    "↑↓ Select · Type to filter · Enter Confirm · Esc Cancel",
                    "↑↓ 选择 · 输入关键字过滤 · Enter 确认切换 · Esc 取消",
                );
                render_rounded_box("\x1b[36m", &title, &body, Some(hint), box_w)
            }
        };
        raw_box.into_iter().map(|l| format!(" {l}")).collect()
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
                ActiveModal::Picker(mut picker) => {
                    if key.code == KeyCode::Esc
                        || (key.modifiers.contains(KeyModifiers::CONTROL)
                            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')))
                    {
                        return;
                    }
                    match key.code {
                        KeyCode::Enter => {
                            if let Some(chosen) = picker.selected_item().cloned() {
                                match picker.kind {
                                    PickerKind::Model => self.cmd_model(&chosen.value),
                                    PickerKind::Thinking => self.cmd_thinking(&chosen.value),
                                    PickerKind::Permission => self.cmd_allow(&chosen.value),
                                }
                                return;
                            }
                        }
                        KeyCode::Up | KeyCode::BackTab => picker.move_up(1),
                        KeyCode::Down | KeyCode::Tab => picker.move_down(1),
                        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            picker.move_up(1);
                        }
                        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            picker.move_down(1);
                        }
                        KeyCode::PageUp => picker.move_up(5),
                        KeyCode::PageDown => picker.move_down(5),
                        KeyCode::Left => picker.filter.move_left(),
                        KeyCode::Right => picker.filter.move_right(),
                        KeyCode::Home => {
                            if picker.filter.buffer.is_empty() {
                                picker.selected = 0;
                                picker.clamp_selection(10);
                            } else {
                                picker.filter.move_home();
                            }
                        }
                        KeyCode::End => {
                            if picker.filter.buffer.is_empty() {
                                let count = picker.filtered_indices().len();
                                if count > 0 {
                                    picker.selected = count - 1;
                                    picker.clamp_selection(10);
                                }
                            } else {
                                picker.filter.move_end();
                            }
                        }
                        KeyCode::Backspace => {
                            picker.filter.backspace();
                            picker.selected = 0;
                            picker.clamp_selection(10);
                        }
                        KeyCode::Delete => {
                            picker.filter.delete();
                            picker.selected = 0;
                            picker.clamp_selection(10);
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            picker.filter.clear();
                            picker.selected = 0;
                            picker.clamp_selection(10);
                        }
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
                                && !key.modifiers.contains(KeyModifiers::ALT) =>
                        {
                            picker.filter.insert_char(c);
                            picker.selected = 0;
                            picker.clamp_selection(10);
                        }
                        _ => {}
                    }
                    self.active_modal = Some(ActiveModal::Picker(picker));
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
                KeyCode::Char('w') | KeyCode::Char('W') => {
                    self.composer.delete_word_backward();
                    self.slash_menu_dismissed = false;
                    self.slash_menu_selected = 0;
                    return;
                }
                KeyCode::Char('k') | KeyCode::Char('K') => {
                    self.composer.kill_to_end();
                    self.slash_menu_dismissed = false;
                    self.slash_menu_selected = 0;
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

        let slash_candidates = self.slash_menu_candidates();
        let slash_menu_open = !slash_candidates.is_empty();

        match key.code {
            KeyCode::Esc => {
                if slash_menu_open {
                    self.slash_menu_dismissed = true;
                    self.slash_menu_selected = 0;
                } else if self.running {
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
            KeyCode::Up => {
                if slash_menu_open {
                    let count = slash_candidates.len();
                    if self.slash_menu_selected == 0 {
                        self.slash_menu_selected = count - 1;
                    } else {
                        self.slash_menu_selected -= 1;
                    }
                } else {
                    self.composer.history_up();
                }
            }
            KeyCode::Down => {
                if slash_menu_open {
                    let count = slash_candidates.len();
                    self.slash_menu_selected = (self.slash_menu_selected + 1) % count;
                } else {
                    self.composer.history_down();
                }
            }
            KeyCode::Backspace => {
                self.composer.backspace();
                self.slash_menu_dismissed = false;
                self.slash_menu_selected = 0;
            }
            KeyCode::Delete => {
                self.composer.delete();
                self.slash_menu_dismissed = false;
                self.slash_menu_selected = 0;
            }
            KeyCode::Tab => {
                if slash_menu_open {
                    let sel = self.slash_menu_selected.min(slash_candidates.len() - 1);
                    let chosen = slash_candidates[sel];
                    self.composer.buffer = format!("{chosen} ");
                    self.composer.cursor = self.composer.buffer.len();
                    self.slash_menu_selected = 0;
                } else {
                    let experts = self.known_expert_names();
                    let candidates = self.composer.apply_tab_completion(&experts);
                    if candidates.len() > 1 {
                        self.completion_hint = Some(format!("Tab: {}", candidates.join("  ")));
                    }
                }
            }
            KeyCode::Enter => {
                if slash_menu_open {
                    let sel = self.slash_menu_selected.min(slash_candidates.len() - 1);
                    let chosen = slash_candidates[sel].to_string();
                    self.composer.clear();
                    self.slash_menu_selected = 0;
                    self.slash_menu_dismissed = false;
                    self.execute_input(&chosen);
                } else {
                    let line = self.composer.submit();
                    self.slash_menu_selected = 0;
                    self.slash_menu_dismissed = false;
                    self.execute_input(&line);
                }
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.composer.insert_char(c);
                self.slash_menu_dismissed = false;
                self.slash_menu_selected = 0;
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

    let git_root = detect_git_root(workspace);
    let mut app = TuiApp {
        store,
        suite,
        workspace: workspace.to_path_buf(),
        git_root,
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
        turn_started_at: None,
        active_tool: None,
        tool_call_previews: HashMap::new(),
        spinner_idx: 0,
        transcript: Vec::new(),
        streaming_text: String::new(),
        streaming_reasoning: String::new(),
        scroll_offset: 0,
        auto_follow: true,
        composer: ComposerState::default(),
        slash_menu_selected: 0,
        slash_menu_dismissed: false,
        completion_hint: None,
        active_modal: None,
        worker_tx,
        worker_rx,
        last_usage: None,
        should_exit: false,
        pending_editor_prompt: false,
        interactive,
    };

    if !first_turn {
        app.push_system(format!("Resuming session {session_id}"));
        app.load_session_preview(&session_id);
    } else if !interactive {
        let welcome = format!(
            "{} | model={} | permission={} | workspace={}",
            locale.t("Nola v2 Interactive TUI", "Nola v2 交互式终端"),
            route.model,
            permission_profile.as_canonical_str(),
            workspace.display()
        );
        app.push_system(welcome);
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
                        if let Some(ActiveModal::Picker(ref mut picker)) = app.active_modal {
                            picker.move_up(1);
                        } else {
                            app.auto_follow = false;
                            app.scroll_offset = app.scroll_offset.saturating_add(3);
                        }
                        dirty = true;
                    }
                    MouseEventKind::ScrollDown => {
                        if let Some(ActiveModal::Picker(ref mut picker)) = app.active_modal {
                            picker.move_down(1);
                        } else {
                            app.scroll_offset = app.scroll_offset.saturating_sub(3);
                            if app.scroll_offset == 0 {
                                app.auto_follow = true;
                            }
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
    if let Some(output) = val.get("output").and_then(|v| v.as_str()) {
        return output.to_owned();
    }
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

fn detect_git_root(workspace: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(workspace)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() {
        None
    } else {
        Some(PathBuf::from(raw))
    }
}

/// Pure read-only shell commands with no redirection, piping, subshell, or command chaining.
fn is_readonly_safe_shell_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return false;
    }
    for forbidden in ['|', ';', '&', '>', '<', '`', '$', '\n', '\r'] {
        if trimmed.contains(forbidden) {
            return false;
        }
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let Some(&first) = tokens.first() else {
        return false;
    };
    match first {
        "pwd" | "ls" | "ll" | "which" | "uname" | "whoami" | "date" | "wc" | "stat" | "file"
        | "head" | "tail" | "cat" => true,
        "git" => matches!(
            tokens.get(1).copied().unwrap_or(""),
            "status" | "rev-parse" | "branch" | "log" | "diff" | "show" | "remote" | "tag"
        ),
        _ => false,
    }
}

fn summarize_tool_args(tool_name: &str, args: &Value) -> String {
    let raw = match tool_name {
        "bash" | "pty_exec" => args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        "git" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let path = args
                .get("path")
                .or_else(|| args.get("target"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if path.is_empty() {
                action.to_string()
            } else {
                format!("{action} {path}")
            }
        }
        "read_file" | "write_file" | "edit_file" | "delete_file" | "list_dir" => args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "grep" | "glob" | "web_search" => args
            .get("pattern")
            .or_else(|| args.get("query"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "web_fetch" => args
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    let redacted = PrivacyRedactor::redact_text(&raw);
    let single_line: String = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_to_width(&single_line, 64)
}

fn format_approval_target(tool_name: &str, args: &Value) -> String {
    if matches!(tool_name, "bash" | "pty_exec") {
        if let Some(cmd) = args
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let normalized: String = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
            let redacted = PrivacyRedactor::redact_text(&normalized);
            return truncate_to_width(&redacted, 120);
        }
    }
    extract_target_key(tool_name, args)
}

fn extract_session_grant_key(tool_name: &str, args: &Value) -> String {
    if matches!(tool_name, "bash" | "pty_exec") {
        if let Some(cmd) = args
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let tokens: Vec<&str> = cmd.split_whitespace().collect();
            if tokens.len() >= 2 && matches!(tokens[0], "git" | "cargo" | "npm" | "npx" | "python3" | "pytest") {
                return format!("prefix:{} {}", tokens[0], tokens[1]);
            }
            if let Some(&first) = tokens.first() {
                return format!("prefix:{first}");
            }
        }
    }
    extract_target_key(tool_name, args)
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
    let Some(cache_bps) = latest_usage.cache_hit_rate_bps_normalized(excl_cache) else {
        return format!("Context {pct:.0}% · {used_k:.1}k/{win_k:.0}k · cache hit —");
    };
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
    /model [id]                            打开交互面板选择或直接实时切换模型
    /thinking [none|low|medium|high|auto]  打开交互面板或直接实时切换推理深度
    /guide <text>                          向运行中的轮次实时注入引导指令
    /allow [cautious|allow_all]            打开交互面板或直接切换权限模式
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
    /model [id]                            Open interactive picker or live-switch model
    /thinking [none|low|medium|high|auto]  Open interactive picker or live-switch reasoning
    /guide <text>                          Push steering guidance into active turn inbox
    /allow [cautious|allow_all]            Open interactive picker or switch permission mode
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
