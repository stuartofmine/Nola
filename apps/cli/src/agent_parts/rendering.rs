fn permission_recovery_hint(outcome: &ToolOutcome) -> Option<&'static str> {
    // Core-authored permission results, not keyword inference from arbitrary
    // command stderr. A rejected action remains rejected until a new request.
    let code = outcome.data.as_ref().and_then(|data| data.get("code")).and_then(Value::as_str);
    let text = outcome.data.as_ref().and_then(Value::as_str).unwrap_or(outcome.text.trim());
    if !outcome.error || !(matches!(text, "Permission denied" | "permission denied by policy" | "Permission denied by user")
        || matches!(code, Some("permission_denied" | "permission_resource_changed"))) {
        return None;
    }
    Some(tr(Message::PermissionDeniedGuidance))
}

fn plan_footer_summary(request: &ToolRequest) -> Option<String> {
    let todos = request.args.get("todos")?.as_array()?;
    if todos.is_empty() {
        return None;
    }
    let completed = todos
        .iter()
        .filter(|todo| todo.get("status").and_then(Value::as_str) == Some("completed"))
        .count();
    let current = todos.iter().find_map(|todo| {
        if todo.get("status").and_then(Value::as_str) != Some("in_progress") {
            return None;
        }
        todo.get("activeForm")
            .or_else(|| todo.get("active_form"))
            .or_else(|| todo.get("content"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    });
    let progress = plan_progress_glyphs(completed, todos.len(), current.is_some());
    Some(match current {
        Some(current) => format!(
            "{} {completed}/{}  {progress}  {} {}",
            ui_text(UiText::Plan),
            todos.len(),
            ui_text(UiText::Running),
            compact_plan_text(current, 38)
        ),
        None if completed == todos.len() => {
            format!(
                "{} {completed}/{}  {progress}  {}",
                ui_text(UiText::Plan),
                todos.len(),
                ui_text(UiText::Completed)
            )
        }
        None => format!(
            "{} {completed}/{}  {progress}  {}",
            ui_text(UiText::Plan),
            todos.len(),
            ui_text(UiText::WaitingNextStep)
        ),
    })
}

fn plan_footer_summary_from_plan(request: &ToolRequest) -> Option<String> {
    let steps = request.args.get("steps")?.as_array()?;
    let steps = steps
        .iter()
        .filter_map(plan_step_text)
        .map(str::trim)
        .filter(|step| !step.is_empty())
        .collect::<Vec<_>>();
    let first = steps.first()?;
    let progress = plan_progress_glyphs(0, steps.len(), true);
    Some(format!(
        "{} 0/{}  {progress}  {} {}",
        ui_text(UiText::Plan),
        steps.len(),
        ui_text(UiText::Running),
        compact_plan_text(first, 38)
    ))
}

fn plan_completion_summary(request: &ToolRequest) -> Option<String> {
    let todos = request.args.get("todos")?.as_array()?;
    if todos.is_empty()
        || !todos
            .iter()
            .all(|todo| todo.get("status").and_then(Value::as_str) == Some("completed"))
    {
        return None;
    }
    let total = todos.len();
    Some(format!(
        "✓ {}  ·  {total}/{total}",
        ui_text(UiText::Completed)
    ))
}

fn plan_progress_glyphs(completed: usize, total: usize, active: bool) -> String {
    let slots = total.min(8);
    if slots == 0 {
        return String::new();
    }
    let filled = if total <= slots {
        completed.min(slots)
    } else {
        completed.saturating_mul(slots) / total
    };
    let mut progress = String::new();
    for index in 0..slots {
        progress.push(if index < filled {
            '●'
        } else if active && index == filled {
            '◉'
        } else {
            '○'
        });
    }
    if total > slots {
        progress.push('…');
    }
    progress
}

fn plan_step_text(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| {
        value
            .get("content")
            .or_else(|| value.get("description"))
            .and_then(Value::as_str)
    })
}

fn compact_plan_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut shortened = compact
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    shortened.push('…');
    shortened
}

fn compact_shell_command_for_ui(command: &str, max_width: usize) -> String {
    let compact = command
        .split_whitespace()
        .filter(|part| *part != "2>&1")
        .collect::<Vec<_>>()
        .join(" ");
    let without_leading_cd = compact
        .split_once(" && ")
        .and_then(|(first, rest)| first.trim_start().starts_with("cd ").then_some(rest))
        .unwrap_or(compact.as_str());
    truncate_display_value(without_leading_cd, max_width.max(24))
}

pub(crate) fn compact_bash_request_for_ui(
    request: &ToolRequest,
    max_width: usize,
) -> Option<String> {
    let command = request
        .args
        .get("command")
        .or_else(|| request.args.get("cmd"))
        .and_then(Value::as_str)?;
    let command = compact_shell_command_for_ui(command, max_width);
    let cwd = request
        .args
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .filter(|cwd| {
            std::env::current_dir()
                .ok()
                .is_none_or(|current| current != std::path::Path::new(cwd))
        })
        .and_then(|cwd| std::path::Path::new(cwd).file_name())
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}/"));
    // Lead with the operation. A long project name must never consume the
    // width budget and hide the command the user is being asked to approve.
    let command = cwd
        .map(|cwd| format!("{command} · {cwd}"))
        .unwrap_or(command);
    let command = truncate_display_value(&command, max_width.max(24));
    (!command.is_empty()).then_some(command)
}

fn plan_ui_style(text: String, color: &str) -> String {
    if terminal_color_enabled() {
        format!("{color}{text}{CODE_COLOR_RESET}")
    } else {
        text
    }
}

fn plan_ui_dimensions() -> (usize, usize) {
    let width = crossterm::terminal::size()
        .map(|(width, _)| usize::from(width))
        .unwrap_or(80)
        .clamp(44, 100);
    (width, width.saturating_sub(14).clamp(24, 82))
}

fn plan_card_heading(label: &str, title: &str, width: usize, max_text: usize) -> String {
    let heading = format!("─ {label} · {} ", compact_plan_text(title, max_text));
    let rule = "─".repeat(width.saturating_sub(heading.chars().count() + 4));
    plan_ui_style(format!("  {heading}{rule}"), "\x1b[1;38;5;117m")
}

