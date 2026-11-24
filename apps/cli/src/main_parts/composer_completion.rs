// SPDX-License-Identifier: Apache-2.0
// Slash commands, mentions and file completion at the terminal caret.

fn interactive_toggle_option_labels() -> [(&'static str, &'static str); 2] {
    [
        ("on", tr(Message::InteractiveStatusOn)),
        ("off", tr(Message::InteractiveStatusOff)),
    ]
}

fn interactive_toggle_matches<'a>(query: &'a str, include_empty: bool) -> Vec<&'static str> {
    let query = query.trim();
    let ascii_query = query.to_ascii_lowercase();
    let mut result = Vec::new();

    if include_empty && query.is_empty() {
        return interactive_toggle_option_labels()
            .iter()
            .map(|(_, label)| *label)
            .collect();
    }

    for (canonical, label) in interactive_toggle_option_labels() {
        if canonical.starts_with(&ascii_query) || label.starts_with(query) {
            result.push(label);
        }
    }

    result
}

struct SlashCompletion {
    name: String,
    label: Option<String>,
}

impl SlashCompletion {
    fn plain(name: impl Into<String>) -> Self {
        Self { name: name.into(), label: None }
    }
}

fn canonical_slash_command_completions(raw_prefix: &str) -> Vec<SlashCompletion> {
    let prefix = raw_prefix.to_ascii_lowercase();
    let resolved = normalize_interactive_command(&prefix);
    let mut entries = Vec::new();
    // Keep an exact typed alias executable, while drawing only from the
    // canonical command list so aliases never pollute completion menus.
    let resolved_canonical = resolved
        .filter(|name| INTERACTIVE_SLASH_COMMANDS.iter().any(|item| item.name == *name));
    if let Some(name) = resolved_canonical {
        entries.push(SlashCompletion::plain(name));
    }
    entries.extend(
        INTERACTIVE_SLASH_COMMANDS
            .iter()
            .filter(|candidate| {
                candidate.name.starts_with(&prefix)
                    && resolved_canonical != Some(candidate.name)
            })
            .map(|candidate| SlashCompletion::plain(candidate.name)),
    );
    entries
}

fn matching_slash_commands_with_config(input: &str, config: &Config) -> Vec<SlashCompletion> {
    if COMPOSER_PURPOSE.with(Cell::get) != ComposerPurpose::Chat {
        return Vec::new();
    }
    matching_slash_commands_inner(input, Some(config))
}

fn matching_slash_commands_inner(input: &str, config: Option<&Config>) -> Vec<SlashCompletion> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') {
        return Vec::new();
    }
    let mut segments = trimmed.splitn(2, char::is_whitespace);
    let raw_command = segments.next().unwrap_or("");
    let rest = segments.next().unwrap_or("").trim();
    // A short alias remains executable with Enter. While typing it, also offer
    // the canonical commands sharing the prefix (e.g. /m → /model, /mcp, /memory).
    if raw_command == trimmed && !INTERACTIVE_SLASH_COMMANDS.iter().any(|item| item.name == raw_command) {
        let matches = canonical_slash_command_completions(raw_command);
        if !matches.is_empty() {
            return matches;
        }
    }
    let command = normalize_interactive_command(raw_command).unwrap_or(raw_command);

    if command == "/room" && !rest.is_empty() {
        return interactive_room_completions(rest);
    }

    if command == "/model" && !rest.is_empty() {
        let (prefix, query) = interactive_model_completion_parts(rest);
        return interactive_model_candidates(config, query)
            .into_iter()
            .map(|item| SlashCompletion {
                name: format!("{prefix}{}", item.name),
                label: item.label.map(|label| format!("{prefix}{label}")),
            })
            .take(6)
            .collect();
    }
    if command == "/executor" {
        let query = rest.to_ascii_lowercase();
        // Completion is display-only. Discovery probes versions, authentication
        // and model catalogs by starting external processes; never do it for a
        // keystroke. The explicit picker/submission still checks availability.
        return nola_domain::execution_backend::ExecutionBackendKind::ALL
            .into_iter()
            .filter(|backend| query.is_empty() || backend.id().starts_with(&query))
            .map(|backend| SlashCompletion::plain(format!("/executor {}", backend.id())))
            .collect();
    }
    if command == "/goal" {
        return interactive_goal_subcommand_matches(rest);
    }
    if command == "/attach" {
        let Some(cfg) = config else {
            return Vec::new();
        };
        return workspace_file_completions(&cfg.project_root, rest)
            .into_iter()
            .filter(|entry| !entry.ends_with('/'))
            .map(|file| {
                let escaped = if file.contains(char::is_whitespace) {
                    format!("\"{file}\"")
                } else {
                    file
                };
                SlashCompletion::plain(format!("/attach {escaped}"))
            })
            .collect();
    }
    if command == "/debug" {
        return interactive_toggle_matches(rest, true)
            .into_iter()
            .map(|value| SlashCompletion::plain(format!("/debug {value}")))
            .collect();
    }
    if command == "/skills" {
        return ["list", "enable", "disable"]
            .into_iter()
            .filter(|sub| rest.is_empty() || sub.starts_with(&rest.to_ascii_lowercase()))
            .map(|sub| SlashCompletion::plain(format!("/skills {sub}")))
            .collect();
    }
    if command == "/mcp" {
        return ["list", "enable", "disable"]
            .into_iter()
            .filter(|sub| rest.is_empty() || sub.starts_with(&rest.to_ascii_lowercase()))
            .map(|sub| SlashCompletion::plain(format!("/mcp {sub}")))
            .collect();
    }
    if command == "/memory" {
        return ["list", "search", "add"]
            .into_iter()
            .filter(|sub| rest.is_empty() || sub.starts_with(&rest.to_ascii_lowercase()))
            .map(|sub| SlashCompletion::plain(format!("/memory {sub}")))
            .collect();
    }
    if command == "/allow" || trimmed.contains(char::is_whitespace) {
        return Vec::new();
    }

    let command = if command == raw_command {
        trimmed
    } else {
        command
    };

    canonical_slash_command_completions(command)
}

