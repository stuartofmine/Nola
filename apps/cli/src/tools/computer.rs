use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use super::{error, ToolContext, ToolOutcome};

pub(super) fn run_native_computer_tool(
    action: &str,
    args: &Map<String, Value>,
    ctx: &ToolContext,
) -> Option<ToolOutcome> {
    match action {
        "computer_wait" => Some(wait(args, ctx)),
        "computer_screenshot" => screenshot(args, ctx),
        "computer_get_clipboard" => Some(get_clipboard()),
        "computer_set_clipboard" => Some(set_clipboard(args)),
        "computer_type_text" => Some(type_text(args)),
        "computer_press_key" => Some(press_key(args)),
        "computer_hotkey" => Some(hotkey(args)),
        "computer_copy_selection" => Some(copy_selection(args)),
        "computer_assert_clipboard" => Some(assert_clipboard(args)),
        "computer_wait_for_clipboard" => Some(wait_for_clipboard(args, ctx)),
        "computer_open_item" => Some(open_item(args)),
        "computer_activate_app" => Some(app_action(args, "activate")),
        "computer_quit_app" => Some(app_action(args, "quit")),
        "computer_list_apps" => Some(list_apps(args)),
        "computer_assert_app" => Some(assert_app(args)),
        "computer_wait_for_app" => Some(wait_for_app(args, ctx)),
        "computer_list_windows" => Some(list_windows(args)),
        "computer_assert_window" => Some(assert_window(args)),
        "computer_wait_for_window" => Some(wait_for_window(args, ctx)),
        "computer_focus_window" => Some(focus_window(args)),
        "computer_close_window" => Some(close_window(args)),
        "computer_set_window_bounds" => Some(set_window_bounds(args)),
        "computer_perform_window_action" => Some(perform_window_action(args)),
        _ => None,
    }
}

fn wait(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let seconds = number_arg(args, "seconds")
        .or_else(|| number_arg(args, "timeout_sec"))
        .or_else(|| number_arg(args, "timeout"))
        .unwrap_or(1.0)
        .clamp(0.0, 300.0);
    if let Err(outcome) = cancellable_sleep(Duration::from_millis((seconds * 1000.0) as u64), ctx) {
        return outcome;
    }
    native_ok(
        format!("waited {:.2}s", seconds),
        format!("waited {:.2}s", seconds),
    )
}