fn plan_card_lines(request: &ToolRequest) -> Vec<String> {
    let title = request
        .args
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_else(|| ui_text(UiText::ExecutionPlan));
    let summary = request
        .args
        .get("summary")
        .or_else(|| request.args.get("plan"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let steps = request
        .args
        .get("steps")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let (width, max_text) = plan_ui_dimensions();
    let mut lines = vec![plan_card_heading(
        ui_text(UiText::Plan),
        title,
        width,
        max_text,
    )];

    if !summary.trim().is_empty() {
        lines.push(format!(
            "  {}  {}",
            plan_ui_style(ui_text(UiText::Goal).to_string(), "\x1b[38;5;244m"),
            plan_ui_style(compact_plan_text(summary, max_text), "\x1b[38;5;252m")
        ));
    }
    lines.push(String::new());
    for (index, step) in steps.iter().filter_map(plan_step_text).enumerate() {
        let active = index == 0;
        let marker = if active { "◉" } else { "○" };
        let color = if active {
            "\x1b[1;38;5;117m"
        } else {
            "\x1b[38;5;244m"
        };
        lines.push(plan_ui_style(
            format!(
                "  {marker}  {:>2}  {}",
                index + 1,
                compact_plan_text(step, max_text)
            ),
            color,
        ));
    }
    append_plan_metadata_lines(&mut lines, request, max_text);
    lines
}

fn todo_card_lines(request: &ToolRequest) -> Vec<String> {
    let todos = request
        .args
        .get("todos")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let (width, max_text) = plan_ui_dimensions();
    let title = format!("{} {}", todos.len(), ui_text(UiText::Items));
    let mut lines = vec![plan_card_heading(
        ui_text(UiText::Tasks),
        &title,
        width,
        max_text,
    )];
    lines.push(String::new());
    for (index, todo) in todos.iter().enumerate() {
        let status = todo
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        let text = todo
            .get("activeForm")
            .or_else(|| todo.get("active_form"))
            .filter(|value| status == "in_progress" && value.as_str().is_some())
            .or_else(|| todo.get("content"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (marker, color) = match status {
            "completed" => ("✓", "\x1b[38;5;114m"),
            "in_progress" => ("◉", "\x1b[1;38;5;117m"),
            "cancelled" => ("–", "\x1b[38;5;244m"),
            _ => ("○", "\x1b[38;5;244m"),
        };
        lines.push(plan_ui_style(
            format!(
                "  {marker}  {:>2}  {}",
                index + 1,
                compact_plan_text(text, max_text)
            ),
            color,
        ));
    }
    lines
}

fn append_plan_metadata_lines(lines: &mut Vec<String>, request: &ToolRequest, max_text: usize) {
    let risks = request
        .args
        .get("risks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|risk| !risk.is_empty())
        .take(2)
        .collect::<Vec<_>>();
    let verification = request
        .args
        .get("verification")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if risks.is_empty() && verification.is_none() {
        return;
    }
    lines.push(String::new());
    if !risks.is_empty() {
        lines.push(format!(
            "  {}  {}",
            plan_ui_style(ui_text(UiText::Risks).to_string(), "\x1b[38;5;244m"),
            plan_ui_style(
                compact_plan_text(&risks.join(" · "), max_text),
                "\x1b[38;5;252m"
            )
        ));
    }
    if let Some(verification) = verification {
        lines.push(format!(
            "  {}  {}",
            plan_ui_style(ui_text(UiText::Verify).to_string(), "\x1b[38;5;244m"),
            plan_ui_style(compact_plan_text(verification, max_text), "\x1b[38;5;252m")
        ));
    }
}

fn tool_argument_summary(request: &ToolRequest) -> Option<String> {
    let mut parts = Vec::new();
    match request.name.as_str() {
        "read_file" | "file_outline" | "read_symbol" | "edit_file" | "edit_file_lines"
        | "write_file" | "append_file" | "notebook_read" | "notebook_edit" => {
            push_tool_arg(
                &mut parts,
                "path",
                first_tool_path_arg(&request.args, &["path", "file", "notebook_path"]),
            );
            push_tool_arg(
                &mut parts,
                "symbol",
                first_tool_arg(&request.args, &["symbol", "name"]),
            );
            push_tool_arg(&mut parts, "lines", line_range_arg_summary(&request.args));
        }
        "read_many_files" | "multi_edit_file" => {
            push_tool_arg(&mut parts, "paths", paths_arg_summary(&request.args));
        }
        "apply_patch" => {
            push_tool_arg(&mut parts, "patch", patch_scope_arg_summary(&request.args));
        }
        "grep" => {
            push_tool_arg(
                &mut parts,
                "path",
                first_tool_path_arg(&request.args, &["path", "root"]),
            );
            push_tool_arg(
                &mut parts,
                "pattern",
                first_tool_arg(&request.args, &["pattern"]),
            );
            push_tool_arg(&mut parts, "glob", first_tool_arg(&request.args, &["glob"]));
        }
        "code_search" | "symbol_search" | "implementation_search" | "workspace_symbols" => {
            push_tool_arg(
                &mut parts,
                "query",
                first_tool_arg(&request.args, &["query", "pattern", "symbol"]),
            );
            push_tool_arg(
                &mut parts,
                "path",
                first_tool_path_arg(&request.args, &["path", "root"]),
            );
        }
        "find_files" | "glob" => {
            push_tool_arg(
                &mut parts,
                "query",
                first_tool_arg(&request.args, &["query", "pattern", "glob"]),
            );
            push_tool_arg(
                &mut parts,
                "path",
                first_tool_path_arg(&request.args, &["path", "root"]),
            );
        }
        "bash" | "bash_command_review" => {
            push_tool_arg(
                &mut parts,
                "cmd",
                first_tool_arg(&request.args, &["command", "cmd"]),
            );
        }
        "bash_background_status" | "bash_background_wait" | "bash_background_stop" => {
            push_tool_arg(
                &mut parts,
                "task",
                first_tool_arg(&request.args, &["task_id"]),
            );
        }
        "web_search" | "tool_search" => {
            push_tool_arg(
                &mut parts,
                "query",
                first_tool_arg(&request.args, &["query"]),
            );
        }
        "web_fetch" | "chrome_native_open_page" | "computer_open_item" => {
            push_tool_arg(
                &mut parts,
                "url",
                first_tool_arg(&request.args, &["url", "target", "item"]),
            );
        }
        "chrome_native_click"
        | "chrome_native_type_text"
        | "chrome_native_assert"
        | "chrome_native_wait_for" => {
            push_tool_arg(
                &mut parts,
                "target",
                first_tool_arg(&request.args, &["selector", "text", "label", "role"]),
            );
        }
        "spawn_agent" => {
            push_tool_arg(
                &mut parts,
                "agent",
                first_tool_arg(&request.args, &["agent_type", "name"]),
            );
            push_tool_arg(
                &mut parts,
                "task",
                first_tool_arg(&request.args, &["description", "prompt"]),
            );
        }
        _ => {
            for key in ["path", "root"] {
                push_tool_arg(&mut parts, key, first_tool_path_arg(&request.args, &[key]));
            }
            for key in [
                "query",
                "pattern",
                "symbol",
                "url",
                "selector",
                "command",
                "operation",
                "action",
                "status",
            ] {
                push_tool_arg(&mut parts, key, first_tool_arg(&request.args, &[key]));
                if parts.len() >= 3 {
                    break;
                }
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        let omit_single_label = parts.len() == 1
            && matches!(
                request.name.as_str(),
                "read_file"
                    | "file_outline"
                    | "read_symbol"
                    | "edit_file"
                    | "edit_file_lines"
                    | "write_file"
                    | "append_file"
                    | "notebook_read"
                    | "notebook_edit"
                    | "read_many_files"
                    | "multi_edit_file"
                    | "apply_patch"
                    | "list_dir"
                    | "file_tree"
                    | "project_overview"
                    | "repo_map"
                    | "find_files"
                    | "glob"
                    | "code_search"
                    | "symbol_search"
                    | "implementation_search"
                    | "workspace_symbols"
                    | "web_search"
                    | "tool_search"
                    | "web_fetch"
                    | "chrome_native_open_page"
                    | "computer_open_item"
                    | "chrome_native_click"
                    | "chrome_native_type_text"
                    | "chrome_native_assert"
                    | "chrome_native_wait_for"
                    | "bash_background_status"
                    | "bash_background_wait"
                    | "bash_background_stop"
            );
        let summary = if omit_single_label {
            parts[0]
                .split_once('=')
                .map(|(_, value)| value)
                .unwrap_or(parts[0].as_str())
                .to_string()
        } else {
            parts.join(" ")
        };
        Some(truncate_display_value(
            &summary,
            HUMAN_STATUS_DETAIL_MAX_WIDTH,
        ))
    }
}

fn tool_result_summary(
    request: &ToolRequest,
    outcome: &ToolOutcome,
    is_error: bool,
) -> Option<String> {
    let fallback = if is_error {
        tool_error_detail(&request.name).map(str::to_string)
    } else {
        tool_done_detail(&request.name).map(str::to_string)
    };

    if is_error {
        let parsed = serde_json::from_str::<Value>(outcome.text.trim()).ok();
        for payload in outcome.data.iter().chain(parsed.iter()) {
            let detail = ["/message", "/error/message", "/error", "/stderr"]
                .into_iter().filter_map(|key| payload.pointer(key).and_then(Value::as_str))
                .find(|text| !text.trim().is_empty())
                .map(crate::i18n::localize_tool_error_display)
                .or_else(|| payload.get("exit_code").and_then(Value::as_i64)
                    .map(|code| format!("{} ({code})", ui_text(UiText::ExitCode))));
            if let Some(detail) = detail {
                return Some(truncate_display_value(
                    &redact_sensitive_text(&detail).replace('\n', " "),
                    HUMAN_STATUS_DETAIL_MAX_WIDTH,
                ));
            }
        }
        // A generic display label is not an explanation. Preserve the first
        // concrete text diagnostic when the host returned a plain-text error.
        if parsed.is_none() {
            if let Some(detail) = outcome.text.lines().map(str::trim).find(|line| !line.is_empty()) {
                let detail = crate::i18n::localize_tool_error_display(detail);
                if !value_contains_sensitive_data(&Value::String(detail.clone())) {
                    return Some(truncate_display_value(&redact_sensitive_text(&detail), HUMAN_STATUS_DETAIL_MAX_WIDTH));
                }
            }
        }
    }

    if !is_error
        && matches!(
            request.name.as_str(),
            "read_file"
                | "read_many_files"
                | "file_outline"
                | "list_dir"
                | "file_tree"
                | "project_overview"
                | "repo_map"
        )
    {
        return None;
    }

    let display = outcome.display.trim();
    if !display.is_empty() && !value_contains_sensitive_data(&Value::String(display.to_string())) {
        return Some(truncate_display_value(
            &redact_sensitive_text(display).replace('\n', " "),
            HUMAN_STATUS_DETAIL_MAX_WIDTH,
        ));
    }

    let Some(first_line) = outcome
        .text
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
    else {
        return fallback;
    };
    if first_line == "(no matches)" {
        return Some(crate::i18n::interaction_text().no_matches.to_string());
    }
    if first_line == "(no matches in requested window)" {
        return Some(
            crate::i18n::interaction_text()
                .no_matches_in_window
                .to_string(),
        );
    }
    if first_line.starts_with("permission_required") {
        return Some(
            crate::i18n::interaction_text()
                .confirmation_required
                .to_string(),
        );
    }
    if value_contains_sensitive_data(&Value::String(first_line.to_string())) {
        return fallback;
    }
    Some(truncate_display_value(
        &redact_sensitive_text(first_line).replace('\n', " "),
        HUMAN_STATUS_DETAIL_MAX_WIDTH,
    ))
}

fn push_tool_arg(parts: &mut Vec<String>, label: &str, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    parts.push(format!("{label}={value}"));
}

fn first_tool_arg(args: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if tool_arg_key_is_hidden(key) {
            continue;
        }
        if let Some(value) = args.get(*key).and_then(tool_arg_summary_value) {
            return Some(value);
        }
    }
    None
}

fn first_tool_path_arg(args: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    first_tool_arg(args, keys).map(|path| compact_tool_path_for_ui(&path))
}

fn compact_tool_path_for_ui(path: &str) -> String {
    let candidate = std::path::Path::new(path);
    if !candidate.is_absolute() {
        return path.to_string();
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(relative) = candidate.strip_prefix(cwd) {
            return if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                relative.to_string_lossy().into_owned()
            };
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(relative) = candidate.strip_prefix(std::path::Path::new(&home)) {
            return if relative.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", relative.to_string_lossy())
            };
        }
    }
    path.to_string()
}

fn tool_arg_summary_value(value: &Value) -> Option<String> {
    if value_contains_sensitive_data(value) {
        return Some("[redacted]".to_string());
    }
    match value {
        Value::String(text) => {
            let text = redact_sensitive_text(text).replace('\n', "\\n");
            let text = text.trim();
            if text.is_empty() {
                None
            } else {
                Some(truncate_display_value(text, 72))
            }
        }
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Array(items) => {
            if items.is_empty() {
                None
            } else if items.len() == 1 {
                tool_arg_summary_value(&items[0])
            } else {
                Some(format!("{} items", items.len()))
            }
        }
        Value::Object(object) => {
            if let Some(path) = object.get("path").and_then(tool_arg_summary_value) {
                Some(path)
            } else {
                Some(format!("{} fields", object.len()))
            }
        }
        Value::Null => None,
    }
}

fn tool_arg_key_is_hidden(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "content"
            | "contents"
            | "text"
            | "input"
            | "value"
            | "old_text"
            | "new_text"
            | "replacement"
            | "patch"
            | "diff"
            | "arguments"
            | "headers"
            | "cookies"
            | "password"
            | "token"
            | "api_key"
            | "api-key"
            | "authorization"
    )
}

fn line_range_arg_summary(args: &Map<String, Value>) -> Option<String> {
    let start = args
        .get("start_line")
        .or_else(|| args.get("line"))
        .or_else(|| args.get("line_start"))
        .and_then(Value::as_i64)?;
    let count = args
        .get("line_count")
        .or_else(|| args.get("count"))
        .and_then(Value::as_i64);
    if let Some(count) = count.filter(|value| *value > 0) {
        Some(format!("{start}+{count}"))
    } else if let Some(end) = args
        .get("end_line")
        .or_else(|| args.get("line_end"))
        .and_then(Value::as_i64)
        .filter(|end| *end >= start)
    {
        Some(format!("{start}-{end}"))
    } else {
        Some(start.to_string())
    }
}

fn paths_arg_summary(args: &Map<String, Value>) -> Option<String> {
    let value = args
        .get("paths")
        .or_else(|| args.get("files"))
        .or_else(|| args.get("edits"))?;
    match value {
        Value::Array(items) if items.is_empty() => None,
        Value::Array(items) => {
            let first = items
                .first()
                .and_then(tool_arg_summary_value)
                .unwrap_or_else(|| "item".to_string());
            if items.len() == 1 {
                Some(first)
            } else {
                Some(format!(
                    "{first} · {}",
                    remaining_items_label(items.len().saturating_sub(1))
                ))
            }
        }
        _ => tool_arg_summary_value(value),
    }
}

fn patch_scope_arg_summary(args: &Map<String, Value>) -> Option<String> {
    let patch = args.get("patch").and_then(Value::as_str)?;
    let files = patch
        .lines()
        .filter_map(|line| {
            line.strip_prefix("*** Update File: ")
                .or_else(|| line.strip_prefix("*** Add File: "))
                .or_else(|| line.strip_prefix("*** Delete File: "))
        })
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    if files.is_empty() {
        Some("[patch hidden]".to_string())
    } else if files.len() == 1 {
        Some(files[0].to_string())
    } else {
        Some(format!(
            "{} · {}",
            files[0],
            remaining_items_label(files.len().saturating_sub(1))
        ))
    }
}

fn remaining_items_label(count: usize) -> String {
    use crate::i18n::Language;

    match crate::i18n::current_language() {
        Language::ZhHans => format!("另有 {count} 项"),
        Language::De => format!("+{count} weitere"),
        Language::Es => format!("+{count} más"),
        Language::Fr => format!("+{count} autres"),
        Language::Ja => format!("ほか {count} 件"),
        Language::Ru => format!("ещё {count}"),
        Language::En => format!("+{count} more"),
    }
}

const CODE_COLOR_COMMENT: &str = "\x1b[38;5;249m";
pub(crate) const CODE_COLOR_KEYWORD: &str = "\x1b[38;5;110m";
const CODE_COLOR_STRING: &str = "\x1b[38;5;108m";
const CODE_COLOR_NUMBER: &str = "\x1b[38;5;179m";
const CODE_COLOR_TYPE: &str = "\x1b[38;5;117m";
const CODE_COLOR_FENCE: &str = "\x1b[2;38;5;245m";
const CODE_COLOR_RESET: &str = "\x1b[0m";
const MD_COLOR_BODY: &str = "\x1b[38;5;252m";
const MD_COLOR_LIST: &str = "\x1b[38;5;110m";
const MD_COLOR_QUOTE: &str = "\x1b[2;38;5;245m";
const MD_COLOR_HEADING_1: &str = "\x1b[1;38;5;117m";
const MD_COLOR_HEADING_2: &str = "\x1b[1;38;5;110m";
const MD_COLOR_HEADING_3: &str = "\x1b[1;38;5;109m";
const MD_COLOR_HEADING_4: &str = "\x1b[1;38;5;108m";
const MD_COLOR_HEADING_5: &str = "\x1b[1;38;5;179m";
const MD_COLOR_HEADING_6: &str = "\x1b[1;38;5;245m";
const MD_COLOR_BOLD: &str = "\x1b[1m";
const MD_COLOR_INLINE_CODE: &str = "\x1b[38;5;179m";
const MD_COLOR_LINK: &str = "\x1b[4;38;5;110m";
const MD_COLOR_RULE: &str = "\x1b[2;38;5;244m";
const MARKDOWN_TABLE_INDENT: usize = 2;
const MARKDOWN_TABLE_SAFETY_MARGIN: usize = 4;
const MARKDOWN_TABLE_MIN_COLUMN_WIDTH: usize = 3;
const MARKDOWN_TABLE_MAX_ROW_LINES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkdownTableAlign {
    Left,
    Center,
    Right,
}

struct MarkdownLine<'a> {
    raw: &'a str,
    has_newline: bool,
}

fn stable_markdown_stream_flush_len(text: &str) -> usize {
    let Some(last_newline) = text.rfind('\n') else {
        // Plain prose has no Markdown delimiter to await. It belongs in the
        // existing mutable transcript tail now, even if a provider pauses
        // after a short fragment without punctuation. Keep syntax-bearing
        // tails buffered so an incomplete span/table/fence is not committed as
        // plain history and stranded when its closing bytes arrive.
        let trimmed = text.trim_start();
        if !trimmed.starts_with(['-', '+'])
            && text.chars().all(|character| {
                !matches!(
                    character,
                    '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '|' | '<' | '>' | '#'
                )
            })
        {
            return text.len();
        }
        if text
            .chars()
            .last()
            .is_some_and(|ch| matches!(ch, '.' | '!' | '?' | '。' | '！' | '？' | ';' | '；'))
        {
            return text.len();
        }
        if text.chars().count() >= 96 {
            return keep_tail_start(text, 24);
        }
        return 0;
    };
    let complete_len = last_newline + 1;
    let complete = &text[..complete_len];
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut offset = 0usize;
    for line in complete.split_inclusive('\n') {
        let raw = line.trim_end_matches('\n').trim_end_matches('\r');
        lines.push((offset, raw));
        offset += line.len();
    }
    let Some((_, last_raw)) = lines.last() else {
        return complete_len;
    };
    if last_raw.trim().is_empty() {
        return complete_len;
    }
    let block_start = lines
        .iter()
        .enumerate()
        .rev()
        .skip(1)
        .find_map(|(index, (_, raw))| raw.trim().is_empty().then_some(index + 1))
        .unwrap_or(0);
    let block = &lines[block_start..];
    if let Some(table_start) = markdown_stream_open_table_start(block) {
        return lines
            .get(block_start + table_start)
            .map(|(start, _)| *start)
            .unwrap_or(0);
    }
    complete_len
}

fn bounded_markdown_stream_flush_len(text: &str, stable_len: usize) -> usize {
    let stable_len = stable_len.min(text.len());
    if stable_len == 0 {
        return 0;
    }
    let stable = &text[..stable_len];
    let mut logical_lines = 0usize;
    let mut character_count = 0usize;
    let mut soft_boundary = None;

    for (index, ch) in stable.char_indices() {
        character_count = character_count.saturating_add(1);
        let end = index + ch.len_utf8();
        if ch == '\n' {
            logical_lines = logical_lines.saturating_add(1);
            soft_boundary = Some(end);
            if logical_lines >= STREAM_COMMIT_MAX_LINES {
                return end;
            }
        } else if ch.is_whitespace()
            || matches!(
                ch,
                '.' | '!' | '?' | ';' | ':' | ',' | '。' | '！' | '？' | '；' | '：' | '，'
            )
        {
            if markdown_inline_boundary_is_stable(&stable[..end]) {
                soft_boundary = Some(end);
            }
        }

        if character_count >= STREAM_COMMIT_MAX_CHARS {
            if let Some(boundary) = soft_boundary.filter(|boundary| *boundary > 0) {
                return boundary;
            }
            if markdown_inline_boundary_is_stable(&stable[..end]) {
                return end;
            }
        }
        if character_count >= STREAM_COMMIT_SOFT_CHARS
            && matches!(ch, '.' | '!' | '?' | ';' | '。' | '！' | '？' | '；')
            && markdown_inline_boundary_is_stable(&stable[..end])
        {
            return end;
        }
    }
    markdown_inline_boundary_is_stable(stable)
        .then_some(stable_len)
        .unwrap_or(0)
}

fn markdown_inline_boundary_is_stable(text: &str) -> bool {
    let line = text
        .trim_end_matches(['\n', '\r'])
        .rsplit(['\n', '\r'])
        .next()
        .unwrap_or_default();
    let mut escaped = false;
    let mut inline_code_open = false;
    let mut strong_delimiters = 0usize;
    let mut strike_delimiters = 0usize;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '`' {
            let mut run = 1usize;
            while chars.peek() == Some(&'`') {
                chars.next();
                run += 1;
            }
            if run < 3 {
                inline_code_open = !inline_code_open;
            }
            continue;
        }
        if !inline_code_open && ch == '*' && chars.peek() == Some(&'*') {
            chars.next();
            strong_delimiters += 1;
        }
        if !inline_code_open && ch == '~' && chars.peek() == Some(&'~') {
            chars.next();
            strike_delimiters += 1;
        }
    }
    !inline_code_open && strong_delimiters % 2 == 0 && strike_delimiters % 2 == 0
}

#[cfg(test)]
#[test]
fn markdown_stream_never_cuts_through_inline_code() {
    let inline = "identifier_with_a_long_scope::that_must_remain_a_single_markdown_span ".repeat(3);
    let text = format!(
        "This prefix is long enough to approach the normal streaming boundary. `{inline}` trailing text.",
    );
    let opening = text.find('`').expect("opening backtick");
    let closing = text.rfind('`').expect("closing backtick");
    let boundary = bounded_markdown_stream_flush_len(&text, text.len());
    assert!(
        boundary <= opening || boundary > closing,
        "boundary={boundary}"
    );
    assert!(markdown_inline_boundary_is_stable(&text[..boundary]));
}

fn markdown_stream_flush_len(text: &str, stable_len: usize) -> usize {
    let stable_len = stable_len.min(text.len());
    if stable_len == 0 {
        return 0;
    }
    if markdown_stream_contains_table(&text[..stable_len]) {
        // Table parsing depends on the header and separator remaining adjacent
        // to every row. The generic three-line stream budget would otherwise
        // commit exactly header + separator + first row and strand the rest as
        // raw pipe-delimited text.
        stable_len
    } else {
        bounded_markdown_stream_flush_len(text, stable_len)
    }
}

fn markdown_stream_contains_table(text: &str) -> bool {
    let lines = text.lines().collect::<Vec<_>>();
    lines.windows(2).any(|pair| {
        markdown_table_cells(pair[0]).is_some_and(|header| {
            header.len() >= 2 && markdown_table_separator(pair[1], header.len())
        })
    })
}

fn markdown_stream_tail_is_open_table(block: &[(usize, &str)]) -> bool {
    markdown_stream_open_table_start(block).is_some()
}

fn markdown_stream_open_table_start(block: &[(usize, &str)]) -> Option<usize> {
    for start in 0..block.len() {
        let Some(header) = markdown_table_cells(block[start].1) else {
            continue;
        };
        if header.len() < 2 {
            continue;
        }
        if start + 1 == block.len() {
            return Some(start);
        }
        if !markdown_table_separator(block[start + 1].1, header.len()) {
            continue;
        }
        if block[start + 2..].iter().all(|(_, raw)| {
            markdown_table_cells(raw)
                .and_then(|cells| normalize_markdown_table_row(cells, header.len()))
                .is_some()
        }) {
            return Some(start);
        }
    }
    None
}

fn markdown_stream_tail_has_open_table(text: &str) -> bool {
    let complete = text
        .rfind('\n')
        .map(|last_newline| &text[..=last_newline])
        .unwrap_or(text);
    let lines = complete.lines().map(str::trim_end).collect::<Vec<_>>();
    if lines.is_empty() || lines.last().is_some_and(|line| line.trim().is_empty()) {
        return false;
    }
    let block_start = lines
        .iter()
        .enumerate()
        .rev()
        .skip(1)
        .find_map(|(index, raw)| raw.trim().is_empty().then_some(index + 1))
        .unwrap_or(0);
    let block = lines[block_start..]
        .iter()
        .enumerate()
        .map(|(index, raw)| (index, *raw))
        .collect::<Vec<_>>();
    markdown_stream_tail_is_open_table(&block)
}

fn next_stream_markdown_line(text: &str, force: bool) -> Option<(&str, usize, bool)> {
    if let Some(index) = text.find('\n') {
        let raw = text[..index].trim_end_matches('\r');
        return Some((raw, index + 1, true));
    }
    if force && !text.is_empty() {
        return Some((text.trim_end_matches('\r'), text.len(), false));
    }
    None
}

fn streaming_vertical_table_start(text: &str) -> Option<(Vec<String>, usize)> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        if !line.ends_with('\n') {
            break;
        }
        let raw = line.trim_end_matches('\n').trim_end_matches('\r');
        lines.push((offset + line.len(), raw));
        offset += line.len();
    }
    if lines.len() < 3 {
        return None;
    }
    let header = markdown_table_cells(lines[0].1)?;
    if header.len() < 2 {
        return None;
    }
    let align = markdown_table_separator_align(lines[1].1, header.len())?;
    let columns = header.len();
    let mut rows = vec![header.clone()];
    for (_, raw) in lines.iter().skip(2) {
        let Some(cells) = markdown_table_cells(raw)
            .and_then(|cells| normalize_markdown_table_row(cells, columns))
        else {
            break;
        };
        rows.push(cells);
    }
    if rows.len() < 2 || !markdown_table_prefers_vertical(&rows, &align) {
        return None;
    }
    Some((header, lines[1].0))
}