fn first_matching_slash_completion(input: &str, config: &Config) -> Option<SlashCompletion> {
    // The visible candidate list and Tab must use the same contract; independent
    // resolvers previously showed /executor candidates that Tab could not insert.
    matching_slash_commands_with_config(input, config).into_iter().next()
}

// Candidate selection is view state. Only Tab or Enter can change the draft.
#[derive(Default)]
struct SlashMenu {
    query: Option<String>,
    entries: Vec<SlashCompletion>,
    selected: Option<usize>,
    dismissed: bool,
}

thread_local! { static SLASH_MENU: std::cell::RefCell<SlashMenu> = Default::default(); }

impl SlashMenu {
    fn sync(&mut self, line: &str, cursor: usize, config: &Config) {
        let text = line.trim_start();
        let query = (COMPOSER_PURPOSE.with(Cell::get) == ComposerPurpose::Chat
            && cursor == line.len() && text.starts_with('/') && !line.contains('\n')).then_some(text);
        if self.query.as_deref() == query { return; }
        *self = Self::default();
        let Some(query) = query else { return };
        self.query = Some(query.to_owned());
        if query.contains(char::is_whitespace) {
            self.entries = matching_slash_commands_inner(query, Some(config));
            return;
        }
        self.entries = canonical_slash_command_completions(query);
        self.selected = (!self.entries.is_empty()).then_some(0);
    }

    fn is_open(&self) -> bool { !self.dismissed && !self.entries.is_empty() }

    fn step(&mut self, backwards: bool) {
        let count = self.entries.len();
        if count == 0 { return; }
        self.selected = Some(match (self.selected, backwards) {
            (Some(index), true) => (index + count - 1) % count,
            (Some(index), false) => (index + 1) % count,
            (None, true) => count - 1,
            (None, false) => 0,
        });
    }