fn screenshot(args: &Map<String, Value>, ctx: &ToolContext) -> Option<ToolOutcome> {
    if has_any(
        args,
        &["x", "y", "width", "height", "display_index", "display_id"],
    ) {
        return None;
    }
    let path = output_path(args).unwrap_or_else(|| default_screenshot_path(ctx));
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return Some(error(format!(
                "failed to create screenshot directory {}: {err}",
                parent.display()
            )));
        }
    }
    match Command::new("/usr/sbin/screencapture")
        .arg("-x")
        .arg(&path)
        .output()
    {
        Ok(output) if output.status.success() => match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() && metadata.len() > 0 => Some(ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                text: format!("screenshot saved: {}", path.display()),
                display: format!("screenshot {}", path.display()),
                error: false,
                artifacts: Vec::new(),
                activation: None,
            }),
            Ok(_) => Some(error(format!(
                "screencapture completed without creating a non-empty image at {}",
                path.display()
            ))),
            Err(err) => Some(error(format!(
                "screencapture completed without creating {}: {err}",
                path.display()
            ))),
        },
        Ok(output) => Some(error(format!(
            "screencapture failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
        Err(err) => Some(error(format!("failed to run screencapture: {err}"))),
    }
}

fn get_clipboard() -> ToolOutcome {
    match Command::new("pbpaste").output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            native_ok(text, "clipboard read")
        }
        Ok(output) => error(format!(
            "pbpaste failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(err) => error(format!("failed to run pbpaste: {err}")),
    }
}

fn copy_selection(args: &Map<String, Value>) -> ToolOutcome {
    let max_chars = int_arg(args, "max_chars").unwrap_or(4000).clamp(1, 50000) as usize;
    let wait_after_ms = int_arg(args, "wait_after_ms").unwrap_or(250).clamp(0, 5000);
    let restore_clipboard = bool_arg(args, "restore_clipboard").unwrap_or(true);
    let previous = Command::new("pbpaste")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout);
    let pressed = hotkey_with_keys(&["command", "c"]);
    if pressed.error {
        return pressed;
    }
    if wait_after_ms > 0 {
        thread::sleep(Duration::from_millis(wait_after_ms as u64));
    }
    let copied = match read_clipboard_text() {
        Ok(value) => value,
        Err(err) => return error(err),
    };
    let mut restore_error = String::new();
    if restore_clipboard {
        if let Some(previous) = previous {
            if let Err(err) = set_clipboard_bytes(&previous) {
                restore_error = err;
            }
        }
    }
    let (output, truncated) = clip_text(&copied, max_chars);
    let mut text = format!(
        "Computer action: copy_selection\nCopied characters: {}\nRestore clipboard: {}\nTruncated: {}\nText:\n{}",
        copied.chars().count(),
        restore_clipboard,
        truncated,
        output
    );
    if !restore_error.is_empty() {
        text.push_str(&format!("\nRestore warning: {restore_error}"));
    }
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text,
        display: "copied selected text".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn assert_clipboard(args: &Map<String, Value>) -> ToolOutcome {
    let condition = match clipboard_condition(args) {
        Ok(value) => value,
        Err(err) => return error(err),
    };
    let value = match read_clipboard_text() {
        Ok(value) => value,
        Err(err) => return error(err),
    };
    let found = clipboard_matches(&value, &condition.expected, &condition.match_mode);
    let (output, truncated) = clip_text(&value, condition.max_chars);
    let text = clipboard_condition_text(
        "assert_clipboard",
        &value,
        &output,
        truncated,
        found,
        &condition,
        &[],
    );
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text,
        display: if found == condition.expected_exists {
            "asserted host clipboard".to_string()
        } else {
            "clipboard assertion failed".to_string()
        },
        error: found != condition.expected_exists,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn wait_for_clipboard(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let condition = match clipboard_condition(args) {
        Ok(value) => value,
        Err(err) => return error(err),
    };
    let timeout_ms = int_arg(args, "timeout_ms").unwrap_or(5000).clamp(0, 60000);
    let interval_ms = int_arg(args, "interval_ms").unwrap_or(250).clamp(100, 5000);
    let started = std::time::Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms as u64);
    let mut attempts = 0i64;
    let (value, found) = loop {
        if ctx.cancellation.is_requested() {
            return error("clipboard wait cancelled by user".to_string());
        }
        attempts += 1;
        let value = match read_clipboard_text() {
            Ok(value) => value,
            Err(err) => return error(err),
        };
        let found = clipboard_matches(&value, &condition.expected, &condition.match_mode);
        if found == condition.expected_exists {
            let (output, truncated) = clip_text(&value, condition.max_chars);
            let text = clipboard_condition_text(
                "wait_for_clipboard",
                &value,
                &output,
                truncated,
                found,
                &condition,
                &[
                    ("Attempts", attempts.to_string()),
                    ("Elapsed ms", started.elapsed().as_millis().to_string()),
                ],
            );
            return ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                text,
                display: "waited for host clipboard".to_string(),
                error: false,
                artifacts: Vec::new(),
                activation: None,
            };
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            break (value, found);
        }
        if let Err(outcome) = cancellable_sleep(
            Duration::from_millis(
                (interval_ms as u64).min(deadline.duration_since(now).as_millis() as u64),
            ),
            ctx,
        ) {
            return outcome;
        }
    };
    let (output, truncated) = clip_text(&value, condition.max_chars);
    let text = clipboard_condition_text(
        "wait_for_clipboard",
        &value,
        &output,
        truncated,
        found,
        &condition,
        &[
            ("Attempts", attempts.to_string()),
            ("Timeout ms", timeout_ms.to_string()),
            ("Interval ms", interval_ms.to_string()),
        ],
    );
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text,
        display: "clipboard wait timed out".to_string(),
        error: true,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn set_clipboard(args: &Map<String, Value>) -> ToolOutcome {
    let text = string_arg(args, "text").unwrap_or_default();
    match set_clipboard_bytes(text.as_bytes()) {
        Ok(()) => native_ok("clipboard updated", "clipboard updated"),
        Err(err) => error(err),
    }
}

fn type_text(args: &Map<String, Value>) -> ToolOutcome {
    let text = string_arg(args, "text").unwrap_or_default();
    if bool_arg(args, "clear").unwrap_or(false) {
        let selected = hotkey_with_keys(&["command", "a"]);
        if selected.error {
            return selected;
        }
    }
    if text.is_empty() {
        return native_ok("typed empty text", "typed empty text");
    }
    let previous = Command::new("pbpaste")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout);
    if let Err(err) = set_clipboard_bytes(text.as_bytes()) {
        return error(err);
    }
    let paste_settle_ms = int_arg(args, "paste_settle_ms")
        .unwrap_or(120)
        .clamp(0, 5000);
    if paste_settle_ms > 0 {
        thread::sleep(Duration::from_millis(paste_settle_ms as u64));
    }
    let outcome = hotkey_with_keys(&["command", "v"]);
    if let Some(previous) = previous {
        let _ = set_clipboard_bytes(&previous);
    }
    if outcome.error {
        return outcome;
    }
    if bool_arg(args, "press_enter").unwrap_or(false) {
        let entered = press_key_value("enter");
        if entered.error {
            return entered;
        }
    }
    let wait_after_ms = int_arg(args, "wait_after_ms").unwrap_or(250).clamp(0, 5000);
    if wait_after_ms > 0 {
        thread::sleep(Duration::from_millis(wait_after_ms as u64));
    }
    native_ok(
        format!("typed {} character(s)", text.chars().count()),
        format!("typed {} character(s)", text.chars().count()),
    )
}

fn press_key(args: &Map<String, Value>) -> ToolOutcome {
    let key = string_arg(args, "key")
        .or_else(|| string_arg(args, "name"))
        .unwrap_or_default();
    if key.is_empty() {
        return error("missing key".to_string());
    }
    press_key_value(&key)
}

fn press_key_value(key: &str) -> ToolOutcome {
    run_key_script(key, &[])
}

fn hotkey(args: &Map<String, Value>) -> ToolOutcome {
    let keys = string_array_arg(args, "keys");
    if keys.is_empty() {
        return error("missing keys".to_string());
    }
    let borrowed = keys.iter().map(String::as_str).collect::<Vec<_>>();
    hotkey_with_keys(&borrowed)
}

fn hotkey_with_keys(keys: &[&str]) -> ToolOutcome {
    let Some((key, modifiers)) = keys.split_last() else {
        return error("missing keys".to_string());
    };
    run_key_script(key, modifiers)
}