thread_local! {
    static DOCUMENT_WIDTH: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[derive(Clone, Debug)]
pub(crate) struct MarkdownSourceBlock {
    pub text: String,
    pub source_start: usize,
    pub source_end: usize,
}

pub(crate) fn render_markdown_document(source: &str, width: usize) -> String {
    render_markdown_document_with_source(source,width).into_iter().map(|block|block.text).collect()
}

/// The existing renderer owns source provenance as well as formatting. A
/// table is one source block; ordinary/code lines keep exact source spans.
pub(crate) fn render_markdown_document_with_source(source: &str, width: usize) -> Vec<MarkdownSourceBlock> {
    struct WidthGuard(Option<usize>);
    impl Drop for WidthGuard {
        fn drop(&mut self) { DOCUMENT_WIDTH.with(|slot| slot.set(self.0)); }
    }
    // Tables and rules are measured inside the assistant's four-column gutter.
    let _guard = WidthGuard(DOCUMENT_WIDTH.with(|slot| slot.replace(Some(width.saturating_sub(4).max(1)))));
    let mut rendered = Vec::new();
    let mut prose = String::new();
    let mut source_offset = 0;
    let mut prose_offset = 0;
    let mut fence: Option<(char, usize)> = None;
    let mut language = String::new();
    for line in source.split_inclusive('\n') {
        let start = source_offset;
        source_offset += line.len();
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().filter(|ch| *ch == '`' || *ch == '~');
        let count = marker.map_or(0, |ch| trimmed.chars().take_while(|c| *c == ch).count());
        if let Some((open, length)) = fence {
            let mut text = String::new();
            if marker == Some(open) && count >= length && trimmed[count..].trim().is_empty() {
                fence = None;
                text.push_str("╰─");
                if line.ends_with('\n') { text.push('\n'); }
            } else {
                // Code is literal, including Markdown delimiters and table pipes.
                text.push_str(&code_highlight_line(line.trim_end_matches('\n'), &language));
                if line.ends_with('\n') { text.push('\n'); }
            }
            rendered.push(MarkdownSourceBlock {text,source_start:start,source_end:source_offset});
        } else if count >= 3 {
            rendered.extend(render_markdown_text_with_source(&std::mem::take(&mut prose),prose_offset));
            fence = marker.map(|ch| (ch, count));
            language = normalize_code_lang(&trimmed[count..]);
            rendered.push(MarkdownSourceBlock {text:format!("╭─ {language}\n"),source_start:start,source_end:source_offset});
        } else {
            if prose.is_empty() { prose_offset = start; }
            prose.push_str(line);
        }
    }
    rendered.extend(render_markdown_text_with_source(&prose,prose_offset));
    let mut at_line_start = true;
    for block in &mut rendered {
        let next = block.text.chars().last().map_or(at_line_start, |ch|ch=='\n');
        block.text = indent_assistant_output(&block.text,at_line_start);
        at_line_start = next;
    }
    rendered
}

pub(crate) fn render_markdown_text(text: &str) -> String {
    render_markdown_text_with_source(text,0).into_iter().map(|block|block.text).collect()
}

fn render_markdown_text_with_source(text: &str, source_base: usize) -> Vec<MarkdownSourceBlock> {
    let color_enabled = terminal_color_enabled();
    let mut lines = Vec::new();
    for line in text.split_inclusive('\n') {
        lines.push(MarkdownLine {
            raw: line.trim_end_matches('\n').trim_end_matches('\r'),
            has_newline: line.ends_with('\n'),
        });
    }
    if lines.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let offsets = std::iter::once(source_base).chain(text.split_inclusive('\n').scan(source_base, |offset,line| {
        *offset += line.len(); Some(*offset)
    })).collect::<Vec<_>>();
    let mut index = 0usize;
    while index < lines.len() {
        if let Some((rendered, consumed, trailing_newline)) =
            try_render_markdown_table(&lines, index, color_enabled)
        {
            let mut text = apply_markdown_base_color(
                rendered,
                MD_COLOR_BODY,
                color_enabled,
            );
            if trailing_newline {
                text.push('\n');
            }
            out.push(MarkdownSourceBlock {text,source_start:offsets[index],source_end:offsets[index+consumed]});
            index += consumed;
            continue;
        }

        let line = lines[index].raw;
        let mut text = String::new();
        if !line.is_empty() {
            text.push_str(&render_markdown_line(line, color_enabled));
        }
        if lines[index].has_newline {
            text.push('\n');
        }
        out.push(MarkdownSourceBlock {text,source_start:offsets[index],source_end:offsets[index+1]});
        index += 1;
    }
    out
}

#[cfg(test)]
mod markdown_source_provenance_tests {
    use super::*;

    #[test]
    fn repeated_code_lines_have_distinct_original_source_spans() {
        let source = "intro\n\n```text\nsame\nsame\n```\n尾行";
        let blocks = render_markdown_document_with_source(source,30);
        let ranges = blocks.iter().map(|block|(block.source_start,block.source_end)).collect::<Vec<_>>();
        let mut expected = 0;
        for (start,end) in ranges {
            assert_eq!(start,expected);
            assert!(source.is_char_boundary(start) && source.is_char_boundary(end));
            assert!(end>start);
            expected=end;
        }
        assert_eq!(expected,source.len());
        let repeated = blocks.iter().filter(|block| &source[block.source_start..block.source_end]=="same\n").collect::<Vec<_>>();
        assert_eq!(repeated.len(),2);
        assert_ne!(repeated[0].source_start,repeated[1].source_start,"no content matching may choose the reader owner");
    }

    #[test]
    fn table_reflow_keeps_one_exact_source_block_not_guessed_rendered_row_owners() {
        let source = "before\n\n| A | B |\n| --- | --- |\n| first | second |\n\nafter\n";
        let table_start = source.find("| A").unwrap();
        let table_end = source.find("\n\nafter").unwrap()+1;
        for width in [18,120] {
            let blocks = render_markdown_document_with_source(source,width);
            assert!(blocks.iter().any(|block|block.source_start==table_start && block.source_end==table_end));
            assert_eq!(blocks.iter().map(|block|block.text.as_str()).collect::<String>(),render_markdown_document(source,width));
        }
    }
}

fn render_markdown_line(line: &str, color_enabled: bool) -> String {
    if line.is_empty() {
        return String::new();
    }

    if let Some(rendered) = markdown_heading(line, color_enabled) {
        return rendered;
    }
    if markdown_is_horizontal_rule(line) {
        let width = agent_terminal_columns().saturating_sub(8).clamp(3, 96);
        let rule = "─".repeat(width);
        if color_enabled {
            return format!("{MD_COLOR_RULE}{rule}{CODE_COLOR_RESET}");
        }
        return rule;
    }
    if let Some((start, end)) = markdown_list_prefix(line) {
        let marker = markdown_rendered_list_marker(&line[start..end], &line[end..]);
        let body_start = if markdown_task_marker(&line[end..]).is_some() {
            let body = line[end..].trim_start();
            let marker_len = body.find(']').map(|index| index + 1).unwrap_or(0);
            let after = body[marker_len..].trim_start();
            line.len() - after.len()
        } else {
            end
        };
        let prefix = if color_enabled {
            format!("{MD_COLOR_LIST}{marker}{CODE_COLOR_RESET}")
        } else {
            marker
        };
        return apply_markdown_base_color(
            format!(
                "{}{}{}",
                &line[..start],
                prefix,
                apply_markdown_inline_styles(line[body_start..].trim_start(), color_enabled)
            ),
            MD_COLOR_BODY,
            color_enabled,
        );
    }
    if let Some((start, end)) = markdown_blockquote_prefix(line) {
        let prefix = if color_enabled {
            format!("{MD_COLOR_QUOTE}┃ {CODE_COLOR_RESET}")
        } else {
            "┃ ".to_string()
        };
        return apply_markdown_base_color(
            format!(
                "{}{}{}",
                &line[..start],
                prefix,
                apply_markdown_inline_styles(line[end..].trim_start(), color_enabled)
            ),
            MD_COLOR_BODY,
            color_enabled,
        );
    }

    apply_markdown_base_color(
        apply_markdown_inline_styles(line, color_enabled),
        MD_COLOR_BODY,
        color_enabled,
    )
}

fn apply_markdown_base_color(rendered: String, base_color: &str, color_enabled: bool) -> String {
    if !color_enabled {
        return rendered;
    }

    // Inline spans reset the SGR state. Restore the semantic line color after
    // every reset instead of falling back to an arbitrary terminal foreground.
    let restore = format!("{CODE_COLOR_RESET}{base_color}");
    format!(
        "{base_color}{}{CODE_COLOR_RESET}",
        rendered.replace(CODE_COLOR_RESET, &restore)
    )
}

fn markdown_heading(line: &str, color_enabled: bool) -> Option<String> {
    let trimmed = line.trim_start();
    let mut mark_len = 0usize;
    for ch in trimmed.chars() {
        if ch == '#' {
            mark_len += 1;
        } else {
            break;
        }
    }
    if !(1..=6).contains(&mark_len) {
        return None;
    }
    let after = &trimmed[mark_len..];
    if after.is_empty() || !after.starts_with(' ') {
        return None;
    }
    let leading = &line[..line.len() - trimmed.len()];
    let title = after.trim_start();
    let rendered_title = apply_markdown_inline_styles(title, color_enabled);
    if !color_enabled {
        return Some(format!("{leading}{rendered_title}"));
    }
    let color = match mark_len {
        1 => MD_COLOR_HEADING_1,
        2 => MD_COLOR_HEADING_2,
        3 => MD_COLOR_HEADING_3,
        4 => MD_COLOR_HEADING_4,
        5 => MD_COLOR_HEADING_5,
        _ => MD_COLOR_HEADING_6,
    };
    Some(apply_markdown_base_color(
        format!("{leading}{rendered_title}"),
        color,
        color_enabled,
    ))
}

fn markdown_is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    (trimmed.chars().all(|ch| ch == '-') && trimmed.len() > 2)
        || (trimmed.chars().all(|ch| ch == '*') && trimmed.len() > 2)
        || (trimmed.chars().all(|ch| ch == '_') && trimmed.len() > 2)
}