    fn render(&self, width: usize, height: usize) -> Option<String> {
        use unicode_width::UnicodeWidthStr;

        if !self.is_open() { return None; }
        let count = self.entries.len().min(height.saturating_sub(6).clamp(1, 5));
        let selected = self.selected.unwrap_or(0);
        let start = selected.saturating_sub(count / 2).min(self.entries.len() - count);
        let visible_entries = &self.entries[start..start + count];
        let descriptions = interactive_help_rows();

        let max_box_width = composer_content_rule_width(width);
        let color = interactive_color_enabled();
        let dim = if color { "\x1b[2;39m" } else { "" };
        let accent = if color { "\x1b[36m" } else { "" };
        let bold = if color { "\x1b[1m" } else { "" };
        let reset = if color { INTERACTIVE_RESET } else { "" };

        if max_box_width < 12 {
            let mut rows = Vec::new();
            for (offset, item) in visible_entries.iter().enumerate() {
                let index = start + offset;
                let marker = if self.selected == Some(index) { "›" } else { " " };
                let label = item.label.as_deref().unwrap_or(&item.name);
                rows.push(truncate_display_width(&format!("  {marker} {label}"), width));
            }
            return Some(rows.join("\n"));
        }

        let inner_width = max_box_width.saturating_sub(4);
        let max_cmd_width = visible_entries
            .iter()
            .map(|item| UnicodeWidthStr::width(item.label.as_deref().unwrap_or(&item.name)))
            .max()
            .unwrap_or(10);
        let cmd_col_width = max_cmd_width
            .clamp(10, 18)
            .min(inner_width.saturating_sub(2));

        let mut rows = Vec::with_capacity(count + 2);

        let title_raw = self.query.as_deref().unwrap_or("/");
        let title = truncate_display_width(title_raw, inner_width.saturating_sub(2));
        let title_w = UnicodeWidthStr::width(title.as_str());
        let top_fill = "─".repeat((inner_width + 2).saturating_sub(title_w + 3));
        rows.push(format!("  {dim}╭─ {title} {top_fill}╮{reset}"));

        for (offset, item) in visible_entries.iter().enumerate() {
            let index = start + offset;
            let is_selected = self.selected == Some(index);
            let raw_label = item.label.as_deref().unwrap_or(&item.name);
            let label = truncate_display_width(raw_label, cmd_col_width);
            let label_w = UnicodeWidthStr::width(label.as_str());
            let cmd_pad = " ".repeat(cmd_col_width.saturating_sub(label_w));
            let cmd_cell = if is_selected {
                format!("{accent}›{reset} {bold}{label}{reset}{cmd_pad}")
            } else {
                format!("  {label}{cmd_pad}")
            };

            let used = 2 + cmd_col_width;
            let detail = descriptions
                .iter()
                .find(|(usage, _)| usage.split_whitespace().next() == Some(item.name.as_str()))
                .map(|(_, detail)| detail.as_str());
            let body = if let Some(detail) = detail {
                let desc_avail = inner_width.saturating_sub(used + 2);
                if desc_avail >= 4 {
                    let desc = truncate_display_width(detail, desc_avail);
                    let desc_w = UnicodeWidthStr::width(desc.as_str());
                    let trail = " ".repeat(inner_width.saturating_sub(used + 2 + desc_w));
                    format!("{cmd_cell}  {dim}{desc}{reset}{trail}")
                } else {
                    let trail = " ".repeat(inner_width.saturating_sub(used));
                    format!("{cmd_cell}{trail}")
                }
            } else {
                let trail = " ".repeat(inner_width.saturating_sub(used));
                format!("{cmd_cell}{trail}")
            };
            rows.push(format!("  {dim}│{reset} {body} {dim}│{reset}"));
        }

        if height > 6 {
            let hint_full = ui_text(UiText::SlashMenuKeys);
            let hint_src = if UnicodeWidthStr::width(hint_full) + 2 <= inner_width {
                hint_full
            } else {
                "↑↓ · Tab · Enter · Esc"
            };
            let hint = truncate_display_width(hint_src, inner_width.saturating_sub(2));
            let hint_w = UnicodeWidthStr::width(hint.as_str());
            let bot_fill = "─".repeat((inner_width + 2).saturating_sub(hint_w + 3));
            rows.push(format!("  {dim}╰─ {hint} {bot_fill}╯{reset}"));
        } else {
            let bot_fill = "─".repeat(inner_width + 2);
            rows.push(format!("  {dim}╰{bot_fill}╯{reset}"));
        }

        Some(rows.join("\n"))
    }
}

#[derive(Clone)]
struct ComposerCompletionCycle {
    span_start: usize,
    candidates: Vec<String>,
    index: usize,
    last_inserted_end: usize,
}