fn open_item(args: &Map<String, Value>) -> ToolOutcome {
    let target = string_arg(args, "target")
        .or_else(|| string_arg(args, "path"))
        .or_else(|| string_arg(args, "url"))
        .unwrap_or_default();
    if target.is_empty() {
        return error("missing target".to_string());
    }
    match Command::new("open").arg(&target).output() {
        Ok(output) if output.status.success() => {
            native_ok(format!("opened {target}"), format!("opened {target}"))
        }
        Ok(output) => error(format!(
            "open failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(err) => error(format!("failed to run open: {err}")),
    }
}

fn app_action(args: &Map<String, Value>, action: &str) -> ToolOutcome {
    let app = string_arg(args, "app")
        .or_else(|| string_arg(args, "name"))
        .unwrap_or_default();
    if app.is_empty() {
        return error("missing app".to_string());
    }
    let script = format!("tell application {} to {action}", applescript_string(&app));
    run_osascript(&script)
        .map(|_| native_ok(format!("{action} {app}"), format!("{action} {app}")))
        .unwrap_or_else(error)
}

fn list_apps(args: &Map<String, Value>) -> ToolOutcome {
    let include_background = bool_arg(args, "include_background").unwrap_or(false);
    let limit = int_arg(args, "max_apps").unwrap_or(100).clamp(1, 500);
    match collect_apps(include_background, limit) {
        Ok((frontmost, apps)) => {
            let count = apps.len();
            let mut text = "Computer action: list_apps".to_string();
            if !frontmost.is_empty() {
                text.push_str(&format!("\nFrontmost app: {frontmost}"));
            }
            text.push_str(&format!("\nApps: {}", json!(&apps)));
            ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                text,
                display: format!("listed {count} apps"),
                error: false,
                artifacts: Vec::new(),
                activation: None,
            }
        }
        Err(err) => error(err),
    }
}

fn assert_app(args: &Map<String, Value>) -> ToolOutcome {
    match app_condition_state(args) {
        Ok(state) => {
            let text = format!("Computer action: assert_app\n{}", pretty_json(&state));
            if state
                .get("condition_met")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                    text,
                    display: "app condition matched".to_string(),
                    error: false,
                    artifacts: Vec::new(),
                    activation: None,
                }
            } else {
                error(format!("expected app condition was not met\n{text}"))
            }
        }
        Err(err) => error(err),
    }
}

fn wait_for_app(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let timeout_ms = int_arg(args, "timeout_ms").unwrap_or(5000).clamp(0, 60000);
    let interval_ms = int_arg(args, "interval_ms").unwrap_or(500).clamp(100, 5000);
    let started = std::time::Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms as u64);
    let mut attempts = 0i64;
    let mut last_state: Value;
    loop {
        if ctx.cancellation.is_requested() {
            return error("app wait cancelled by user".to_string());
        }
        attempts += 1;
        match app_condition_state(args) {
            Ok(mut state) => {
                if let Some(object) = state.as_object_mut() {
                    object.insert("attempts".to_string(), json!(attempts));
                    object.insert(
                        "elapsed_ms".to_string(),
                        json!(started.elapsed().as_millis() as i64),
                    );
                }
                if state
                    .get("condition_met")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                        text: format!("Computer action: wait_for_app\n{}", pretty_json(&state)),
                        display: "app condition matched".to_string(),
                        error: false,
                        artifacts: Vec::new(),
                        activation: None,
                    };
                }
                last_state = state;
            }
            Err(err) => return error(err),
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        if let Err(outcome) = cancellable_sleep(
            Duration::from_millis(
                (interval_ms as u64).min(deadline.duration_since(now).as_millis() as u64),
            ),
            ctx,
        ) {
            return outcome;
        }
    }
    error(format!(
        "app condition timed out after {timeout_ms}ms\n{}",
        compact_json(&last_state)
    ))
}

fn list_windows(args: &Map<String, Value>) -> ToolOutcome {
    let limit = int_arg(args, "max_windows").unwrap_or(30).clamp(1, 200);
    match collect_windows(limit) {
        Ok((frontmost, windows)) => {
            let count = windows.len();
            let mut text = "Computer action: list_windows".to_string();
            if !frontmost.is_empty() {
                text.push_str(&format!("\nFrontmost app: {frontmost}"));
            }
            text.push_str(&format!("\nWindows: {}", json!(&windows)));
            ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                text,
                display: format!("listed {count} windows"),
                error: false,
                artifacts: Vec::new(),
                activation: None,
            }
        }
        Err(err) => error(err),
    }
}

fn assert_window(args: &Map<String, Value>) -> ToolOutcome {
    match window_condition_state(args) {
        Ok(state) => {
            let text = format!(
                "Computer action: assert_window\nWindow condition: {}",
                compact_json(&state)
            );
            if state
                .get("condition_met")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                    text,
                    display: "window condition matched".to_string(),
                    error: false,
                    artifacts: Vec::new(),
                    activation: None,
                }
            } else {
                error(format!("expected window condition was not met\n{text}"))
            }
        }
        Err(err) => error(err),
    }
}

fn wait_for_window(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let timeout_ms = int_arg(args, "timeout_ms").unwrap_or(5000).clamp(0, 60000);
    let interval_ms = int_arg(args, "interval_ms").unwrap_or(500).clamp(100, 5000);
    let started = std::time::Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms as u64);
    let mut attempts = 0i64;
    let mut last_state: Value;
    loop {
        if ctx.cancellation.is_requested() {
            return error("window wait cancelled by user".to_string());
        }
        attempts += 1;
        match window_condition_state(args) {
            Ok(mut state) => {
                if let Some(object) = state.as_object_mut() {
                    object.insert("attempts".to_string(), json!(attempts));
                    object.insert(
                        "elapsed_ms".to_string(),
                        json!(started.elapsed().as_millis() as i64),
                    );
                }
                if state
                    .get("condition_met")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                        text: format!(
                            "Computer action: wait_for_window\nWindow condition: {}",
                            compact_json(&state)
                        ),
                        display: "window condition matched".to_string(),
                        error: false,
                        artifacts: Vec::new(),
                        activation: None,
                    };
                }
                last_state = state;
            }
            Err(err) => return error(err),
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        if let Err(outcome) = cancellable_sleep(
            Duration::from_millis(
                (interval_ms as u64).min(deadline.duration_since(now).as_millis() as u64),
            ),
            ctx,
        ) {
            return outcome;
        }
    }
    error(format!(
        "window condition timed out after {timeout_ms}ms\n{}",
        compact_json(&last_state)
    ))
}