fn markdown_list_prefix(line: &str) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut start = 0usize;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    if start >= bytes.len() {
        return None;
    }
    let rest = &line[start..];
    if rest.starts_with('•') {
        let mut end = start + '•'.len_utf8();
        if end < line.len() && line.as_bytes()[end].is_ascii_whitespace() {
            while end < line.len() && line.as_bytes()[end].is_ascii_whitespace() {
                end += 1;
            }
            return Some((start, end));
        }
    }
    if (bytes[start] == b'-' || bytes[start] == b'*' || bytes[start] == b'+')
        && start + 1 < bytes.len()
    {
        if bytes[start + 1].is_ascii_whitespace() {
            let mut end = start + 1;
            while end < bytes.len() && bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            return Some((start, end));
        }
    }
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end > start && end < bytes.len() && bytes[end] == b'.' {
        let mut marker_end = end + 1;
        if marker_end < bytes.len() && bytes[marker_end].is_ascii_whitespace() {
            while marker_end < bytes.len() && bytes[marker_end].is_ascii_whitespace() {
                marker_end += 1;
            }
            return Some((start, marker_end));
        }
    }
    None
}

fn is_user_cancelled_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("cancelled by user") || lower.contains("canceled by user")
}

fn markdown_blockquote_prefix(line: &str) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut start = 0usize;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    if start >= bytes.len() {
        return None;
    }
    if bytes[start] != b'>' {
        return None;
    }
    let mut end = start + 1;
    if end < bytes.len() && bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    Some((start, end))
}

fn markdown_rendered_list_marker(raw_marker: &str, body: &str) -> String {
    if let Some(done) = markdown_task_marker(body) {
        return if done {
            "☑ ".to_string()
        } else {
            "☐ ".to_string()
        };
    }
    let trimmed = raw_marker.trim();
    if matches!(trimmed, "-" | "*" | "+") {
        "• ".to_string()
    } else {
        format!("{trimmed} ")
    }
}

fn markdown_task_marker(body: &str) -> Option<bool> {
    let trimmed = body.trim_start();
    if trimmed.len() < 3 || !trimmed.starts_with('[') {
        return None;
    }
    let bytes = trimmed.as_bytes();
    if bytes.get(2) != Some(&b']') {
        return None;
    }
    match bytes.get(1).copied() {
        Some(b'x') | Some(b'X') => Some(true),
        Some(b' ') => Some(false),
        _ => None,
    }
}