fn active_at_file_token(line: &str, cursor: usize) -> Option<(usize, usize, &str)> {
    let idx = cursor.min(line.len());
    if !line.is_char_boundary(idx) {
        return None;
    }
    let before = &line[..idx];
    let start = before.rfind('@')?;
    let prefix = &before[start + 1..];
    if prefix.chars().any(char::is_whitespace)
        || before[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@'))
    {
        return None;
    }
    Some((start, idx, prefix))
}

#[test]
fn inline_mentions_query_uses_the_caret_and_leaves_email_literal() {
    let line = "🙂请@审查之后继续";
    let at = "🙂请@审查".len();
    assert_eq!(
        active_at_file_token(line, at),
        Some(("🙂请".len(), at, "审查"))
    );
    for literal in ["mail@example.com", "a.b@example", "@@double", "@选完 继续"] {
        assert!(active_at_file_token(literal, literal.len()).is_none());
    }
}

fn interactive_mention_candidates(config: &Config, prefix: &str) -> Vec<(String, String)> {
    let experts = if config.save_session && config.tools_enabled {
        crate::cli_expert_mentions::completions(prefix, config.session_id.as_deref())
    } else {
        Vec::new()
    };
    let mut candidates = experts.into_iter().map(|e| (e.token, e.label)).collect::<Vec<_>>();
    candidates.extend(workspace_file_completions(&config.project_root, prefix).into_iter().map(|file| {
        let token = if file.starts_with("expert[") { format!("@./{file}") } else { format!("@{file}") };
        (token.clone(), token)
    }));
    candidates
}

const WORKSPACE_FILE_COMPLETION_SCAN_LIMIT: usize = 4_000;

#[derive(Debug, Clone, Copy)]
enum GitignoreGlobToken {
    DoubleStarSlash,
    DoubleStar,
    Star,
    AnyNonSlash,
    Literal(char),
}

fn compile_gitignore_glob(pattern: &str) -> Vec<GitignoreGlobToken> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::with_capacity(chars.len());
    let mut idx = 0usize;
    while idx < chars.len() {
        if chars[idx] == '*' && idx + 1 < chars.len() && chars[idx + 1] == '*' {
            if idx + 2 < chars.len() && chars[idx + 2] == '/' {
                tokens.push(GitignoreGlobToken::DoubleStarSlash);
                idx += 3;
            } else {
                tokens.push(GitignoreGlobToken::DoubleStar);
                idx += 2;
            }
        } else if chars[idx] == '*' {
            tokens.push(GitignoreGlobToken::Star);
            idx += 1;
        } else if chars[idx] == '?' {
            tokens.push(GitignoreGlobToken::AnyNonSlash);
            idx += 1;
        } else {
            tokens.push(GitignoreGlobToken::Literal(chars[idx]));
            idx += 1;
        }
    }
    tokens
}

fn gitignore_glob_matches(pattern: &str, text: &str) -> bool {
    let tokens = compile_gitignore_glob(pattern);
    let chars: Vec<char> = text.chars().collect();
    let mut prev = vec![false; chars.len() + 1];
    prev[0] = true;
    for token in tokens {
        let mut curr = vec![false; chars.len() + 1];
        match token {
            GitignoreGlobToken::Literal(expected) => {
                for j in 0..chars.len() {
                    if prev[j] && chars[j] == expected {
                        curr[j + 1] = true;
                    }
                }
            }
            GitignoreGlobToken::AnyNonSlash => {
                for j in 0..chars.len() {
                    if prev[j] && chars[j] != '/' {
                        curr[j + 1] = true;
                    }
                }
            }
            GitignoreGlobToken::Star => {
                for j in 0..=chars.len() {
                    if prev[j] {
                        curr[j] = true;
                    }
                    if j > 0 && curr[j - 1] && chars[j - 1] != '/' {
                        curr[j] = true;
                    }
                }
            }
            GitignoreGlobToken::DoubleStar => {
                let mut seen = false;
                for j in 0..=chars.len() {
                    if prev[j] {
                        seen = true;
                    }
                    if seen {
                        curr[j] = true;
                    }
                }
            }
            GitignoreGlobToken::DoubleStarSlash => {
                let mut seen = false;
                for j in 0..=chars.len() {
                    if prev[j] {
                        curr[j] = true;
                        seen = true;
                    }
                    if j > 0 && seen && chars[j - 1] == '/' {
                        curr[j] = true;
                    }
                }
            }
        }
        prev = curr;
    }
    prev[chars.len()]
}