fn focus_window(args: &Map<String, Value>) -> ToolOutcome {
    let (matches, matched) = match find_window_match(args) {
        Ok(value) => value,
        Err(err) => return error(err),
    };
    let app = matched
        .get("app")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let app_window_index = matched
        .get("app_window_index")
        .and_then(Value::as_i64)
        .unwrap_or(1);
    if app.is_empty() {
        return error("matched window did not include an app name".to_string());
    }
    let script = format!(
        "tell application \"System Events\"\n  tell application process {}\n    set frontmost to true\n    try\n      perform action \"AXRaise\" of window {}\n    end try\n  end tell\nend tell",
        applescript_string(&app),
        app_window_index
    );
    match run_osascript(&script) {
        Ok(output) => {
            let mut text = format!(
                "Computer action: focus_window\nFocused window: {}\nMatch count: {}",
                compact_json(&matched),
                matches.len()
            );
            if !output.is_empty() {
                text.push_str(&format!("\nOutput: {output}"));
            }
            ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                text,
                display: format!("focused {app}"),
                error: false,
                artifacts: Vec::new(),
                activation: None,
            }
        }
        Err(err) => error(err),
    }
}

fn close_window(args: &Map<String, Value>) -> ToolOutcome {
    let (matches, matched) = match find_window_match(args) {
        Ok(value) => value,
        Err(err) => return error(err),
    };
    let app = matched
        .get("app")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let app_window_index = matched
        .get("app_window_index")
        .and_then(Value::as_i64)
        .unwrap_or(1);
    if app.is_empty() {
        return error("matched window did not include an app name".to_string());
    }
    let script = format!(
        "tell application \"System Events\"\n  tell application process {}\n    set frontmost to true\n    set targetWindow to window {}\n    try\n      perform action \"AXRaise\" of targetWindow\n    end try\n    try\n      perform action \"AXClose\" of targetWindow\n    on error\n      perform action \"AXPress\" of button 1 of targetWindow\n    end try\n  end tell\nend tell",
        applescript_string(&app),
        app_window_index
    );
    match run_osascript(&script) {
        Ok(output) => {
            let wait_after_ms = int_arg(args, "wait_after_ms").unwrap_or(500).clamp(0, 5000);
            if wait_after_ms > 0 {
                thread::sleep(Duration::from_millis(wait_after_ms as u64));
            }
            let mut text = format!(
                "Computer action: close_window\nClosed window: {}\nMatch count: {}\nWait after ms: {}",
                compact_json(&matched),
                matches.len(),
                wait_after_ms
            );
            if !output.is_empty() {
                text.push_str(&format!("\nOutput: {output}"));
            }
            ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                text,
                display: format!("closed {app} window"),
                error: false,
                artifacts: Vec::new(),
                activation: None,
            }
        }
        Err(err) => error(err),
    }
}

fn set_window_bounds(args: &Map<String, Value>) -> ToolOutcome {
    let (matches, matched) = match find_window_match(args) {
        Ok(value) => value,
        Err(err) => return error(err),
    };
    let app = matched
        .get("app")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let app_window_index = matched
        .get("app_window_index")
        .and_then(Value::as_i64)
        .unwrap_or(1);
    let x = number_arg(args, "x").unwrap_or(0.0).round() as i64;
    let y = number_arg(args, "y").unwrap_or(0.0).round() as i64;
    let width = int_arg(args, "width").unwrap_or(1).max(1);
    let height = int_arg(args, "height").unwrap_or(1).max(1);
    if app.is_empty() {
        return error("matched window did not include an app name".to_string());
    }
    let script = format!(
        "tell application \"System Events\"\n  tell application process {}\n    set frontmost to true\n    set targetWindow to window {}\n    try\n      perform action \"AXRaise\" of targetWindow\n    end try\n    set position of targetWindow to {{{}, {}}}\n    set size of targetWindow to {{{}, {}}}\n  end tell\nend tell",
        applescript_string(&app),
        app_window_index,
        x,
        y,
        width,
        height
    );
    match run_osascript(&script) {
        Ok(output) => {
            let mut text = format!(
                "Computer action: set_window_bounds\nWindow: {}\nRequested bounds: {x},{y},{width},{height}\nMatch count: {}",
                compact_json(&matched),
                matches.len()
            );
            if !output.is_empty() {
                text.push_str(&format!("\nOutput: {output}"));
            }
            ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                text,
                display: format!("set {app} window bounds"),
                error: false,
                artifacts: Vec::new(),
                activation: None,
            }
        }
        Err(err) => error(err),
    }
}