fn try_render_markdown_table(
    lines: &[MarkdownLine<'_>],
    start: usize,
    color_enabled: bool,
) -> Option<(String, usize, bool)> {
    if start + 1 >= lines.len() {
        return None;
    }
    let header = markdown_table_cells(lines[start].raw)?;
    let align = markdown_table_separator_align(lines[start + 1].raw, header.len())?;
    if header.len() < 2 {
        return None;
    }
    let columns = header.len();
    let mut rows = vec![header];
    let mut consumed = 2usize;
    let mut cursor = start + 2;
    while cursor < lines.len() {
        let Some(cells) = markdown_table_cells(lines[cursor].raw)
            .and_then(|cells| normalize_markdown_table_row(cells, columns))
        else {
            break;
        };
        rows.push(cells);
        consumed += 1;
        cursor += 1;
    }
    let trailing_newline = lines[start + consumed - 1].has_newline;
    Some((
        render_markdown_table(&rows, &align, color_enabled),
        consumed,
        trailing_newline,
    ))
}

fn markdown_table_cells(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let mut cells = markdown_split_table_cells(trimmed);
    if cells.first().is_some_and(|cell| cell.is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|cell| cell.is_empty()) {
        cells.pop();
    }
    if cells.len() < 2 || cells.iter().all(|cell| cell.is_empty()) {
        return None;
    }
    Some(cells)
}

fn normalize_markdown_table_row(
    mut cells: Vec<String>,
    expected_columns: usize,
) -> Option<Vec<String>> {
    if cells.is_empty() || cells.len() > expected_columns {
        return None;
    }
    cells.resize(expected_columns, String::new());
    Some(cells)
}

fn markdown_split_table_cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    let mut in_code_span = false;
    for ch in line.chars() {
        if escaped {
            if ch == '|' {
                current.push('|');
            } else {
                current.push('\\');
                current.push(ch);
            }
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '`' {
            in_code_span = !in_code_span;
            current.push(ch);
            continue;
        }
        if ch == '|' && !in_code_span {
            cells.push(current.trim().to_string());
            current.clear();
            continue;
        }
        current.push(ch);
    }
    if escaped {
        current.push('\\');
    }
    cells.push(current.trim().to_string());
    cells
}

fn markdown_table_separator(line: &str, expected_columns: usize) -> bool {
    markdown_table_separator_align(line, expected_columns).is_some()
}

fn markdown_table_separator_align(
    line: &str,
    expected_columns: usize,
) -> Option<Vec<MarkdownTableAlign>> {
    let Some(cells) = markdown_table_cells(line) else {
        return None;
    };
    if cells.len() != expected_columns {
        return None;
    }
    let mut align = Vec::with_capacity(cells.len());
    for cell in cells {
        let compact = cell
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>();
        let dash_count = compact.chars().filter(|ch| *ch == '-').count();
        if dash_count < 3 || !compact.chars().all(|ch| ch == '-' || ch == ':') {
            return None;
        }
        let starts = compact.starts_with(':');
        let ends = compact.ends_with(':');
        align.push(match (starts, ends) {
            (true, true) => MarkdownTableAlign::Center,
            (false, true) => MarkdownTableAlign::Right,
            _ => MarkdownTableAlign::Left,
        });
    }
    Some(align)
}

fn render_markdown_table(
    rows: &[Vec<String>],
    align: &[MarkdownTableAlign],
    color_enabled: bool,
) -> String {
    let Some((widths, hard_wrap)) = markdown_table_layout(rows) else {
        return render_markdown_table_vertical(rows, color_enabled);
    };
    if markdown_table_row_lines_exceed_vertical_threshold(rows, &widths, hard_wrap) {
        return render_markdown_table_vertical(rows, color_enabled);
    }
    let column_count = widths.len();
    if column_count == 0 {
        return String::new();
    }
    let terminal_width = agent_terminal_columns().max(20);

    let mut out = String::new();
    out.push_str(&render_markdown_table_border(
        &widths,
        "╭",
        "┬",
        "╮",
        color_enabled,
    ));
    for (row_index, row) in rows.iter().enumerate() {
        out.push('\n');
        out.push_str(&render_markdown_table_row(
            row,
            &widths,
            align,
            row_index == 0,
            hard_wrap,
            color_enabled,
        ));
        if row_index == 0 {
            out.push('\n');
            out.push_str(&render_markdown_table_border(
                &widths,
                "├",
                "┼",
                "┤",
                color_enabled,
            ));
        } else if row_index + 1 < rows.len() {
            out.push('\n');
            out.push_str(&render_markdown_table_border(
                &widths,
                "├",
                "┼",
                "┤",
                color_enabled,
            ));
        }
    }
    out.push('\n');
    out.push_str(&render_markdown_table_border(
        &widths,
        "╰",
        "┴",
        "╯",
        color_enabled,
    ));
    let max_line_width = out.lines().map(display_width).max().unwrap_or(0);
    if max_line_width > terminal_width.saturating_sub(MARKDOWN_TABLE_SAFETY_MARGIN) {
        return render_markdown_table_vertical(rows, color_enabled);
    }
    out
}

fn markdown_table_layout(rows: &[Vec<String>]) -> Option<(Vec<usize>, bool)> {
    let column_count = rows.first().map(Vec::len).unwrap_or(0);
    if column_count == 0 {
        return Some((Vec::new(), false));
    }
    let terminal_width = agent_terminal_columns().max(20);
    let border_overhead = 1 + column_count * 3;
    let available_width = terminal_width
        .saturating_sub(MARKDOWN_TABLE_INDENT + border_overhead + MARKDOWN_TABLE_SAFETY_MARGIN);
    if available_width < column_count * MARKDOWN_TABLE_MIN_COLUMN_WIDTH {
        return None;
    }

    let mut min_widths = vec![MARKDOWN_TABLE_MIN_COLUMN_WIDTH; column_count];
    let mut ideal_widths = vec![MARKDOWN_TABLE_MIN_COLUMN_WIDTH; column_count];
    for row in rows {
        for (index, cell) in row.iter().enumerate().take(column_count) {
            min_widths[index] = min_widths[index].max(markdown_table_cell_min_width(cell));
            ideal_widths[index] = ideal_widths[index].max(markdown_table_cell_ideal_width(cell));
        }
    }
    let total_min = min_widths.iter().sum::<usize>();
    let total_ideal = ideal_widths.iter().sum::<usize>();
    if total_ideal <= available_width {
        Some((ideal_widths, false))
    } else if total_min <= available_width {
        Some((
            shrink_markdown_table_widths(&min_widths, &ideal_widths, available_width),
            false,
        ))
    } else {
        Some((
            distribute_markdown_table_widths(column_count, available_width),
            true,
        ))
    }
}

fn markdown_table_row_lines_exceed_vertical_threshold(
    rows: &[Vec<String>],
    widths: &[usize],
    hard_wrap: bool,
) -> bool {
    rows.iter()
        .map(|row| markdown_table_row_line_count(row, widths, hard_wrap))
        .max()
        .unwrap_or(1)
        > MARKDOWN_TABLE_MAX_ROW_LINES
}

fn markdown_table_prefers_vertical(rows: &[Vec<String>], _align: &[MarkdownTableAlign]) -> bool {
    let Some((widths, hard_wrap)) = markdown_table_layout(rows) else {
        return true;
    };
    markdown_table_row_lines_exceed_vertical_threshold(rows, &widths, hard_wrap)
}

fn render_markdown_table_row(
    row: &[String],
    widths: &[usize],
    align: &[MarkdownTableAlign],
    header: bool,
    hard_wrap: bool,
    color_enabled: bool,
) -> String {
    let cells = widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            wrap_markdown_table_cell(cell, *width, hard_wrap)
        })
        .collect::<Vec<_>>();
    let max_lines = cells.iter().map(Vec::len).max().unwrap_or(1);
    let offsets = cells
        .iter()
        .map(|lines| max_lines.saturating_sub(lines.len()) / 2)
        .collect::<Vec<_>>();
    let mut rendered_rows = Vec::with_capacity(max_lines);
    for line_index in 0..max_lines {
        let mut out = String::from("  ");
        push_table_rule(&mut out, "│", color_enabled);
        for (index, width) in widths.iter().enumerate() {
            if index > 0 {
                push_table_rule(&mut out, "│", color_enabled);
            }
            out.push(' ');
            let offset = offsets.get(index).copied().unwrap_or(0);
            let cell_line_index = line_index.saturating_sub(offset);
            let raw = if line_index >= offset {
                cells
                    .get(index)
                    .and_then(|lines| lines.get(cell_line_index))
                    .map(String::as_str)
                    .unwrap_or("")
            } else {
                ""
            };
            let rendered = apply_markdown_inline_styles(raw, color_enabled);
            let cell_align = if header {
                MarkdownTableAlign::Center
            } else {
                align
                    .get(index)
                    .copied()
                    .unwrap_or(MarkdownTableAlign::Left)
            };
            let rendered = pad_aligned_display_width(&rendered, *width, cell_align);
            if header && color_enabled {
                out.push_str(MD_COLOR_HEADING_6);
                out.push_str(&rendered);
                out.push_str(CODE_COLOR_RESET);
            } else {
                out.push_str(&rendered);
            }
            out.push(' ');
        }
        push_table_rule(&mut out, "│", color_enabled);
        rendered_rows.push(out);
    }
    rendered_rows.join("\n")
}

fn render_markdown_table_border(
    widths: &[usize],
    left: &str,
    join: &str,
    right: &str,
    color_enabled: bool,
) -> String {
    let segments = widths
        .iter()
        .map(|width| "─".repeat(width.saturating_add(2)))
        .collect::<Vec<_>>()
        .join(join);
    let line = format!(
        "{}{}{}{}",
        " ".repeat(MARKDOWN_TABLE_INDENT),
        left,
        segments,
        right
    );
    if color_enabled {
        format!("{MD_COLOR_RULE}{line}{CODE_COLOR_RESET}")
    } else {
        line
    }
}

fn push_table_rule(out: &mut String, text: &str, color_enabled: bool) {
    if color_enabled {
        out.push_str(MD_COLOR_RULE);
        out.push_str(text);
        out.push_str(CODE_COLOR_RESET);
    } else {
        out.push_str(text);
    }
}

fn markdown_table_cell_ideal_width(cell: &str) -> usize {
    display_width(&apply_markdown_inline_styles(cell, false)).max(MARKDOWN_TABLE_MIN_COLUMN_WIDTH)
}

fn markdown_table_cell_min_width(cell: &str) -> usize {
    markdown_table_cell_plain_text(cell)
        .split_whitespace()
        .map(display_width)
        .max()
        .unwrap_or(MARKDOWN_TABLE_MIN_COLUMN_WIDTH)
        .max(MARKDOWN_TABLE_MIN_COLUMN_WIDTH)
}

fn markdown_table_cell_plain_text(cell: &str) -> String {
    strip_ansi_sequences(&apply_markdown_inline_styles(cell, false))
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn shrink_markdown_table_widths(
    min_widths: &[usize],
    ideal_widths: &[usize],
    available_width: usize,
) -> Vec<usize> {
    let total_min = min_widths.iter().sum::<usize>();
    let extra_space = available_width.saturating_sub(total_min);
    let overflows = ideal_widths
        .iter()
        .zip(min_widths)
        .map(|(ideal, min)| ideal.saturating_sub(*min))
        .collect::<Vec<_>>();
    let total_overflow = overflows.iter().sum::<usize>();
    let mut widths = min_widths.to_vec();
    if total_overflow == 0 || extra_space == 0 {
        return widths;
    }
    let mut used_extra = 0usize;
    for index in 0..widths.len() {
        let extra = overflows[index] * extra_space / total_overflow;
        widths[index] += extra;
        used_extra += extra;
    }
    let mut remaining = extra_space.saturating_sub(used_extra);
    while remaining > 0 {
        let Some(index) = overflows
            .iter()
            .enumerate()
            .filter(|(index, overflow)| widths[*index] < ideal_widths[*index] && **overflow > 0)
            .max_by_key(|(_, overflow)| **overflow)
            .map(|(index, _)| index)
        else {
            break;
        };
        widths[index] += 1;
        remaining -= 1;
    }
    widths
}

fn distribute_markdown_table_widths(column_count: usize, available_width: usize) -> Vec<usize> {
    let base = (available_width / column_count).max(MARKDOWN_TABLE_MIN_COLUMN_WIDTH);
    let mut widths = vec![base; column_count];
    let mut current = widths.iter().sum::<usize>();
    while current > available_width {
        let Some((index, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > MARKDOWN_TABLE_MIN_COLUMN_WIDTH)
            .max_by_key(|(_, width)| **width)
        else {
            break;
        };
        widths[index] -= 1;
        current -= 1;
    }
    let mut index = 0usize;
    while current < available_width && !widths.is_empty() {
        let len = widths.len();
        let target = index % len;
        widths[target] += 1;
        current += 1;
        index += 1;
    }
    widths
}

fn markdown_table_row_line_count(row: &[String], widths: &[usize], hard_wrap: bool) -> usize {
    widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            wrap_markdown_table_cell(cell, *width, hard_wrap).len()
        })
        .max()
        .unwrap_or(1)
}