#[derive(Debug, Clone)]
struct GitignoreRule {
    negated: bool,
    dir_only: bool,
    anchored: bool,
    pattern: String,
}

#[derive(Debug, Default, Clone)]
struct RootGitignoreMatcher {
    rules: Vec<GitignoreRule>,
}

impl RootGitignoreMatcher {
    fn from_workspace(project_root: &std::path::Path) -> Self {
        let Ok(content) = std::fs::read_to_string(project_root.join(".gitignore")) else {
            return Self::default();
        };
        Self::parse(&content)
    }

    fn parse(content: &str) -> Self {
        let mut rules = Vec::new();
        for raw_line in content.lines() {
            let mut line = raw_line.trim_end_matches('\r');
            if !line.ends_with("\\ ") {
                line = line.trim_end_matches(' ');
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negated, rest) = if let Some(stripped) = line.strip_prefix('!') {
                (true, stripped)
            } else if let Some(stripped) = line.strip_prefix("\\#").or_else(|| line.strip_prefix("\\!")) {
                (false, stripped)
            } else {
                (false, line)
            };
            if rest.is_empty() {
                continue;
            }
            let (dir_only, rest) = if let Some(stripped) = rest.strip_suffix('/') {
                (true, stripped)
            } else {
                (false, rest)
            };
            if rest.is_empty() {
                continue;
            }
            let (anchored, pattern) = if let Some(stripped) = rest.strip_prefix('/') {
                (true, stripped)
            } else {
                (rest.contains('/'), rest)
            };
            if pattern.is_empty() {
                continue;
            }
            rules.push(GitignoreRule {
                negated,
                dir_only,
                anchored,
                pattern: pattern.to_string(),
            });
        }
        Self { rules }
    }

    fn is_ignored(&self, rel_path: &str, name: &str, is_dir: bool) -> bool {
        let mut ignored = false;
        for rule in &self.rules {
            if rule.dir_only && !is_dir {
                continue;
            }
            let matched = if rule.anchored {
                gitignore_glob_matches(&rule.pattern, rel_path)
            } else {
                gitignore_glob_matches(&rule.pattern, name)
                    || gitignore_glob_matches(&rule.pattern, rel_path)
            };
            if matched {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

fn is_ignored_completion_dir_name(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "target"
                | "node_modules"
                | "dist"
                | "build"
                | "out"
                | "coverage"
                | "vendor"
                | "__pycache__"
                | "DerivedData"
                | "Pods"
                | "venv"
                | "env"
        )
}

fn subsequence_match_score(candidate_lower: &str, query_lower: &str) -> Option<usize> {
    let mut query_chars = query_lower.chars();
    let mut target = query_chars.next()?;
    let mut first_pos: Option<usize> = None;
    let mut prev_pos: Option<usize> = None;
    let mut prev_ch: Option<char> = None;
    let mut penalty = 0usize;

    for (pos, ch) in candidate_lower.chars().enumerate() {
        if ch == target {
            if first_pos.is_none() {
                first_pos = Some(pos);
            }
            if let Some(previous) = prev_pos {
                let gap = pos.saturating_sub(previous + 1);
                penalty = penalty.saturating_add(gap * 2);
            }
            let at_boundary = pos == 0
                || prev_ch.is_some_and(|before| matches!(before, '/' | '_' | '-' | '.'));
            if !at_boundary {
                penalty = penalty.saturating_add(2);
            }
            prev_pos = Some(pos);
            if let Some(next) = query_chars.next() {
                target = next;
            } else {
                let span = pos.saturating_sub(first_pos.unwrap_or(pos));
                return Some(penalty.saturating_add(span));
            }
        }
        prev_ch = Some(ch);
    }
    None
}

fn score_workspace_completion_match(candidate: &str, query_lower: &str) -> Option<(u8, usize, usize)> {
    if query_lower.is_empty() {
        let depth = candidate.trim_end_matches('/').matches('/').count();
        return Some((0, depth, candidate.len()));
    }
    let cand_lower = candidate.to_ascii_lowercase();
    let trimmed = cand_lower.trim_end_matches('/');
    let basename = trimmed.rsplit('/').next().unwrap_or(trimmed);

    if cand_lower.starts_with(query_lower) {
        return Some((0, 0, candidate.len()));
    }
    if basename.starts_with(query_lower) {
        return Some((1, 0, candidate.len()));
    }
    if let Some(pos) = basename.find(query_lower) {
        return Some((2, pos, candidate.len()));
    }
    if let Some(pos) = cand_lower.find(query_lower) {
        return Some((3, pos, candidate.len()));
    }
    let fuzzy_penalty = subsequence_match_score(basename, query_lower)
        .or_else(|| subsequence_match_score(&cand_lower, query_lower).map(|p| p.saturating_add(16)))?;
    Some((4, fuzzy_penalty, candidate.len()))
}

fn workspace_file_completions(project_root: &std::path::Path, prefix: &str) -> Vec<String> {
    let gitignore = RootGitignoreMatcher::from_workspace(project_root);
    let mut queue = std::collections::VecDeque::from([project_root.to_path_buf()]);
    let mut scored: Vec<((u8, usize, usize), String)> = Vec::new();
    let mut scanned = 0usize;
    let query = prefix.trim().to_ascii_lowercase();

    while let Some(dir) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if scanned >= WORKSPACE_FILE_COMPLETION_SCAN_LIMIT {
                break;
            }
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let is_dir = kind.is_dir();
            if is_dir && is_ignored_completion_dir_name(name_str.as_ref()) {
                continue;
            }
            if !is_dir && name_str.starts_with('.') {
                continue;
            }
            let Ok(rel) = path.strip_prefix(project_root) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if gitignore.is_ignored(&rel_str, name_str.as_ref(), is_dir) {
                continue;
            }
            scanned += 1;
            if is_dir {
                queue.push_back(path);
                let dir_entry = format!("{rel_str}/");
                if let Some(score) = score_workspace_completion_match(&dir_entry, &query) {
                    scored.push((score, dir_entry));
                }
            } else if let Some(score) = score_workspace_completion_match(&rel_str, &query) {
                scored.push((score, rel_str));
            }
        }
        if scanned >= WORKSPACE_FILE_COMPLETION_SCAN_LIMIT {
            break;
        }
    }
    scored.sort_by(|(a_score, a_path), (b_score, b_path)| {
        a_score.cmp(b_score).then_with(|| a_path.cmp(b_path))
    });
    scored.truncate(12);
    scored.into_iter().map(|(_, path)| path).collect()
}

#[cfg(test)]
mod workspace_completion_upgrade_tests {
    use super::*;