fn perform_window_action(args: &Map<String, Value>) -> ToolOutcome {
    let action = string_arg(args, "action").unwrap_or_default();
    let (matches, matched) = match find_window_match(args) {
        Ok(value) => value,
        Err(err) => return error(err),
    };
    let app = matched
        .get("app")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let app_window_index = matched
        .get("app_window_index")
        .and_then(Value::as_i64)
        .unwrap_or(1);
    if app.is_empty() {
        return error("matched window did not include an app name".to_string());
    }
    let body = match action.as_str() {
        "raise" => "perform action \"AXRaise\" of targetWindow\nreturn \"AXRaise\"",
        "minimize" => "set value of attribute \"AXMinimized\" of targetWindow to true\nreturn \"AXMinimized=true\"",
        "enter_full_screen" => "set value of attribute \"AXFullScreen\" of targetWindow to true\nreturn \"AXFullScreen=true\"",
        "exit_full_screen" => "set value of attribute \"AXFullScreen\" of targetWindow to false\nreturn \"AXFullScreen=false\"",
        "toggle_full_screen" => "set isFullScreen to false\ntry\n  set isFullScreen to value of attribute \"AXFullScreen\" of targetWindow as boolean\nend try\nif isFullScreen then\n  set value of attribute \"AXFullScreen\" of targetWindow to false\n  return \"AXFullScreen=false\"\nelse\n  set value of attribute \"AXFullScreen\" of targetWindow to true\n  return \"AXFullScreen=true\"\nend if",
        "zoom" => "try\n  perform action \"AXZoomWindow\" of targetWindow\n  return \"AXZoomWindow\"\non error\n  perform action \"AXPress\" of button 3 of targetWindow\n  return \"AXZoomButton\"\nend try",
        _ => return error("action is invalid".to_string()),
    };
    let script = format!(
        "tell application \"System Events\"\n  tell application process {}\n    set frontmost to true\n    set targetWindow to window {}\n{}\n  end tell\nend tell",
        applescript_string(&app),
        app_window_index,
        indent_applescript(body, 4)
    );
    match run_osascript(&script) {
        Ok(method) => {
            let wait_after_ms = int_arg(args, "wait_after_ms").unwrap_or(500).clamp(0, 5000);
            if wait_after_ms > 0 {
                thread::sleep(Duration::from_millis(wait_after_ms as u64));
            }
            ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                text: format!(
                    "Computer action: perform_window_action\nAction: {action}\nMethod: {}\nWindow: {}\nMatch count: {}\nWait after ms: {}",
                    method.trim(),
                    compact_json(&matched),
                    matches.len(),
                    wait_after_ms
                ),
                display: format!("performed {action} on {app} window"),
                error: false,
                            artifacts: Vec::new(),
                            activation: None,
}
        }
        Err(err) => error(err),
    }
}

fn run_key_script(key: &str, modifiers: &[&str]) -> ToolOutcome {
    let key = key.trim();
    let lowered = key.to_ascii_lowercase();
    let modifier_expr = modifier_expr(modifiers);
    let script = if let Some(code) = key_code(&lowered) {
        if modifier_expr.is_empty() {
            format!("tell application \"System Events\" to key code {code}")
        } else {
            format!("tell application \"System Events\" to key code {code} using {modifier_expr}")
        }
    } else {
        let value = applescript_string(key);
        if modifier_expr.is_empty() {
            format!("tell application \"System Events\" to keystroke {value}")
        } else {
            format!("tell application \"System Events\" to keystroke {value} using {modifier_expr}")
        }
    };
    run_osascript(&script)
        .map(|_| native_ok(format!("pressed {key}"), format!("pressed {key}")))
        .unwrap_or_else(error)
}