fn wrap_markdown_table_cell(cell: &str, width: usize, hard_wrap: bool) -> Vec<String> {
    if width == 0 {
        return vec![cell.to_string()];
    }
    let cleaned = cell
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in cleaned.split(' ') {
        let word_width = display_width(word);
        if hard_wrap && word_width > width {
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
                current_width = 0;
            }
            lines.extend(wrap_display_word(word, width));
            continue;
        }
        let separator_width = usize::from(!current.is_empty());
        if current_width + separator_width + word_width <= width {
            if !current.is_empty() {
                current.push(' ');
                current_width += 1;
            }
            current.push_str(word);
            current_width += word_width;
        } else {
            if !current.is_empty() {
                lines.push(current);
            }
            current = word.to_string();
            current_width = word_width;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn wrap_display_word(word: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in word.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if !current.is_empty() && current_width + char_width > width {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += char_width;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn pad_aligned_display_width(text: &str, width: usize, align: MarkdownTableAlign) -> String {
    let current = display_width(text);
    if current >= width {
        return text.to_string();
    }
    let padding = width - current;
    match align {
        MarkdownTableAlign::Left => pad_right_display_width(text, width),
        MarkdownTableAlign::Right => format!("{}{text}", " ".repeat(padding)),
        MarkdownTableAlign::Center => {
            let left = padding / 2;
            let right = padding - left;
            format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
        }
    }
}

fn render_markdown_table_vertical(rows: &[Vec<String>], color_enabled: bool) -> String {
    let Some(header) = rows.first() else {
        return String::new();
    };
    let mut out = String::new();
    for (row_index, row) in rows.iter().enumerate().skip(1) {
        out.push_str(&render_markdown_table_vertical_row(
            header,
            row,
            row_index > 1,
            color_enabled,
        ));
    }
    out
}

fn render_markdown_table_vertical_separator(color_enabled: bool) -> String {
    let terminal_width = agent_terminal_columns().max(20);
    let separator_width = terminal_width
        .saturating_sub(MARKDOWN_TABLE_INDENT)
        .min(40)
        .max(3);
    let separator = format!(
        "{}{}",
        " ".repeat(MARKDOWN_TABLE_INDENT),
        "─".repeat(separator_width)
    );
    if color_enabled {
        format!("{MD_COLOR_RULE}{separator}{CODE_COLOR_RESET}")
    } else {
        separator
    }
}

fn render_markdown_table_vertical_row(
    header: &[String],
    row: &[String],
    include_separator: bool,
    color_enabled: bool,
) -> String {
    let terminal_width = agent_terminal_columns().max(20);
    let mut out = String::new();
    if include_separator {
        out.push('\n');
        out.push_str(&render_markdown_table_vertical_separator(color_enabled));
        out.push('\n');
    }
    for (column, label) in header.iter().enumerate() {
        if column > 0 {
            out.push('\n');
        }
        let raw_value = row.get(column).map(String::as_str).unwrap_or("");
        let label_rendered = apply_markdown_inline_styles(label, color_enabled);
        let label_width = display_width(&label_rendered);
        let first_width = terminal_width
            .saturating_sub(MARKDOWN_TABLE_INDENT + label_width + 2)
            .max(10);
        let continuation_width = terminal_width
            .saturating_sub(MARKDOWN_TABLE_INDENT + 2)
            .max(10);
        let wrapped = wrap_markdown_table_cell(raw_value, first_width, true);
        out.push_str(&" ".repeat(MARKDOWN_TABLE_INDENT));
        if color_enabled {
            out.push_str(MD_COLOR_HEADING_6);
        }
        out.push_str(&label_rendered);
        if color_enabled {
            out.push_str(CODE_COLOR_RESET);
        }
        out.push_str(": ");
        let first = wrapped.first().map(String::as_str).unwrap_or("");
        out.push_str(&apply_markdown_inline_styles(first, color_enabled));
        for line in wrapped.iter().skip(1) {
            for (chunk_index, chunk) in wrap_markdown_table_cell(line, continuation_width, true)
                .into_iter()
                .enumerate()
            {
                if chunk_index > 0 || !chunk.is_empty() {
                    out.push('\n');
                    out.push_str(&" ".repeat(MARKDOWN_TABLE_INDENT + 2));
                }
                out.push_str(&apply_markdown_inline_styles(&chunk, color_enabled));
            }
        }
    }
    out
}

fn pad_right_display_width(text: &str, width: usize) -> String {
    let current = display_width(text);
    if current >= width {
        return text.to_string();
    }
    format!("{}{}", text, " ".repeat(width - current))
}

fn apply_markdown_inline_styles(line: &str, color_enabled: bool) -> String {
    let mut out = String::new();
    let mut index = 0usize;
    while index < line.len() {
        let rest = &line[index..];
        if let Some((label, url, consumed)) = parse_markdown_link(rest) {
            push_markdown_styled_span(&mut out, &label, MD_COLOR_LINK, color_enabled);
            if !url.trim().is_empty() {
                if color_enabled {
                    out.push_str(CODE_COLOR_COMMENT);
                    out.push_str(" (");
                    out.push_str(url.trim());
                    out.push(')');
                    out.push_str(CODE_COLOR_RESET);
                } else {
                    out.push_str(" (");
                    out.push_str(url.trim());
                    out.push(')');
                }
            }
            index += consumed;
            continue;
        }
        if rest.starts_with('`') {
            if let Some((content, consumed)) = parse_markdown_inline_code(rest) {
                push_markdown_styled_span(&mut out, &content, MD_COLOR_INLINE_CODE, color_enabled);
                index += consumed;
                continue;
            }
        }
        if rest.starts_with("**") {
            if let Some(end) = rest[2..].find("**") {
                let content = &rest[2..2 + end];
                push_markdown_nested_styled_span(&mut out, content, MD_COLOR_BOLD, color_enabled);
                index += end + 4;
                continue;
            }
        }
        if rest.starts_with("__") {
            if let Some(end) = rest[2..].find("__") {
                let content = &rest[2..2 + end];
                push_markdown_nested_styled_span(&mut out, content, MD_COLOR_BOLD, color_enabled);
                index += end + 4;
                continue;
            }
        }
        if rest.starts_with("~~") {
            if let Some(end) = rest[2..].find("~~") {
                let content = &rest[2..2 + end];
                push_markdown_nested_styled_span(
                    &mut out,
                    content,
                    CODE_COLOR_COMMENT,
                    color_enabled,
                );
                index += end + 4;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn parse_markdown_inline_code(rest: &str) -> Option<(String, usize)> {
    let tick_count = rest.chars().take_while(|ch| *ch == '`').count();
    if tick_count == 0 {
        return None;
    }
    let marker = "`".repeat(tick_count);
    let content_start = tick_count;
    let search = &rest[content_start..];
    let end = search.find(&marker)?;
    let raw = &search[..end];
    let content = if tick_count > 1
        && raw.starts_with(' ')
        && raw.ends_with(' ')
        && raw.len() >= 2
        && raw[1..raw.len() - 1].contains('`')
    {
        raw[1..raw.len() - 1].to_string()
    } else {
        raw.to_string()
    };
    Some((content, content_start + end + tick_count))
}

fn push_markdown_styled_span(out: &mut String, content: &str, color: &str, color_enabled: bool) {
    if color_enabled {
        out.push_str(color);
        out.push_str(content);
        out.push_str(CODE_COLOR_RESET);
    } else {
        out.push_str(content);
    }
}

fn push_markdown_nested_styled_span(
    out: &mut String,
    content: &str,
    color: &str,
    color_enabled: bool,
) {
    let rendered = apply_markdown_inline_styles(content, color_enabled);
    if color_enabled {
        out.push_str(color);
        out.push_str(&rendered);
        out.push_str(color);
        out.push_str(CODE_COLOR_RESET);
    } else {
        out.push_str(&rendered);
    }
}

fn parse_markdown_link(rest: &str) -> Option<(String, String, usize)> {
    if !rest.starts_with('[') {
        return None;
    }
    let close_label = rest[1..].find(']')? + 1;
    let after_label = &rest[close_label + 1..];
    if !after_label.starts_with('(') {
        return None;
    }
    let close_url = after_label[1..].find(')')? + 1;
    let label = &rest[1..close_label];
    let url = &after_label[1..close_url];
    let consumed = close_label + 1 + close_url + 1;
    Some((
        apply_markdown_inline_styles(label, false),
        url.to_string(),
        consumed,
    ))
}

fn normalize_code_lang(raw: &str) -> String {
    let trimmed = raw
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match trimmed.as_str() {
        "rust" | "rs" => "rust".to_string(),
        "json" => "json".to_string(),
        "toml" => "toml".to_string(),
        "bash" | "sh" | "shell" => "bash".to_string(),
        "py" | "python" => "python".to_string(),
        "js" | "javascript" | "ts" | "typescript" => "javascript".to_string(),
        "sql" => "sql".to_string(),
        "yaml" | "yml" => "yaml".to_string(),
        _ => trimmed.to_string(),
    }
}

fn print_code_fence_label(language: Option<&str>) {
    if !terminal_color_enabled() {
        if let Some(lang) = language {
            println!("╭─ {lang}");
        } else {
            println!("╰─");
        }
        return;
    }
    if let Some(lang) = language {
        println!("{CODE_COLOR_FENCE}╭─ {lang}{CODE_COLOR_RESET}");
    } else {
        println!("{CODE_COLOR_FENCE}╰─{CODE_COLOR_RESET}");
    }
}

fn code_highlight_line(line: &str, language: &str) -> String {
    if !terminal_color_enabled() {
        return line.to_string();
    }
    let trimmed = line.trim_start();
    if language == "bash" && (trimmed.starts_with('#') || trimmed.starts_with("$ ")) {
        return format!("{CODE_COLOR_COMMENT}{line}{CODE_COLOR_RESET}");
    }
    if trimmed.starts_with("//") || (language == "python" && trimmed.starts_with('#')) {
        return format!("{CODE_COLOR_COMMENT}{line}{CODE_COLOR_RESET}");
    }
    match language {
        "rust" => code_highlight_generic(line, "rust", RUST_KEYWORDS),
        "python" => code_highlight_generic(line, "python", PYTHON_KEYWORDS),
        "javascript" => code_highlight_generic(line, "javascript", JS_KEYWORDS),
        "json" | "toml" | "yaml" | "sql" => code_highlight_structured(line),
        _ => code_highlight_generic(line, "text", &[]),
    }
}

fn code_highlight_structured(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut index = 0usize;
    while index < line.len() {
        let ch = line[index..].chars().next().unwrap();
        if ch == '"' || ch == '\'' {
            let (consumed, colored) = highlight_string(line, index, ch, CODE_COLOR_STRING);
            output.push_str(&colored);
            index += consumed;
            continue;
        }
        if ch.is_ascii_digit() {
            let (consumed, colored) = highlight_number(line, index, CODE_COLOR_NUMBER);
            output.push_str(&colored);
            index += consumed;
            continue;
        }
        if is_ident_start_char(ch) {
            let start = index;
            let mut end = index + ch.len_utf8();
            while end < line.len() {
                let next = line[end..].chars().next().unwrap();
                if is_ident_continue_char(next) {
                    end += next.len_utf8();
                } else {
                    break;
                }
            }
            let token = &line[start..end];
            let prev = previous_non_space_char(line, start);
            let next = next_non_space_char(line, end);
            if is_type_like_token(token, "json", prev, next) {
                output.push_str(CODE_COLOR_TYPE);
                output.push_str(token);
                output.push_str(CODE_COLOR_RESET);
            } else if token == "true" || token == "false" || token == "null" {
                output.push_str(CODE_COLOR_KEYWORD);
                output.push_str(token);
                output.push_str(CODE_COLOR_RESET);
            } else {
                output.push_str(token);
            }
            index = end;
            continue;
        }
        output.push(ch);
        index += ch.len_utf8();
    }
    output
}

fn code_highlight_generic(line: &str, language: &str, keywords: &[&str]) -> String {
    let mut output = String::with_capacity(line.len());
    let mut index = 0usize;
    while index < line.len() {
        let ch = line[index..].chars().next().unwrap();
        if ch == '"' || ch == '\'' {
            let (consumed, colored) = highlight_string(line, index, ch, CODE_COLOR_STRING);
            output.push_str(&colored);
            index += consumed;
            continue;
        }
        if ch == '#' && language == "python" {
            let leading = &line[..index];
            if leading.chars().all(|value| value.is_whitespace()) {
                output.push_str(CODE_COLOR_COMMENT);
                output.push_str(&line[index..]);
                output.push_str(CODE_COLOR_RESET);
                break;
            }
        }
        if ch == '/' && line[index..].starts_with("//") {
            output.push_str(CODE_COLOR_COMMENT);
            output.push_str(&line[index..]);
            output.push_str(CODE_COLOR_RESET);
            break;
        }
        if ch.is_ascii_digit() {
            let (consumed, colored) = highlight_number(line, index, CODE_COLOR_NUMBER);
            output.push_str(&colored);
            index += consumed;
            continue;
        }
        if is_ident_start_char(ch) {
            let start = index;
            let mut end = index + ch.len_utf8();
            while end < line.len() {
                let next = line[end..].chars().next().unwrap();
                if is_ident_continue_char(next) {
                    end += next.len_utf8();
                } else {
                    break;
                }
            }
            let token = &line[start..end];
            let prev = previous_non_space_char(line, start);
            let next = next_non_space_char(line, end);

            if keywords.iter().any(|keyword| keyword == &token) {
                output.push_str(CODE_COLOR_KEYWORD);
                output.push_str(token);
                output.push_str(CODE_COLOR_RESET);
            } else if is_type_like_token(token, language, prev, next) {
                output.push_str(CODE_COLOR_TYPE);
                output.push_str(token);
                output.push_str(CODE_COLOR_RESET);
            } else {
                output.push_str(token);
            }
            index = end;
            continue;
        }
        output.push(ch);
        index += ch.len_utf8();
    }
    output
}

fn is_type_like_token(token: &str, language: &str, prev: Option<char>, next: Option<char>) -> bool {
    if token.is_empty() {
        return false;
    }

    let first = token.chars().next().unwrap_or_default();
    if first.is_ascii_uppercase() {
        return true;
    }

    if matches!(prev, Some(':') | Some('|') | Some('>') | Some('<')) {
        return true;
    }

    if matches!(next, Some('(') | Some('<') | Some('[') | Some('{')) {
        return true;
    }

    match language {
        "rust" => RUST_TYPES.iter().any(|item| item == &token),
        "python" => PYTHON_TYPES.iter().any(|item| item == &token),
        "javascript" => JS_TYPES.iter().any(|item| item == &token),
        _ => false,
    }
}

fn previous_non_space_char(line: &str, start: usize) -> Option<char> {
    if start == 0 {
        return None;
    }
    let mut pos = start;
    while pos > 0 {
        let ch = line[..pos].chars().next_back();
        if let Some(ch) = ch {
            if ch.is_whitespace() {
                pos -= ch.len_utf8();
                continue;
            }
            return Some(ch);
        }
        return None;
    }
    None
}

fn next_non_space_char(line: &str, end: usize) -> Option<char> {
    if end >= line.len() {
        return None;
    }
    let mut pos = end;
    while pos < line.len() {
        let ch = line[pos..].chars().next();
        if let Some(ch) = ch {
            if ch.is_whitespace() {
                pos += ch.len_utf8();
                continue;
            }
            return Some(ch);
        }
        return None;
    }
    None
}

fn is_ident_start_char(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || ch == '$'
}

fn is_ident_continue_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

fn highlight_string(line: &str, start: usize, quote: char, color: &str) -> (usize, String) {
    let quote_len = quote.len_utf8();
    let mut end = start + 1;
    let mut escaped = false;
    while end < line.len() {
        let current = line[end..].chars().next().unwrap();
        if escaped {
            escaped = false;
            end += current.len_utf8();
            continue;
        }
        if current == '\\' {
            escaped = true;
            end += current.len_utf8();
            continue;
        }
        if current == quote as char {
            end += quote_len;
            break;
        }
        if current == '\n' {
            end += current.len_utf8();
            break;
        }
        end += current.len_utf8();
    }
    let token = &line[start..end.min(line.len())];
    (
        end.saturating_sub(start),
        format!("{color}{token}{CODE_COLOR_RESET}"),
    )
}

fn highlight_number(line: &str, start: usize, color: &str) -> (usize, String) {
    let mut end = start + 1;
    while end < line.len() {
        let ch = line[end..].chars().next().unwrap();
        if ch.is_ascii_digit() || matches!(ch, '.' | '_' | 'x' | 'X' | 'a'..='f' | 'A'..='F') {
            end += ch.len_utf8();
            continue;
        }
        break;
    }
    let token = &line[start..end.min(line.len())];
    (
        end.saturating_sub(start),
        format!("{color}{token}{CODE_COLOR_RESET}"),
    )
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "use", "where",
    "while",
];
const PYTHON_KEYWORDS: &[&str] = &[
    "def", "class", "if", "elif", "else", "for", "while", "return", "yield", "try", "except",
    "finally", "with", "as", "import", "from", "in", "is", "not", "and", "or", "or", "pass",
    "break", "continue", "raise", "True", "False", "None",
];
const JS_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "for",
    "function",
    "if",
    "let",
    "new",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "var",
    "while",
    "yield",
    "true",
    "null",
    "undefined",
];
const RUST_TYPES: &[&str] = &[
    "String",
    "Vec",
    "Option",
    "Result",
    "Box",
    "Arc",
    "Rc",
    "Path",
    "PathBuf",
    "HashMap",
    "HashSet",
    "BTreeMap",
    "BTreeSet",
    "Cow",
    "Duration",
    "Command",
    "Child",
    "ExitStatus",
];
const PYTHON_TYPES: &[&str] = &[
    "str",
    "int",
    "float",
    "bool",
    "list",
    "tuple",
    "dict",
    "set",
    "type",
    "object",
    "bytes",
    "bytearray",
    "None",
    "True",
    "False",
];
const JS_TYPES: &[&str] = &[
    "String", "Number", "Boolean", "Array", "Object", "Map", "Set", "Promise", "RegExp", "Date",
    "Function",
];

fn terminal_color_enabled() -> bool {
    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn clear_current_output_line() -> &'static str {
    "\r\x1b[2K"
}

fn display_width(text: &str) -> usize {
    crate::display_width(text)
}

fn strip_ansi_sequences(text: &str) -> String {
    crate::strip_ansi_sequences(text)
}

fn agent_terminal_columns() -> usize {
    if let Some(width) = DOCUMENT_WIDTH.with(|slot| slot.get()) { return width; }
    crossterm::terminal::size()
        .map(|(columns, _)| usize::from(columns))
        .unwrap_or(100)
}

fn agent_terminal_rows() -> usize {
    crossterm::terminal::size()
        .map(|(_, rows)| usize::from(rows))
        .unwrap_or(24)
}

fn truncate_display_value(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(text) <= max_width {
        return text.to_string();
    }
    let ellipsis = "…";
    let content_width = max_width.saturating_sub(display_width(ellipsis));
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width.saturating_add(ch_width) > content_width {
            break;
        }
        out.push(ch);
        width = width.saturating_add(ch_width);
    }
    out.push_str(ellipsis);
    out
}

fn keep_tail_start(text: &str, tail_chars: usize) -> usize {
    if tail_chars == 0 {
        return text.len();
    }
    text.char_indices()
        .rev()
        .nth(tail_chars.saturating_sub(1))
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn value_contains_sensitive_data(value: &Value) -> bool {
    fn key_is_sensitive(key: &str) -> bool {
        let key = key.to_ascii_lowercase();
        [
            "api_key",
            "apikey",
            "authorization",
            "cookie",
            "credential",
            "password",
            "private_key",
            "secret",
            "session",
            "token",
        ]
        .iter()
        .any(|marker| key.contains(marker))
    }

    match value {
        Value::String(text) => redact_sensitive_text(text) != *text,
        Value::Array(items) => items.iter().any(value_contains_sensitive_data),
        Value::Object(object) => object
            .iter()
            .any(|(key, child)| key_is_sensitive(key) || value_contains_sensitive_data(child)),
        _ => false,
    }
}

fn tool_result_event_payload(request: &ToolRequest, outcome: &ToolOutcome) -> Value {
    json!({
        "type": "tool_finished",
        "call_id": request.id,
        "tool_name": request.name,
        "detail": redact_sensitive_text(&outcome.text),
        "summary": redact_sensitive_text(&outcome.display),
        "error": outcome.error,
        "status": if outcome.error { "error" } else { "ok" },
    })
}

fn tool_debug_use_payload(
    request: &ToolRequest,
    repaired_from: Option<&str>,
    repaired_fields: Option<&[String]>,
) -> Value {
    // Diagnostic files are not conversation transcripts. Even an innocuous
    // argument key, tool name or repair label may contain user-provided text.
    // Keep this projection separate from the visible tool event contract.
    json!({
        "event": "tool_use",
        "argument_count": request.args.len(),
        "arguments_json_bytes": serde_json::to_vec(&request.args).map(|value| value.len()).unwrap_or(0),
        "repaired": repaired_from.is_some(),
        "repaired_field_count": repaired_fields.map_or(0, |fields| fields.len()),
    })
}

fn tool_debug_result_payload(outcome: &ToolOutcome) -> Value {
    json!({
        "event": "tool_result",
        "error": outcome.error,
        "status": if outcome.error { "error" } else { "ok" },
        "text_bytes": outcome.text.len(),
        "display_bytes": outcome.display.len(),
        "artifact_count": outcome.artifacts.len(),
        "has_full_output": outcome.full_output_ref.is_some(),
        "has_data": outcome.data.is_some(),
        "has_execution_receipt": outcome.execution.is_some(),
    })
}

fn human_status_line(kind: &str, title: &str, detail: Option<&str>) -> String {
    // Review state qualifies the adjacent answer's authority. Never abbreviate
    // it into an ambiguous tool-status label (e.g. "Draft — p…").
    if kind == "review" {
        return format!("  [{}]", title.trim());
    }
    let label = if title.trim().is_empty() {
        kind.trim()
    } else {
        title.trim()
    };
    let label = truncate_display_value(label, HUMAN_STATUS_LABEL_WIDTH);
    let detail = detail
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_display_value(value, HUMAN_STATUS_DETAIL_MAX_WIDTH));
    match (terminal_color_enabled(), detail) {
        (true, Some(detail)) => format!(
            "  \x1b[38;5;75m{label:<width$}{CODE_COLOR_RESET}  {CODE_COLOR_COMMENT}{detail}{CODE_COLOR_RESET}",
            width = HUMAN_STATUS_LABEL_WIDTH
        ),
        (true, None) => format!(
            "  \x1b[38;5;75m{label}{CODE_COLOR_RESET}"
        ),
        (false, Some(detail)) => {
            format!("  {label:<width$}  {detail}", width = HUMAN_STATUS_LABEL_WIDTH)
        }
        (false, None) => format!("  {label}"),
    }
}

fn core_citation_status_detail(citation: &nola_core::agent::parts::CitationPart) -> String {
    let uri = citation.uri.trim();
    let resource = if let Some(path) = uri.strip_prefix("file://") {
        path.to_string()
    } else if let Some((_, rest)) = uri.split_once("://") {
        rest.split('/').next().unwrap_or(uri).to_string()
    } else {
        uri.to_string()
    };
    let mut location = Vec::new();
    if let Some(value) = citation.location.as_ref() {
        if let Some(page) = value.page {
            location.push(format!("{} {page}", ui_text(UiText::Page)));
        }
        match (value.line_start, value.line_end) {
            (Some(start), Some(end)) if end != start => {
                location.push(format!("{} {start}-{end}", ui_text(UiText::Lines)));
            }
            (Some(line), _) => location.push(format!("{} {line}", ui_text(UiText::Lines))),
            _ => {}
        }
        if let Some(section) = value
            .section
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            location.push(section.to_string());
        }
    }
    let location = location.join(" · ");
    [citation.title.trim(), resource.trim(), location.as_str()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

pub(crate) fn nola_host_system_prompt() -> &'static str {
    r#"The active user-facing surface is an authenticated Nola product client. Present one coherent assistant experience; terminal, subprocess, transport, and Host details are runtime facts rather than product identity."#
}

fn restore_agent_plan_state(
    state: &crate::tools::AgentPlanState,
    messages: &[Value],
) -> Option<String> {
    for message in messages.iter().rev() {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
            continue;
        };
        for call in calls.iter().rev() {
            let Some(function) = call.get("function") else {
                continue;
            };
            if function.get("name").and_then(Value::as_str) != Some("update_agent_todos") {
                continue;
            }
            let Some(arguments) = function.get("arguments") else {
                continue;
            };
            let args = if let Some(object) = arguments.as_object() {
                object.clone()
            } else if let Some(raw) = arguments.as_str() {
                let Some(parsed) = serde_json::from_str::<Value>(raw).ok() else {
                    continue;
                };
                let Some(object) = parsed.as_object().cloned() else {
                    continue;
                };
                object
            } else {
                continue;
            };
            if let Ok(restored) = state.restore_from_args(&args) {
                return Some(restored);
            }
        }
    }
    None
}

#[cfg(test)]
pub fn session_plan_summary(messages: &[Value]) -> Option<SessionPlanSummary> {
    let state = crate::tools::AgentPlanState::default();
    restore_agent_plan_state(&state, messages)?;
    session_plan_summary_from_state(&state)
}

pub fn session_plan_summary_for_session(
    session_id: &str,
    messages: &[Value],
) -> Option<SessionPlanSummary> {
    let state = crate::tools::AgentPlanState::default();
    if !matches!(state.bind_session(session_id), Ok(true)) {
        restore_agent_plan_state(&state, messages)?;
    }
    session_plan_summary_from_state(&state)
}

fn session_plan_summary_from_state(
    state: &crate::tools::AgentPlanState,
) -> Option<SessionPlanSummary> {
    let snapshot = state.snapshot()?;
    Some(SessionPlanSummary {
        completed: snapshot.completed,
        total: snapshot.total,
        current: snapshot.current,
        steps: snapshot
            .steps
            .into_iter()
            .map(|step| SessionPlanStep {
                id: step.id,
                content: step.content,
                active_form: step.active_form,
                status: step.status,
                priority: step.priority,
                dependency_ids: step.dependency_ids,
                approval_status: step.approval_status,
            })
            .collect(),
    })
}

#[cfg(test)]
mod emitter_output_tests {
    use super::{
        is_internal_control_tool, markdown_stream_flush_len, markdown_stream_tail_has_open_table,
        merge_footer_context_usage, nola_host_system_prompt, render_markdown_text,
        select_sticky_footer_lines, stable_markdown_stream_flush_len, Emitter,
    };
    use crate::i18n::{tr, Message};
    use serde_json::{Map, Value};

    #[test]
    fn tool_diagnostics_exclude_content_without_changing_visible_results() {
        let request = crate::tools::ToolRequest {
            id: "call-secret".into(),
            name: "tool-secret".into(),
            args: serde_json::from_value(serde_json::json!({
                "private-key-name": "私密文件 /private/user/document.txt",
                "nested": {"text": "Bearer credential-secret", "uri": "https://example.invalid/private?key=query-secret"}
            })).unwrap(),
        };
        let fields = vec!["repair-field-secret".to_owned()];
        let started = super::tool_debug_use_payload(&request, Some("repair-source-secret"), Some(&fields));
        assert_eq!(started["event"], "tool_use");
        assert_eq!(started["argument_count"], 2);
        assert_eq!(started["arguments_json_bytes"], serde_json::to_vec(&request.args).unwrap().len());
        assert_eq!(started["repaired"], true);
        assert_eq!(started["repaired_field_count"], 1);
        let unrepaired = super::tool_debug_use_payload(&request, None, None);
        assert_eq!(unrepaired["repaired"], false);
        assert_eq!(unrepaired["repaired_field_count"], 0);

        for error in [false, true] {
            let outcome = crate::tools::ToolOutcome {
                full_output_ref: Some("/private/output-secret".into()),
                data: Some(serde_json::json!({"body": "data-secret"})),
                execution: None,
                text: "私密正文 /private/user/document.txt".into(),
                display: "个人摘要".into(),
                error,
                artifacts: vec![],
                activation: None,
            };
            let finished = super::tool_debug_result_payload(&outcome);
            assert_eq!(finished["error"], error);
            assert_eq!(finished["status"], if error { "error" } else { "ok" });
            assert_eq!(finished["text_bytes"], outcome.text.len());
            assert_eq!(finished["display_bytes"], outcome.display.len());
            assert_eq!(finished["has_full_output"], true);
            assert_eq!(finished["has_data"], true);
            let serialized = format!("{started}{finished}");
            for private in ["secret", "private", "私密", "个人", "Bearer", "https://"] {
                assert!(!serialized.contains(private), "diagnostic copied {private}");
            }
            for field in ["arguments", "detail", "summary", "call_id", "tool_name", "repaired_from", "repaired_fields"] {
                assert!(started.get(field).is_none());
                assert!(finished.get(field).is_none());
            }
            // The user-facing event is a distinct contract, not this projection.
            let visible = super::tool_result_event_payload(&request, &outcome);
            assert_eq!(visible["detail"], outcome.text);
            assert_eq!(visible["summary"], outcome.display);
            assert_eq!(visible["call_id"], request.id);
            assert_eq!(visible["tool_name"], request.name);
        }
    }

    #[test]
    fn a_short_plain_stream_tail_is_visible_without_another_provider_callback() {
        for text in ["A short unfinished phrase", "你好，当前"] {
            assert!(text.chars().count() < 96);
            let stable = stable_markdown_stream_flush_len(text);
            assert_eq!(stable, text.len());
            assert_eq!(markdown_stream_flush_len(text, stable), text.len());
            let mut emitter = Emitter::new(false);
            emitter.preview_text(text);
            assert!(emitter.stream_text_buffer.is_empty());
            assert!(emitter.stream_text_is_preview);
        }
    }

    #[test]
    fn short_incomplete_markdown_tail_does_not_become_plain_history() {
        for text in [
            "`unfinished",
            "**unfinished",
            "[label](",
            "| A | B |",
            "```rust",
        ] {
            assert_eq!(stable_markdown_stream_flush_len(text), 0, "{text:?}");
            let mut emitter = Emitter::new(false);
            emitter.preview_text(text);
            assert_eq!(emitter.stream_text_buffer, text);
        }
    }

    #[test]
    fn split_strikethrough_keeps_one_markdown_span_until_the_closer_arrives() {
        for body in ["unfinished".to_string(), "long unfinished text ".repeat(12)] {
            let first = format!("~~{body}");
            assert!(!super::markdown_inline_boundary_is_stable(&first));
            let mut emitter = Emitter::new(false);
            emitter.preview_text(&first);
            assert_eq!(emitter.stream_text_buffer, first);
            emitter.preview_text("~~.");
            assert!(emitter.stream_text_buffer.is_empty());
            let rendered = render_markdown_text(&format!("{first}~~."));
            assert!(!rendered.contains("~~"), "{rendered}");
            assert_eq!(rendered.matches(body.as_str()).count(), 1);
        }
    }

    #[test]
    fn refreshed_context_usage_replaces_the_stale_footer_segment() {
        let label = tr(Message::InteractiveContextLabel);
        let mut lines = vec![format!("{label} 1% · 512/64k · qwen · [safe]")];

        merge_footer_context_usage(&mut lines, format!("{label} 8% · 5.1k/64k"));

        assert_eq!(
            lines,
            vec![format!("{label} 8% · 5.1k/64k · qwen · [safe]")]
        );
    }

    #[test]
    fn durable_stream_output_restores_sticky_footer_during_the_turn() {
        let mut emitter = Emitter::new(false);
        emitter.sticky_footer_base = Some("Context 12%".to_string());

        emitter.text("first line\nsecond line\n");

        assert!(!emitter.sticky_footer_suspended);
        assert_eq!(emitter.sticky_footer_base.as_deref(), Some("Context 12%"));
        emitter.done("completed");
        assert!(!emitter.sticky_footer_suspended);
    }

    #[test]
    fn sticky_footer_compaction_preserves_context_and_active_tail() {
        let text = "Context 18% · model\nWorkspace\nSafety mode\nHost\nPlan 2/4\nEsc to stop\nQueued input";

        let lines = select_sticky_footer_lines(text, 80, 5);

        assert_eq!(
            lines,
            vec![
                "Context 18% · model",
                "Workspace",
                "Safety mode",
                "Esc to stop",
                "Queued input",
            ]
        );
    }

    #[test]
    fn product_host_prompt_uses_memory_tools_without_cli_identity() {
        let prompt = nola_host_system_prompt();

        assert!(prompt.contains("authenticated Nola product client"));
        assert!(!prompt.contains("active structured capability manifest is the sole source"));
        assert!(!prompt.contains("memory_save"));
        assert!(!prompt.contains("memory_search"));
        assert!(!prompt.contains("A one-time grant applies only"));
        assert!(!prompt.contains("Image understanding"));
        assert!(!prompt.contains("You are Nola CLI"));
        assert!(!prompt.contains("native Nola for Mac"));
    }

    #[test]
    fn internal_tool_output_navigation_is_not_rendered_as_user_activity() {
        for name in [
            "tool_output_list",
            "tool_output_read",
            "tool_output_search",
            "tool_output_find",
        ] {
            assert!(is_internal_control_tool(name), "{name}");
        }
        assert!(!is_internal_control_tool("read_file"));
    }

    #[test]
    fn large_streaming_markdown_table_is_not_split_into_raw_pipe_rows() {
        let mut table = String::from("| # | 行号 | 测试名 |\n|---|---|---|\n");
        for index in 1..=24 {
            table.push_str(&format!(
                "| {index} | {} | very_long_descriptive_test_name_{index}_that_must_stay_in_the_same_markdown_table |\n",
                1600 + index
            ));
        }
        assert!(table.len() > 2_048);
        assert!(markdown_stream_tail_has_open_table(&table));
        assert_eq!(stable_markdown_stream_flush_len(&table), 0);
        assert_eq!(markdown_stream_flush_len(&table, table.len()), table.len());

        let rendered = render_markdown_text(&table);
        assert!(!rendered.contains("| 2 |"));
        assert_eq!(rendered.matches("very_").count(), 24);
    }

    #[test]
    fn streamed_ragged_table_waits_for_rows_and_renders_as_one_table() {
        let prefix = "LLM layer (LLM/) — 17 files\n";
        let header = "| File | Size | Description |\n|---|---|---|\n";
        let ragged_row = "| LLMClient.swift | Protocol and streaming API |\n";
        let complete_row = "| OpenAIProvider.swift | ~1K lines | SSE provider |\n";

        let header_chunk = format!("{prefix}{header}");
        assert_eq!(
            stable_markdown_stream_flush_len(&header_chunk),
            prefix.len()
        );
        let body_chunk = format!("{header_chunk}{ragged_row}");
        assert_eq!(stable_markdown_stream_flush_len(&body_chunk), prefix.len());

        let rendered =
            render_markdown_text(&format!("{prefix}{header}{ragged_row}{complete_row}\n"));
        assert_eq!(rendered.matches('╭').count(), 1, "{rendered}");
        assert!(rendered.contains("LLMClient.swift"), "{rendered}");
        assert!(
            rendered.contains("Protocol and streaming API"),
            "{rendered}"
        );
        assert!(rendered.contains("OpenAIProvider.swift"), "{rendered}");
        assert!(!rendered.contains("| LLMClient.swift |"), "{rendered}");
    }

    #[test]
    fn inclusive_end_line_is_visible_as_a_range_in_tool_activity() {
        let args = Map::from_iter([
            ("start_line".to_string(), Value::from(1)),
            ("end_line".to_string(), Value::from(3)),
        ]);
        assert_eq!(super::line_range_arg_summary(&args).as_deref(), Some("1-3"));
    }

    #[test]
    fn citation_status_uses_structured_uri_and_location() {
        let citation = nola_core::agent::parts::CitationPart {
            id: "citation-1".to_string(),
            ownership: nola_core::agent::parts::PartOwnership::for_turn("turn-1"),
            citation_id: "citation-1".to_string(),
            source_id: "source-1".to_string(),
            title: "Runtime source".to_string(),
            uri: "file:///tmp/runtime.rs".to_string(),
            mime_type: Some("text/x-rust".to_string()),
            location: Some(nola_core::agent::parts::SourceLocation {
                page: None,
                line_start: Some(12),
                line_end: Some(18),
                section: None,
            }),
            range: None,
            origin_tool_call_id: "read-1".to_string(),
        };

        let detail = super::core_citation_status_detail(&citation);
        assert!(detail.contains("Runtime source"));
        assert!(detail.contains("/tmp/runtime.rs"));
        assert!(detail.contains("12-18"));
        assert!(!detail.contains("tool output"));
    }
}