    #[test]
    fn root_gitignore_filters_files_directories_and_negations() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join(".gitignore"),
            "# comment\n*.log\n!keep.log\ngenerated/\n**/scratch/*.tmp\n",
        )
        .unwrap();
        std::fs::create_dir_all(workspace.path().join("generated")).unwrap();
        std::fs::create_dir_all(workspace.path().join("src/scratch")).unwrap();
        std::fs::create_dir_all(workspace.path().join("__pycache__")).unwrap();
        std::fs::write(workspace.path().join("debug.log"), "ignored").unwrap();
        std::fs::write(workspace.path().join("keep.log"), "kept").unwrap();
        std::fs::write(workspace.path().join("generated/out.rs"), "ignored").unwrap();
        std::fs::write(workspace.path().join("src/scratch/a.tmp"), "ignored").unwrap();
        std::fs::write(workspace.path().join("__pycache__/mod.pyc"), "ignored").unwrap();
        std::fs::write(workspace.path().join("src/lib.rs"), "kept").unwrap();

        let all = workspace_file_completions(workspace.path(), "");
        assert!(all.iter().any(|item| item == "keep.log"), "{all:?}");
        assert!(all.iter().any(|item| item == "src/lib.rs"), "{all:?}");
        assert!(!all.iter().any(|item| item.contains("debug.log")), "{all:?}");
        assert!(!all.iter().any(|item| item.contains("generated")), "{all:?}");
        assert!(!all.iter().any(|item| item.contains("a.tmp")), "{all:?}");
        assert!(!all.iter().any(|item| item.contains("__pycache__")), "{all:?}");
    }

    #[test]
    fn workspace_file_completions_rank_prefix_then_basename_then_fuzzy() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src/deep")).unwrap();
        std::fs::write(workspace.path().join("main.rs"), "").unwrap();
        std::fs::write(workspace.path().join("src/main_loop.rs"), "").unwrap();
        std::fs::write(workspace.path().join("src/domain.rs"), "").unwrap();
        std::fs::write(workspace.path().join("src/deep/my_api_index_node.rs"), "").unwrap();

        let matches = workspace_file_completions(workspace.path(), "main");
        assert_eq!(matches[0], "main.rs", "{matches:?}");
        assert_eq!(matches[1], "src/main_loop.rs", "{matches:?}");
        assert_eq!(matches[2], "src/domain.rs", "{matches:?}");
        assert!(
            matches.iter().any(|item| item == "src/deep/my_api_index_node.rs"),
            "subsequence fuzzy match should be included: {matches:?}"
        );
    }

    #[test]
    fn workspace_file_completions_scan_cap_exceeds_old_400_limit() {
        let workspace = tempfile::tempdir().unwrap();
        let bulk = workspace.path().join("bulk");
        std::fs::create_dir_all(&bulk).unwrap();
        for index in 0..500 {
            std::fs::write(bulk.join(format!("item_{index:04}.txt")), "").unwrap();
        }
        std::fs::write(bulk.join("zzz_needle_target.rs"), "").unwrap();

        let matches = workspace_file_completions(workspace.path(), "zzz_needle");
        assert_eq!(matches, vec!["bulk/zzz_needle_target.rs"]);
    }
}

