// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Own the interactive CLI surface and keep the composer as the cursor owner.

use std::collections::VecDeque;
use crate::terminal_document::TranscriptCell;
use std::io::{self, IsTerminal, Write};
use std::sync::{Mutex, OnceLock};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use unicode_segmentation::UnicodeSegmentation;

const MAX_ACTIVE_LINES: usize = 6;
const MAX_FOOTER_LINES: usize = 2;
const MAX_CONTENT_WIDTH: usize = 120;
const MAX_SOFT_WRAP_LOOKBACK: usize = 24;
const MAX_VIEWPORT_REPLAY_BYTES: usize = 256 * 1024;
const MAX_VIEWPORT_ROWS_PER_FRAME: usize = 4;

/// Terminal emulators do not expose the desktop Reduce Motion preference.
/// Set NOLA_REDUCE_MOTION=1 for a stationary activity indicator.
pub(crate) fn reduced_motion() -> bool {
    static REDUCED: OnceLock<bool> = OnceLock::new();
    *REDUCED.get_or_init(|| std::env::var("NOLA_REDUCE_MOTION")
        .is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnPhase {
    Idle,
    Running { turn_id: u64 },
    AwaitingPermission { turn_id: u64 },
    Cancelling { turn_id: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GoalPhase {
    None,
    Active { goal_id: String },
    Completed { goal_id: String },
    Blocked { goal_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComposerState {
    draft: String,
    cursor: usize,
    queued_count: usize,
    placeholder: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActivityState {
    key: String,
    glyph: String,
    detail: String,
}

impl ActivityState {
    fn render(&self, width: usize) -> String {
        // Status can include model/tool text. Keep it to one safe, bounded row
        // so updates cannot wrap into the transcript or disturb the composer.
        let text = format!("  {} {}", self.glyph, self.detail);
        let text: String = text.chars().map(|ch| if ch == '\n' || ch == '\r' || ch == '\t' { ' ' } else { ch }).collect();
        let text = strip_ansi(&safe_styled_line(&text));
        truncate_display(&text, width)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IdleFrameState {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_column: u16,
}

#[derive(Debug, Default, Clone)]
struct StreamBoundaryTracker {
    scanned_bytes: usize,
    line_boundaries: VecDeque<usize>,
    total_scanned_bytes: usize,
}

impl StreamBoundaryTracker {
    fn stable_prefix(&mut self, text: &str) -> Option<usize> {
        let bytes = text.as_bytes();
        let start = self.scanned_bytes.min(bytes.len());
        for (index, byte) in bytes.iter().enumerate().skip(start) {
            if *byte == b'\n' {
                self.line_boundaries.push_back(index + 1);
            }
        }
        self.total_scanned_bytes = self
            .total_scanned_bytes
            .saturating_add(bytes.len().saturating_sub(start));
        self.scanned_bytes = bytes.len();

        for index in (1..self.line_boundaries.len()).rev() {
            let previous = self.line_boundaries[index - 1];
            let boundary = self.line_boundaries[index];
            if boundary == previous + 1 && !text[boundary..].trim().is_empty() {
                return Some(boundary);
            }
        }
        let retained = MAX_ACTIVE_LINES.saturating_sub(2).max(1);
        (self.line_boundaries.len() > retained)
            .then(|| self.line_boundaries[self.line_boundaries.len() - retained - 1])
    }

    fn discard_prefix(&mut self, boundary: usize) {
        self.scanned_bytes = self.scanned_bytes.saturating_sub(boundary);
        while self
            .line_boundaries
            .front()
            .is_some_and(|position| *position <= boundary)
        {
            self.line_boundaries.pop_front();
        }
        for position in &mut self.line_boundaries {
            *position = position.saturating_sub(boundary);
        }
    }

    fn reset(&mut self) {
        self.scanned_bytes = 0;
        self.line_boundaries.clear();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UiEvent {
    TurnStarted(u64),
    TurnCancelling(u64),
    TurnCompleted(u64),
    PromptStarted,
    PromptCompleted,
    GoalChanged(GoalPhase),
    ComposerChanged(ComposerState),
    IdleFrameChanged(Option<IdleFrameState>),
    ActivityChanged(ActivityState),
    ActivityCleared,
    ToolReceiptChanged { turn_id: u64, text: String },
    ToolReceiptCommitted { turn_id: u64 },
    FooterChanged(Vec<String>),
    FooterContextChanged(String),
    FooterCleared,
    MarkdownDelta(String),
    AssistantDelta(String),
    AssistantPreviewDelta(String),
    AssistantPreviewCommitted,
    AssistantPreviewDiscarded,
    AssistantCompleted,
    AssistantSegmentCompleted,
    UserHistoryQueued(String),
    HistoryCommitted(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SourceAnchor {
    cell_id: u64,
    source_start: usize,
    source_end: usize,
    logical_line: usize,
    wrap_offset: usize,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ViewportBookmark {
    reading: bool,
    source_digest: Option<String>,
    source_occurrence: usize,
    source_anchor: Option<SourceAnchor>,
    visible_line: Option<String>,
    line_offset: usize,
    line_occurrence: usize,
}

fn source_digest(source: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

pub(crate) fn capture_viewport_bookmark() -> ViewportBookmark {
    // Resolve a just-pressed PgUp/PgDn before persisting its source anchor.
    // Saving only the old painted anchor would lose the user's last scroll.
    paint_pending_frame();
    let state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    let anchor = state.reader_anchor.clone();
    let digest = anchor.as_ref().and_then(|anchor| anchor.cell_id.checked_sub(state.history_first_cell_id))
        .and_then(|index| state.history_tail.get(index as usize)).map(|cell| source_digest(cell.source()));
    let start = state.history_viewport_extent.map(|(_, length, capacity)|
        length.saturating_sub(state.history_scroll).saturating_sub(capacity)).unwrap_or(0);
    let visible = state.history_layout.as_ref().and_then(|(_, _, lines)| lines.iter()
        .skip(start).take(8).enumerate().find(|(_, line)| strip_ansi(line).chars().any(char::is_alphanumeric))
        .map(|(offset, line)| (offset, line.clone())));
    // We already hold the source text here. Counting equal occurrences does
    // not require hashing every history cell (or accepting digest collisions).
    let source_occurrence = anchor.as_ref()
        .and_then(|anchor| anchor.cell_id.checked_sub(state.history_first_cell_id))
        .map_or(0, |index| {
            let Some(source) = state.history_tail.get(index as usize).map(|cell| cell.source()) else { return 0; };
            state.history_tail.iter().take(index as usize)
                .filter(|cell| cell.source() == source).count()
        });
    let line_occurrence = visible.as_ref().map_or(0, |(offset, line)| state.history_layout.as_ref()
        .map_or(0, |(_, _, lines)| lines.iter().take(start + offset).filter(|candidate| *candidate == line).count()));
    ViewportBookmark {
        reading: state.history_reading, source_digest: digest, source_anchor: anchor,
        source_occurrence, line_occurrence,
        visible_line: visible.as_ref().map(|(_, line)| line.clone()),
        line_offset: visible.map_or(0, |(offset, _)| offset),
    }
}

pub(crate) fn restore_viewport_bookmark(bookmark: &ViewportBookmark) -> bool {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    establish_history_tail_baseline(&mut state);
    if !bookmark.reading { return true; }
    let exact = bookmark.source_digest.as_ref().and_then(|digest| state.history_tail.iter().enumerate()
        .filter(|(_, cell)| source_digest(cell.source()) == *digest).nth(bookmark.source_occurrence).map(|(index, _)| index));
    let mut anchor = exact.and_then(|index| bookmark.source_anchor.clone().map(|mut anchor| {
        anchor.cell_id = state.history_first_cell_id + index as u64;
        anchor
    }));
    if anchor.is_none() {
        let width = transcript_width(crossterm::terminal::size().map_or(80, |(columns, _)| columns as usize));
        let mut lines = Vec::new();
        let mut anchors = Vec::new();
        for (index, cell) in state.history_tail.iter().enumerate() {
            let (text, positions) = source_visual_lines(cell, state.history_first_cell_id + index as u64, width);
            lines.extend(text); anchors.extend(positions);
        }
        let mut row = bookmark.visible_line.as_ref().and_then(|line| lines.iter().enumerate()
            .filter(|(_, candidate)| *candidate == line).nth(bookmark.line_occurrence).map(|(index, _)| index));
        // Replay cells may have been composed at a different terminal width.
        // Match the visible source across new wrap boundaries, not an old row
        // number or an equal-width screen string.
        if row.is_none() {
            let normalize = |text: &str| strip_ansi(text).chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
            let normalized = lines.iter().map(|line| normalize(line)).collect::<Vec<_>>();
            let joined = normalized.concat();
            if let Some(needle) = bookmark.visible_line.as_ref().map(|line| normalize(line)).filter(|line| !line.is_empty()) {
                if let Some((offset, _)) = joined.match_indices(&needle).nth(bookmark.line_occurrence) {
                    let mut end = 0;
                    row = normalized.iter().position(|line| { end += line.len(); end > offset });
                }
            }
        }
        if let Some(index) = row {
            anchor = anchors.get(index.saturating_sub(bookmark.line_offset)).cloned();
        }
    }
    if let Some(anchor) = anchor {
        state.reader_anchor = Some(anchor);
        state.history_reading = true;
        state.history_layout = None;
        state.frame_pending = true;
        true
    } else {
        false
    }
}

pub(crate) fn set_restoring_view(restoring: bool) {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    state.restoring_view = restoring;
    if !restoring { state.frame_pending = true; }
}

#[derive(Debug, Clone)]
struct UiState {
    restoring_view: bool,
    session_active: bool,
    turn: TurnPhase,
    goal: GoalPhase,
    composer: Option<ComposerState>,
    activity: Option<ActivityState>,
    tool_receipt: String,
    footer: Vec<String>,
    markdown_source: String,
    active_output: String,
    active_output_boundaries: StreamBoundaryTracker,
    assistant_preview: String,
    assistant_body_started: bool,
    user_history_after_assistant: VecDeque<String>,
    idle_frame: Option<IdleFrameState>,
    prompt_frame: Option<Vec<String>>,
    prompt_scroll: usize,
    deferred_history: VecDeque<TranscriptCell>,
    rendered_tail: Option<(u16, usize)>,
    rendered_rows: usize,
    rendered_geometry: Option<(u16, u16, usize)>,
    painted_viewport: Option<(u16, u16, Vec<String>)>,
    history_tail: VecDeque<TranscriptCell>,
    history_tail_bytes: usize,
    history_cursor_row: Option<u16>,
    prompt_depth: usize,
    frame_pending: bool,
    history_scroll: usize,
    history_reading: bool,
    history_viewport_extent: Option<(usize, usize, usize)>,
    history_revision: u64,
    history_layout: Option<(usize, u64, Vec<String>)>,
    history_anchors: Vec<SourceAnchor>,
    history_cell_ends: Vec<usize>,
    history_first_cell_id: u64,
    reader_anchor: Option<SourceAnchor>,
    frontier_anchor: Option<SourceAnchor>,
    frontier_width: Option<usize>,
    committed_paint_frontier: u64,
    // Source acceptance and terminal presentation are independent. This is a
    // visual-row prefix, including an unfinished Markdown cell, never a slice
    // or rewrite of the canonical source or a business completion signal.
    presented_document_rows: usize,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            restoring_view: false,
            session_active: false,
            turn: TurnPhase::Idle,
            goal: GoalPhase::None,
            composer: None,
            activity: None,
            tool_receipt: String::new(),
            footer: Vec::new(),
            markdown_source: String::new(),
            active_output: String::new(),
            active_output_boundaries: StreamBoundaryTracker::default(),
            assistant_preview: String::new(),
            assistant_body_started: false,
            user_history_after_assistant: VecDeque::new(),
            idle_frame: None,
            prompt_frame: None,
            prompt_scroll: 0,
            deferred_history: VecDeque::new(),
            rendered_tail: None,
            rendered_rows: 0,
            rendered_geometry: None,
            painted_viewport: None,
            history_tail: VecDeque::new(),
            history_tail_bytes: 0,
            history_cursor_row: None,
            prompt_depth: 0,
            frame_pending: false,
            history_scroll: 0,
            history_reading: false,
            history_viewport_extent: None,
            history_revision: 0,
            history_layout: None,
            history_anchors: Vec::new(),
            history_cell_ends: Vec::new(),
            history_first_cell_id: 0,
            reader_anchor: None,
            frontier_anchor: None,
            frontier_width: None,
            committed_paint_frontier: 0,
            presented_document_rows: 0,
        }
    }
}

impl UiState {
    fn owns_terminal(&self, is_terminal: bool) -> bool {
        // Permission and idle input are children of the same session surface,
        // not permission to fall back to an independent stdout renderer.
        is_terminal && self.session_active
    }

    fn assistant_at_line_start(&self) -> bool {
        // Activity/composer rows have their own layout. Only the document's
        // retained body determines whether its next chunk needs indentation.
        let body = if self.assistant_preview.is_empty() {
            &self.active_output
        } else {
            &self.assistant_preview
        };
        body.is_empty() || body.ends_with('\r') || body.ends_with('\n')
    }

    fn turn_id(&self) -> Option<u64> {
        match self.turn {
            TurnPhase::Idle => None,
            TurnPhase::Running { turn_id }
            | TurnPhase::AwaitingPermission { turn_id }
            | TurnPhase::Cancelling { turn_id } => Some(turn_id),
        }
    }

    fn reduce(&mut self, event: UiEvent) -> Vec<TranscriptCell> {
        let mut committed = Vec::new();
        match event {
            UiEvent::TurnStarted(turn_id) => {
                if !self.tool_receipt.is_empty() {
                    committed.push(std::mem::take(&mut self.tool_receipt).into());
                }
                self.turn = TurnPhase::Running { turn_id };
                self.idle_frame = None;
                self.assistant_preview.clear();
                self.assistant_body_started = false;
            }
            UiEvent::TurnCancelling(turn_id) => {
                if matches!(self.turn, TurnPhase::Running { turn_id: active } if active == turn_id)
                {
                    self.turn = TurnPhase::Cancelling { turn_id };
                }
            }
            UiEvent::TurnCompleted(turn_id) => {
                let matching = matches!(
                    self.turn,
                    TurnPhase::Running { turn_id: active }
                        | TurnPhase::AwaitingPermission { turn_id: active }
                        | TurnPhase::Cancelling { turn_id: active }
                        if active == turn_id
                );
                if matching {
                    if !self.markdown_source.is_empty() {
                        committed.push(TranscriptCell::Markdown(std::mem::take(&mut self.markdown_source)));
                    }
                    // A lease can end after an exceptional execution exit that
                    // emitted no model terminal event. Observed tool outcomes
                    // still settle once; an unaccepted text preview does not.
                    if !self.tool_receipt.is_empty() {
                        committed.push(std::mem::take(&mut self.tool_receipt).into());
                    }
                    // AssistantCompleted is the sole durable assistant-body boundary.
                    // TurnCompleted only transfers terminal ownership back to
                    // the idle composer. Committing here as well makes a late
                    // lifecycle event replay the same answer a second time.
                    self.active_output.clear();
                    self.active_output_boundaries.reset();
                    self.assistant_preview.clear();
                    self.turn = TurnPhase::Idle;
                    self.activity = None;
                }
            }
            UiEvent::PromptStarted => {
                self.prompt_depth = self.prompt_depth.saturating_add(1);
                let turn_id = match self.turn {
                    TurnPhase::Running { turn_id } | TurnPhase::Cancelling { turn_id } => {
                        Some(turn_id)
                    }
                    TurnPhase::Idle | TurnPhase::AwaitingPermission { .. } => None,
                };
                if let Some(turn_id) = turn_id {
                    self.turn = TurnPhase::AwaitingPermission { turn_id };
                }
            }
            UiEvent::PromptCompleted => {
                self.prompt_frame = None;
                self.prompt_depth = self.prompt_depth.saturating_sub(1);
                if self.prompt_depth == 0 {
                    if let TurnPhase::AwaitingPermission { turn_id } = self.turn {
                        self.turn = TurnPhase::Running { turn_id };
                    }
                }
            }
            UiEvent::GoalChanged(goal) => self.goal = goal,
            UiEvent::ComposerChanged(mut composer) => {
                composer.queued_count = composer
                    .queued_count
                    .max(self.user_history_after_assistant.len());
                self.composer = Some(composer);
            }
            UiEvent::IdleFrameChanged(frame) => {
                self.idle_frame = frame;
                if self.idle_frame.is_some() {
                    // The idle editor and the running turn used to draw two
                    // independent bottom surfaces. Keep one projection owner:
                    // an idle frame already contains its composer, footer and
                    // completion row, so stale running-state rows must not be
                    // rendered underneath it.
                    self.composer = None;
                    self.activity = None;
                }
            }
            UiEvent::ActivityChanged(activity) => self.activity = Some(activity),
            UiEvent::ActivityCleared => {
                // Activity is presentation state, not a transcript receipt.
                // Only explicit history/tool events may commit durable rows.
                self.activity = None;
            }
            UiEvent::ToolReceiptChanged { turn_id, text } => {
                if self.turn_id() == Some(turn_id) {
                    self.tool_receipt = text;
                }
            }
            UiEvent::ToolReceiptCommitted { turn_id } => {
                // Settle the completed semantic block at its existing transcript
                // anchor, independently of the spinner and composer lifecycle.
                if self.turn_id() == Some(turn_id) && !self.tool_receipt.is_empty() {
                    committed.push(std::mem::take(&mut self.tool_receipt).into());
                }
            }
            UiEvent::FooterChanged(lines) => self.footer = lines,
            UiEvent::FooterContextChanged(usage) => crate::agent::merge_footer_context_usage(&mut self.footer, usage),
            UiEvent::FooterCleared => self.footer.clear(),
            UiEvent::MarkdownDelta(text) => {
                let scan_boundary = text.contains('\n') || self.markdown_source.ends_with("\n\n");
                self.markdown_source.push_str(&text);
                let boundary = if scan_boundary { crate::terminal_document::stable_boundary(&self.markdown_source) } else { 0 };
                if boundary > 0 {
                    committed.extend(crate::terminal_document::stable_cells(self.markdown_source.drain(..boundary).collect()));
                }
            }
            UiEvent::AssistantDelta(text) => {
                let visible = if self.assistant_body_started {
                    text.as_str()
                } else {
                    text.trim_start_matches(['\r', '\n'])
                };
                self.assistant_body_started |= !visible.is_empty();
                self.active_output.push_str(visible);
                if let Some(boundary) = self
                    .active_output_boundaries
                    .stable_prefix(&self.active_output)
                {
                    committed.push(self.active_output.drain(..boundary).collect::<String>().into());
                    self.active_output_boundaries.discard_prefix(boundary);
                }
            }
            UiEvent::AssistantPreviewDelta(text) => {
                let visible = if self.assistant_body_started || !self.assistant_preview.is_empty() {
                    text.as_str()
                } else {
                    text.trim_start_matches(['\r', '\n'])
                };
                self.assistant_preview.push_str(visible);
            }
            UiEvent::AssistantPreviewCommitted => {
                // Approval of one journal part is not a paragraph boundary:
                // a recovered provider stream can continue this same text.
                self.assistant_body_started |= !self.assistant_preview.is_empty();
                self.active_output.push_str(&self.assistant_preview);
                self.assistant_preview.clear();
                if let Some(boundary) = self
                    .active_output_boundaries
                    .stable_prefix(&self.active_output)
                {
                    committed.push(self.active_output.drain(..boundary).collect::<String>().into());
                    self.active_output_boundaries.discard_prefix(boundary);
                }
            }
            UiEvent::AssistantPreviewDiscarded => {
                self.assistant_preview.clear();
                self.markdown_source.clear();
            }
            UiEvent::AssistantSegmentCompleted => {
                if !self.markdown_source.is_empty() {
                    committed.push(TranscriptCell::Markdown(std::mem::take(&mut self.markdown_source)));
                }
                // A review/tool boundary settles the body without completing
                // the turn or consuming a queued user follow-up.
                if !self.active_output.is_empty() {
                    committed.push(std::mem::take(&mut self.active_output).into());
                }
                self.active_output_boundaries.reset();
                self.assistant_body_started = false;
            }
            UiEvent::AssistantCompleted => {
                if !self.markdown_source.is_empty() {
                    committed.push(TranscriptCell::Markdown(std::mem::take(&mut self.markdown_source)));
                }
                if !self.tool_receipt.is_empty() {
                    committed.push(std::mem::take(&mut self.tool_receipt).into());
                }
                self.assistant_body_started |= !self.assistant_preview.is_empty();
                self.active_output.push_str(&self.assistant_preview);
                self.assistant_preview.clear();
                if !self.active_output.is_empty() {
                    committed.push(std::mem::take(&mut self.active_output).into());
                }
                self.active_output_boundaries.reset();
                if let Some(user_history) = self.user_history_after_assistant.pop_front() {
                    committed.push(user_history.into());
                    if let Some(composer) = self.composer.as_mut() {
                        composer.queued_count = composer.queued_count.saturating_sub(1);
                    }
                }
            }
            UiEvent::UserHistoryQueued(text) => {
                self.user_history_after_assistant.push_back(text);
                if let Some(composer) = self.composer.as_mut() {
                    composer.queued_count = composer
                        .queued_count
                        .max(self.user_history_after_assistant.len());
                }
            }
            UiEvent::HistoryCommitted(text) => committed.push(text.into()),
        }
        committed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    lines: Vec<String>,
    cursor: Option<(u16, u16)>,
}

static UI: OnceLock<Mutex<UiState>> = OnceLock::new();
static LAST_WRAPPED_BLOCK: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn shared_state() -> &'static Mutex<UiState> {
    UI.get_or_init(|| Mutex::new(UiState::default()))
}

pub(crate) fn begin_turn(turn_id: u64) {
    dispatch(UiEvent::TurnStarted(turn_id));
}

pub(crate) fn mark_turn_cancelling(turn_id: u64) {
    dispatch(UiEvent::TurnCancelling(turn_id));
}

pub(crate) fn end_turn(turn_id: u64) {
    dispatch(UiEvent::TurnCompleted(turn_id));
}

pub(crate) fn set_goal_active(goal_id: impl Into<String>) {
    dispatch(UiEvent::GoalChanged(GoalPhase::Active {
        goal_id: goal_id.into(),
    }));
}

pub(crate) fn set_goal_completed(goal_id: impl Into<String>) {
    dispatch(UiEvent::GoalChanged(GoalPhase::Completed {
        goal_id: goal_id.into(),
    }));
}

pub(crate) fn set_goal_blocked(goal_id: impl Into<String>) {
    dispatch(UiEvent::GoalChanged(GoalPhase::Blocked {
        goal_id: goal_id.into(),
    }));
}

pub(crate) fn pause_for_prompt() {
    dispatch(UiEvent::PromptStarted);
}

pub(crate) fn resume_after_prompt() {
    dispatch(UiEvent::PromptCompleted);
}

pub(crate) fn update_composer(
    draft: impl Into<String>,
    cursor: usize,
    queued_count: usize,
    placeholder: impl Into<String>,
) {
    let draft = draft.into();
    dispatch(UiEvent::ComposerChanged(ComposerState {
        cursor: cursor.min(draft.len()),
        draft,
        queued_count,
        placeholder: placeholder.into(),
    }));
}

pub(crate) fn sync_composer(cancellation: &crate::interaction::ExecutionCancellation) {
    let snapshot = cancellation.input_snapshot();
    update_composer(
        snapshot.draft,
        snapshot.cursor,
        snapshot.queued_count,
        crate::i18n::tr(crate::i18n::Message::InteractiveInputPlaceholder),
    );
}

/// Start one interactive terminal session from a deterministic viewport.
///
/// Cursor-position reports (`CSI 6 n`) are not universally supported. In
/// particular, pipes and several PTY harnesses never answer them, which used
/// to block the editor before it had accepted the first command. The inline
/// renderer owns the viewport, so establish row one as the durable transcript
/// origin instead of asking the terminal where the previous shell prompt was.
pub(crate) fn begin_interactive_session() {
    let mut state = shared_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *state = UiState::default();
    state.session_active = io::stdout().is_terminal();
    state.history_cursor_row = Some(1);
    if io::stdout().is_terminal() {
        let stdout = io::stdout();
        let mut stdout = io::BufWriter::new(stdout.lock());
        let _ = write!(stdout, "\x1b[?1049h\x1b[r\x1b[2J\x1b[H\x1b[?25h");
        let _ = stdout.flush();
    }
}

pub(crate) fn owns_terminal() -> bool {
    let state = shared_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.owns_terminal(io::stdout().is_terminal())
}

/// A temporary screen (such as the resume picker) has overwritten the TTY.
/// Keep the transcript and composer, but repaint instead of diffing against it.
pub(crate) fn invalidate_presented_screen() {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    state.painted_viewport = None;
    state.frame_pending = true;
    paint_pending(&mut state);
}

/// Called by the interactive session guard on every exit, including errors.
/// A turn finishing must not release ownership; only the session ending does.
pub(crate) fn end_interactive_session() {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    if state.owns_terminal(io::stdout().is_terminal()) {
        if let Ok((columns, rows)) = crossterm::terminal::size() {
            if columns > 0 && rows > 0 {
                let mut stdout = io::stdout().lock();
                let _ = write_pending_delivery_frames(&mut state, columns, rows, &mut stdout);
                let mut output = Vec::new();
                clear_transcript_tail(&mut state, &mut output);
                clear_rows(&mut output, rows, state.rendered_rows);
                let row = state.history_cursor_row.unwrap_or(rows).clamp(1, rows);
                let _ = write!(output, "\x1b[?6l\x1b[r\x1b[0m\x1b[?7h\x1b[{row};1H\x1b[?25h\x1b[?1049l");
                let _ = stdout.write_all(&output);
                let _ = stdout.flush();
            }
        }
    }
    *state = UiState::default();
}

pub(crate) fn has_active_document() -> bool {
    let state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    state.owns_terminal(io::stdout().is_terminal()) && state.turn_id().is_some()
}

/// Transfer the idle bottom surface to the interactive editor without first
/// erasing it. Both renderers project the same composer and footer rows; only
/// their retained geometry must change hands. This keeps the input surface
/// continuously visible and prevents the next turn from clearing the submitted
/// user line as if it were stale live UI.
pub(crate) fn release_idle_surface() -> usize {
    let mut state = shared_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !matches!(state.turn, TurnPhase::Idle) {
        return 0;
    }
    let rendered_rows = state.rendered_rows;
    state.rendered_rows = 0;
    state.composer = None;
    rendered_rows
}

pub(crate) fn set_activity(
    key: impl Into<String>,
    glyph: &str,
    detail: &str,
) {
    dispatch(UiEvent::ActivityChanged(ActivityState {
        key: key.into(),
        glyph: glyph.to_string(),
        detail: detail.trim().to_string(),
    }));
}

pub(crate) fn clear_activity() {
    dispatch(UiEvent::ActivityCleared);
}

pub(crate) fn update_tool_receipt(turn_id: u64, text: String) {
    dispatch(UiEvent::ToolReceiptChanged { turn_id, text });
}

pub(crate) fn commit_tool_receipt(turn_id: u64) {
    dispatch(UiEvent::ToolReceiptCommitted { turn_id });
}

pub(crate) fn transcript_width(columns: usize) -> usize {
    columns.saturating_sub(2).max(1).min(MAX_CONTENT_WIDTH)
}

pub(crate) fn set_footer(text: &str) {
    dispatch(UiEvent::FooterChanged(
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect(),
    ));
}

pub(crate) fn clear_footer() {
    dispatch(UiEvent::FooterCleared);
}

pub(crate) fn update_footer_context(usage: String) {
    dispatch(UiEvent::FooterContextChanged(usage));
}

pub(crate) fn append_assistant(text: &str) {
    if !text.is_empty() {
        dispatch(UiEvent::AssistantDelta(text.to_string()));
    }
}

pub(crate) fn append_markdown(text: &str) {
    if !text.is_empty() { dispatch(UiEvent::MarkdownDelta(text.to_owned())); }
}

pub(crate) fn assistant_at_line_start() -> bool {
    shared_state().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
        .assistant_at_line_start()
}

pub(crate) fn append_assistant_preview(text: &str) {
    if !text.is_empty() {
        dispatch(UiEvent::AssistantPreviewDelta(text.to_string()));
    }
}

pub(crate) fn commit_assistant_preview() {
    dispatch(UiEvent::AssistantPreviewCommitted);
}

pub(crate) fn discard_assistant_preview() {
    dispatch(UiEvent::AssistantPreviewDiscarded);
}

pub(crate) fn finish_assistant() {
    dispatch(UiEvent::AssistantCompleted);
}

pub(crate) fn finish_assistant_segment() {
    dispatch(UiEvent::AssistantSegmentCompleted);
}

pub(crate) fn commit_history(text: &str) {
    if !text.trim().is_empty() {
        dispatch(UiEvent::HistoryCommitted(text.to_string()));
    }
}

/// Restored, completed history starts at its tail; it is not a new stream.
/// This establishes only a viewport baseline, never a presentation receipt.
/// Replace a replay, rather than appending a second copy of the same archive.
/// Keep composer and live stream ownership untouched.
pub(crate) fn clear_replayed_history() {
    let mut state = shared_state().lock().unwrap_or_else(|error| error.into_inner());
    state.history_tail.clear();
    state.history_tail_bytes = 0;
    state.history_cell_ends.clear();
    state.history_viewport_extent = None;
    state.history_revision = state.history_revision.wrapping_add(1);
    state.presented_document_rows = 0;
    state.deferred_history.clear();
    state.history_first_cell_id = 0;
    state.history_layout = None;
    state.history_anchors.clear();
    state.history_scroll = 0;
    state.history_reading = false;
    state.reader_anchor = None;
    state.frontier_anchor = None;
    state.frontier_width = None;
    state.committed_paint_frontier = 0;
    state.frame_pending = true;
}

pub(crate) fn establish_restored_history_tail() {
    let mut state = shared_state().lock().unwrap_or_else(|error| error.into_inner());
    if establish_history_tail_baseline(&mut state) {
        crate::terminal_frames::request();
    }
}

fn establish_history_tail_baseline(state: &mut UiState) -> bool {
    if !state.session_active || state.turn != TurnPhase::Idle
        || !state.markdown_source.is_empty() || !state.active_output.is_empty()
        || !state.assistant_preview.is_empty() || !state.user_history_after_assistant.is_empty()
    {
        return false;
    }
    while let Some(cell) = state.deferred_history.pop_front() {
        remember_cell(state, cell);
    }
    state.committed_paint_frontier = state.history_first_cell_id + state.history_tail.len() as u64;
    // The next layout resolves this immutable-cell frontier at the actual
    // width. Do not guess a visual row count or rewrite the retained source.
    state.frontier_anchor = None;
    state.frontier_width = None;
    state.history_scroll = 0;
    state.history_reading = false;
    state.reader_anchor = None;
    state.frame_pending = true;
    true
}

/// Synchronous presentation boundary for durable notifications. Ordinary
/// dispatch is best-effort and cannot authorize a persistent delivery ACK.
pub use crate::task_runtime::NotificationPresentation as HistoryPresentation;

pub(crate) fn deliver_history(text: &str) -> io::Result<HistoryPresentation> {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    let mut stdout = io::stdout().lock();
    if !state.owns_terminal(stdout.is_terminal()) {
        write_delivered_frame(&mut stdout, format!("{text}\n").as_bytes())?;
        return Ok(HistoryPresentation::Presented);
    }
    let (columns, rows) = crossterm::terminal::size()?;
    present_history_in_viewport(&mut state, &mut stdout, text, columns, rows)
}

fn present_history_in_viewport(state: &mut UiState, output: &mut impl Write,
    text: &str, columns: u16, rows: u16) -> io::Result<HistoryPresentation> {
    if state.prompt_depth > 0 && state.prompt_frame.is_none() {
        return Ok(HistoryPresentation::Deferred);
    }
    if columns == 0 || rows == 0 {
        return Ok(HistoryPresentation::Deferred);
    }
    if state.history_reading || state.history_scroll > 0
        || usize::from(rows) <= build_frame(state, columns, rows).lines.len()
    {
        return Ok(HistoryPresentation::Deferred);
    }
    // Stage presentation state while holding the owner lock. A partial write
    // or failed flush is not a delivery and must leave the retryable source
    // unchanged (rather than append another copy on the next notification).
    let mut presented = state.clone();
    let committed = presented.reduce(UiEvent::HistoryCommitted(text.into()));
    presented.deferred_history.extend(committed);
    // This caller emits a real delivery receipt. A lifecycle barrier may leave
    // animation pending, but delivery must write every preceding committed
    // frame before its caller can ACK the notification.
    if let Err(error) = write_pending_delivery_frames(&mut presented, columns, rows, output) {
        state.painted_viewport = None;
        return Err(error);
    }
    *state = presented;
    Ok(HistoryPresentation::Presented)
}

fn write_delivered_frame(output: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    output.write_all(bytes)?;
    output.flush()
}

pub(crate) fn queue_user_history_after_assistant(text: String) {
    if !text.trim().is_empty() {
        dispatch(UiEvent::UserHistoryQueued(text));
    }
}

pub(crate) fn present_idle_frame(lines: Vec<String>, cursor_row: usize, cursor_column: u16) {
    dispatch(UiEvent::IdleFrameChanged(Some(IdleFrameState {
        lines,
        cursor_row,
        cursor_column,
    })));
}

pub(crate) fn clear_idle_frame() {
    dispatch(UiEvent::IdleFrameChanged(None));
}

pub(crate) fn present_permission_frame(text: &str, focus_selection: bool) -> io::Result<()> {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    if state.prompt_frame.is_none() {
        state.prompt_scroll = 0;
    }
    state.prompt_frame = Some(text.lines().map(str::to_string).collect());
    if focus_selection {
        let rows = crossterm::terminal::size()?.1;
        let visible = usize::from(rows.saturating_sub(2).max(1));
        if let Some(index) = state
            .prompt_frame
            .as_ref()
            .and_then(|lines| lines.iter().position(|line| line.contains('›')))
        {
            if index < state.prompt_scroll {
                state.prompt_scroll = index;
            }
            if index >= state.prompt_scroll + visible {
                state.prompt_scroll = index + 1 - visible;
            }
        }
    }
    paint_pending(&mut state);
    io::stdout().flush()
}

pub(crate) fn scroll_permission_frame(down: bool) {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    let rows = crossterm::terminal::size().map_or(24, |size| size.1);
    let step = usize::from(rows.saturating_sub(3).max(1));
    let max = state
        .prompt_frame
        .as_ref()
        .map_or(0, |lines| lines.len().saturating_sub(step + 1));
    state.prompt_scroll = if down {
        state.prompt_scroll.saturating_add(step).min(max)
    } else {
        state.prompt_scroll.saturating_sub(step)
    };
    paint_pending(&mut state);
}

pub(crate) fn clear_permission_frame() -> bool {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    if state.prompt_frame.take().is_none() {
        return false;
    }
    state.frame_pending = false;
    clear_surface(&mut state);
    true
}

fn dispatch(event: UiEvent) {
    // Terminal handoffs are barriers; ordinary deltas only invalidate a frame.
    let barrier = matches!(event, UiEvent::PromptStarted | UiEvent::PromptCompleted
        | UiEvent::TurnCompleted(_) | UiEvent::IdleFrameChanged(None)
        | UiEvent::AssistantCompleted | UiEvent::HistoryCommitted(_));
    let terminal = io::stdout().is_terminal();
    let mut state = shared_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let was_prompting = state.prompt_depth > 0;
    let committed = state.reduce(event);
    if !state.owns_terminal(terminal) {
        for block in committed {
            print_block(&block.render(100));
        }
        return;
    }
    if !was_prompting && state.prompt_depth > 0 {
        state.deferred_history.extend(committed);
        paint_pending_with_mode(&mut state, true);
        clear_surface(&mut state);
        return;
    }
    if state.prompt_depth > 0 && state.prompt_frame.is_none() {
        // External line-mode prompts cannot share the renderer, but their
        // committed output must still survive the pause and a later resize.
        state.deferred_history.extend(committed);
        return;
    }
    state.deferred_history.extend(committed);
    state.frame_pending = true;
    if barrier { paint_pending_with_mode(&mut state, true); }
    else { crate::terminal_frames::request(); }
}

pub(crate) fn paint_pending_frame() {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    if state.session_active && state.frame_pending && (state.prompt_depth == 0 || state.prompt_frame.is_some()) {
        paint_pending(&mut state);
    }
}

fn paint_pending(state: &mut UiState) {
    paint_pending_with_mode(state, false);
}

fn paint_pending_with_mode(state: &mut UiState, drain: bool) {
    if state.restoring_view { return; }
    let Ok((columns, rows)) = crossterm::terminal::size() else { return };
    if columns == 0 || rows == 0 { return; }
    let before = (state.frontier_anchor.clone(),state.frontier_width,state.committed_paint_frontier,state.reader_anchor.clone(),
        state.presented_document_rows,state.history_viewport_extent,state.history_scroll);
    let output = pending_frame_bytes(state, columns, rows, drain);
    let mut stdout = io::stdout().lock();
    if stdout.write_all(&output).and_then(|_|stdout.flush()).is_ok() {
        evict_painted_history(state);
    } else {
        (state.frontier_anchor,state.frontier_width,state.committed_paint_frontier,state.reader_anchor,state.presented_document_rows,
            state.history_viewport_extent,state.history_scroll) = before;
        state.painted_viewport = None; // a partial write makes all screen cells uncertain
        state.frame_pending = true;
    }
    if state.frame_pending {
        crate::terminal_frames::request();
    }
}

// Source cells remain complete. Active-session frames admit only a bounded
// visual prefix, including at lifecycle barriers; the scheduler paints the
// remaining prefix independently of business completion. The non-session
// transport path retains its cell-at-a-time/drain distinction.
fn pending_frame_bytes(state: &mut UiState, columns: u16, rows: u16, drain: bool) -> Vec<u8> {
    let mut output = b"\x1b[?2026h".to_vec();
    if state.session_active {
        while let Some(cell) = state.deferred_history.pop_front() {
            remember_cell(state, cell);
        }
        render_session_viewport(state, &mut output, columns, rows);
        output.extend_from_slice(b"\x1b[?2026l");
        return output;
    }
    while let Some(block) = state.deferred_history.pop_front() {
        write_cell_to(state, &block, &mut output, columns, rows);
        if !drain { break; }
    }
    render_surface_to(state, &mut output, columns, rows);
    output.extend_from_slice(b"\x1b[?2026l");
    state.frame_pending = !state.deferred_history.is_empty();
    output
}

/// A transport delivery barrier, not an ordinary model/lifecycle redraw.
/// Each prefix is a complete synchronized frame; no per-token producer sleeps
/// and no ACK is returned merely because source data was queued in memory.
fn write_pending_delivery_frames(state: &mut UiState, columns: u16, rows: u16,
    output: &mut impl Write) -> io::Result<()> {
    loop {
        output.write_all(&pending_frame_bytes(state, columns, rows, true))?;
        if !state.frame_pending {
            output.flush()?;
            evict_painted_history(state);
            return Ok(());
        }
    }
}

#[cfg(test)]
fn delivered_pending_frames(state: &mut UiState, columns: u16, rows: u16) -> Vec<u8> {
    let mut output = Vec::new();
    write_pending_delivery_frames(state, columns, rows, &mut output).unwrap();
    output
}

fn source_visual_lines(cell: &TranscriptCell, cell_id: u64, width: usize) -> (Vec<String>,Vec<SourceAnchor>) {
    let blocks = match cell {
        TranscriptCell::Markdown(source) => crate::agent::render_markdown_document_with_source(source,width),
        TranscriptCell::Text(source) => {
            let mut offset = 0;
            source.split_inclusive('\n').map(|line| {
                let start = offset; offset += line.len();
                crate::agent::MarkdownSourceBlock {text:line.into(),source_start:start,source_end:offset}
            }).collect()
        },
    };
    let mut lines = Vec::new();
    let mut anchors = Vec::new();
    for block in blocks {
        // A width-dependent table owns a complete source block. Reflow may
        // conservatively restore its start, never infer a different source row
        // by searching repeated rendered words or reusing old visual ordinals.
        let coarse = cell.source()[block.source_start..block.source_end].lines().count() > 1;
        let normalized = block.text.replace("\r\n","\n").replace('\r',"\n");
        for (logical_line,line) in normalized.split_terminator('\n').enumerate() {
            for (text,offset) in wrap_styled_line_with_offsets(&safe_styled_line(line),width.max(1)) {
                lines.push(text);
                anchors.push(SourceAnchor {cell_id,source_start:block.source_start,source_end:block.source_end,
                    logical_line:if coarse {0} else {logical_line},wrap_offset:if coarse {0} else {offset}});
            }
        }
    }
    (lines,anchors)
}

fn normalize_source_anchor(state: &UiState, anchor: &SourceAnchor) -> SourceAnchor {
    let mut anchor = anchor.clone();
    // A mutable Markdown source becomes consecutive immutable source cells.
    // Carry its exact byte position across that split, never match by text.
    if anchor.cell_id >= state.history_first_cell_id && anchor.cell_id != u64::MAX {
        let mut index = anchor.cell_id.saturating_sub(state.history_first_cell_id) as usize;
        while let Some(cell) = state.history_tail.get(index) {
            let bytes = cell.source().len();
            if anchor.source_start < bytes { break; }
            anchor.source_start -= bytes;
            anchor.source_end = anchor.source_end.saturating_sub(bytes);
            anchor.cell_id += 1;
            index += 1;
        }
    }
    anchor
}

fn project_source_anchor(rows: &[&SourceAnchor], anchor: &SourceAnchor) -> Option<usize> {
    let mut best: Option<(usize,usize)> = None;
    for (index,candidate) in rows.iter().enumerate() {
        if candidate.cell_id == anchor.cell_id && candidate.source_start == anchor.source_start
            && candidate.source_end >= anchor.source_end && candidate.logical_line == anchor.logical_line
            && candidate.wrap_offset <= anchor.wrap_offset
            && best.is_none_or(|(_,offset)|candidate.wrap_offset > offset) {
            best = Some((index,candidate.wrap_offset));
        }
    }
    best.map(|(index,_)|index)
}

// Interactive chrome never enters terminal scrollback. The document owns
// history; the alternate screen is only a projection of its current viewport.
fn render_session_viewport(state: &mut UiState, output: &mut impl Write, columns: u16, rows: u16) {
    let frame = build_frame(state, columns, rows);
    let capacity = usize::from(rows).saturating_sub(frame.lines.len());
    let width = transcript_width(usize::from(columns));
    let layout_changed = !state.history_layout.as_ref().is_some_and(|(w, revision, _)| *w == width && *revision == state.history_revision);
    let width_changed = state.frontier_width != Some(width);
    if layout_changed {
        let mut lines = Vec::new();
        state.history_anchors.clear();
        state.history_cell_ends.clear();
        for (index,cell) in state.history_tail.iter().enumerate() {
            let (cell_lines,anchors) = source_visual_lines(cell,state.history_first_cell_id+index as u64,width);
            lines.extend(cell_lines);
            state.history_anchors.extend(anchors);
            state.history_cell_ends.push(lines.len());
        }
        state.history_layout = Some((width, state.history_revision, lines));
    }
    let mut tail = state.tool_receipt.clone();
    if let Some(activity) = &state.activity {
        if !tail.is_empty() { tail.push('\n'); }
        tail.push_str(&activity.render(width));
    }
    if !tail.is_empty() { tail.push('\n'); }
    tail.push_str(&state.active_output);
    tail.push_str(&state.assistant_preview);
    let prefix = tail.clone();
    tail.push_str(&crate::terminal_document::render_markdown(&state.markdown_source, width));
    let tail_lines = visual_lines(&tail, width);
    let (_,mut tail_anchors) = source_visual_lines(&TranscriptCell::Text(prefix.clone()),u64::MAX,width);
    let (_,markdown_anchors) = source_visual_lines(&TranscriptCell::Markdown(state.markdown_source.clone()),
        state.history_first_cell_id+state.history_tail.len() as u64,width);
    tail_anchors.extend(markdown_anchors);
    if tail_anchors.len() != tail_lines.len() {
        // Ephemeral activity/preview can share a row with the Markdown tail.
        // That synthetic mixed row has only a block-level UI anchor, never a
        // guessed byte position in canonical source.
        tail_anchors = vec![SourceAnchor {cell_id:u64::MAX,source_start:0,source_end:prefix.len(),
            logical_line:0,wrap_offset:0};tail_lines.len()];
    }
    let anchors = state.history_anchors.iter().chain(tail_anchors.iter()).collect::<Vec<_>>();
    let history = &state.history_layout.as_ref().expect("history layout prepared").2;
    let source_length = history.len() + tail_lines.len();
    // Reading owns its anchor. Do not keep scheduling invisible progress or
    // spin when chrome occupies the entire terminal.
    let can_advance = !state.history_reading && state.history_scroll == 0 && capacity > 0;
    let budget = if can_advance { MAX_VIEWPORT_ROWS_PER_FRAME.min(capacity) } else { 0 };
    let completed = state.committed_paint_frontier.saturating_sub(state.history_first_cell_id) as usize;
    let committed_rows = completed.checked_sub(1).and_then(|index|state.history_cell_ends.get(index)).copied().unwrap_or(0);
    let remapped_frontier = if layout_changed || width_changed { state.frontier_anchor.as_ref().and_then(|anchor|
        project_source_anchor(&anchors,&normalize_source_anchor(state,anchor)))
        .map(|row|row+usize::from(state.frontier_width==Some(width)))
        .unwrap_or(committed_rows).max(committed_rows) } else {state.presented_document_rows};
    state.presented_document_rows = remapped_frontier.min(source_length)
        .saturating_add(budget).min(source_length);
    let length = state.presented_document_rows;
    state.frame_pending = can_advance && length < source_length;
    state.history_reading |= state.history_scroll > 0;
    let reader_start = if state.history_reading && (layout_changed || width_changed) { state.reader_anchor.as_ref().and_then(|anchor|
        project_source_anchor(&anchors,&normalize_source_anchor(state,anchor))) } else {None};
    if state.history_reading && reader_start.is_none() {
        if let Some((previous_width, previous_length, previous_capacity)) = state.history_viewport_extent {
            if previous_width == width {
                // PgUp owns a document position, not a distance from the moving
                // stream tail. Keep that position through append and completion.
                let start = previous_length.saturating_sub(state.history_scroll)
                    .saturating_sub(previous_capacity);
                state.history_scroll = length.saturating_sub(start.saturating_add(capacity));
            }
        }
    }
    state.history_scroll = state.history_scroll.min(length.saturating_sub(capacity));
    state.history_viewport_extent = Some((width, length, capacity));
    let end = reader_start.map(|start|start.saturating_add(capacity).min(length))
        .unwrap_or_else(||length.saturating_sub(state.history_scroll));
    let start = reader_start.unwrap_or_else(||end.saturating_sub(capacity)).min(end);
    state.history_scroll = length.saturating_sub(end);
    let retained_reader = state.reader_anchor.as_ref().map(|anchor|normalize_source_anchor(state,anchor))
        .filter(|anchor|project_source_anchor(&anchors,anchor).is_some());
    // A wider line can place the intent character in the middle of a visual
    // row. Keep the original source/wrap intent instead of replacing it with
    // that row's floor and progressively drifting on each subsequent resize.
    state.reader_anchor = if state.history_reading { retained_reader.or_else(||anchors.get(start)
        .map(|anchor|(**anchor).clone())) } else {None};
    state.frontier_anchor = length.checked_sub(1).and_then(|last|anchors.get(last)).map(|anchor|(**anchor).clone());
    state.frontier_width = Some(width);
    state.committed_paint_frontier = state.history_first_cell_id
        + state.history_cell_ends.iter().take_while(|end|**end<=length).count() as u64;
    let visible = history.iter().chain(tail_lines.iter()).skip(start).take(end - start).collect::<Vec<_>>();
    let _ = write!(output, "\x1b[?6l\x1b[r\x1b[?25l");
    let screen = (0..capacity).map(|index| visible.get(index).map(|line| (*line).clone()).unwrap_or_default())
        .chain(frame.lines.iter().cloned()).collect::<Vec<_>>();
    let previous = state.painted_viewport.as_ref().filter(|(w, h, _)| *w == columns && *h == rows);
    for (index, line) in screen.iter().enumerate() {
        if previous.and_then(|(_, _, lines)| lines.get(index)) != Some(line) {
            paint_row(output, index as u16 + 1, columns, line);
        }
    }
    state.painted_viewport = Some((columns, rows, screen));
    if let Some((row, column)) = frame.cursor {
        let _ = write!(output, "\x1b[{row};{column}H\x1b[6 q\x1b[?25h");
    }
    state.rendered_rows = frame.lines.len();
    state.rendered_tail = None;
}

fn navigate_history(state: &mut UiState, key: &crossterm::event::KeyEvent, page: usize) -> bool {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    if key.kind == KeyEventKind::Release || !state.session_active || state.prompt_depth > 0 { return false; }
    let control = key.modifiers == KeyModifiers::CONTROL;
    match key.code {
        KeyCode::PageUp if key.modifiers.is_empty() => state.history_scroll = state.history_scroll.saturating_add(page),
        KeyCode::PageDown if key.modifiers.is_empty() => state.history_scroll = state.history_scroll.saturating_sub(page),
        KeyCode::Home if control => state.history_scroll = state.presented_document_rows,
        KeyCode::End if control => state.history_scroll = 0,
        _ => return false,
    }
    state.history_reading = state.history_scroll > 0;
    state.reader_anchor = None;
    state.frame_pending = true;
    true
}

pub(crate) fn scroll_history_key(key: &crossterm::event::KeyEvent) -> bool {
    let mut state = shared_state().lock().unwrap_or_else(|e| e.into_inner());
    let page = state.history_viewport_extent.map_or(10, |(_, _, capacity)| capacity.saturating_sub(1).max(1));
    if !navigate_history(&mut state, key, page) { return false; }
    crate::terminal_frames::request();
    true
}

fn stopping_hint() -> &'static str {
    use crate::i18n::Language;
    match crate::i18n::current_language() {
        Language::ZhHans => "正在停止 · 输入草稿会保留",
        Language::En => "Stopping · your draft is preserved",
        Language::De => "Wird beendet · Entwurf bleibt erhalten",
        Language::Es => "Deteniendo · se conserva el borrador",
        Language::Fr => "Arrêt en cours · brouillon conservé",
        Language::Ja => "停止中 · 下書きは保持されます",
        Language::Ru => "Остановка · черновик сохранён",
    }
}

fn local_command_hint() -> &'static str {
    use crate::i18n::Language;
    match crate::i18n::current_language() {
        Language::ZhHans => "Enter 执行本地指令 · 不进入消息队列",
        Language::En => "Enter: local command · never queued as a message",
        Language::De => "Enter: lokaler Befehl · keine Nachrichtenwarteschlange",
        Language::Es => "Enter: comando local · no se encola como mensaje",
        Language::Fr => "Entrée : commande locale · aucun message en attente",
        Language::Ja => "Enter: ローカルコマンド · メッセージとして予約しません",
        Language::Ru => "Enter: локальная команда · не сообщение в очереди",
    }
}

fn history_reading_hint() -> &'static str {
    use crate::i18n::Language;
    match crate::i18n::current_language() {
        Language::ZhHans => "Ctrl+End 返回最新 · 浏览历史 · PgUp/PgDn",
        Language::En => "Ctrl+End: latest · Reading history · PgUp/PgDn",
        Language::De => "Strg+Ende: neueste · Verlauf · Bild↑/Bild↓",
        Language::Es => "Ctrl+Fin: reciente · Historial · RePág/AvPág",
        Language::Fr => "Ctrl+Fin : récent · Historique · PgUp/PgDn",
        Language::Ja => "Ctrl+End: 最新へ · 履歴表示 · PgUp/PgDn",
        Language::Ru => "Ctrl+End: к последнему · История · PgUp/PgDn",
    }
}

fn build_frame(state: &UiState, columns: u16, rows: u16) -> Frame {
    let content_width = usize::from(columns.saturating_sub(2).max(1)).min(MAX_CONTENT_WIDTH);
    let reading_hint = history_reading_hint().to_string();
    let footer = state.history_reading.then_some(&reading_hint).into_iter().chain(state.footer.iter()).take(MAX_FOOTER_LINES)
        .take(usize::from(rows.saturating_sub(2)))
        .map(|line| {
            let line = truncate_styled_display(&safe_styled_line(line), content_width.saturating_sub(2));
            if crate::interactive_color_enabled() {
                format!("  {}{}{}", crate::INTERACTIVE_LABEL, line, crate::INTERACTIVE_RESET)
            } else { format!("  {line}") }
        }).collect::<Vec<_>>();
    if let Some(prompt) = &state.prompt_frame {
        let capacity = usize::from(rows.saturating_sub(1).max(1)).saturating_sub(footer.len()).max(1);
        let mut lines = prompt.clone();
        if lines.len() > capacity {
            let count = capacity.saturating_sub(1);
            let offset = state.prompt_scroll.min(lines.len().saturating_sub(count));
            lines = lines[offset..offset + count].to_vec();
            lines.push(truncate_display(
                "PgUp/PgDn: details · ↑/↓: select",
                usize::from(columns.saturating_sub(1)),
            ));
        }
        lines.extend(footer);
        return Frame {
            lines,
            cursor: None,
        };
    }
    if matches!(state.turn, TurnPhase::Idle) {
        if let Some(idle) = &state.idle_frame {
            let max_rows = usize::from(rows.saturating_sub(1).max(1)).saturating_sub(footer.len()).max(1);
            let dropped = idle.lines.len().saturating_sub(max_rows);
            let mut lines = idle.lines.iter().skip(dropped).cloned().collect::<Vec<_>>();
            lines.extend(footer);
            let start = rows.saturating_sub(lines.len() as u16).saturating_add(1);
            let relative_cursor = idle.cursor_row.saturating_sub(dropped);
            let cursor_row = start
                .saturating_add(relative_cursor as u16)
                .min(rows.max(1));
            return Frame {
                lines,
                cursor: Some((cursor_row, idle.cursor_column.min(columns.max(1)))),
            };
        }
    }
    let surface_width = usize::from(columns.saturating_sub(2).max(1));
    let content_width = surface_width.min(MAX_CONTENT_WIDTH);
    let mut lines = Vec::new();
    let mut cursor = None;
    if let Some(composer) = &state.composer {
        // Guidance belongs to the input surface, never the transcript. On
        // tiny terminals retain the editable row and session facts first.
        if !matches!(state.turn, TurnPhase::Idle) {
            let hint = if matches!(state.turn, TurnPhase::Cancelling { .. }) {
                stopping_hint().to_string()
            } else if composer.draft.trim_start().starts_with('/') {
                if composer.queued_count > 0 {
                    format!("{} · {} {}", local_command_hint(),
                        crate::i18n::tr(crate::i18n::Message::InteractiveExecutionQueuedPrefix).trim_end_matches([':', '：']),
                        composer.queued_count)
                } else { local_command_hint().to_string() }
            } else if composer.queued_count > 0 {
                format!(
                    "{} · {} · {}",
                    composer.queued_count,
                    crate::i18n::tr(crate::i18n::Message::InteractiveExecutionQueuedPrefix)
                        .trim_end_matches([':', '：']),
                    crate::i18n::tr(crate::i18n::Message::InteractiveExecutionInputHint),
                )
            } else {
                crate::i18n::tr(crate::i18n::Message::InteractiveExecutionInputHint).to_string()
            };
            lines.push(format!(
                "  {}",
                truncate_display(&hint, content_width.saturating_sub(2))
            ));
        }
        let prefix = crate::INTERACTIVE_PROMPT_PREFIX;
        let prefix_width = UnicodeWidthStr::width(prefix);
        let (input, cursor_row, cursor_column) = crate::composer_input_multiline(
            &composer.draft, composer.cursor, surface_width.saturating_sub(prefix_width).max(1), prefix,
        );
        let capacity = usize::from(rows.saturating_sub(2).max(1)).saturating_sub(footer.len()).max(1).min(6);
        let first = cursor_row.saturating_sub(capacity - 1);
        let count = input.len().saturating_sub(first).min(capacity);
        let continuation = " ".repeat(prefix_width);
        for (index, (text, placeholder)) in input.iter().skip(first).take(count).enumerate() {
            lines.push(crate::interactive_input_bar_line(text, *placeholder, false,
                if first + index == 0 { prefix } else { &continuation }, surface_width));
        }
        let row = rows.saturating_sub(footer.len() as u16).saturating_sub(count as u16).saturating_add((cursor_row - first) as u16 + 1);
        cursor = Some((row.max(1), (cursor_column as u16 + 1).min(columns.max(1))));
    }
    lines.extend(footer);
    let max_rows = usize::from(rows.saturating_sub(1).max(1));
    if lines.len() > max_rows {
        lines.drain(..lines.len() - max_rows);
    }
    Frame { lines, cursor }
}

fn composer_line(composer: &ComposerState, width: usize) -> (String, u16) {
    let prefix_width = UnicodeWidthStr::width(crate::INTERACTIVE_PROMPT_PREFIX);
    let available = width.saturating_sub(prefix_width).max(1);
    let cursor = composer.cursor.min(composer.draft.len());
    let before = composer.draft.get(..cursor).unwrap_or_default();
    let mut cursor_width = UnicodeWidthStr::width(before);
    let mut visible = composer.draft.clone();
    let placeholder = composer.placeholder.clone();
    if cursor_width >= available {
        let trim = cursor_width.saturating_sub(available).saturating_add(1);
        visible = trim_display_prefix(&visible, trim);
        cursor_width = cursor_width.saturating_sub(trim);
    }
    visible = truncate_display(&visible, available);
    (
        crate::interactive_input_bar(&visible, Some(&placeholder), false),
        (prefix_width + cursor_width + 1).min(u16::MAX as usize) as u16,
    )
}

fn render_surface(state: &mut UiState) {
    let Ok((columns, rows)) = crossterm::terminal::size() else {
        return;
    };
    if columns == 0 || rows == 0 {
        return;
    }
    let stdout = io::stdout();
    let mut stdout = io::BufWriter::new(stdout.lock());
    render_surface_to(state, &mut stdout, columns, rows);
}

fn render_surface_to(state: &mut UiState, stdout: &mut impl Write, columns: u16, rows: u16) {
    state.painted_viewport = None;
    prepare_geometry(state, stdout, columns, rows);
    let frame = build_frame(state, columns, rows);
    // Absolute surface coordinates must not inherit the transcript's scroll
    // margins or an origin mode supplied by the enclosing terminal session.
    let _ = write!(stdout, "\x1b[?6l\x1b[r\x1b[0m");
    clear_transcript_tail(state, stdout);
    let previous_rows = state.rendered_rows.min(usize::from(rows));
    clear_rows(stdout, rows, state.rendered_rows);
    if state.history_cursor_row.is_none() {
        reclaim_released_surface_rows(stdout, rows, previous_rows, frame.lines.len());
    }
    if frame.lines.is_empty()
        && state.activity.is_none()
        && state.tool_receipt.is_empty()
        && state.active_output.is_empty()
        && state.assistant_preview.is_empty()
        && state.markdown_source.is_empty()
    {
        // Hand terminal ownership back to the idle composer from a known
        // absolute position. Leaving the cursor wherever the last cleared
        // surface row happened to be lets the idle composer's CSI J erase
        // freshly committed transcript rows.
        let _ = write!(stdout, "\x1b[r\x1b[{rows};1H\x1b[?25h");
        let _ = stdout.flush();
        state.rendered_rows = 0;
        return;
    }
    let start = rows
        .saturating_sub(frame.lines.len() as u16)
        .saturating_add(1);
    let content_bottom = start.saturating_sub(1).max(1);
    // Activity is a mutable transcript tail at the same insertion anchor as
    // durable history, never a row reserved above the composer. Spinner ticks
    // repaint that tail; they do not append history or move the insertion point.
    let mut visible_output = String::new();
    if !state.tool_receipt.is_empty() {
        visible_output.push_str(&state.tool_receipt);
        if state.activity.is_some()
            || !state.active_output.is_empty()
            || !state.assistant_preview.is_empty()
        {
            visible_output.push('\n');
        }
    }
    if let Some(activity) = &state.activity {
        visible_output.push_str(&activity.render(transcript_width(usize::from(columns))));
        if !state.active_output.is_empty() || !state.assistant_preview.is_empty() || !state.markdown_source.is_empty() {
            visible_output.push('\n');
        }
    }
    // The mutable tail may not overtake an earlier immutable cell waiting for
    // its frame. Rendering the suffix first corrupts scrollback ordering.
    if state.deferred_history.is_empty() {
        visible_output.push_str(&state.active_output);
        visible_output.push_str(&state.assistant_preview);
        visible_output.push_str(&crate::terminal_document::render_markdown(&state.markdown_source, transcript_width(usize::from(columns))));
    }
    let tail = tail_visual_lines(
        &visible_output,
        transcript_width(usize::from(columns)),
        usize::from(content_bottom),
    );
    let overflow = state
        .history_cursor_row
        .map(|row| {
            row.saturating_add(tail.len().saturating_sub(1) as u16)
                .saturating_sub(content_bottom) as usize
        })
        .unwrap_or_else(|| frame.lines.len().saturating_sub(previous_rows));
    if overflow > 0 {
        let previous_content_bottom = rows.saturating_sub(previous_rows as u16).max(1);
        let _ = write!(stdout, "\x1b[1;{previous_content_bottom}r");
        for _ in 0..overflow {
            let _ = write!(stdout, "\x1b[{previous_content_bottom};1H\r\n");
        }
        if let Some(row) = state.history_cursor_row.as_mut() {
            *row = row
                .saturating_sub(overflow.min(usize::from(u16::MAX)) as u16)
                .max(1);
        }
    }
    // Only history insertion needs a restricted scrolling region. Drawing
    // the composer below that region is not portable across terminal clients.
    let _ = write!(stdout, "\x1b[r\x1b[?7l");
    if !tail.is_empty() {
        let first = state.history_cursor_row.unwrap_or(1).min(content_bottom);
        for (index, line) in tail.iter().enumerate() {
            let row = first.saturating_add(index as u16);
            paint_row(stdout, row, columns, line);
        }
        state.rendered_tail = Some((first, tail.len()));
    }
    for (index, line) in frame.lines.iter().enumerate() {
        let row = start.saturating_add(index as u16);
        paint_row(stdout, row, columns, line);
    }
    if let Some((row, column)) = frame.cursor {
        let _ = write!(stdout, "\x1b[{row};{column}H\x1b[6 q\x1b[?25h");
    } else {
        let _ = write!(stdout, "\x1b[?25l");
    }
    let _ = write!(stdout, "\x1b[0m\x1b[?7h");
    let _ = stdout.flush();
    state.rendered_rows = frame.lines.len();
}

fn clear_transcript_tail(state: &mut UiState, stdout: &mut impl Write) {
    if let Some((first, count)) = state.rendered_tail.take() {
        for index in 0..count {
            let row = first.saturating_add(index as u16);
            let _ = write!(stdout, "\x1b[{row};1H\x1b[2K");
        }
    }
}

fn remember_history(state: &mut UiState, text: &str) {
    // Plain diagnostic output may be byte-bounded; Markdown cells must keep
    // their syntax intact and are evicted as whole cells.
    let mut start = text.len().saturating_sub(MAX_VIEWPORT_REPLAY_BYTES);
    while !text.is_char_boundary(start) { start += 1; }
    remember_cell(state, TranscriptCell::Text(text[start..].to_owned()));
}

fn remember_cell(state: &mut UiState, cell: TranscriptCell) {
    if cell.source().is_empty() { return; }
    state.history_revision = state.history_revision.wrapping_add(1);
    state.history_tail_bytes += cell.source().len();
    state.history_tail.push_back(cell);
    // Admission is not presentation. Never evict here: an entire provider batch
    // may be admitted before its first synchronized frame reaches the terminal.
}

/// Only called after a real write_all + flush, with the owner lock held.
/// Entire cells before the *current* visible/reader anchor may leave the replay
/// cache; pending source and any cell intersecting the viewport are retained.
fn evict_painted_history(state: &mut UiState) {
    if !state.session_active { return; }
    let Some((_, length, capacity)) = state.history_viewport_extent else { return; };
    let mut before_viewport = length.saturating_sub(state.history_scroll).saturating_sub(capacity);
    let mut removed_cells = 0;
    let mut removed_rows = 0;
    while state.history_tail_bytes > MAX_VIEWPORT_REPLAY_BYTES && state.history_tail.len() > 1 {
        let Some(end) = state.history_cell_ends.get(removed_cells).copied() else { break; };
        let count = end.saturating_sub(removed_rows);
        if count > before_viewport || end > state.presented_document_rows { break; }
        let old = state.history_tail.pop_front().expect("front cell exists");
        state.history_tail_bytes -= old.source().len();
        removed_rows = end;
        removed_cells += 1;
        before_viewport -= count;
    }
    if removed_cells > 0 {
        state.history_first_cell_id += removed_cells as u64;
        state.presented_document_rows = state.presented_document_rows.saturating_sub(removed_rows);
        if let Some((_, length, _)) = state.history_viewport_extent.as_mut() { *length = length.saturating_sub(removed_rows); }
        state.history_anchors.drain(..removed_rows);
        state.history_cell_ends.drain(..removed_cells);
        for end in &mut state.history_cell_ends { *end -= removed_rows; }
        if let Some((_, _, lines)) = state.history_layout.as_mut() { lines.drain(..removed_rows); }
    }
}

fn prepare_geometry(state: &mut UiState, stdout: &mut impl Write, columns: u16, rows: u16) {
    let reserved = build_frame(state, columns, rows).lines.len();
    let geometry = (columns, rows, reserved);
    let previous = state.rendered_geometry.replace(geometry);
    if previous.is_none() || previous == Some(geometry) {
        return;
    }
    // Terminal emulators disagree about reflow/scrolling on resize. Old cell
    // coordinates are no longer authoritative. Opening/closing a permission
    // panel changes the viewport just as a resize does. Rebuild the transcript
    // from committed text, never from the previous screen (which has a draft).
    let _ = write!(stdout, "\x1b[?6l\x1b[r\x1b[0m\x1b[2J\x1b[H");
    let bottom = rows.saturating_sub(reserved as u16).max(1);
    let width = usize::from(columns.saturating_sub(2).max(1)).min(MAX_CONTENT_WIDTH);
    let mut lines = VecDeque::new();
    for block in &state.history_tail {
        for line in visual_lines(&block.render(width), width) {
            lines.push_back(line);
            if lines.len() >= usize::from(bottom) {
                lines.pop_front();
            }
        }
    }
    for (index, line) in lines.iter().enumerate() {
        paint_row(stdout, index as u16 + 1, columns, line);
    }
    state.history_cursor_row = Some((lines.len() as u16 + 1).min(bottom));
    state.rendered_rows = 0;
    state.rendered_tail = None;
}

fn write_history(state: &mut UiState, text: &str) {
    let Ok((columns, rows)) = crossterm::terminal::size() else {
        print_block(text);
        return;
    };
    let stdout = io::stdout();
    let mut stdout = io::BufWriter::new(stdout.lock());
    write_history_to(state, text, &mut stdout, columns, rows);
}

fn write_history_to(
    state: &mut UiState,
    text: &str,
    stdout: &mut impl Write,
    columns: u16,
    rows: u16,
) {
    prepare_geometry(state, stdout, columns, rows);
    remember_history(state, text);
    write_history_rendered_to(state, text, stdout, columns, rows);
}

fn write_cell_to(
    state: &mut UiState,
    cell: &TranscriptCell,
    stdout: &mut impl Write,
    columns: u16,
    rows: u16,
) {
    // Reflow old cells before inserting the new one, so resize never replays it twice.
    prepare_geometry(state, stdout, columns, rows);
    let text = cell.render(transcript_width(usize::from(columns)));
    remember_cell(state, cell.clone());
    write_history_rendered_to(state, &text, stdout, columns, rows);
}

fn write_history_rendered_to(
    state: &mut UiState,
    text: &str,
    mut stdout: &mut impl Write,
    columns: u16,
    rows: u16,
) {
    prepare_geometry(state, &mut stdout, columns, rows);
    let target_rows = build_frame(state, columns, rows).lines.len();
    let old_rows = state.rendered_rows;
    let reserved_rows = if target_rows <= old_rows {
        state.rendered_rows = target_rows;
        target_rows
    } else {
        old_rows
    };
    let bottom = rows.saturating_sub(reserved_rows as u16).max(1);
    let content_width = transcript_width(usize::from(columns));
    let _ = write!(stdout, "\x1b[?6l\x1b[r\x1b[0m");
    clear_transcript_tail(state, &mut stdout);
    clear_rows(&mut stdout, rows, old_rows);
    if state.history_cursor_row.is_none() {
        reclaim_released_surface_rows(&mut stdout, rows, old_rows, reserved_rows);
    }
    let lines = visual_lines(text, content_width);
    if let Some(mut row) = state.history_cursor_row.map(|row| row.clamp(1, bottom)) {
        let _ = write!(stdout, "\x1b[1;{bottom}r");
        for line in lines {
            paint_row(stdout, row, columns, &line);
            if row < bottom {
                row = row.saturating_add(1);
            } else {
                // Advance the insertion point by scrolling only when the
                // durable transcript has actually reached the live surface.
                let _ = write!(stdout, "\x1b[{bottom};1H\r\n");
            }
        }
        state.history_cursor_row = Some(row);
    } else {
        let _ = write!(stdout, "\x1b[1;{bottom}r");
        for line in lines {
            // Fallback for non-interactive output committed before the first
            // surface has established a concrete transcript cursor.
            let _ = write!(stdout, "\x1b[{bottom};1H\r\n");
            paint_row(stdout, bottom, columns, &line);
        }
    }
    let _ = stdout.flush();
}

fn reclaim_released_surface_rows(
    stdout: &mut impl Write,
    terminal_rows: u16,
    previous_rows: usize,
    next_rows: usize,
) {
    let released = previous_rows.saturating_sub(next_rows);
    if released == 0 {
        return;
    }
    let content_bottom = terminal_rows
        .saturating_sub(next_rows.min(usize::from(terminal_rows)) as u16)
        .max(1);
    // The released bottom rows are cleared cells, not scrollback. Move the
    // durable transcript down into the enlarged content region before writing
    // the next history block, otherwise each smaller live tail leaves another
    // visible blank band between paragraphs.
    let _ = write!(stdout, "\x1b[1;{content_bottom}r\x1b[1;1H\x1b[{released}T");
}

fn visual_lines(text: &str, max_width: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut logical = normalized.split('\n').collect::<Vec<_>>();
    if logical.last().is_some_and(|line| line.is_empty()) {
        logical.pop();
    }
    logical
        .into_iter()
        .flat_map(|line| wrap_styled_line(&safe_styled_line(line), max_width.max(1)))
        .collect()
}

fn tail_visual_lines(text: &str, max_width: usize, max_lines: usize) -> Vec<String> {
    if text.is_empty() || max_lines == 0 {
        return Vec::new();
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut tail = Vec::with_capacity(max_lines);
    for logical in normalized.trim_end_matches('\n').split('\n').rev() {
        let wrapped = wrap_styled_line(&safe_styled_line(logical), max_width.max(1));
        for line in wrapped.into_iter().rev() {
            tail.push(line);
            if tail.len() == max_lines {
                tail.reverse();
                return tail;
            }
        }
    }
    tail.reverse();
    tail
}

pub(crate) fn print_wrapped_block(text: &str) {
    *LAST_WRAPPED_BLOCK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(text.to_string());
    print_wrapped_block_without_storing(text);
}

fn print_wrapped_block_without_storing(text: &str) {
    let Ok((columns, _)) = crossterm::terminal::size() else {
        print_block(text);
        return;
    };
    let width = usize::from(columns.saturating_sub(2).max(1)).min(MAX_CONTENT_WIDTH);
    for line in visual_lines(text, width) {
        // Resize redraws can run while crossterm raw mode has disabled the
        // terminal's LF-to-CRLF output translation. Always return to column
        // one explicitly or every subsequent line drifts to the right.
        print!("{line}\r\n");
    }
    let _ = io::stdout().flush();
}

pub(crate) fn redraw_last_wrapped_block() -> bool {
    let block = LAST_WRAPPED_BLOCK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(block) = block else {
        return false;
    };
    print!("\x1b[2J\x1b[H");
    print_wrapped_block_without_storing(&block);
    true
}

pub(crate) fn clear_last_wrapped_block() {
    *LAST_WRAPPED_BLOCK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

fn wrap_styled_line(line: &str, max_width: usize) -> Vec<String> {
    wrap_styled_line_with_offsets(line,max_width).into_iter().map(|(line,_)|line).collect()
}

fn wrap_styled_line_with_offsets(line: &str, max_width: usize) -> Vec<(String,usize)> {
    if line.is_empty() {
        return vec![(String::new(),0)];
    }
    if UnicodeWidthStr::width(strip_ansi(line).as_str()) <= max_width {
        return vec![(line.to_string(),0)];
    }

    let plain = strip_ansi(line);
    let continuation = continuation_prefix(&plain);
    let continuation_width =
        UnicodeWidthStr::width(continuation.as_str()).min(max_width.saturating_sub(1));
    let mut output = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    let mut active_sgr = String::new();
    let mut soft_break = None;
    let mut source_offset = 0;
    let mut current_offset = 0;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let mut sequence = String::from("\u{1b}");
            while let Some(control) = chars.next() {
                sequence.push(control);
                if control.is_ascii_alphabetic() {
                    break;
                }
            }
            current.push_str(&sequence);
            if sequence.ends_with('m') {
                if sequence == "\x1b[0m" || sequence == "\x1b[m" {
                    active_sgr.clear();
                } else {
                    active_sgr.push_str(&sequence);
                }
            }
            continue;
        }

        let char_offset = source_offset;
        source_offset += 1;
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width > 0 && current_width.saturating_add(char_width) > max_width {
            let wrapped_at_word = soft_break
                .take()
                .filter(|(byte, width, _)| {
                    *byte < current.len()
                        && *width > continuation_width
                        && current_width.saturating_sub(*width) <= MAX_SOFT_WRAP_LOOKBACK
                        && continuation_width + current_width.saturating_sub(*width) < max_width
                })
                .map(|(byte, width, offset)| {
                    let remainder = current.split_off(byte);
                    let remainder_width = current_width.saturating_sub(width);
                    while current.ends_with(' ') || current.ends_with('\t') {
                        current.pop();
                    }
                    if !active_sgr.is_empty() {
                        current.push_str("\x1b[0m");
                    }
                    output.push((std::mem::take(&mut current),current_offset));
                    current_offset = offset;
                    current = continuation.clone();
                    if !active_sgr.is_empty() {
                        current.push_str(&active_sgr);
                    }
                    current.push_str(&remainder);
                    current_width = continuation_width.saturating_add(remainder_width);
                })
                .is_some();

            let mut preserve_overflow_whitespace = wrapped_at_word;
            if !wrapped_at_word || current_width.saturating_add(char_width) > max_width {
                if !active_sgr.is_empty() {
                    current.push_str("\x1b[0m");
                }
                output.push((current,current_offset));
                current_offset = char_offset;
                current = continuation.clone();
                current_width = continuation_width;
                if !active_sgr.is_empty() {
                    current.push_str(&active_sgr);
                }
                preserve_overflow_whitespace = false;
            }
            if ch.is_whitespace() && !preserve_overflow_whitespace {
                current_offset = source_offset;
                continue;
            }
        }
        current.push(ch);
        current_width = current_width.saturating_add(char_width);
        if ch.is_whitespace() {
            soft_break = Some((current.len(), current_width,source_offset));
        }
    }

    if !current.is_empty() {
        if !active_sgr.is_empty() {
            current.push_str("\x1b[0m");
        }
        output.push((current,current_offset));
    }
    output
}

fn continuation_prefix(plain: &str) -> String {
    let leading = plain.chars().take_while(|ch| ch.is_whitespace()).count();
    let rest = plain.trim_start();
    let marker_width = if let Some(dot) = rest.find(". ") {
        if dot > 0 && rest[..dot].chars().all(|ch| ch.is_ascii_digit()) {
            dot + 2
        } else {
            0
        }
    } else if ["• ", "- ", "* ", "> "]
        .iter()
        .any(|marker| rest.starts_with(marker))
    {
        2
    } else {
        0
    };
    " ".repeat(leading + marker_width)
}

fn clear_surface(state: &mut UiState) {
    state.painted_viewport = None;
    let Ok((columns, rows)) = crossterm::terminal::size() else {
        return;
    };
    let stdout = io::stdout();
    let mut stdout = io::BufWriter::new(stdout.lock());
    prepare_geometry(state, &mut stdout, columns, rows);
    let _ = write!(stdout, "\x1b[?6l\x1b[r\x1b[0m");
    clear_rows(&mut stdout, rows, state.rendered_rows);
    let _ = write!(stdout, "\x1b[r\x1b[{rows};1H\x1b[?25h");
    let _ = stdout.flush();
    state.rendered_rows = 0;
}

fn clear_rows(stdout: &mut impl Write, rows: u16, count: usize) {
    let count = count.min(usize::from(rows));
    let start = rows.saturating_sub(count as u16).saturating_add(1);
    for offset in 0..count {
        let row = start.saturating_add(offset as u16);
        let _ = write!(stdout, "\x1b[{row};1H\x1b[2K");
    }
}

/// The renderer alone owns cursor movement, scroll margins and terminal modes.
/// Model/tool text and externally prepared input panels may carry SGR styling,
/// but must never be able to move outside the row assigned by the layout.
fn safe_styled_line(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.next() {
                Some('[') => {
                    let mut parameters = String::new();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            if next == 'm' && parameters.chars().all(|c| c.is_ascii_digit() || c == ';' || c == ':') {
                                output.push_str("\x1b[");
                                output.push_str(&parameters);
                                output.push('m');
                            }
                            break;
                        }
                        parameters.push(next);
                    }
                }
                // OSC (including hyperlinks), DCS, SOS, PM and APC payloads
                // have zero display width. Drop the complete payload, not
                // merely ESC: otherwise its closing bracket leaks onscreen.
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    while let Some(next) = chars.next() {
                        if next == '\x07' { break; }
                        if next == '\x1b' && chars.next_if_eq(&'\\').is_some() { break; }
                    }
                }
                Some('(' | ')' | '*' | '+') => { chars.next(); }
                _ => {}
            }
        } else if ch == '\t' {
            output.push_str("    ");
        } else if !ch.is_control() {
            output.push(ch);
        }
    }
    output
}

fn paint_row(stdout: &mut impl Write, row: u16, columns: u16, line: &str) {
    let safe = safe_styled_line(line);
    let clipped = truncate_styled_display(&safe, usize::from(columns.saturating_sub(1).max(1)));
    // Isolate style and autowrap per row, including replay/history writes.
    // A full-width last cell must not create an unaccounted terminal line.
    let _ = write!(stdout, "\x1b[?7l\x1b[{row};1H\x1b[0m\x1b[2K{clipped}\x1b[0m\x1b[?7h");
}

fn print_block(text: &str) {
    print!("{text}");
    if !text.ends_with('\n') {
        println!();
    }
    let _ = io::stdout().flush();
}

fn strip_ansi(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            output.push(ch);
            continue;
        }
        if chars.next_if_eq(&'[').is_some() {
            while let Some(next) = chars.next() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        }
    }
    output
}

fn truncate_display(value: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }
    let mut output = String::new();
    let mut width = 0usize;
    // Keep combining marks, flags and joined emoji together. Splitting scalars
    // produces broken glyphs and changes the width of the supposedly fixed row.
    for grapheme in value.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width + grapheme_width > max_width - 1 {
            break;
        }
        output.push_str(grapheme);
        width += grapheme_width;
    }
    output.push('…');
    output
}

fn truncate_styled_display(value: &str, max_width: usize) -> String {
    let visible = strip_ansi(value);
    if UnicodeWidthStr::width(visible.as_str()) <= max_width {
        return value.to_string();
    }
    if max_width <= 1 {
        return "…\x1b[0m".to_string();
    }

    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut width = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            output.push(ch);
            if let Some(next) = chars.next() {
                output.push(next);
                if next == '[' {
                    for control in chars.by_ref() {
                        output.push(control);
                        if ('@'..='~').contains(&control) {
                            break;
                        }
                    }
                }
            }
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width - 1 {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    output.push('…');
    output.push_str("\x1b[0m");
    output
}

fn trim_display_prefix(value: &str, target: usize) -> String {
    let mut width = 0usize;
    let mut start = 0usize;
    for (index, ch) in value.char_indices() {
        if width >= target {
            start = index;
            break;
        }
        width += UnicodeWidthChar::width(ch).unwrap_or(0);
        start = index + ch.len_utf8();
    }
    value.get(start..).unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_palette_preserves_terminal_foreground_and_background() {
        let mut terminal = vt100::Parser::new(4, 40, 0);
        terminal.process(format!("{}{}body{}", crate::INTERACTIVE_COMPOSER_BG,
            crate::INTERACTIVE_VALUE, crate::INTERACTIVE_RESET).as_bytes());
        let cell = terminal.screen().cell(0, 0).unwrap();
        assert_eq!(cell.fgcolor(), vt100::Color::Default);
        assert_eq!(cell.bgcolor(), vt100::Color::Idx(236));
        terminal.process(format!("\r\n{}{}selected{}", crate::INTERACTIVE_SELECTION_BG,
            crate::INTERACTIVE_VALUE, crate::INTERACTIVE_RESET).as_bytes());
        let selected = terminal.screen().cell(1, 0).unwrap();
        assert!(selected.inverse());
        assert_eq!(selected.fgcolor(), vt100::Color::Default);
        assert_eq!(selected.bgcolor(), vt100::Color::Default);
    }
    use super::*;

    #[test]
    fn durable_notification_presentation_propagates_write_and_flush_failure() {
        struct BrokenOutput { fail_write: bool, flushed: bool }
        impl Write for BrokenOutput {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.fail_write { Err(io::Error::new(io::ErrorKind::BrokenPipe, "write failed")) }
                else { Ok(bytes.len()) }
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushed = true;
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush failed"))
            }
        }
        let mut write_failure = BrokenOutput { fail_write: true, flushed: false };
        assert!(write_delivered_frame(&mut write_failure, b"notification").is_err());
        assert!(!write_failure.flushed);
        let mut flush_failure = BrokenOutput { fail_write: false, flushed: false };
        assert!(write_delivered_frame(&mut flush_failure, b"notification").is_err());
        assert!(flush_failure.flushed);
    }

    #[test]
    fn session_ownership_survives_idle_and_modal_but_never_claims_a_pipe() {
        let mut state = UiState::default();
        assert!(!state.owns_terminal(true));
        state.reduce(UiEvent::TurnStarted(1));
        assert!(!state.owns_terminal(true), "a standalone turn is not an interactive session");
        state.session_active = true;
        for event in [UiEvent::PromptStarted, UiEvent::PromptCompleted, UiEvent::TurnCompleted(1)] {
            state.reduce(event);
            assert!(state.owns_terminal(true));
            assert!(!state.owns_terminal(false));
        }
        assert_eq!(state.turn, TurnPhase::Idle);
        state = UiState::default(); // the session guard's terminal release
        state.reduce(UiEvent::TurnStarted(2)); // a late producer cannot reacquire the session
        assert!(!state.owns_terminal(true));
    }

    #[test]
    fn asynchronous_history_during_modal_uses_the_same_document_and_frame() {
        let mut state = UiState::default();
        state.session_active = true;
        state.history_cursor_row = Some(1);
        state.reduce(UiEvent::TurnStarted(1));
        state.reduce(UiEvent::PromptStarted);
        state.prompt_frame = Some(vec!["Allow this operation?".into(), "› Allow once".into()]);
        let mut terminal = vt100::Parser::new(12, 50, 20);
        terminal.process(&pending_frame_bytes(&mut state, 50, 12, true));
        assert!(state.owns_terminal(true), "emitter must not select its raw stdout fallback");
        let cells = state.reduce(UiEvent::HistoryCommitted("background task completed".into()));
        state.deferred_history.extend(cells);
        terminal.process(&pending_frame_bytes(&mut state, 50, 12, true));
        let visible = terminal.screen().contents();
        assert_eq!(visible.matches("background task completed").count(), 1, "{visible}");
        assert_eq!(visible.matches("Allow this operation?").count(), 1, "{visible}");
        assert!(visible.ends_with("› Allow once"), "{visible}");
        state.reduce(UiEvent::PromptCompleted);
        terminal.process(&pending_frame_bytes(&mut state, 50, 12, true));
        assert_eq!(terminal.screen().contents().matches("background task completed").count(), 1);
        assert!(!terminal.screen().contents().contains("Allow this operation?"));
    }

    #[test]
    fn permission_panel_release_restores_the_document_instead_of_a_blank_band() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        state.idle_frame = Some(IdleFrameState {
            lines: vec!["› next".into()], cursor_row: 0, cursor_column: 7,
        });
        let mut terminal = vt100::Parser::new(24, 60, 200);
        for index in 0..18 {
            paint_event(&mut state, &mut terminal,
                UiEvent::HistoryCommitted(format!("history-{index:02}")), 60, 24);
        }
        for height in [18, 7, 20, 2] {
            state.prompt_frame = Some((0..height).map(|index| format!("permission-{index:02}")).collect());
            terminal.process(&pending_frame_bytes(&mut state, 60, 24, true));
            assert!(terminal.screen().contents().contains("permission-00"));
            state.prompt_frame = None;
            terminal.process(&pending_frame_bytes(&mut state, 60, 24, true));
            let visible = terminal.screen().contents();
            assert!(!visible.contains("permission-"), "{visible}");
            for index in 0..18 {
                assert_eq!(visible.matches(&format!("history-{index:02}")).count(), 1, "{visible}");
            }
            assert_eq!(visible.matches("› next").count(), 1);
            assert_eq!(terminal.screen().cursor_position(), (23, 6));
        }
    }

    #[test]
    fn terminal_rows_cannot_inject_cursor_commands_or_leak_color() {
        let mut terminal = vt100::Parser::new(8, 30, 20);
        let mut bytes = Vec::new();
        paint_row(&mut bytes, 1, 30, "\x1b[32mtool result\x1b[2J\x1b[8;1H");
        paint_row(&mut bytes, 2, 30, "normal reply");
        paint_row(&mut bytes, 3, 30, "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\");
        terminal.process(&bytes);
        assert!(terminal.screen().contents().starts_with("tool result\nnormal reply\nlink"));
        assert_eq!(terminal.screen().cell(1, 0).unwrap().fgcolor(), vt100::Color::Default);
        assert_eq!(terminal.screen().cell(1, 0).unwrap().bgcolor(), vt100::Color::Default);
        assert!(terminal.screen().contents().lines().count() == 3);
    }

    #[test]
    fn narrow_cjk_panel_has_no_last_column_wrap_or_stale_suffix() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        let mut terminal = vt100::Parser::new(8, 12, 20);
        paint_event(&mut state, &mut terminal, UiEvent::HistoryCommitted("已提交".into()), 12, 8);
        for label in ["\x1b[32m全部允许所有操作（最宽松）", "谨慎"] {
            state.prompt_frame = Some(vec![label.into()]);
            terminal.process(&pending_frame_bytes(&mut state, 12, 8, true));
            let visible = terminal.screen().contents();
            assert_eq!(visible.matches("已提交").count(), 1, "{visible}");
            assert!(!visible.contains(']'), "{visible}");
        }
        assert!(terminal.screen().contents().ends_with("谨慎"));
        assert!(!terminal.screen().contents().contains("全部"));
    }

    #[test]
    fn history_append_during_resize_does_not_replay_the_new_cell_twice() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        let mut terminal = vt100::Parser::new(12, 40, 20);
        let mut bytes = Vec::new();
        write_history_to(&mut state, "before", &mut bytes, 40, 12);
        terminal.process(&bytes);
        terminal.set_size(12, 30);
        bytes.clear();
        write_history_to(&mut state, "after", &mut bytes, 30, 12);
        render_surface_to(&mut state, &mut bytes, 30, 12);
        terminal.process(&bytes);
        let visible = terminal.screen().contents();
        assert_eq!(visible.matches("before").count(), 1, "{visible}");
        assert_eq!(visible.matches("after").count(), 1, "{visible}");
    }

    #[test]
    fn ownership_barrier_drains_queued_history_once_in_order() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        for text in ["FIRST", "SECOND", "THIRD"] {
            state.deferred_history.push_back(text.to_string().into());
        }
        let mut terminal = vt100::Parser::new(24, 60, 100);
        terminal.process(&pending_frame_bytes(&mut state, 60, 24, false));
        assert_eq!(state.deferred_history.len(), 2);
        assert!(state.frame_pending);
        terminal.process(&pending_frame_bytes(&mut state, 60, 24, true));
        assert!(state.deferred_history.is_empty());
        assert!(!state.frame_pending);
        let contents = terminal.screen().contents();
        assert!(contents.find("FIRST").unwrap() < contents.find("SECOND").unwrap());
        assert!(contents.find("SECOND").unwrap() < contents.find("THIRD").unwrap());
        terminal.process(&pending_frame_bytes(&mut state, 60, 24, false));
        assert_eq!(state.history_tail.len(), 3);
        assert_eq!(terminal.screen().contents(), contents);
    }

    #[test]
    fn prompt_handoff_settles_history_without_scheduling_late_frames() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        state.reduce(UiEvent::TurnStarted(1));
        for text in ["BEFORE-PROMPT", "LAST-RECEIPT"] {
            state.deferred_history.push_back(text.to_string().into());
        }
        state.reduce(UiEvent::PromptStarted);
        let mut terminal = vt100::Parser::new(24, 60, 100);
        terminal.process(&pending_frame_bytes(&mut state, 60, 24, true));
        assert!(!state.frame_pending);
        assert!(state.deferred_history.is_empty());
        assert!(terminal.screen().contents().contains("LAST-RECEIPT"));
    }

    #[test]
    fn source_backed_stream_finalizes_at_the_same_anchor_and_reflows() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        let mut terminal = vt100::Parser::new(24, 60, 100);
        paint_event(&mut state, &mut terminal, UiEvent::TurnStarted(7), 60, 24);
        paint_event(&mut state, &mut terminal, UiEvent::MarkdownDelta("**hello".into()), 60, 24);
        assert!(terminal.screen().contents().contains("hello"));
        paint_event(&mut state, &mut terminal, UiEvent::MarkdownDelta(" world**".into()), 60, 24);
        let before = terminal.screen().contents();
        paint_event(&mut state, &mut terminal, UiEvent::AssistantCompleted, 60, 24);
        assert_eq!(terminal.screen().contents(), before);
        assert!(matches!(state.history_tail.back(), Some(TranscriptCell::Markdown(source)) if source == "**hello world**"));
        terminal.set_size(24, 30);
        let mut bytes = Vec::new();
        render_surface_to(&mut state, &mut bytes, 30, 24);
        terminal.process(&bytes);
        assert!(terminal.screen().contents().contains("hello world"));
        paint_event(&mut state, &mut terminal, UiEvent::AssistantCompleted, 30, 24);
        assert_eq!(state.history_tail.len(), 1);
    }

    #[test]
    fn source_backed_stream_keeps_tables_and_fences_mutable_until_safe_boundary() {
        let mut state = UiState::default();
        let source = "intro\n\n```rust\n\nlet x = 1;\n";
        let committed = state.reduce(UiEvent::MarkdownDelta(source.into()));
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].source(), "intro\n\n");
        assert_eq!(state.markdown_source, "```rust\n\nlet x = 1;\n");
        let rest = state.reduce(UiEvent::MarkdownDelta("```\n\n| A | B |\n| --- | --- |\n".into()));
        assert_eq!(rest.len(), 1);
        assert!(state.markdown_source.starts_with("| A | B |"));
        let end = state.reduce(UiEvent::AssistantCompleted);
        assert!(matches!(&end[0], TranscriptCell::Markdown(_)));
    }

    #[test]
    fn source_backed_stream_continues_across_confirmed_provider_parts() {
        let mut state = UiState::default();
        state.reduce(UiEvent::MarkdownDelta("PREFIX-ALPHA-".into()));
        assert!(state.reduce(UiEvent::AssistantPreviewCommitted).is_empty());
        state.reduce(UiEvent::MarkdownDelta("OMEGA".into()));
        let cells = state.reduce(UiEvent::AssistantCompleted);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].source(), "PREFIX-ALPHA-OMEGA");
    }

    fn paint_event(
        state: &mut UiState,
        terminal: &mut vt100::Parser,
        event: UiEvent,
        columns: u16,
        rows: u16,
    ) {
        let committed = state.reduce(event);
        let mut bytes = Vec::new();
        for block in committed {
            write_cell_to(state, &block, &mut bytes, columns, rows);
        }
        render_surface_to(state, &mut bytes, columns, rows);
        terminal.process(&bytes);
    }

    #[test]
    fn execution_composer_describes_slash_input_as_control_not_a_queued_message() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(7));
        state.composer = Some(ComposerState { draft: " /allow".into(), cursor: 7, queued_count: 2, placeholder: "input".into() });
        let command = build_frame(&state, 120, 24).lines.join("\n");
        assert!(command.contains(local_command_hint()), "{command}");
        assert!(!command.contains(crate::i18n::tr(crate::i18n::Message::InteractiveExecutionInputHint)));
        state.composer.as_mut().unwrap().draft = "ordinary follow-up".into();
        let follow_up = build_frame(&state, 120, 24).lines.join("\n");
        assert!(!follow_up.contains(local_command_hint()));
        assert!(follow_up.contains(crate::i18n::tr(crate::i18n::Message::InteractiveExecutionInputHint)));
    }

    #[test]
    fn completed_read_receipt_updates_and_settles_at_one_transcript_anchor() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        state.reduce(UiEvent::TurnStarted(7));
        state.composer = Some(ComposerState {
            draft: "draft".into(),
            cursor: 5,
            queued_count: 0,
            placeholder: "input".into(),
        });
        let mut terminal = vt100::Parser::new(24, 60, 100);
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::HistoryCommitted("user message".into()),
            60,
            24,
        );
        for count in 1..=50 {
            paint_event(
                &mut state,
                &mut terminal,
                UiEvent::ToolReceiptChanged {
                    turn_id: 7,
                    text: format!("    • Read file ×{count}"),
                },
                60,
                24,
            );
            let screen = terminal.screen().contents();
            assert!(
                screen
                    .lines()
                    .nth(1)
                    .unwrap()
                    .contains(&format!("×{count}")),
                "{screen}"
            );
            assert_eq!(state.history_cursor_row, Some(2));
            assert_eq!(
                state.history_tail.len(),
                1,
                "completed reads must update one owned timeline slot"
            );
            assert!(!build_frame(&state, 60, 24)
                .lines
                .iter()
                .any(|line| line.contains("Read file")));
        }
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::ActivityChanged(ActivityState {
                key: "thinking".into(),
                glyph: "⠋".into(),
                detail: "Thinking".into(),
            }),
            60,
            24,
        );
        paint_event(&mut state, &mut terminal, UiEvent::ActivityCleared, 60, 24);
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::AssistantPreviewDelta("FINAL-ANSWER".into()),
            60,
            24,
        );
        let before = terminal.screen().contents();
        let receipt_row = before.lines().position(|line| line.contains("×50"));
        let answer_row = before
            .lines()
            .position(|line| line.contains("FINAL-ANSWER"));
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::ToolReceiptCommitted { turn_id: 7 },
            60,
            24,
        );
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::ToolReceiptCommitted { turn_id: 7 },
            60,
            24,
        );
        let settled = terminal.screen().contents();
        assert_eq!(
            settled.lines().position(|line| line.contains("×50")),
            receipt_row
        );
        assert_eq!(
            settled
                .lines()
                .position(|line| line.contains("FINAL-ANSWER")),
            answer_row
        );
        assert_eq!(settled.matches("×50").count(), 1);
        assert_eq!(state.history_tail.len(), 2, "settlement is idempotent");
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::AssistantCompleted,
            60,
            24,
        );
        paint_event(&mut state, &mut terminal, UiEvent::TurnCompleted(7), 60, 24);
        let final_screen = terminal.screen().contents();
        assert_eq!(final_screen.matches("×50").count(), 1);
        assert_eq!(final_screen.matches("FINAL-ANSWER").count(), 1);
        assert!(state.tool_receipt.is_empty());
    }

    #[test]
    fn cancelled_lease_settles_observed_reads_without_accepting_text_or_leaking_to_next_turn() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(7));
        state.reduce(UiEvent::ToolReceiptChanged {
            turn_id: 7,
            text: "    • Read file ×3".into(),
        });
        state.reduce(UiEvent::AssistantPreviewDelta("UNACCEPTED".into()));
        state.reduce(UiEvent::TurnCancelling(7));
        assert!(state.reduce(UiEvent::TurnCompleted(8)).is_empty());
        assert_eq!(state.tool_receipt, "    • Read file ×3");
        assert_eq!(
            state.reduce(UiEvent::TurnCompleted(7)),
            vec!["    • Read file ×3"]
        );
        assert!(state.reduce(UiEvent::TurnCompleted(7)).is_empty());
        assert!(state.assistant_preview.is_empty());
        state.reduce(UiEvent::TurnStarted(9));
        state.reduce(UiEvent::ToolReceiptChanged {
            turn_id: 9,
            text: "new receipt".into(),
        });
        state.reduce(UiEvent::ToolReceiptChanged {
            turn_id: 7,
            text: "late old receipt".into(),
        });
        assert!(state
            .reduce(UiEvent::ToolReceiptCommitted { turn_id: 7 })
            .is_empty());
        assert_eq!(state.tool_receipt, "new receipt");
        assert_eq!(
            state.reduce(UiEvent::ToolReceiptCommitted { turn_id: 9 }),
            vec!["new receipt"]
        );
    }

    #[test]
    fn new_turn_settles_the_previous_observed_read_block_once() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(7));
        state.reduce(UiEvent::ToolReceiptChanged {
            turn_id: 7,
            text: "old receipt".into(),
        });
        assert_eq!(state.reduce(UiEvent::TurnStarted(9)), vec!["old receipt"]);
        assert!(state.tool_receipt.is_empty());
        state.reduce(UiEvent::ToolReceiptChanged {
            turn_id: 7,
            text: "late receipt".into(),
        });
        assert!(state.tool_receipt.is_empty());
    }

    #[test]
    fn idle_composer_does_not_advertise_live_execution_controls() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(7));
        state.composer = Some(ComposerState {
            draft: "draft".into(),
            cursor: 5,
            queued_count: 0,
            placeholder: "input".into(),
        });
        let live_hint = crate::i18n::tr(crate::i18n::Message::InteractiveExecutionInputHint);
        assert!(build_frame(&state, 80, 24)
            .lines
            .iter()
            .any(|line| line.contains(live_hint)));
        state.reduce(UiEvent::TurnCompleted(7));
        let idle = build_frame(&state, 80, 24);
        assert!(!idle.lines.iter().any(|line| line.contains(live_hint)));
        assert!(idle.lines.iter().any(|line| line.contains("draft")));
    }

    #[test]
    fn activity_is_one_safe_row_at_narrow_and_wide_widths() {
        let activity = ActivityState {
            key: "processing".into(),
            glyph: "⠋".into(),
            detail: "正在思考中\n检查依赖\tThinking\x1b[2J\x1b]0;hidden-title\x07".into(),
        };
        for width in [1, 8, 24, 80] {
            let line = activity.render(width);
            assert!(UnicodeWidthStr::width(line.as_str()) <= width, "{line:?}");
            assert!(!line.chars().any(char::is_control));
            assert!(!line.contains("hidden-title"));
        }
        assert!(activity.render(24).ends_with('…'));
    }

    #[test]
    fn status_truncation_keeps_graphemes_and_respects_zero_width() {
        assert_eq!(truncate_display("working", 0), "");
        assert_eq!(truncate_display("👩‍💻abcdef", 3), "👩‍💻…");
        assert_eq!(truncate_display("e\u{301}abcdef", 2), "e\u{301}…");
        assert_eq!(truncate_display("中abcdef", 2), "…");
    }

    #[test]
    fn repeated_model_rounds_and_cancellation_leave_only_meaningful_history() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        let mut terminal = vt100::Parser::new(24, 80, 100);
        paint_event(&mut state, &mut terminal, UiEvent::TurnStarted(7), 80, 24);
        for detail in ["正在思考中", "Thinking", "正在思考中"] {
            paint_event(&mut state, &mut terminal, UiEvent::ActivityChanged(ActivityState {
                key: "processing".into(), glyph: "⠋".into(), detail: detail.into(),
            }), 80, 24);
            assert!(terminal.screen().contents().contains(detail));
            paint_event(&mut state, &mut terminal, UiEvent::ActivityCleared, 80, 24);
            assert!(!terminal.screen().contents().contains(detail));
        }
        assert!(state.history_tail.is_empty());
        paint_event(&mut state, &mut terminal, UiEvent::HistoryCommitted("tool result".into()), 80, 24);
        paint_event(&mut state, &mut terminal, UiEvent::ActivityChanged(ActivityState {
            key: "processing".into(), glyph: "⠋".into(), detail: "正在思考中".into(),
        }), 80, 24);
        paint_event(&mut state, &mut terminal, UiEvent::TurnCancelling(7), 80, 24);
        paint_event(&mut state, &mut terminal, UiEvent::TurnCompleted(7), 80, 24);
        assert!(state.activity.is_none());
        assert_eq!(state.history_tail.len(), 1);
        let screen = terminal.screen().contents();
        assert!(!screen.contains("正在思考中"), "{screen}");
        assert!(screen.contains("tool result"), "{screen}");
    }

    #[test]
    fn activity_ticks_and_command_receipt_stay_at_the_transcript_anchor() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        state.composer = Some(ComposerState {
            draft: "draft".into(),
            cursor: 5,
            queued_count: 0,
            placeholder: "input".into(),
        });
        let mut terminal = vt100::Parser::new(24, 80, 100);
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::HistoryCommitted("user message".into()),
            80,
            24,
        );
        for glyph in ["⠋", "⠙", "⠹", "⠸"] {
            paint_event(
                &mut state,
                &mut terminal,
                UiEvent::ActivityChanged(ActivityState {
                    key: "thinking".into(),
                    glyph: glyph.into(),
                    detail: "Thinking".into(),
                }),
                80,
                24,
            );
            assert!(terminal
                .screen()
                .contents()
                .lines()
                .nth(1)
                .unwrap()
                .contains("Thinking"));
            assert_eq!(state.history_cursor_row, Some(2));
            assert_eq!(
                state.history_tail.len(),
                1,
                "animation must not append history"
            );
            assert!(!build_frame(&state, 80, 24)
                .lines
                .iter()
                .any(|line| line.contains("Thinking")));
        }
        paint_event(&mut state, &mut terminal, UiEvent::ActivityCleared, 80, 24);
        assert!(!terminal.screen().contents().contains("Thinking"));
        paint_event(&mut state, &mut terminal, UiEvent::ActivityCleared, 80, 24);
        assert_eq!(state.history_tail.len(), 1, "thinking never enters history");
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::ActivityChanged(ActivityState {
                key: "command".into(),
                glyph: "⠋".into(),
                detail: "Running test-command".into(),
            }),
            80,
            24,
        );
        assert!(terminal
            .screen()
            .contents()
            .lines()
            .nth(1)
            .unwrap()
            .contains("Running test-command"));
        paint_event(&mut state, &mut terminal, UiEvent::ActivityCleared, 80, 24);
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::HistoryCommitted("  • Ran test-command".into()),
            80,
            24,
        );
        let screen = terminal.screen().contents();
        assert!(screen.lines().nth(1).unwrap().contains("Ran test-command"));
        assert_eq!(screen.matches("test-command").count(), 1);
        assert!(screen.lines().last().unwrap().contains("draft"));
    }

    #[test]
    fn activity_without_composer_is_visible_and_resize_does_not_duplicate_it() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        let mut terminal = vt100::Parser::new(24, 80, 100);
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::HistoryCommitted("earlier history".into()),
            80,
            24,
        );
        for (columns, rows) in [(80, 24), (30, 12), (80, 24)] {
            terminal.set_size(rows, columns);
            paint_event(
                &mut state,
                &mut terminal,
                UiEvent::ActivityChanged(ActivityState {
                    key: "thinking".into(),
                    glyph: "⠋".into(),
                    detail: "Thinking".into(),
                }),
                columns,
                rows,
            );
            let screen = terminal.screen().contents();
            assert_eq!(screen.matches("Thinking").count(), 1, "{screen}");
            assert_eq!(screen.matches("earlier history").count(), 1, "{screen}");
            assert!(screen.lines().nth(1).unwrap().contains("Thinking"));
        }
    }

    #[test]
    fn activity_animation_at_a_full_viewport_does_not_keep_scrolling_history() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        state.composer = Some(ComposerState {
            draft: "draft".into(),
            cursor: 5,
            queued_count: 0,
            placeholder: "input".into(),
        });
        let mut terminal = vt100::Parser::new(12, 30, 100);
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::HistoryCommitted((0..15).map(|index| format!("history {index}\n")).collect()),
            30,
            12,
        );
        let activity = ActivityState {
            key: "command".into(),
            glyph: "•".into(),
            detail: "a long command that wraps over several rows".into(),
        };
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::ActivityChanged(activity.clone()),
            30,
            12,
        );
        let visible = terminal.screen().contents();
        let anchor = state.history_cursor_row;
        let history = state.history_tail.clone();
        for _ in 0..30 {
            paint_event(
                &mut state,
                &mut terminal,
                UiEvent::ActivityChanged(activity.clone()),
                30,
                12,
            );
            assert_eq!(terminal.screen().contents(), visible);
            assert_eq!(state.history_cursor_row, anchor);
            assert_eq!(state.history_tail, history);
        }
    }

    #[test]
    fn permission_overlay_does_not_move_activity_into_the_composer() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        state.composer = Some(ComposerState {
            draft: "draft".into(),
            cursor: 5,
            queued_count: 0,
            placeholder: "input".into(),
        });
        let mut terminal = vt100::Parser::new(24, 80, 100);
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::ActivityChanged(ActivityState {
                key: "thinking".into(),
                glyph: "⠋".into(),
                detail: "Thinking".into(),
            }),
            80,
            24,
        );
        state.prompt_frame = Some(vec!["Permission required".into(), "Allow / Deny".into()]);
        paint_event(&mut state, &mut terminal, UiEvent::PromptStarted, 80, 24);
        assert!(terminal
            .screen()
            .contents()
            .lines()
            .next()
            .unwrap()
            .contains("Thinking"));
        assert!(!terminal.screen().contents().contains("draft"));
        paint_event(&mut state, &mut terminal, UiEvent::PromptCompleted, 80, 24);
        assert_eq!(terminal.screen().contents().matches("Thinking").count(), 1);
        assert!(terminal
            .screen()
            .contents()
            .lines()
            .last()
            .unwrap()
            .contains("draft"));
    }

    #[test]
    fn review_boundary_settles_text_without_consuming_queued_input() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        state.reduce(UiEvent::TurnStarted(1));
        state.reduce(UiEvent::UserHistoryQueued("follow-up".into()));
        let mut terminal = vt100::Parser::new(24, 80, 100);
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::AssistantDelta("DRAFT-BODY".into()),
            80,
            24,
        );
        let row = terminal
            .screen()
            .contents()
            .lines()
            .position(|line| line.contains("DRAFT-BODY"));
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::AssistantSegmentCompleted,
            80,
            24,
        );
        assert_eq!(state.user_history_after_assistant.len(), 1);
        assert!(state.active_output.is_empty());
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::HistoryCommitted("[Answer verified]".into()),
            80,
            24,
        );
        let screen = terminal.screen().contents();
        assert_eq!(screen.matches("DRAFT-BODY").count(), 1);
        assert_eq!(
            screen.lines().position(|line| line.contains("DRAFT-BODY")),
            row
        );
        assert!(!screen.contains("follow-up"));
        paint_event(
            &mut state,
            &mut terminal,
            UiEvent::AssistantCompleted,
            80,
            24,
        );
        assert_eq!(terminal.screen().contents().matches("follow-up").count(), 1);
    }

    #[test]
    fn assistant_stream_and_final_share_the_history_anchor_not_the_bottom_pane() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        state.reduce(UiEvent::TurnStarted(1));
        state.composer = Some(ComposerState {
            draft: "draft".into(),
            cursor: 5,
            queued_count: 0,
            placeholder: "input".into(),
        });
        let mut parser = vt100::Parser::new(24, 80, 100);
        for delta in ["Hello", " there", "!", "\n\nNext paragraph"] {
            let committed = state.reduce(UiEvent::AssistantDelta(delta.into()));
            let mut bytes = Vec::new();
            for block in committed {
                write_cell_to(&mut state, &block, &mut bytes, 80, 24);
            }
            render_surface_to(&mut state, &mut bytes, 80, 24);
            parser.process(&bytes);
            assert!(parser
                .screen()
                .contents()
                .lines()
                .next()
                .unwrap()
                .contains("Hello"));
            assert_eq!(parser.screen().contents().matches("Hello").count(), 1);
            assert!(!build_frame(&state, 80, 24)
                .lines
                .iter()
                .any(|line| line.contains("Hello")));
        }
        let before = parser.screen().contents();
        let final_blocks = state.reduce(UiEvent::AssistantCompleted);
        let mut bytes = Vec::new();
        for block in final_blocks {
            write_cell_to(&mut state, &block, &mut bytes, 80, 24);
        }
        render_surface_to(&mut state, &mut bytes, 80, 24);
        parser.process(&bytes);
        assert_eq!(
            parser.screen().contents(),
            before,
            "finalization must not relocate visible text"
        );
    }

    #[test]
    fn permission_overlay_replaces_composer_and_keeps_all_details_pageable() {
        let mut state = UiState::default();
        state.composer = Some(ComposerState {
            draft: "private draft".into(),
            cursor: 0,
            queued_count: 0,
            placeholder: "input".into(),
        });
        state.prompt_frame = Some((0..30).map(|n| format!("permission detail {n}")).collect());
        let first = build_frame(&state, 80, 12);
        assert_eq!(first.lines.len(), 11);
        assert!(first.lines[0].contains("detail 0"));
        assert!(first.lines.last().unwrap().contains("PgUp/PgDn"));
        assert!(!first
            .lines
            .iter()
            .any(|line| line.contains("private draft")));
        state.prompt_scroll = 20;
        assert!(build_frame(&state, 80, 12)
            .lines
            .iter()
            .any(|line| line.contains("detail 29")));
        state.prompt_frame = None;
        assert!(build_frame(&state, 80, 12)
            .lines
            .iter()
            .any(|line| line.contains("private draft")));
    }

    #[test]
    fn resized_chinese_composer_replays_history_without_draft_or_footer_ghosts() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        remember_history(&mut state, "你好！这是已经提交的回答。\n");
        let mut terminal = vt100::Parser::new(24, 100, 100);
        terminal.process(b"\x1b[3;20r\x1b[?6h");
        for (columns, rows) in [(100, 24), (90, 60), (40, 12), (100, 40), (90, 60)] {
            terminal.set_size(rows, columns);
            for draft in ["审", "审计", "审计下", "审计下当前目录"] {
                state.idle_frame = Some(IdleFrameState {
                    lines: vec![
                        format!("\x1b[48;5;236m  {draft}\x1b[0m"),
                        "Context 10%".into(),
                    ],
                    cursor_row: 0,
                    cursor_column: (UnicodeWidthStr::width(draft) + 3) as u16,
                });
                let mut bytes = Vec::new();
                render_surface_to(&mut state, &mut bytes, columns, rows);
                terminal.process(&bytes);
                let visible = terminal.screen().contents();
                assert_eq!(visible.matches("Context").count(), 1, "{visible}");
                assert_eq!(visible.matches('审').count(), 1, "{visible}");
                assert!(visible.contains(draft), "{visible}");
                assert_eq!(terminal.screen().cursor_position().0, rows - 2);
                if (columns, rows) != (100, 24) {
                    assert_eq!(visible.matches("已经提交").count(), 1, "{visible}");
                }
            }
        }
    }

    #[test]
    fn viewport_diff_matches_full_repaint_through_resize_modal_and_wide_input() {
        let mut state = UiState::default();
        state.session_active = true;
        state.composer = Some(ComposerState { draft: "审计 👩‍💻 é".into(), cursor: 6,
            queued_count: 0, placeholder: "input".into() });
        state.turn = TurnPhase::Running { turn_id: 1 };
        state.deferred_history.push_back(TranscriptCell::Text("\x1b[31m红色历史\x1b[0m\nsecond line".into()));
        let mut incremental = vt100::Parser::new(12, 60, 0);
        for (columns, rows) in [(60, 12), (30, 8), (80, 20), (15, 6)] {
            incremental.set_size(rows, columns);
            for step in 0..12 {
                state.activity = Some(ActivityState { key: "thinking".into(), glyph: if step % 2 == 0 { "⠋" } else { "⠙" }.into(), detail: "Thinking".into() });
                state.prompt_frame = (step == 4).then(|| vec!["允许读取？".into(), "› 仅本次".into()]);
                state.composer.as_mut().unwrap().draft = if step % 2 == 0 { "审计 👩‍💻 é" } else { "短" }.into();
                state.composer.as_mut().unwrap().cursor = 0;
                let mut reference = state.clone();
                reference.painted_viewport = None;
                let mut full = vt100::Parser::new(rows, columns, 0);
                full.process(&pending_frame_bytes(&mut reference, columns, rows, false));
                incremental.process(&pending_frame_bytes(&mut state, columns, rows, false));
                assert_eq!(incremental.screen().contents_formatted(), full.screen().contents_formatted());
                assert_eq!(incremental.screen().cursor_position(), full.screen().cursor_position());
                assert_eq!(incremental.screen().hide_cursor(), full.screen().hide_cursor());
            }
        }
        let bytes = pending_frame_bytes(&mut state, 15, 6, false);
        assert!(!bytes.windows(4).any(|value| value == b"\x1b[2K"), "an unchanged frame must not clear any rows");
    }

    #[test]
    fn cancelling_has_distinct_feedback_and_preserves_draft_cursor() {
        let mut state = UiState::default();
        state.composer = Some(ComposerState { draft: "未发送的草稿".into(), cursor: 3,
            queued_count: 1, placeholder: "input".into() });
        state.turn = TurnPhase::Running { turn_id: 1 };
        let before = build_frame(&state, 80, 12);
        state.reduce(UiEvent::TurnCancelling(1));
        let after = build_frame(&state, 80, 12);
        assert!(after.lines.iter().any(|line| line.contains(stopping_hint())));
        assert_eq!(before.cursor, after.cursor);
        assert_eq!(state.composer.as_ref().unwrap().draft, "未发送的草稿");
        assert!(!after.lines.iter().any(|line| line.contains(crate::i18n::tr(crate::i18n::Message::InteractiveExecutionInputHint))));
    }

    #[test]
    fn history_navigation_preserves_editor_keys_and_modal_focus() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = UiState::default();
        state.session_active = true;
        state.presented_document_rows = 100;
        assert!(!navigate_history(&mut state, &KeyEvent::new(KeyCode::End, KeyModifiers::NONE), 9));
        assert!(navigate_history(&mut state, &KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), 9));
        assert_eq!(state.history_scroll, 9);
        assert!(state.history_reading);
        assert!(build_frame(&state, 80, 12).lines.iter().any(|line| line.contains("Ctrl+End") || line.contains("Strg+Ende") || line.contains("Ctrl+Fin")));
        assert!(navigate_history(&mut state, &KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL), 9));
        assert_eq!(state.history_scroll, 100);
        assert!(navigate_history(&mut state, &KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL), 9));
        assert!(!state.history_reading);
        assert_eq!(state.history_scroll, 0);
        state.prompt_depth = 1;
        assert!(!navigate_history(&mut state, &KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL), 9));
    }

    #[test]
    fn session_viewport_never_commits_composer_or_footer_to_scrollback() {
        let mut state = UiState::default();
        state.session_active = true;
        state.idle_frame = Some(IdleFrameState {
            lines: vec!["DRAFT 输入消息".into(), "FOOTER 缓存命中率".into()],
            cursor_row: 0, cursor_column: 1,
        });
        let mut terminal = vt100::Parser::new(24, 80, 200);
        terminal.process(b"shell-before-nola\r\n\x1b[?1049h");
        for (columns, rows) in [(80, 24), (40, 8), (100, 30), (60, 12)] {
            terminal.set_size(rows, columns);
            for index in 0..30 {
                state.deferred_history.push_back(TranscriptCell::Text(format!("answer {index}")));
                terminal.process(&pending_frame_bytes(&mut state, columns, rows, false));
                let visible = terminal.screen().contents();
                assert_eq!(visible.matches("DRAFT").count(), 1, "{visible}");
                assert_eq!(visible.matches("FOOTER").count(), 1, "{visible}");
            }
            state.history_scroll = 20;
            terminal.process(&pending_frame_bytes(&mut state, columns, rows, false));
            assert_eq!(terminal.screen().contents().matches("FOOTER").count(), 1);
            state.history_scroll = 0;
            state.history_reading = false;
        }
        terminal.process(b"\x1b[?1049l");
        let restored = terminal.screen().contents();
        assert!(!restored.contains("DRAFT"), "{restored}");
        assert!(!restored.contains("FOOTER"), "{restored}");
        assert!(restored.contains("shell-before-nola"), "{restored}");
    }

    #[test]
    fn history_reader_stays_on_same_lines_while_stream_and_chrome_grow() {
        let mut state = UiState::default();
        state.session_active = true;
        for index in 0..60 {
            state.deferred_history.push_back(TranscriptCell::Text(format!("history-{index:03}")));
        }
        let mut terminal = vt100::Parser::new(12, 80, 0);
        // This fixture starts from already presented history, not a pending
        // provider batch. Subsequent appends still use ordinary bounded frames.
        terminal.process(&delivered_pending_frames(&mut state, 80, 12));
        state.history_scroll = 25;
        terminal.process(&pending_frame_bytes(&mut state, 80, 12, false));
        let before = terminal.screen().contents();
        let first = before.lines().next().unwrap().to_owned();
        for index in 0..8 {
            state.deferred_history.push_back(TranscriptCell::Text(format!("new-{index}")));
            terminal.process(&pending_frame_bytes(&mut state, 80, 12, false));
            assert_eq!(terminal.screen().contents().lines().next().unwrap(), first);
        }
        state.footer = vec!["session footer".into()];
        terminal.process(&pending_frame_bytes(&mut state, 80, 12, false));
        assert_eq!(terminal.screen().contents().lines().next().unwrap(), first);
        assert!(state.history_reading);
    }

    fn frontier_fixture() -> UiState {
        let mut state = UiState::default();
        state.session_active = true;
        state.turn = TurnPhase::Running { turn_id:1 };
        state.composer = Some(ComposerState { draft:String::new(), cursor:0,
            queued_count:0, placeholder:crate::i18n::tr(crate::i18n::Message::InteractiveInputPlaceholder).into() });
        state.footer = vec!["Context 10%".into()];
        state
    }

    #[test]
    fn completed_business_keeps_a_bounded_paint_frontier_and_ordered_source() {
        let mut state = frontier_fixture();
        let source = (1..=48).map(|index|format!("FRONTIER-{index:02}\n")).collect::<String>();
        let cells = state.reduce(UiEvent::MarkdownDelta(source.clone()));
        state.deferred_history.extend(cells);
        let cells = state.reduce(UiEvent::AssistantCompleted);
        state.deferred_history.extend(cells);
        state.reduce(UiEvent::TurnCompleted(1));
        assert_eq!(state.turn,TurnPhase::Idle,"paint is not model business state");
        let mut terminal = vt100::Parser::new(12,80,0);
        terminal.process(&pending_frame_bytes(&mut state,80,12,true));
        let first = terminal.screen().contents();
        assert!(first.contains("FRONTIER-01"),"{first}");
        assert!(!first.contains("FRONTIER-48"),"{first}");
        assert!(state.frame_pending);
        let mut frames = 1;
        while state.frame_pending {
            terminal.process(&pending_frame_bytes(&mut state,80,12,false));
            let screen = terminal.screen().contents();
            assert_eq!(screen.matches(crate::i18n::tr(crate::i18n::Message::InteractiveInputPlaceholder)).count(),1,"{screen}");
            assert_eq!(screen.matches("Context").count(),1,"{screen}");
            frames += 1;
            assert!(frames <= 20,"frontier must converge without another model event");
        }
        assert!(frames > 1);
        assert!(terminal.screen().contents().contains("FRONTIER-48"));
        assert_eq!(state.history_tail.iter().map(TranscriptCell::source).collect::<String>(),source);
    }

    #[test]
    fn restored_completed_history_does_not_delay_new_help_or_permission_delivery() {
        let mut state = frontier_fixture();
        state.turn = TurnPhase::Idle;
        let source = (1..=80).map(|index| format!("OLD-{index:02}\n")).collect::<String>();
        let cells = state.reduce(UiEvent::HistoryCommitted(source.clone()));
        state.deferred_history.extend(cells);
        assert!(establish_history_tail_baseline(&mut state));
        let mut terminal = vt100::Parser::new(12, 80, 0);
        terminal.process(&pending_frame_bytes(&mut state, 80, 12, false));
        assert!(terminal.screen().contents().contains("OLD-80"));
        assert!(!state.frame_pending);
        assert_eq!(state.history_tail.iter().map(TranscriptCell::source).collect::<String>(), source);

        let cells = state.reduce(UiEvent::HistoryCommitted("Available commands: /help\n".into()));
        state.deferred_history.extend(cells);
        terminal.process(&pending_frame_bytes(&mut state, 80, 12, false));
        assert!(terminal.screen().contents().contains("Available commands: /help"));
        let mut delivered = Vec::new();
        assert_eq!(present_history_in_viewport(&mut state, &mut delivered,
            "Permission request: exact pending resource", 80, 12).unwrap(), HistoryPresentation::Presented);
        terminal.process(&delivered);
        assert!(terminal.screen().contents().contains("Permission request: exact pending resource"));
        assert_eq!(delivered.windows(b"\x1b[?2026l".len())
            .filter(|bytes| *bytes == b"\x1b[?2026l").count(), 1);

        state.history_scroll = 30;
        state.history_reading = true;
        state.reader_anchor = None;
        terminal.process(&pending_frame_bytes(&mut state, 80, 12, false));
        assert!(terminal.screen().contents().contains("OLD-"));
        assert!(state.history_tail.iter().map(TranscriptCell::source).collect::<String>().starts_with(&source));
    }

    #[test]
    fn history_tail_baseline_cannot_skip_running_or_mutable_business_output() {
        let mut state = frontier_fixture();
        state.deferred_history.push_back(TranscriptCell::Text("unpainted business\n".into()));
        assert!(!establish_history_tail_baseline(&mut state));
        assert_eq!(state.deferred_history.len(), 1);
        assert_eq!(state.committed_paint_frontier, 0);
        state.turn = TurnPhase::Idle;
        state.markdown_source = "unfinished streamed body".into();
        assert!(!establish_history_tail_baseline(&mut state));
        assert_eq!(state.deferred_history.len(), 1);
        assert_eq!(state.committed_paint_frontier, 0);
    }

    #[test]
    fn one_tall_markdown_cell_and_unfinished_tail_never_skip_their_first_rows() {
        for complete in [false,true] {
            let mut state = frontier_fixture();
            let source = format!("```rust\n{}{}",(1..=64)
                .map(|index|format!("CODE-{index:02}\n")).collect::<String>(),if complete {"```"} else {""});
            let cells = state.reduce(UiEvent::MarkdownDelta(source.clone()));
            state.deferred_history.extend(cells);
            if complete {
                let cells = state.reduce(UiEvent::AssistantCompleted);
                state.deferred_history.extend(cells);
            }
            let mut terminal = vt100::Parser::new(12,80,0);
            terminal.process(&pending_frame_bytes(&mut state,80,12,true));
            let first = terminal.screen().contents();
            assert!(first.contains("CODE-01"),"{first}");
            assert!(!first.contains("CODE-64"),"{first}");
            assert!(state.frame_pending);
            for _ in 0..40 {
                if !state.frame_pending { break; }
                terminal.process(&pending_frame_bytes(&mut state,80,12,false));
            }
            assert!(!state.frame_pending);
            assert!(terminal.screen().contents().contains("CODE-64"));
            if complete { assert_eq!(state.history_tail.back().unwrap().source(),source); }
            else { assert_eq!(state.markdown_source,source); }
        }
    }

    #[test]
    fn mutable_tail_cannot_jump_ahead_of_unpainted_committed_prefix() {
        let mut state = frontier_fixture();
        let cells = state.reduce(UiEvent::MarkdownDelta((1..=48)
            .map(|index|format!("PREFIX-{index:02}\n")).collect()));
        state.deferred_history.extend(cells);
        let mut terminal = vt100::Parser::new(12,80,0);
        terminal.process(&pending_frame_bytes(&mut state,80,12,false));
        let screen = terminal.screen().contents();
        assert!(screen.contains("PREFIX-01"),"{screen}");
        assert!(!screen.contains("PREFIX-47") && !screen.contains("PREFIX-48"),"{screen}");
        assert!(!state.markdown_source.is_empty(),"mutable source remains retained");
    }

    #[test]
    fn oversized_semantic_cell_is_retained_whole_instead_of_evicted_unpainted() {
        let mut state = frontier_fixture();
        let source = format!("```text\nOVERSIZED-FIRST\n{}\nOVERSIZED-LAST\n```",
            "retained-code\n".repeat(MAX_VIEWPORT_REPLAY_BYTES / 10));
        assert!(source.len() > MAX_VIEWPORT_REPLAY_BYTES);
        state.deferred_history.push_back(TranscriptCell::Markdown(source.clone()));
        let mut terminal = vt100::Parser::new(12,80,0);
        terminal.process(&pending_frame_bytes(&mut state,80,12,false));
        assert_eq!(state.history_tail.len(),1);
        assert_eq!(state.history_tail[0].source(),source);
        let screen = terminal.screen().contents();
        assert!(screen.contains("OVERSIZED-FIRST"),"{screen}");
        assert!(!screen.contains("OVERSIZED-LAST"),"{screen}");
        assert!(state.frame_pending);
    }

    struct FrameWitness {
        terminal: vt100::Parser,
        first: bool,
        later: bool,
        frames: usize,
    }
    impl FrameWitness {
        fn new() -> Self { Self {terminal:vt100::Parser::new(12,80,0),first:false,later:false,frames:0} }
    }
    impl Write for FrameWitness {
        fn write(&mut self,bytes:&[u8])->io::Result<usize> {
            self.terminal.process(bytes);
            let screen = self.terminal.screen().contents();
            self.first |= screen.contains("RETAINED-FIRST");
            if screen.contains("LATER-NOTIFICATION") {
                assert!(self.first,"a later notification cannot replace an unseen prefix");
                self.later = true;
            }
            self.frames += 1;
            Ok(bytes.len())
        }
        fn flush(&mut self)->io::Result<()> {Ok(())}
    }

    #[test]
    fn oversized_prefix_plus_later_text_and_notification_is_painted_before_eviction() {
        let mut state = frontier_fixture();
        let source = format!("```text\nRETAINED-FIRST\n{}\n```","retained-code\n".repeat(MAX_VIEWPORT_REPLAY_BYTES/10));
        state.deferred_history.push_back(TranscriptCell::Markdown(source.clone()));
        state.deferred_history.push_back(TranscriptCell::Text("later text".into()));
        state.deferred_history.push_back(TranscriptCell::Text("LATER-NOTIFICATION".into()));
        let mut witness = FrameWitness::new();
        let first = pending_frame_bytes(&mut state,80,12,false);
        witness.write_all(&first).unwrap(); witness.flush().unwrap(); evict_painted_history(&mut state);
        assert!(witness.first && !witness.later);
        assert_eq!(state.history_tail.len(),3,"unpainted prefix must survive post-flush cache pressure");
        assert_eq!(state.history_tail[0].source(),source);
        write_pending_delivery_frames(&mut state,80,12,&mut witness).unwrap();
        assert!(witness.later && witness.frames>1);
        assert_eq!(state.history_tail[0].source(),source,"giant cell still intersects the current viewport");
    }

    #[test]
    fn one_admission_batch_of_small_cells_cannot_evict_its_unpainted_first_cell() {
        let mut state = frontier_fixture();
        state.deferred_history.push_back(TranscriptCell::Text("RETAINED-FIRST".into()));
        for index in 0..300 { state.deferred_history.push_back(TranscriptCell::Text(format!("small-{index} {}","x".repeat(1024)))); }
        state.deferred_history.push_back(TranscriptCell::Text("LATER-NOTIFICATION".into()));
        let mut witness = FrameWitness::new();
        witness.write_all(&pending_frame_bytes(&mut state,80,12,false)).unwrap();
        witness.flush().unwrap(); evict_painted_history(&mut state);
        assert!(state.history_tail_bytes>MAX_VIEWPORT_REPLAY_BYTES);
        assert_eq!(state.history_tail.front().unwrap().source(),"RETAINED-FIRST");
        assert!(witness.first && !witness.later);
        write_pending_delivery_frames(&mut state,80,12,&mut witness).unwrap();
        assert!(witness.later);
        assert!(state.history_tail_bytes<=MAX_VIEWPORT_REPLAY_BYTES,"only fully painted offscreen cells may now leave the cache");
    }

    #[test]
    fn a_reader_anchor_prevents_eviction_of_its_cell_under_new_source_pressure() {
        let mut state = frontier_fixture();
        for label in ["first","reader","last"] {
            state.deferred_history.push_back(TranscriptCell::Text(format!("{label}\n{}","retained row\n".repeat(8000))));
        }
        let _ = delivered_pending_frames(&mut state,80,12);
        // Rewind into the first retained cell (earlier fully offscreen cells
        // may already have left the cache at the successful delivery flush).
        let length = state.presented_document_rows;
        state.history_reading = true;
        state.history_scroll = length.saturating_sub(20);
        let mut witness = FrameWitness::new();
        witness.write_all(&pending_frame_bytes(&mut state,80,12,false)).unwrap();
        witness.flush().unwrap(); evict_painted_history(&mut state);
        let anchor = state.reader_anchor.clone().unwrap();
        let first_id = state.history_first_cell_id;
        state.deferred_history.push_back(TranscriptCell::Text("new pending row\n".repeat(10000)));
        witness.write_all(&pending_frame_bytes(&mut state,80,12,false)).unwrap();
        witness.flush().unwrap(); evict_painted_history(&mut state);
        assert_eq!(state.reader_anchor.as_ref(),Some(&anchor));
        assert_eq!(state.history_first_cell_id,first_id,"viewport cell cannot be evicted");
        assert!(state.history_tail_bytes>MAX_VIEWPORT_REPLAY_BYTES);
        assert!(!state.frame_pending,"reading does not spin through unseen source");
    }

    #[test]
    fn pending_giant_cell_reflows_by_source_span_not_an_old_visual_row_count() {
        for (before,after) in [(14,120),(120,14)] {
            let mut state = frontier_fixture();
            let source = format!("```text\n{}\n```",(1..=12).map(|line|
                format!("SOURCE-{line:02} {}\n","abcdefghij".repeat(6))).collect::<String>());
            state.deferred_history.push_back(TranscriptCell::Markdown(source.clone()));
            for _ in 0..2 { let _ = pending_frame_bytes(&mut state,before,12,false); }
            let old = state.frontier_anchor.clone().unwrap();
            let mut terminal = vt100::Parser::new(12,after,0);
            terminal.process(&pending_frame_bytes(&mut state,after,12,false));
            assert_eq!(state.history_tail[0].source(),source);
            assert!(state.frame_pending,"resize must not silently promote the whole cell");
            assert!(!terminal.screen().contents().contains("SOURCE-12"));
            assert_eq!(state.frontier_anchor.as_ref().unwrap().cell_id,old.cell_id);
            assert!(state.presented_document_rows<state.history_layout.as_ref().unwrap().2.len());
        }
    }

    #[test]
    fn reader_reflow_restores_the_owned_source_line_and_wrap_offset() {
        let mut state = frontier_fixture();
        state.deferred_history.push_back(TranscriptCell::Markdown(format!("```text\n{}\n```",(0..64)
            .map(|line|format!("ROW-{line:02} {}\n","abcdefghij".repeat(30))).collect::<String>())));
        let _ = delivered_pending_frames(&mut state,80,12);
        let row = state.history_anchors.iter().enumerate().find(|(index,anchor)|
            *index>state.history_anchors.len()/2 && anchor.wrap_offset>0).unwrap().0;
        let (_,length,capacity) = state.history_viewport_extent.unwrap();
        state.history_scroll = length-row-capacity;
        state.history_reading = true;
        let _ = pending_frame_bytes(&mut state,80,12,false);
        let anchor = state.reader_anchor.clone().unwrap();
        assert!(anchor.wrap_offset>0,"the fixture must exercise an interior wrapped source line");
        for width in [18,120,40,80] {
            let _ = pending_frame_bytes(&mut state,width,12,false);
            let actual = state.reader_anchor.as_ref().unwrap();
            assert_eq!((actual.cell_id,actual.source_start,actual.source_end,actual.logical_line),
                (anchor.cell_id,anchor.source_start,anchor.source_end,anchor.logical_line));
            assert_eq!(actual.wrap_offset,anchor.wrap_offset,"resize must not progressively floor away the original source intent");
            let (_,length,capacity) = state.history_viewport_extent.unwrap();
            let first = length.saturating_sub(state.history_scroll).saturating_sub(capacity);
            let projected = &state.history_anchors[first];
            assert_eq!(projected.source_start,anchor.source_start);
            assert!(projected.wrap_offset<=anchor.wrap_offset);
            assert!(state.history_reading);
            assert!(!state.frame_pending);
        }
    }

    #[test]
    fn a_disappearing_synthetic_tail_keeps_the_confirmed_committed_prefix() {
        let mut state = frontier_fixture();
        let mut terminal = vt100::Parser::new(12,80,0);
        for line in 0..64 { state.deferred_history.push_back(TranscriptCell::Text(format!("old-history-{line}"))); }
        terminal.process(&delivered_pending_frames(&mut state,80,12));
        let old_rows = state.presented_document_rows;
        state.activity = Some(ActivityState {key:"model".into(),glyph:"*".into(),detail:"working".into()});
        state.assistant_preview = "committed preview".into();
        terminal.process(&delivered_pending_frames(&mut state,80,12));
        assert_eq!(state.frontier_anchor.as_ref().unwrap().cell_id,u64::MAX);
        let cells = state.reduce(UiEvent::AssistantCompleted);
        state.deferred_history.extend(cells);
        state.activity = None;
        terminal.process(&pending_frame_bytes(&mut state,80,12,false));
        assert!(state.presented_document_rows>=old_rows,"synthetic disappearance cannot replay from history zero");
        assert!(terminal.screen().contents().contains("committed preview"));
        assert!(!state.frame_pending,"one new source row converges without replaying all old history");
    }

    #[test]
    fn unfinished_markdown_tail_resize_does_not_promote_unpainted_source() {
        let mut state = frontier_fixture();
        state.markdown_source = format!("```text\n{}",(1..=12).map(|line|
            format!("MUTABLE-{line:02} {}\n","abcdefghij".repeat(6))).collect::<String>());
        for _ in 0..4 { let _ = pending_frame_bytes(&mut state,14,12,false); }
        let owner = state.frontier_anchor.clone().unwrap();
        let mut terminal = vt100::Parser::new(12,120,0);
        terminal.process(&pending_frame_bytes(&mut state,120,12,false));
        assert!(state.frame_pending);
        assert!(!terminal.screen().contents().contains("MUTABLE-12"));
        assert_eq!(state.frontier_anchor.as_ref().unwrap().cell_id,owner.cell_id);
        assert!(state.history_tail.is_empty(),"a presentation anchor is not a source commit");
    }

    #[test]
    fn delivery_frames_finish_the_visible_prefix_without_dropping_notification() {
        let mut state = frontier_fixture();
        state.deferred_history.push_back(TranscriptCell::Text((1..=48)
            .map(|index|format!("DELIVERY-{index:02}\n")).collect()));
        state.deferred_history.push_back(TranscriptCell::Text("EXACT-NOTIFICATION".into()));
        let bytes = delivered_pending_frames(&mut state,80,12);
        assert!(bytes.windows(b"\x1b[?2026l".len()).filter(|part|*part == b"\x1b[?2026l").count() > 1);
        assert!(!state.frame_pending);
        let mut terminal = vt100::Parser::new(12,80,0);
        terminal.process(&bytes);
        assert!(terminal.screen().contents().contains("EXACT-NOTIFICATION"));
        assert_eq!(state.history_tail.len(),2);
    }

    #[test]
    fn three_visible_rows_present_the_first_prefix_and_pause_while_reading() {
        let mut state = frontier_fixture();
        state.deferred_history.push_back(TranscriptCell::Text((1..=48)
            .map(|index|format!("THREE-{index:02}\n")).collect()));
        // Choose an exact three-row document viewport above existing chrome.
        let rows = build_frame(&state,80,24).lines.len() as u16 + 3;
        let mut terminal = vt100::Parser::new(rows,80,0);
        terminal.process(&pending_frame_bytes(&mut state,80,rows,true));
        let screen = terminal.screen().contents();
        assert!(screen.contains("THREE-01") && screen.contains("THREE-03"),"{screen}");
        assert!(!screen.contains("THREE-04"),"{screen}");
        assert_eq!(state.presented_document_rows,3);
        // A reader's redraw does not consume the backlog or schedule forever.
        state.history_reading = true;
        state.history_scroll = 1;
        terminal.process(&pending_frame_bytes(&mut state,80,rows,false));
        assert_eq!(state.presented_document_rows,3);
        assert!(!state.frame_pending);
        state.history_reading = false;
        state.history_scroll = 0;
        terminal.process(&pending_frame_bytes(&mut state,80,rows,false));
        assert_eq!(state.presented_document_rows,6);
        assert!(state.frame_pending);
    }

    #[test]
    fn deferred_or_failed_notification_does_not_mutate_retryable_presentation() {
        struct BrokenFlush;
        impl Write for BrokenFlush {
            fn write(&mut self, bytes:&[u8])->io::Result<usize> { Ok(bytes.len()) }
            fn flush(&mut self)->io::Result<()> { Err(io::Error::new(io::ErrorKind::BrokenPipe,"flush")) }
        }
        let mut state = frontier_fixture();
        state.deferred_history.push_back(TranscriptCell::Text("retained".into()));
        let before = format!("{state:?}");
        state.painted_viewport = Some((80, 12, vec!["previous screen".into()]));
        assert!(present_history_in_viewport(&mut state,&mut BrokenFlush,"notice",80,12).is_err());
        assert!(state.painted_viewport.is_none(), "failed delivery must invalidate uncertain screen cells");
        assert_eq!(format!("{state:?}"),before,"failed flush must not commit or duplicate source");
        state.history_reading = true;
        state.history_scroll = 1;
        let before = format!("{state:?}");
        let mut bytes = Vec::new();
        assert_eq!(present_history_in_viewport(&mut state,&mut bytes,"notice",80,12).unwrap(),HistoryPresentation::Deferred);
        assert!(bytes.is_empty());
        assert_eq!(format!("{state:?}"),before);
        state.history_reading = false;
        state.history_scroll = 0;
        assert_eq!(present_history_in_viewport(&mut state,&mut bytes,"notice",80,12).unwrap(),HistoryPresentation::Presented);
        assert_eq!(state.history_tail.iter().filter(|cell|cell.source()=="notice").count(),1);
    }

    #[test]
    fn leading_model_newlines_do_not_allocate_empty_presentation_rows() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(1));
        assert!(state
            .reduce(UiEvent::AssistantDelta("\n\n".into()))
            .is_empty());
        assert!(state.active_output.is_empty());
        state.reduce(UiEvent::AssistantDelta("你好".into()));
        let mut committed = state.reduce(UiEvent::AssistantDelta("\n\n第二段".into()));
        committed.extend(state.reduce(UiEvent::AssistantCompleted));
        assert_eq!(committed.concat(), "你好\n\n第二段");
        state.reduce(UiEvent::TurnCompleted(1));
        state.reduce(UiEvent::TurnStarted(2));
        state.reduce(UiEvent::AssistantPreviewDelta("\n\n  indented".into()));
        assert_eq!(state.assistant_preview, "  indented");
    }

    #[test]
    fn unpainted_replay_source_is_not_evicted_to_meet_a_cache_budget() {
        let mut state = UiState::default();
        remember_history(&mut state, &"中".repeat(MAX_VIEWPORT_REPLAY_BYTES));
        assert!(state.history_tail_bytes <= MAX_VIEWPORT_REPLAY_BYTES);
        remember_history(&mut state, "last answer\n");
        assert!(state.history_tail_bytes > MAX_VIEWPORT_REPLAY_BYTES);
        assert_eq!(state.history_tail.len(),2,"admission is not evidence of presentation");
        assert_eq!(state.history_tail.back().unwrap(), "last answer\n");
    }

    #[test]
    fn only_matching_completion_closes_a_turn() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(7));
        state.reduce(UiEvent::TurnCompleted(8));
        assert_eq!(state.turn, TurnPhase::Running { turn_id: 7 });
        state.reduce(UiEvent::TurnCompleted(7));
        assert_eq!(state.turn, TurnPhase::Idle);
    }

    #[test]
    fn tool_boundary_discards_only_the_mutable_assistant_preview() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(7));
        state.reduce(UiEvent::AssistantDelta("accepted answer".to_string()));
        state.reduce(UiEvent::AssistantPreviewDelta(
            "I will inspect the files before answering.".to_string(),
        ));
        assert_eq!(state.active_output, "accepted answer");
        assert!(!state.assistant_preview.is_empty());
        state.reduce(UiEvent::AssistantPreviewDiscarded);
        assert_eq!(state.active_output, "accepted answer");
        assert!(state.assistant_preview.is_empty());
    }

    #[test]
    fn accepted_preview_commits_without_replaying_its_text() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(7));
        state.reduce(UiEvent::AssistantPreviewDelta("final answer".to_string()));

        let committed = state.reduce(UiEvent::AssistantPreviewCommitted);

        assert!(committed.is_empty());
        assert_eq!(state.active_output, "final answer");
        assert!(state.assistant_preview.is_empty());
    }

    #[test]
    fn partial_completion_and_next_model_activity_keep_one_contiguous_document() {
        let mut state = UiState::default();
        state.history_cursor_row = Some(1);
        state.reduce(UiEvent::TurnStarted(7));
        state.composer = Some(ComposerState {
            draft: String::new(), cursor: 0, queued_count: 0, placeholder: "input".into(),
        });
        let mut terminal = vt100::Parser::new(24, 80, 100);
        paint_event(&mut state, &mut terminal, UiEvent::AssistantPreviewDelta("    PREFIX-ALPHA-".into()), 80, 24);
        paint_event(&mut state, &mut terminal, UiEvent::AssistantPreviewCommitted, 80, 24);
        assert!(!state.assistant_at_line_start());
        for _ in 0..3 {
            paint_event(&mut state, &mut terminal, UiEvent::ActivityChanged(ActivityState {
                key: "next-model".into(), glyph: "•".into(), detail: "thinking".into(),
            }), 80, 24);
            paint_event(&mut state, &mut terminal, UiEvent::ActivityCleared, 80, 24);
            assert!(!state.assistant_at_line_start(), "activity is not a document newline");
        }
        paint_event(&mut state, &mut terminal, UiEvent::AssistantPreviewDelta("OMEGA".into()), 80, 24);
        paint_event(&mut state, &mut terminal, UiEvent::AssistantPreviewCommitted, 80, 24);
        let visible = terminal.screen().contents();
        assert_eq!(visible.matches("PREFIX-ALPHA-OMEGA").count(), 1, "{visible}");
        assert_eq!(visible.matches("PREFIX-ALPHA-").count(), 1, "{visible}");
        assert_eq!(state.active_output, "    PREFIX-ALPHA-OMEGA");
        assert!(!state.assistant_at_line_start());
        paint_event(&mut state, &mut terminal, UiEvent::AssistantCompleted, 80, 24);
        let visible = terminal.screen().contents();
        assert_eq!(visible.matches("PREFIX-ALPHA-OMEGA").count(), 1, "{visible}");
        assert!(state.assistant_at_line_start());
    }

    #[test]
    fn fresh_attempt_retirement_does_not_retract_an_accepted_continuation_prefix() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(7));
        state.reduce(UiEvent::AssistantPreviewDelta("PREFIX-ALPHA-".into()));
        state.reduce(UiEvent::AssistantPreviewCommitted);
        state.reduce(UiEvent::AssistantPreviewDelta("obsolete attempt".into()));
        state.reduce(UiEvent::AssistantPreviewDiscarded);
        assert_eq!(state.active_output, "PREFIX-ALPHA-");
        assert!(state.assistant_preview.is_empty());
        assert!(!state.assistant_at_line_start());
        state.reduce(UiEvent::AssistantPreviewDelta("OMEGA".into()));
        state.reduce(UiEvent::AssistantPreviewCommitted);
        assert_eq!(state.reduce(UiEvent::AssistantCompleted), vec!["PREFIX-ALPHA-OMEGA"]);
    }

    #[test]
    fn goal_and_visible_turn_have_independent_lifecycles() {
        let mut state = UiState::default();
        state.reduce(UiEvent::GoalChanged(GoalPhase::Active {
            goal_id: "goal-1".to_string(),
        }));
        state.reduce(UiEvent::TurnStarted(1));
        state.reduce(UiEvent::TurnCompleted(1));
        assert!(matches!(state.goal, GoalPhase::Active { .. }));
        assert_eq!(state.turn, TurnPhase::Idle);
    }

    #[test]
    fn cjk_cursor_uses_terminal_columns() {
        let composer = ComposerState {
            draft: "你好a".to_string(),
            cursor: "你好".len(),
            queued_count: 0,
            placeholder: "message".to_string(),
        };
        let (line, cursor) = composer_line(&composer, 40);
        assert_eq!(line, "  你好a");
        assert_eq!(cursor, 7);
    }

    #[test]
    fn queued_input_is_not_part_of_draft_and_survives_small_viewport() {
        let mut state = UiState::default();
        state.turn = TurnPhase::Running { turn_id: 1 };
        state.footer = vec!["model details".into(), "context details".into()];
        state.composer = Some(ComposerState {
            draft: "你好a".into(),
            cursor: "你好".len(),
            queued_count: 12,
            placeholder: "message".into(),
        });
        let frame = build_frame(&state, 40, 3);
        assert_eq!(frame.lines.len(), 2);
        assert_eq!(frame.lines[0], "  你好a");
        assert_eq!(frame.lines[1], "  model details");
        assert_eq!(frame.cursor, Some((2, 7)));
        assert!(!frame.lines.join("\n").contains("context details"));
        let tiny = build_frame(&state, 12, 2);
        assert_eq!(tiny.lines, vec!["  你好a"]);
    }

    #[test]
    fn session_footer_stays_below_input_across_turn_and_permission_transitions() {
        let mut state = UiState::default();
        state.reduce(UiEvent::FooterChanged(vec!["Context 10% · model · safe".into()]));
        state.reduce(UiEvent::IdleFrameChanged(Some(IdleFrameState {
            lines: vec!["draft".into()], cursor_row: 0, cursor_column: 3,
        })));
        let idle = build_frame(&state, 80, 24);
        assert_eq!(idle.lines.last().unwrap(), "  Context 10% · model · safe");
        assert_eq!(idle.cursor, Some((23, 3)));
        state.reduce(UiEvent::TurnStarted(1));
        state.reduce(UiEvent::ComposerChanged(ComposerState {
            draft: "next".into(), cursor: 4, queued_count: 0, placeholder: "message".into(),
        }));
        let active = build_frame(&state, 80, 24);
        assert_eq!(active.lines.last(), idle.lines.last());
        assert_eq!(active.cursor.unwrap().0, 23);
        state.reduce(UiEvent::PromptStarted);
        state.prompt_frame = Some(vec!["Permission".into(), "Allow once".into()]);
        assert_eq!(build_frame(&state, 80, 24).lines.last(), idle.lines.last());
        state.reduce(UiEvent::PromptCompleted);
        state.reduce(UiEvent::TurnCompleted(1));
        assert_eq!(build_frame(&state, 80, 24).lines.last(), idle.lines.last());
    }

    #[test]
    fn completed_active_reply_is_committed_once() {
        let mut state = UiState::default();
        state.composer = Some(ComposerState {
            draft: String::new(),
            cursor: 0,
            queued_count: 0,
            placeholder: "message".to_string(),
        });
        state.active_output = "first line\nsecond line".to_string();
        state.rendered_rows = build_frame(&state, 100, 30).lines.len();

        let committed = state.reduce(UiEvent::AssistantCompleted);
        assert_eq!(committed, vec!["first line\nsecond line"]);
        assert!(state.active_output.is_empty());

        state.reduce(UiEvent::TurnStarted(7));
        state.active_output = "must not be committed by lifecycle handoff".to_string();
        let committed = state.reduce(UiEvent::TurnCompleted(7));
        assert!(committed.is_empty());
        assert!(state.active_output.is_empty());
    }

    #[test]
    fn wrapped_stream_keeps_only_a_bounded_visual_tail() {
        let mut state = UiState::default();
        state.active_output = (1..=9)
            .map(|index| format!("{index}. a deliberately long visible line"))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            tail_visual_lines(&state.active_output, 30, MAX_ACTIVE_LINES).len(),
            MAX_ACTIVE_LINES
        );
        assert!(state.active_output.starts_with("1."));
    }

    #[test]
    fn active_reply_keeps_ansi_style_while_truncating_by_visible_width() {
        let rendered = truncate_styled_display("\x1b[31m你好abc\x1b[0m", 5);
        assert!(rendered.starts_with("\x1b[31m"));
        assert!(rendered.ends_with("\x1b[0m"));
        assert_eq!(strip_ansi(&rendered), "你好…");
    }

    #[test]
    fn streaming_keeps_a_bounded_mutable_tail() {
        let mut state = UiState::default();
        let input = (0..10)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let committed = state.reduce(UiEvent::AssistantDelta(input.clone()));
        assert!(!committed.is_empty());
        assert_eq!(committed.concat() + &state.active_output, input);
        assert!(state.active_output.lines().count() < 10);
        assert!(
            tail_visual_lines(&state.active_output, 80, MAX_ACTIVE_LINES).len() <= MAX_ACTIVE_LINES
        );
    }

    #[test]
    fn streaming_commits_stable_paragraphs_and_finishes_only_the_mutable_tail() {
        let mut state = UiState::default();
        let source = "    First paragraph.\n\n    Second paragraph.\n\n    Current paragraph.";

        let committed = state.reduce(UiEvent::AssistantDelta(source.to_string()));
        assert_eq!(
            committed,
            vec!["    First paragraph.\n\n    Second paragraph.\n\n"]
        );
        assert_eq!(state.active_output, "    Current paragraph.");

        let final_tail = state.reduce(UiEvent::AssistantCompleted);
        assert_eq!(final_tail, vec!["    Current paragraph."]);
        assert_eq!(committed.concat() + &final_tail.concat(), source);
    }

    #[test]
    fn long_single_line_stream_scans_each_delta_byte_once() {
        let mut state = UiState::default();
        for _ in 0..50_000 {
            assert!(state
                .reduce(UiEvent::AssistantDelta("x".to_string()))
                .is_empty());
        }

        assert_eq!(state.active_output.len(), 50_000);
        assert_eq!(state.active_output_boundaries.total_scanned_bytes, 50_000);
    }

    #[test]
    fn incremental_boundary_tracker_detects_paragraph_split_across_deltas() {
        let mut state = UiState::default();
        assert!(state
            .reduce(UiEvent::AssistantDelta("first\n".to_string()))
            .is_empty());
        assert!(state
            .reduce(UiEvent::AssistantDelta("\n".to_string()))
            .is_empty());
        let committed = state.reduce(UiEvent::AssistantDelta("second".to_string()));

        assert_eq!(committed, vec!["first\n\n"]);
        assert_eq!(state.active_output, "second");
        assert_eq!(state.active_output_boundaries.total_scanned_bytes, 13);
    }

    #[test]
    fn visual_lines_wrap_cjk_without_exceeding_content_width() {
        let lines = visual_lines("    这是一个用于验证中文终端主动换行的长句子", 16);
        assert!(lines.len() >= 2);
        assert!(lines
            .iter()
            .all(|line| { UnicodeWidthStr::width(strip_ansi(line).as_str()) <= 16 }));
        assert!(lines.iter().skip(1).all(|line| line.starts_with("    ")));
    }

    #[test]
    fn visual_lines_wrap_ascii_at_word_boundaries() {
        let lines = visual_lines(
            "    terminal_ui and emitter must never split an English identifier",
            28,
        );
        let plain = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();
        assert!(plain.len() > 1);
        assert!(plain
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 28));
        assert!(!plain.iter().any(|line| line.ends_with("identi")));
        assert!(plain
            .last()
            .is_some_and(|line| line.ends_with("identifier")));
    }

    #[test]
    fn styled_word_wrap_preserves_every_word_boundary() {
        let plain = "Both metadata runs succeeded and the workspace members are valid. Note the metadata output includes workspace members, but the two requested packages remain distinct.";
        let styled = format!("\x1b[38;5;252m{plain}\x1b[0m");
        let wrapped = visual_lines(&styled, 54)
            .into_iter()
            .map(|line| strip_ansi(&line))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            wrapped.split_whitespace().collect::<Vec<_>>(),
            plain.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn mixed_cjk_markdown_does_not_wrap_at_an_old_ascii_space() {
        let styled = concat!(
            "    \x1b[38;5;252m根目录 Cargo.toml 的 workspace members 有 ",
            "\x1b[1m12\x1b[0m\x1b[38;5;252m 个（core、core/nola-application、",
            "core/nola-domain、core/nola-host-protocol、core/nola-mobile-bindings、",
            "core/nola-model-runtime-host）。\x1b[0m"
        );
        let lines = visual_lines(styled, 120)
            .into_iter()
            .map(|line| strip_ansi(&line))
            .collect::<Vec<_>>();

        assert!(lines.len() >= 2);
        assert!(UnicodeWidthStr::width(lines[0].as_str()) >= 90, "{lines:?}");
        assert!(!lines[0].trim_end().ends_with("12"), "{lines:?}");
        assert!(lines
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 120));
    }

    #[test]
    fn visual_lines_use_hanging_indent_for_numbered_lists() {
        let lines = visual_lines("    12. numbered list content that must wrap cleanly", 22);
        assert!(lines.len() >= 2);
        assert!(lines[1].starts_with("        "));
        assert!(!lines[1].trim_start().starts_with("12."));
    }

    #[test]
    fn visual_lines_preserve_ansi_styles_across_wraps() {
        let lines = visual_lines("\x1b[38;5;252m    styled response content\x1b[0m", 14);
        assert!(lines.len() >= 2);
        assert!(lines.iter().all(|line| line.ends_with("\x1b[0m")));
        assert!(lines[1].contains("\x1b[38;5;252m"));
    }

    #[test]
    fn composer_owns_the_final_frame_cursor() {
        let mut state = UiState::default();
        state.reduce(UiEvent::TurnStarted(1));
        state.reduce(UiEvent::AssistantDelta("answer\n".to_string()));
        state.reduce(UiEvent::ActivityChanged(ActivityState {
            key: "tool-1".to_string(),
            glyph: "⠋".to_string(),
            detail: "reading".to_string(),
        }));
        state.reduce(UiEvent::ComposerChanged(ComposerState {
            draft: "next".to_string(),
            cursor: 4,
            queued_count: 0,
            placeholder: "message".to_string(),
        }));
        let frame = build_frame(&state, 80, 24);
        assert!(frame
            .lines
            .last()
            .is_some_and(|line| line.contains("  next")));
        assert_eq!(frame.cursor.map(|cursor| cursor.0), Some(24));
    }
}