fn native_ok(text: impl Into<String>, display: impl Into<String>) -> ToolOutcome {
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: text.into(),
        display: display.into(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn run_osascript(script: &str) -> Result<String, String> {
    match Command::new("osascript").arg("-e").arg(script).output() {
        Ok(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        Ok(output) => Err(format!(
            "osascript failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(err) => Err(format!("failed to run osascript: {err}")),
    }
}

fn cancellable_sleep(duration: Duration, ctx: &ToolContext) -> Result<(), ToolOutcome> {
    let deadline = std::time::Instant::now() + duration;
    loop {
        if ctx.cancellation.is_requested() {
            return Err(error("computer action cancelled by user".to_string()));
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(remaining.min(Duration::from_millis(50)));
    }
}

fn collect_apps(include_background: bool, limit: i64) -> Result<(String, Vec<Value>), String> {
    let include_background_text = if include_background { "true" } else { "false" };
    let script = format!(
        r#"set previousDelimiters to AppleScript's text item delimiters
set AppleScript's text item delimiters to linefeed
set rows to {{}}
set includeBackground to {include_background_text}
set maxRows to {limit}
tell application "System Events"
    set frontName to ""
    try
        set frontName to name of first application process whose frontmost is true
    end try
    repeat with p in application processes
        if (count of rows) >= maxRows then exit repeat
        set procVisible to false
        set procFrontmost to false
        set appName to ""
        set bundleId to ""
        try
            set procVisible to visible of p
        end try
        if includeBackground or procVisible then
            try
                set appName to name of p
            end try
            try
                set bundleId to bundle identifier of p
            end try
            try
                set procFrontmost to frontmost of p
            end try
            if appName is not "" then
                set end of rows to appName & tab & bundleId & tab & (procVisible as text) & tab & (procFrontmost as text) & tab & frontName
            end if
        end if
    end repeat
end tell
set outputText to rows as text
set AppleScript's text item delimiters to previousDelimiters
return outputText"#
    );
    let output = run_osascript(&script)?;
    let mut apps = Vec::new();
    let mut frontmost = String::new();
    for line in output.lines() {
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() < 4 {
            continue;
        }
        if parts.len() >= 5 && !parts[4].is_empty() {
            frontmost = parts[4].to_string();
        }
        apps.push(json!({
            "index": apps.len() + 1,
            "name": parts[0],
            "bundle_identifier": parts[1],
            "visible": parse_bool_text(parts[2]).unwrap_or(false),
            "frontmost": parse_bool_text(parts[3]).unwrap_or(false),
        }));
        if apps.len() >= limit as usize {
            break;
        }
    }
    if frontmost.is_empty() {
        frontmost = apps
            .iter()
            .find(|app| {
                app.get("frontmost")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .and_then(|app| app.get("name").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
    }
    Ok((frontmost, apps))
}

fn app_condition_state(args: &Map<String, Value>) -> Result<Value, String> {
    let (app_query, bundle_query) = app_query_parts(args)?;
    let match_mode = string_arg(args, "match_mode").unwrap_or_else(|| "contains".to_string());
    let expected_running = bool_arg(args, "running").unwrap_or(true);
    let visible_expected = optional_bool_arg(args, "visible");
    let frontmost_expected = optional_bool_arg(args, "frontmost");
    let max_apps = int_arg(args, "max_apps").unwrap_or(100).clamp(1, 500);
    let (frontmost, apps) = collect_apps(true, max_apps)?;
    let matches = apps
        .iter()
        .filter(|app| app_matches(app, &app_query, &bundle_query, &match_mode))
        .cloned()
        .collect::<Vec<_>>();
    let matched = matches.first().cloned();
    let found = matched.is_some();
    let visible_actual = matched
        .as_ref()
        .and_then(|app| app.get("visible"))
        .and_then(Value::as_bool);
    let frontmost_actual = matched
        .as_ref()
        .and_then(|app| app.get("frontmost"))
        .and_then(Value::as_bool);
    let visible_ok = visible_expected
        .map(|expected| visible_actual == Some(expected))
        .unwrap_or(true);
    let frontmost_ok = frontmost_expected
        .map(|expected| frontmost_actual == Some(expected))
        .unwrap_or(true);
    let mut condition_met = found == expected_running;
    if expected_running && found {
        condition_met = condition_met && visible_ok && frontmost_ok;
    }
    Ok(json!({
        "found": found,
        "condition_met": condition_met,
        "expected_running": expected_running,
        "app_query": app_query,
        "bundle_id_query": bundle_query,
        "match_mode": match_mode,
        "match_count": matches.len(),
        "matched_app": matched.unwrap_or_else(|| json!({})),
        "frontmost_app": frontmost,
        "visible_expected": visible_expected,
        "visible_actual": visible_actual,
        "frontmost_expected": frontmost_expected,
        "frontmost_actual": frontmost_actual,
        "candidate_apps": apps.into_iter().take(20).collect::<Vec<_>>(),
    }))
}

fn collect_windows(limit: i64) -> Result<(String, Vec<Value>), String> {
    let script = r#"set previousDelimiters to AppleScript's text item delimiters
set AppleScript's text item delimiters to linefeed
set rows to {}
tell application "System Events"
    set frontName to ""
    try
        set frontName to name of first application process whose frontmost is true
    end try
    repeat with p in (application processes whose visible is true)
        set appName to name of p
        set appWindowIndex to 0
        repeat with w in windows of p
            try
                set appWindowIndex to appWindowIndex + 1
                set windowName to name of w
                set windowPosition to position of w
                set windowSize to size of w
                set minimizedText to ""
                set fullScreenText to ""
                try
                    set minimizedText to value of attribute "AXMinimized" of w as text
                end try
                try
                    set fullScreenText to value of attribute "AXFullScreen" of w as text
                end try
                set end of rows to appName & tab & windowName & tab & (appWindowIndex as text) & tab & ((item 1 of windowPosition) as text) & "," & ((item 2 of windowPosition) as text) & "," & ((item 1 of windowSize) as text) & "," & ((item 2 of windowSize) as text) & tab & frontName & tab & minimizedText & tab & fullScreenText
            end try
        end repeat
    end repeat
end tell
set outputText to rows as text
set AppleScript's text item delimiters to previousDelimiters
return outputText"#;
    let output = run_osascript(script)?;
    let mut windows = Vec::new();
    let mut frontmost = String::new();
    for line in output.lines() {
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() < 5 {
            continue;
        }
        if !parts[4].is_empty() {
            frontmost = parts[4].to_string();
        }
        let mut item = json!({
            "index": windows.len() + 1,
            "app": parts[0],
            "title": parts[1],
            "app_window_index": parts[2].parse::<i64>().unwrap_or(1),
        });
        if let Some(object) = item.as_object_mut() {
            let bounds = parts[3].split(',').collect::<Vec<_>>();
            if bounds.len() == 4 {
                object.insert("x".to_string(), json!(parse_i64(bounds[0]).unwrap_or(0)));
                object.insert("y".to_string(), json!(parse_i64(bounds[1]).unwrap_or(0)));
                object.insert(
                    "width".to_string(),
                    json!(parse_i64(bounds[2]).unwrap_or(0)),
                );
                object.insert(
                    "height".to_string(),
                    json!(parse_i64(bounds[3]).unwrap_or(0)),
                );
            } else {
                object.insert("bounds".to_string(), json!(parts[3]));
            }
            if let Some(value) = parts.get(5).and_then(|value| parse_bool_text(value)) {
                object.insert("minimized".to_string(), json!(value));
            }
            if let Some(value) = parts.get(6).and_then(|value| parse_bool_text(value)) {
                object.insert("full_screen".to_string(), json!(value));
            }
        }
        windows.push(item);
        if windows.len() >= limit as usize {
            break;
        }
    }
    Ok((frontmost, windows))
}

fn window_condition_state(args: &Map<String, Value>) -> Result<Value, String> {
    let expected_exists = bool_arg(args, "exists").unwrap_or(true);
    let max_windows = int_arg(args, "max_windows").unwrap_or(50).clamp(1, 200);
    let (frontmost, windows) = collect_windows(max_windows)?;
    let matches = matching_windows(args, &windows)?;
    let occurrence = int_arg(args, "occurrence").unwrap_or(1).clamp(1, 20) as usize;
    let matched = matches.get(occurrence - 1).cloned();
    let found = matched.is_some();
    let focused_expected = optional_bool_arg(args, "focused");
    let minimized_expected = optional_bool_arg(args, "minimized");
    let full_screen_expected = optional_bool_arg(args, "full_screen");
    let focused_actual = matched
        .as_ref()
        .and_then(|window| window.get("app"))
        .and_then(Value::as_str)
        .map(|app| app == frontmost);
    let minimized_actual = matched
        .as_ref()
        .and_then(|window| window.get("minimized"))
        .and_then(Value::as_bool);
    let full_screen_actual = matched
        .as_ref()
        .and_then(|window| window.get("full_screen"))
        .and_then(Value::as_bool);
    let focused_ok = focused_expected
        .map(|expected| focused_actual == Some(expected))
        .unwrap_or(true);
    let minimized_ok = minimized_expected
        .map(|expected| minimized_actual == Some(expected))
        .unwrap_or(true);
    let full_screen_ok = full_screen_expected
        .map(|expected| full_screen_actual == Some(expected))
        .unwrap_or(true);
    let (bounds_ok, bounds) = matched
        .as_ref()
        .map(|window| window_bounds_state(window, args))
        .transpose()?
        .unwrap_or((true, json!({})));
    let mut condition_met = found == expected_exists;
    if expected_exists && found {
        condition_met = condition_met && focused_ok && minimized_ok && full_screen_ok && bounds_ok;
    }
    Ok(json!({
        "found": found,
        "condition_met": condition_met,
        "expected_exists": expected_exists,
        "match_count": matches.len(),
        "matched_window": matched.unwrap_or_else(|| json!({})),
        "frontmost_app": frontmost,
        "focused_expected": focused_expected,
        "focused_actual": focused_actual,
        "minimized_expected": minimized_expected,
        "minimized_actual": minimized_actual,
        "full_screen_expected": full_screen_expected,
        "full_screen_actual": full_screen_actual,
        "bounds": bounds,
        "candidate_windows": windows.into_iter().take(20).collect::<Vec<_>>(),
    }))
}

fn find_window_match(args: &Map<String, Value>) -> Result<(Vec<Value>, Value), String> {
    let max_windows = int_arg(args, "max_windows").unwrap_or(50).clamp(1, 200);
    let (_, windows) = collect_windows(max_windows)?;
    let matches = matching_windows(args, &windows)?;
    let occurrence = int_arg(args, "occurrence").unwrap_or(1).clamp(1, 20) as usize;
    let Some(matched) = matches.get(occurrence - 1).cloned() else {
        return Err("no visible window matched app/title/index".to_string());
    };
    Ok((matches, matched))
}

fn matching_windows(args: &Map<String, Value>, windows: &[Value]) -> Result<Vec<Value>, String> {
    let index = int_arg(args, "index");
    let app = string_arg(args, "app").unwrap_or_default();
    let title = string_arg(args, "title").unwrap_or_default();
    if index.is_none() && app.is_empty() && title.is_empty() {
        return Err("app, title, or index is required".to_string());
    }
    let match_mode = string_arg(args, "match_mode").unwrap_or_else(|| "contains".to_string());
    let matches = windows
        .iter()
        .filter(|window| {
            if let Some(index) = index {
                if window.get("index").and_then(Value::as_i64) != Some(index) {
                    return false;
                }
            }
            if !app.is_empty()
                && !text_matches(
                    window
                        .get("app")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    &app,
                    &match_mode,
                )
            {
                return false;
            }
            if !title.is_empty()
                && !text_matches(
                    window
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    &title,
                    &match_mode,
                )
            {
                return false;
            }
            true
        })
        .cloned()
        .collect();
    Ok(matches)
}

fn window_bounds_state(window: &Value, args: &Map<String, Value>) -> Result<(bool, Value), String> {
    let keys = ["x", "y", "width", "height"];
    let has_any = keys.iter().any(|key| args.get(*key).is_some());
    if !has_any {
        return Ok((true, json!({})));
    }
    if keys.iter().any(|key| args.get(*key).is_none()) {
        return Err("x, y, width, and height must be provided together".to_string());
    }
    let expected = json!({
        "x": number_arg(args, "x").unwrap_or(0.0).round() as i64,
        "y": number_arg(args, "y").unwrap_or(0.0).round() as i64,
        "width": int_arg(args, "width").unwrap_or(0),
        "height": int_arg(args, "height").unwrap_or(0),
    });
    let actual = json!({
        "x": window.get("x").and_then(Value::as_i64),
        "y": window.get("y").and_then(Value::as_i64),
        "width": window.get("width").and_then(Value::as_i64),
        "height": window.get("height").and_then(Value::as_i64),
    });
    let tolerance = int_arg(args, "tolerance").unwrap_or(8).clamp(0, 500);
    let mut matched = true;
    let mut deltas = Map::new();
    for key in keys {
        let expected_value = expected.get(key).and_then(Value::as_i64).unwrap_or(0);
        let actual_value = actual.get(key).and_then(Value::as_i64);
        let delta = actual_value.map(|actual| (actual - expected_value).abs());
        if delta.map_or(true, |value| value > tolerance) {
            matched = false;
        }
        deltas.insert(key.to_string(), delta.map_or(Value::Null, Value::from));
    }
    Ok((
        matched,
        json!({
            "expected": expected,
            "actual": actual,
            "tolerance": tolerance,
            "deltas": deltas,
        }),
    ))
}

fn set_clipboard_bytes(bytes: &[u8]) -> Result<(), String> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to run pbcopy: {err}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(bytes)
            .map_err(|err| format!("failed to write clipboard: {err}"))?;
    }
    let status = child
        .wait()
        .map_err(|err| format!("failed to wait for pbcopy: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("pbcopy failed with status {status}"))
    }
}

struct ClipboardCondition {
    expected: String,
    match_mode: String,
    expected_exists: bool,
    max_chars: usize,
}

fn read_clipboard_text() -> Result<String, String> {
    match Command::new("pbpaste").output() {
        Ok(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        Ok(output) => Err(format!(
            "pbpaste failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(err) => Err(format!("failed to run pbpaste: {err}")),
    }
}

fn clipboard_condition(args: &Map<String, Value>) -> Result<ClipboardCondition, String> {
    let expected = args
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| "text is required".to_string())?
        .to_string();
    let match_mode = string_arg(args, "match_mode").unwrap_or_else(|| "contains".to_string());
    if !matches!(match_mode.as_str(), "contains" | "exact") {
        return Err("match_mode must be contains/exact".to_string());
    }
    if expected.is_empty() && match_mode != "exact" {
        return Err("text is required unless match_mode is exact".to_string());
    }
    Ok(ClipboardCondition {
        expected,
        match_mode,
        expected_exists: bool_arg(args, "exists").unwrap_or(true),
        max_chars: int_arg(args, "max_chars").unwrap_or(4000).clamp(1, 50000) as usize,
    })
}

fn clipboard_matches(value: &str, expected: &str, match_mode: &str) -> bool {
    if match_mode == "exact" {
        value == expected
    } else {
        value.contains(expected)
    }
}

fn clipboard_condition_text(
    action: &str,
    value: &str,
    output: &str,
    truncated: bool,
    found: bool,
    condition: &ClipboardCondition,
    extra: &[(&str, String)],
) -> String {
    let mut lines = vec![
        format!("Computer action: {action}"),
        format!("Clipboard characters: {}", value.chars().count()),
        format!("Output characters: {}", output.chars().count()),
        format!("Truncated: {truncated}"),
        format!(
            "Expected characters: {}",
            condition.expected.chars().count()
        ),
        format!("Match mode: {}", condition.match_mode),
        format!("Expected exists: {}", condition.expected_exists),
        format!("Found: {found}"),
    ];
    lines.extend(extra.iter().map(|(key, value)| format!("{key}: {value}")));
    lines.push(format!("Text:\n{output}"));
    lines.join("\n")
}

fn clip_text(value: &str, max_chars: usize) -> (String, bool) {
    let mut output = String::new();
    for ch in value.chars().take(max_chars) {
        output.push(ch);
    }
    let truncated = value.chars().count() > max_chars;
    (output, truncated)
}

fn key_code(key: &str) -> Option<u16> {
    match key {
        "return" | "enter" => Some(36),
        "tab" => Some(48),
        "space" => Some(49),
        "delete" | "backspace" => Some(51),
        "escape" | "esc" => Some(53),
        "command" | "cmd" => Some(55),
        "shift" => Some(56),
        "capslock" | "caps_lock" => Some(57),
        "option" | "alt" => Some(58),
        "control" | "ctrl" => Some(59),
        "left" | "arrowleft" | "left_arrow" => Some(123),
        "right" | "arrowright" | "right_arrow" => Some(124),
        "down" | "arrowdown" | "down_arrow" => Some(125),
        "up" | "arrowup" | "up_arrow" => Some(126),
        "home" => Some(115),
        "end" => Some(119),
        "pageup" | "page_up" => Some(116),
        "pagedown" | "page_down" => Some(121),
        "f1" => Some(122),
        "f2" => Some(120),
        "f3" => Some(99),
        "f4" => Some(118),
        "f5" => Some(96),
        "f6" => Some(97),
        "f7" => Some(98),
        "f8" => Some(100),
        "f9" => Some(101),
        "f10" => Some(109),
        "f11" => Some(103),
        "f12" => Some(111),
        _ => None,
    }
}

fn app_query_parts(args: &Map<String, Value>) -> Result<(String, String), String> {
    let mut app_query = string_arg(args, "app").unwrap_or_default();
    let mut bundle_query = string_arg(args, "bundle_id").unwrap_or_default();
    if app_query.starts_with("id:") && bundle_query.is_empty() {
        bundle_query = app_query[3..].trim().to_string();
        app_query.clear();
    } else if app_query.starts_with("app:") {
        app_query = app_query[4..].trim().to_string();
    }
    if app_query.is_empty() && bundle_query.is_empty() {
        return Err("app or bundle_id is required".to_string());
    }
    Ok((app_query, bundle_query))
}

fn app_matches(app: &Value, app_query: &str, bundle_query: &str, match_mode: &str) -> bool {
    if !app_query.is_empty()
        && !text_matches(
            app.get("name").and_then(Value::as_str).unwrap_or_default(),
            app_query,
            match_mode,
        )
    {
        return false;
    }
    if !bundle_query.is_empty()
        && !app
            .get("bundle_identifier")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .eq_ignore_ascii_case(bundle_query)
    {
        return false;
    }
    true
}

fn text_matches(value: &str, expected: &str, match_mode: &str) -> bool {
    let haystack = normalize_match_text(value);
    let needle = normalize_match_text(expected);
    if match_mode == "exact" {
        haystack == needle
    } else {
        haystack.contains(&needle)
    }
}

fn normalize_match_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn parse_bool_text(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn parse_i64(value: &str) -> Option<i64> {
    value.trim().parse::<f64>().ok().map(|value| value as i64)
}

fn bool_arg(args: &Map<String, Value>, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn optional_bool_arg(args: &Map<String, Value>, key: &str) -> Option<bool> {
    bool_arg(args, key)
}

fn int_arg(args: &Map<String, Value>, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

fn indent_applescript(script: &str, spaces: usize) -> String {
    let indent = " ".repeat(spaces);
    script
        .lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn modifier_expr(modifiers: &[&str]) -> String {
    let values = modifiers
        .iter()
        .filter_map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "command" | "cmd" | "meta" => Some("command down"),
            "control" | "ctrl" => Some("control down"),
            "option" | "alt" => Some("option down"),
            "shift" => Some("shift down"),
            _ => None,
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", values.join(", "))
    }
}

fn applescript_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn string_arg(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn string_array_arg(args: &Map<String, Value>, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn number_arg(args: &Map<String, Value>, key: &str) -> Option<f64> {
    args.get(key).and_then(Value::as_f64)
}

fn output_path(args: &Map<String, Value>) -> Option<PathBuf> {
    string_arg(args, "path")
        .or_else(|| string_arg(args, "output_path"))
        .map(PathBuf::from)
}

fn default_screenshot_path(ctx: &ToolContext) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    ctx.browser
        .state_dir
        .join(Path::new("computer"))
        .join(format!("screenshot-{millis}.png"))
}

fn has_any(args: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|key| args.contains_key(*key))
}