fn insert_composer_candidate(
    line: &mut String, start: usize, end: usize, token: &str,
    mentions: &mut Vec<crate::cli_expert_mentions::Occurrence>,
    pastes: &mut Vec<InteractivePastedContent>,
) -> usize {
    let selected = crate::cli_expert_mentions::completion(token, start);
    let display = selected.as_ref().map(|r| r.text.as_str()).unwrap_or(token);
    adjust_pasted_ranges(pastes, start, end, display.len());
    line.replace_range(start..end, display);
    crate::cli_expert_mentions::replace(mentions, start, end, display.len());
    let at = start + display.len();
    if let Some(run) = selected {
        let index = mentions.partition_point(|r| r.start < start);
        mentions.insert(index, run);
    }
    at
}

fn apply_composer_completion_cycle(
    line: &mut String,
    cursor: &mut usize,
    config: &Config,
    cycle: &mut Option<ComposerCompletionCycle>,
    reverse: bool,
    mentions: &mut Vec<crate::cli_expert_mentions::Occurrence>,
    pastes: &mut Vec<InteractivePastedContent>,
) -> bool {
    if let Some(existing) = cycle.as_mut() {
        if *cursor == existing.last_inserted_end
            && existing.span_start <= existing.last_inserted_end
            && existing.last_inserted_end <= line.len()
            && !existing.candidates.is_empty()
        {
            if reverse {
                existing.index = (existing.index + existing.candidates.len() - 1)
                    % existing.candidates.len();
            } else {
                existing.index = (existing.index + 1) % existing.candidates.len();
            }
            let next = existing.candidates[existing.index].clone();
            let new_end = insert_composer_candidate(line, existing.span_start, existing.last_inserted_end, &next, mentions, pastes);
            existing.last_inserted_end = new_end;
            *cursor = new_end;
            return true;
        }
    }

    let trimmed = line.trim_start();
    let (span_start, span_end, candidates) = if trimmed.starts_with('/') {
        if *cursor != line.len() || line.contains('\n') { return false; }
        let items: Vec<String> = matching_slash_commands_with_config(line, config)
            .into_iter()
            .map(|c| c.name)
            .collect();
        (0usize, line.len(), items)
    } else if let Some((start, end, prefix)) = active_at_file_token(line, *cursor) {
        let items = interactive_mention_candidates(config, prefix).into_iter().map(|(token, _)| token).collect();
        (start, end, items)
    } else {
        return false;
    };
    if candidates.is_empty() {
        return false;
    }
    let idx = if reverse { candidates.len() - 1 } else { 0 };
    let replacement = candidates[idx].clone();
    let new_end = insert_composer_candidate(line, span_start, span_end, &replacement, mentions, pastes);
    *cursor = new_end;
    *cycle = Some(ComposerCompletionCycle {
        span_start,
        candidates,
        index: idx,
        last_inserted_end: new_end,
    });
    true
}
