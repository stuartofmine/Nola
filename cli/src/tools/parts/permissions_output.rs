fn normalize_search_url(href: &str) -> String {
    let value = decode_html_entities(href).trim().to_string();
    if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value
    }
}

fn parse_duckduckgo_results(html: &str, limit: usize) -> Vec<(String, String, String)> {
    let result_regex =
        Regex::new("(?is)<a\\b([^>]*)>(.*?)</a>").expect("valid DuckDuckGo result anchor regex");
    let snippet_regex = Regex::new("(?is)<(?:a|div)\\b([^>]*)>(.*?)</(?:a|div)>")
        .expect("valid DuckDuckGo snippet regex");
    let mut results = Vec::new();
    for captures in result_regex.captures_iter(html) {
        let attrs = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        if !html_class_contains(attrs, "result__a") {
            continue;
        }
        let Some(raw_href) = html_attr_value(attrs, "href") else {
            continue;
        };
        let title = strip_html(captures.get(2).map(|value| value.as_str()).unwrap_or(""));
        let link = normalize_duckduckgo_url(&raw_href);
        if !search_result_is_usable(&title, &link, &results) {
            continue;
        }
        let snippet_window = captures
            .get(0)
            .map(|matched| {
                let tail = &html[matched.end()..];
                next_duckduckgo_result_start(tail, &result_regex)
                    .map(|next_start| &tail[..next_start])
                    .unwrap_or(tail)
            })
            .unwrap_or("");
        let snippet = parse_duckduckgo_snippet(snippet_window, &snippet_regex);
        results.push((title, link, snippet));
        if results.len() >= limit {
            break;
        }
    }
    results
}

fn next_duckduckgo_result_start(window: &str, result_regex: &Regex) -> Option<usize> {
    for captures in result_regex.captures_iter(window) {
        let attrs = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        if html_class_contains(attrs, "result__a") {
            return captures.get(0).map(|matched| matched.start());
        }
    }
    None
}

fn parse_duckduckgo_snippet(window: &str, snippet_regex: &Regex) -> String {
    for captures in snippet_regex.captures_iter(window) {
        let attrs = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        if html_class_contains(attrs, "result__snippet") {
            return strip_html(captures.get(2).map(|value| value.as_str()).unwrap_or(""));
        }
    }
    String::new()
}

fn html_class_contains(attrs: &str, class_name: &str) -> bool {
    html_attr_value(attrs, "class")
        .map(|classes| {
            classes
                .split_whitespace()
                .any(|value| value.eq_ignore_ascii_case(class_name))
        })
        .unwrap_or(false)
}

fn html_attr_value(attrs: &str, name: &str) -> Option<String> {
    let attr_regex = Regex::new(
        "(?is)\\b([a-zA-Z_:][-a-zA-Z0-9_:.]*)\\s*=\\s*(?:\"([^\"]*)\"|'([^']*)'|([^\\s\"'>]+))",
    )
    .expect("valid html attribute regex");
    for captures in attr_regex.captures_iter(attrs) {
        let attr_name = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        if attr_name.eq_ignore_ascii_case(name) {
            let value = captures
                .get(2)
                .or_else(|| captures.get(3))
                .or_else(|| captures.get(4))
                .map(|value| value.as_str())
                .unwrap_or("");
            return Some(decode_html_entities(value));
        }
    }
    None
}

fn normalize_duckduckgo_url(href: &str) -> String {
    let value = decode_html_entities(href).trim().to_string();
    let value = if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value
    };
    if value.starts_with("/l/")
        || value.starts_with("https://duckduckgo.com/l/")
        || value.starts_with("https://html.duckduckgo.com/l/")
    {
        if let Some(encoded) = query_param(&value, "uddg") {
            return percent_decode_form(&encoded);
        }
    }
    value
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1.split('#').next().unwrap_or("");
    for pair in query.split('&') {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode_form(raw_key) == key {
            return Some(raw_value.to_string());
        }
    }
    None
}

fn percent_encode_query(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(*byte))
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn percent_decode_form(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'+' {
            decoded.push(b' ');
            index += 1;
        } else if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
            } else {
                decoded.push(bytes[index]);
                index += 1;
            }
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).to_string()
}

fn validate_open_item_file_url(target: &str) -> Result<(), ToolOutcome> {
    let rest = target.get(7..).unwrap_or("");
    if rest.contains('?') || rest.contains('#') {
        return Err(error(
            "file URL cannot include query or fragment".to_string(),
        ));
    }
    let _ = browser_file_url_path(target)?;
    Ok(())
}

fn optional_nonempty_string(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn url_scheme(value: &str) -> Option<&str> {
    let (scheme, _) = value.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '.' || ch == '-') {
        Some(scheme)
    } else {
        None
    }
}

fn validate_open_item_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let target = required_string(args, "target")?;
    if target.chars().count() > 2048
        || target.contains('\0')
        || target.contains('\n')
        || target.contains('\r')
    {
        return Err(error("target is invalid".to_string()));
    }
    let scheme = url_scheme(&target)
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if !scheme.is_empty() && !matches!(scheme.as_str(), "http" | "https" | "file") {
        return Err(error(
            "target URL scheme must be http, https, or file".to_string(),
        ));
    }
    match scheme.as_str() {
        "http" | "https" => validate_open_item_http_url(&target)?,
        "file" => validate_open_item_file_url(&target)?,
        _ => {}
    }
    let app = arg_string(args, "app").unwrap_or_default();
    if app.len() > 120 || app.contains('\n') || app.contains('\r') {
        return Err(error("app is invalid".to_string()));
    }
    let reveal = args.get("reveal").and_then(Value::as_bool).unwrap_or(false);
    if args.get("reveal").is_some_and(|value| !value.is_boolean()) {
        return Err(error("reveal must be a boolean".to_string()));
    }
    if args
        .get("screenshot")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(error("screenshot must be a boolean".to_string()));
    }
    if reveal && !app.is_empty() {
        return Err(error("reveal cannot be combined with app".to_string()));
    }
    if reveal && !scheme.is_empty() && scheme != "file" {
        return Err(error(
            "reveal requires a file or directory path".to_string(),
        ));
    }
    validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
    Ok(())
}

fn validate_optional_enum(
    args: &Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<(), ToolOutcome> {
    let Some(value) = optional_nonempty_string(args, key) else {
        return Ok(());
    };
    if allowed.iter().any(|item| *item == value) {
        Ok(())
    } else {
        Err(error(format!("{key} is not supported")))
    }
}

fn validate_boolean_arg(args: &Map<String, Value>, key: &str) -> Result<(), ToolOutcome> {
    if args.get(key).is_some_and(|value| !value.is_boolean()) {
        return Err(error(format!("{key} must be a boolean")));
    }
    Ok(())
}

fn validate_boolean_args(args: &Map<String, Value>, keys: &[&str]) -> Result<(), ToolOutcome> {
    for key in keys {
        validate_boolean_arg(args, key)?;
    }
    Ok(())
}

fn validate_optional_browser_click_modifiers(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let Some(value) = args.get("modifiers") else {
        return Ok(());
    };
    let Some(items) = value.as_array() else {
        return Err(error("modifiers must be a list".to_string()));
    };
    if items.len() > 4 {
        return Err(error("modifiers may contain at most 4 items".to_string()));
    }
    for item in items {
        let Some(modifier) = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Err(error("modifiers must be a list of strings".to_string()));
        };
        if !matches!(
            modifier,
            "Alt" | "Command" | "Control" | "Meta" | "Option" | "Shift"
        ) {
            return Err(error(
                "modifiers must contain only Alt, Command, Control, Meta, Option, or Shift"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_required_string(
    args: &Map<String, Value>,
    key: &str,
    max_chars: usize,
) -> Result<(), ToolOutcome> {
    let Some(value) = args.get(key).and_then(Value::as_str) else {
        return Err(error(format!("{key} is required")));
    };
    if value.chars().count() > max_chars {
        return Err(error(format!("{key} must be <= {max_chars} characters")));
    }
    Ok(())
}

fn validate_optional_mouse_modifiers(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let Some(value) = args.get("modifiers") else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let Some(modifiers) = value.as_array() else {
        return Err(error("modifiers must be an array".to_string()));
    };
    if modifiers.len() > 4 {
        return Err(error("modifiers must contain at most 4 items".to_string()));
    }
    let allowed = ["Alt", "Command", "Control", "Meta", "Option", "Shift"];
    for modifier in modifiers {
        let Some(modifier) = modifier.as_str() else {
            return Err(error("modifier is not supported".to_string()));
        };
        if !allowed.contains(&modifier) {
            return Err(error("modifier is not supported".to_string()));
        }
    }
    Ok(())
}

fn validate_integer_arg(
    args: &Map<String, Value>,
    key: &str,
    default: i64,
    low: i64,
    high: i64,
) -> Result<i64, ToolOutcome> {
    let value = match args.get(key) {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .unwrap_or(default),
        Some(Value::String(text)) if !text.trim().is_empty() => text
            .trim()
            .parse::<i64>()
            .map_err(|_| error(format!("{key} must be an integer")))?,
        Some(Value::Null) | None => default,
        _ => return Err(error(format!("{key} must be an integer"))),
    };
    if value < low || value > high {
        return Err(error(format!("{key} must be between {low} and {high}")));
    }
    Ok(value)
}

fn validate_float_arg(
    args: &Map<String, Value>,
    key: &str,
    default: f64,
    low: f64,
    high: f64,
) -> Result<f64, ToolOutcome> {
    let value = match args.get(key) {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(default),
        Some(Value::String(text)) if !text.trim().is_empty() => text
            .trim()
            .parse::<f64>()
            .map_err(|_| error(format!("{key} must be a number")))?,
        Some(Value::Null) | None => default,
        _ => return Err(error(format!("{key} must be a number"))),
    };
    if value < low || value > high {
        return Err(error(format!("{key} must be between {low} and {high}")));
    }
    Ok(value)
}

fn optional_element_index(args: &Map<String, Value>) -> Result<Option<i64>, ToolOutcome> {
    optional_named_index(args, "index")
}

fn optional_named_index(args: &Map<String, Value>, key: &str) -> Result<Option<i64>, ToolOutcome> {
    let has_index = match args.get(key) {
        Some(Value::Null) | None => false,
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    };
    if !has_index {
        return Ok(None);
    }
    validate_integer_arg(args, key, 1, 1, 500).map(Some)
}

fn validate_query_or_index(args: &Map<String, Value>) -> Result<Option<String>, ToolOutcome> {
    let query = optional_nonempty_string(args, "query");
    let index = optional_element_index(args)?;
    if query.is_none() && index.is_none() {
        return Err(error("query or index is required".to_string()));
    }
    if let Some(query) = &query {
        if query.chars().count() > 200 || query.contains('\n') || query.contains('\r') {
            return Err(error("query is invalid".to_string()));
        }
    }
    Ok(query)
}

fn validate_drag_element_target(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let has_target_x = args.get("target_x").is_some_and(|value| !value.is_null());
    let has_target_y = args.get("target_y").is_some_and(|value| !value.is_null());
    let has_target_offset = ["target_offset_x", "target_offset_y"]
        .iter()
        .any(|key| has_non_empty_arg(args, key));
    let target_query = optional_nonempty_string(args, "target_query");
    let target_index = optional_named_index(args, "target_index")?;
    if has_target_x || has_target_y {
        if has_target_x != has_target_y {
            return Err(error(
                "target_x and target_y must be provided together".to_string(),
            ));
        }
        if target_query.is_some() || target_index.is_some() {
            return Err(error(
                "target coordinates cannot be combined with target_query or target_index"
                    .to_string(),
            ));
        }
        if has_target_offset {
            return Err(error(
                "target_offset_x/target_offset_y can only be used with target elements".to_string(),
            ));
        }
        for key in ["target_x", "target_y"] {
            let value = args
                .get(key)
                .and_then(Value::as_f64)
                .ok_or_else(|| error(format!("{key} must be a number")))?;
            if !(MIN_SCREEN_COORDINATE..=MAX_SCREEN_COORDINATE).contains(&value) {
                return Err(error(format!("{key} must be between -100000 and 100000")));
            }
        }
    } else if target_query.is_none() && target_index.is_none() {
        return Err(error(
            "target_x/target_y or target_query/target_index is required".to_string(),
        ));
    }
    if let Some(query) = &target_query {
        if query.chars().count() > 200 || query.contains('\n') || query.contains('\r') {
            return Err(error("target_query is invalid".to_string()));
        }
    }
    if let Some(role) = arg_string(args, "target_role") {
        if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
            return Err(error("target_role is invalid".to_string()));
        }
    }
    validate_optional_enum(args, "target_match_mode", &["contains", "exact"])?;
    validate_integer_arg(args, "target_occurrence", 1, 1, 20)?;
    validate_optional_element_offsets(args, &["target_offset_x", "target_offset_y"])?;
    Ok(())
}

fn validate_window_target(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let app = optional_nonempty_string(args, "app");
    let title = optional_nonempty_string(args, "title");
    let index = optional_element_index(args)?;
    if app.is_none() && title.is_none() && index.is_none() {
        return Err(error("app, title, or index is required".to_string()));
    }
    if let Some(app) = &app {
        if app.chars().count() > 120 || app.contains('\n') || app.contains('\r') {
            return Err(error("app is invalid".to_string()));
        }
    }
    if let Some(title) = &title {
        if title.chars().count() > 300 || title.contains('\n') || title.contains('\r') {
            return Err(error("title is invalid".to_string()));
        }
    }
    validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
    validate_integer_arg(args, "occurrence", 1, 1, 20)?;
    validate_integer_arg(args, "max_windows", 50, 1, 200)?;
    Ok(())
}

fn validate_window_bounds_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    validate_window_target(args)?;
    validate_required_coordinate_args(args, &["x", "y"])?;
    validate_integer_arg(args, "width", 0, 1, 100_000)?;
    validate_integer_arg(args, "height", 0, 1, 100_000)?;
    Ok(())
}

fn validate_optional_window_bounds_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let keys = ["x", "y", "width", "height"];
    let has_any = keys.iter().any(|key| match args.get(*key) {
        Some(Value::Null) | None => false,
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    });
    validate_integer_arg(args, "tolerance", 8, 0, 500)?;
    if !has_any {
        return Ok(());
    }
    for key in keys {
        if matches!(args.get(key), Some(Value::Null) | None)
            || matches!(args.get(key), Some(Value::String(text)) if text.trim().is_empty())
        {
            return Err(error(
                "x, y, width, and height must be provided together".to_string(),
            ));
        }
    }
    validate_required_coordinate_args(args, &["x", "y"])?;
    validate_integer_arg(args, "width", 0, 1, 100_000)?;
    validate_integer_arg(args, "height", 0, 1, 100_000)?;
    Ok(())
}

fn validate_window_condition_args(
    args: &Map<String, Value>,
    include_wait: bool,
) -> Result<(), ToolOutcome> {
    validate_window_target(args)?;
    if args.get("exists").is_some_and(|value| !value.is_boolean()) {
        return Err(error("exists must be a boolean".to_string()));
    }
    if args.get("focused").is_some_and(|value| !value.is_boolean()) {
        return Err(error("focused must be a boolean".to_string()));
    }
    for key in ["minimized", "full_screen"] {
        if args.get(key).is_some_and(|value| !value.is_boolean()) {
            return Err(error(format!("{key} must be a boolean")));
        }
    }
    validate_optional_window_bounds_args(args)?;
    if include_wait {
        validate_integer_arg(args, "timeout_ms", 5000, 0, 60_000)?;
        validate_integer_arg(args, "interval_ms", 500, 100, 5000)?;
    }
    Ok(())
}

fn validate_app_condition_args(
    args: &Map<String, Value>,
    include_wait: bool,
) -> Result<(), ToolOutcome> {
    let app = arg_string(args, "app");
    let bundle_id = arg_string(args, "bundle_id");
    if app.is_none() && bundle_id.is_none() {
        return Err(error("app or bundle_id is required".to_string()));
    }
    if let Some(app) = &app {
        if app.chars().count() > 120 || app.contains('\n') || app.contains('\r') {
            return Err(error("app is invalid".to_string()));
        }
    }
    if let Some(bundle_id) = &bundle_id {
        if bundle_id.chars().count() > 200 || bundle_id.contains('\n') || bundle_id.contains('\r') {
            return Err(error("bundle_id is invalid".to_string()));
        }
    }
    validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
    for key in ["running", "visible", "frontmost"] {
        if args.get(key).is_some_and(|value| !value.is_boolean()) {
            return Err(error(format!("{key} must be a boolean")));
        }
    }
    validate_integer_arg(args, "max_apps", 100, 1, 500)?;
    if include_wait {
        validate_integer_arg(args, "timeout_ms", 5000, 0, 60_000)?;
        validate_integer_arg(args, "interval_ms", 500, 100, 5000)?;
    }
    Ok(())
}

fn validate_menu_path_arg(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let raw_path = args
        .get("menu_path")
        .and_then(Value::as_array)
        .ok_or_else(|| error("menu_path must be an array".to_string()))?;
    if raw_path.len() < 2 || raw_path.len() > 8 {
        return Err(error("menu_path must include 2 to 8 items".to_string()));
    }
    for item in raw_path {
        let label = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| error("menu_path contains an invalid item".to_string()))?;
        if label.chars().count() > 120 || label.contains('\n') || label.contains('\r') {
            return Err(error("menu_path contains an invalid item".to_string()));
        }
    }
    Ok(())
}

fn validate_optional_menu_label_arg(
    args: &Map<String, Value>,
    key: &str,
) -> Result<(), ToolOutcome> {
    if matches!(args.get(key), Some(Value::Null) | None)
        || matches!(args.get(key), Some(Value::String(text)) if text.trim().is_empty())
    {
        return Ok(());
    }
    let label = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| error(format!("{key} is invalid")))?;
    if label.is_empty()
        || label.chars().count() > 120
        || label.contains('\n')
        || label.contains('\r')
    {
        return Err(error(format!("{key} is invalid")));
    }
    Ok(())
}

fn validate_clipboard_condition_args(
    args: &Map<String, Value>,
    include_wait: bool,
) -> Result<(), ToolOutcome> {
    let text = args
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| error("text is required".to_string()))?;
    validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
    let match_mode = arg_string(args, "match_mode").unwrap_or_else(|| "contains".to_string());
    if text.is_empty() && match_mode != "exact" {
        return Err(error(
            "text is required unless match_mode is exact".to_string(),
        ));
    }
    validate_integer_arg(args, "max_chars", 4000, 1, 50_000)?;
    if include_wait {
        validate_integer_arg(args, "timeout_ms", 5000, 0, 60_000)?;
        validate_integer_arg(args, "interval_ms", 250, 100, 5000)?;
    }
    Ok(())
}

fn validate_optional_coordinates(args: &Map<String, Value>) -> Result<bool, ToolOutcome> {
    let has_x = args.get("x").is_some_and(|value| !value.is_null());
    let has_y = args.get("y").is_some_and(|value| !value.is_null());
    if has_x != has_y {
        return Err(error("x and y must be provided together".to_string()));
    }
    if !has_x {
        return Ok(false);
    }
    for key in ["x", "y"] {
        let value = args
            .get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| error(format!("{key} must be a number")))?;
        if !(MIN_SCREEN_COORDINATE..=MAX_SCREEN_COORDINATE).contains(&value) {
            return Err(error(format!("{key} must be between -100000 and 100000")));
        }
    }
    Ok(true)
}

fn validate_optional_browser_coordinate_pair(
    args: &Map<String, Value>,
    x_key: &str,
    y_key: &str,
) -> Result<bool, ToolOutcome> {
    let has_x = args.get(x_key).is_some_and(|value| !value.is_null());
    let has_y = args.get(y_key).is_some_and(|value| !value.is_null());
    if has_x != has_y {
        return Err(error(format!(
            "{x_key} and {y_key} must be provided together"
        )));
    }
    if !has_x {
        return Ok(false);
    }
    for key in [x_key, y_key] {
        let value = args
            .get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| error(format!("{key} must be a number")))?;
        if !(0.0..=10_000.0).contains(&value) {
            return Err(error(format!("{key} must be between 0 and 10000")));
        }
    }
    Ok(true)
}

fn validate_optional_element_offsets(
    args: &Map<String, Value>,
    keys: &[&str],
) -> Result<(), ToolOutcome> {
    for key in keys {
        let Some(value) = args.get(*key) else {
            continue;
        };
        if value.is_null() || value.as_str() == Some("") {
            continue;
        }
        let Some(offset) = value.as_f64() else {
            return Err(error(format!("{key} must be a number")));
        };
        if !(MIN_ELEMENT_OFFSET..=MAX_ELEMENT_OFFSET).contains(&offset) {
            return Err(error(format!("{key} must be between -10000 and 10000")));
        }
    }
    Ok(())
}

fn validate_browser_screenshot_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    if args
        .get("full_page")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(error("full_page must be a boolean".to_string()));
    }
    let keys = ["x", "y", "width", "height"];
    let has_clip = keys.iter().any(|key| has_non_empty_arg(args, key));
    if args
        .get("full_page")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && has_clip
    {
        return Err(error(
            "full_page cannot be combined with x/y/width/height".to_string(),
        ));
    }
    if !has_clip {
        return Ok(());
    }
    if keys.iter().any(|key| !has_non_empty_arg(args, key)) {
        return Err(error(
            "x, y, width, and height must be provided together".to_string(),
        ));
    }
    for key in ["x", "y"] {
        let value = args
            .get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| error(format!("{key} must be a number")))?;
        if !(0.0..=10_000.0).contains(&value) {
            return Err(error(format!("{key} must be between 0 and 10000")));
        }
    }
    validate_integer_arg(args, "width", 0, 1, 100_000)?;
    validate_integer_arg(args, "height", 0, 1, 100_000)?;
    Ok(())
}

fn validate_browser_switch_page_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let has_index = has_non_empty_arg(args, "index");
    let target_text = |key: &str| -> Result<Option<String>, ToolOutcome> {
        let Some(value) = args.get(key) else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        let Some(text) = value.as_str() else {
            return Err(error(format!("{key} is invalid")));
        };
        if text.chars().count() > 500
            || text.contains('\0')
            || text.contains('\n')
            || text.contains('\r')
        {
            return Err(error(format!("{key} is invalid")));
        }
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        Ok(Some(trimmed.to_owned()))
    };
    let title_contains = target_text("title_contains")?;
    let url_contains = target_text("url_contains")?;
    let targets = [has_index, title_contains.is_some(), url_contains.is_some()]
        .into_iter()
        .filter(|value| *value)
        .count();
    if targets != 1 {
        return Err(error(
            "provide exactly one page target: index, title_contains, or url_contains".to_string(),
        ));
    }
    if has_index {
        validate_integer_arg(args, "index", 1, 1, 100)?;
    }
    validate_integer_arg(args, "max_text_chars", 8000, 500, 40_000)?;
    validate_boolean_args(args, &["screenshot", "full_page"])?;
    Ok(())
}

fn validate_browser_select_option_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let selector = optional_nonempty_string(args, "selector");
    let label = optional_nonempty_string(args, "label");
    let targets = [selector.is_some(), label.is_some()]
        .into_iter()
        .filter(|value| *value)
        .count();
    if targets != 1 {
        return Err(error(
            "provide exactly one target: selector or label".to_string(),
        ));
    }
    for (key, value, limit) in [
        ("selector", selector.as_deref().unwrap_or(""), 500_usize),
        ("label", label.as_deref().unwrap_or(""), 200_usize),
    ] {
        if value.chars().count() > limit || value.contains('\n') || value.contains('\r') {
            return Err(error(format!("{key} is invalid")));
        }
    }
    let value = optional_nonempty_string(args, "value");
    let option_label = optional_nonempty_string(args, "option_label");
    let has_index = has_non_empty_arg(args, "index");
    let options = [value.is_some(), option_label.is_some(), has_index]
        .into_iter()
        .filter(|item| *item)
        .count();
    if options != 1 {
        return Err(error(
            "provide exactly one option: value, option_label, or index".to_string(),
        ));
    }
    for (key, text) in [
        ("value", value.as_deref().unwrap_or("")),
        ("option_label", option_label.as_deref().unwrap_or("")),
    ] {
        if text.chars().count() > 500 || text.contains('\n') || text.contains('\r') {
            return Err(error(format!("{key} is invalid")));
        }
    }
    if has_index {
        validate_integer_arg(args, "index", 0, 0, 10_000)?;
    }
    validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
    validate_boolean_arg(args, "screenshot")?;
    Ok(())
}

fn validate_browser_set_checked_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let selector = optional_nonempty_string(args, "selector");
    let label = optional_nonempty_string(args, "label");
    let targets = [selector.is_some(), label.is_some()]
        .into_iter()
        .filter(|value| *value)
        .count();
    if targets != 1 {
        return Err(error(
            "provide exactly one target: selector or label".to_string(),
        ));
    }
    for (key, value, limit) in [
        ("selector", selector.as_deref().unwrap_or(""), 500_usize),
        ("label", label.as_deref().unwrap_or(""), 200_usize),
    ] {
        if value.chars().count() > limit || value.contains('\n') || value.contains('\r') {
            return Err(error(format!("{key} is invalid")));
        }
    }
    if args.get("checked").is_some_and(|value| !value.is_boolean()) {
        return Err(error("checked must be a boolean".to_string()));
    }
    validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
    validate_boolean_arg(args, "screenshot")?;
    Ok(())
}

fn browser_upload_raw_paths(args: &Map<String, Value>) -> Result<Vec<String>, ToolOutcome> {
    if let Some(path) = optional_nonempty_string(args, "path") {
        return Ok(vec![path]);
    }
    let Some(raw_paths) = args.get("paths") else {
        return Ok(Vec::new());
    };
    let Some(items) = raw_paths.as_array() else {
        return Err(error("paths must be a list of strings".to_string()));
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| error("paths must be a list of strings".to_string()))
        })
        .collect()
}

fn validate_browser_upload_file_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let selector = optional_nonempty_string(args, "selector");
    let label = optional_nonempty_string(args, "label");
    let targets = [selector.is_some(), label.is_some()]
        .into_iter()
        .filter(|value| *value)
        .count();
    if targets != 1 {
        return Err(error(
            "provide exactly one target: selector or label".to_string(),
        ));
    }
    for (key, value, limit) in [
        ("selector", selector.as_deref().unwrap_or(""), 500_usize),
        ("label", label.as_deref().unwrap_or(""), 200_usize),
    ] {
        if value.chars().count() > limit || value.contains('\n') || value.contains('\r') {
            return Err(error(format!("{key} is invalid")));
        }
    }
    let has_path = optional_nonempty_string(args, "path").is_some();
    let has_paths = args.get("paths").is_some();
    if has_path == has_paths {
        return Err(error(
            "provide exactly one upload source: path or paths".to_string(),
        ));
    }
    let paths = browser_upload_raw_paths(args)?;
    if paths.is_empty() || paths.len() > 20 {
        return Err(error("paths must contain 1 to 20 files".to_string()));
    }
    for path in paths {
        if path.trim().is_empty()
            || path.chars().count() > 4096
            || path.contains('\0')
            || path.contains('\n')
            || path.contains('\r')
        {
            return Err(error("path is invalid".to_string()));
        }
    }
    validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
    validate_boolean_arg(args, "screenshot")?;
    Ok(())
}

fn validate_browser_dialog_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    if let Some(action) = optional_nonempty_string(args, "dialog_action") {
        if !matches!(action.as_str(), "accept" | "dismiss") {
            return Err(error("dialog_action must be accept or dismiss".to_string()));
        }
    }
    if let Some(text) = args.get("dialog_prompt_text") {
        let Some(prompt_text) = text.as_str() else {
            return Err(error("dialog_prompt_text must be a string".to_string()));
        };
        if prompt_text.chars().count() > 2000 || prompt_text.contains('\0') {
            return Err(error("dialog_prompt_text is invalid".to_string()));
        }
        if !prompt_text.is_empty()
            && optional_nonempty_string(args, "dialog_action").as_deref() != Some("accept")
        {
            return Err(error(
                "dialog_prompt_text requires dialog_action=accept".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_browser_download_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    if args
        .get("expect_download")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(error("expect_download must be a boolean".to_string()));
    }
    if args
        .get("expect_download")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        validate_integer_arg(args, "download_timeout_ms", 15_000, 1_000, 120_000)?;
    }
    Ok(())
}

fn validate_browser_popup_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    if args
        .get("expect_popup")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(error("expect_popup must be a boolean".to_string()));
    }
    if args
        .get("expect_popup")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && args
            .get("expect_download")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Err(error(
            "expect_popup cannot be combined with expect_download".to_string(),
        ));
    }
    if args
        .get("expect_popup")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        validate_integer_arg(args, "popup_timeout_ms", 15_000, 1_000, 120_000)?;
    }
    Ok(())
}

fn validate_required_coordinate_args(
    args: &Map<String, Value>,
    keys: &[&str],
) -> Result<(), ToolOutcome> {
    for key in keys {
        let value = args
            .get(*key)
            .and_then(Value::as_f64)
            .ok_or_else(|| error(format!("{key} is required")))?;
        if !(MIN_SCREEN_COORDINATE..=MAX_SCREEN_COORDINATE).contains(&value) {
            return Err(error(format!("{key} must be between -100000 and 100000")));
        }
    }
    Ok(())
}

fn has_non_empty_arg(args: &Map<String, Value>, key: &str) -> bool {
    args.get(key)
        .is_some_and(|value| !value.is_null() && value.as_str() != Some(""))
}

fn validate_screenshot_crop_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let keys = ["x", "y", "width", "height"];
    let has_any = keys.iter().any(|key| has_non_empty_arg(args, key));
    let has_display_index = has_non_empty_arg(args, "display_index");
    let has_display_id = has_non_empty_arg(args, "display_id");
    if has_display_index && has_display_id {
        return Err(error(
            "provide display_index or display_id, not both".to_string(),
        ));
    }
    if has_any && (has_display_index || has_display_id) {
        return Err(error(
            "display_index/display_id cannot be combined with x/y/width/height".to_string(),
        ));
    }
    if has_display_index {
        validate_integer_arg(args, "display_index", 1, 1, 32)?;
    }
    if has_display_id {
        validate_integer_arg(args, "display_id", 1, 1, 2_147_483_647)?;
    }
    if !has_any {
        return Ok(());
    }
    if keys.iter().any(|key| !has_non_empty_arg(args, key)) {
        return Err(error(
            "x, y, width, and height must be provided together".to_string(),
        ));
    }
    validate_required_coordinate_args(args, &["x", "y"])?;
    validate_integer_arg(args, "width", 0, 1, 100_000)?;
    validate_integer_arg(args, "height", 0, 1, 100_000)?;
    Ok(())
}

fn is_allowed_browser_key(key: &str) -> bool {
    let key = key.trim();
    if key.is_empty() || key.len() > 40 || key.contains('\n') || key.contains('\r') {
        return false;
    }
    let allowed_keys = [
        "Enter",
        "Tab",
        "Escape",
        "Backspace",
        "Delete",
        "Space",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
    ];
    let allowed_modifiers = ["Control", "Meta", "Command", "Shift", "Alt", "Option"];
    let parts = key.split('+').collect::<Vec<_>>();
    let final_key = parts.last().copied().unwrap_or("");
    let final_ok = allowed_keys.contains(&final_key)
        || (final_key.chars().count() == 1
            && final_key.chars().all(|ch| ch.is_ascii_alphanumeric()));
    if parts.len() == 1 {
        return final_ok;
    }
    parts[..parts.len() - 1]
        .iter()
        .all(|part| allowed_modifiers.contains(part))
        && final_ok
}

fn is_allowed_computer_key(key: &str) -> bool {
    let key = key.trim();
    if key.is_empty() || key.len() > 40 || key.contains('\n') || key.contains('\r') {
        return false;
    }
    let allowed_keys = [
        "Enter",
        "Tab",
        "Escape",
        "Backspace",
        "Delete",
        "Space",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "F11",
        "F12",
        "F13",
        "F14",
        "F15",
        "F16",
        "F17",
        "F18",
        "F19",
        "F20",
    ];
    let allowed_modifiers = ["Control", "Meta", "Command", "Shift", "Alt", "Option"];
    let parts = key.split('+').collect::<Vec<_>>();
    let final_key = parts.last().copied().unwrap_or("");
    let final_ok = allowed_keys.contains(&final_key)
        || (final_key.chars().count() == 1
            && final_key.chars().all(|ch| ch.is_ascii_alphanumeric()));
    if parts.len() == 1 {
        return final_ok;
    }
    parts[..parts.len() - 1]
        .iter()
        .all(|part| allowed_modifiers.contains(part))
        && final_ok
}

fn is_allowed_computer_key_state_key(key: &str) -> bool {
    let key = key.trim();
    if key.is_empty() || key.len() > 40 || key.contains('\n') || key.contains('\r') {
        return false;
    }
    if key.contains('+') {
        return false;
    }
    let allowed_keys = [
        "Enter",
        "Tab",
        "Escape",
        "Backspace",
        "Delete",
        "Space",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "F11",
        "F12",
        "F13",
        "F14",
        "F15",
        "F16",
        "F17",
        "F18",
        "F19",
        "F20",
        "Control",
        "Meta",
        "Command",
        "Shift",
        "Alt",
        "Option",
    ];
    allowed_keys.contains(&key)
        || (key.chars().count() == 1 && key.chars().all(|ch| ch.is_ascii_alphanumeric()))
}

fn validate_computer_hotkey_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let key = required_string(args, "key")?;
    let modifier_keys = ["Control", "Meta", "Command", "Shift", "Alt", "Option"];
    if modifier_keys.contains(&key.as_str()) || !is_allowed_computer_key_state_key(&key) {
        return Err(error("key is not supported".to_string()));
    }
    let modifiers = args
        .get("modifiers")
        .and_then(Value::as_array)
        .ok_or_else(|| error("modifiers must be an array".to_string()))?;
    if modifiers.is_empty() || modifiers.len() > 4 {
        return Err(error("modifiers must contain 1 to 4 items".to_string()));
    }
    for modifier in modifiers {
        let Some(modifier) = modifier.as_str() else {
            return Err(error("modifier is not supported".to_string()));
        };
        if !modifier_keys.contains(&modifier) {
            return Err(error("modifier is not supported".to_string()));
        }
    }
    validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
    if args
        .get("screenshot")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(error("screenshot must be a boolean".to_string()));
    }
    Ok(())
}

fn python_candidates() -> Vec<String> {
    if let Ok(value) = std::env::var("NOLA_CLI_PYTHON") {
        let value = value.trim();
        if !value.is_empty() {
            return vec![value.to_string()];
        }
    }
    vec!["python3".to_string(), "python".to_string()]
}

fn preferred_python_command() -> String {
    let candidates = python_candidates();
    candidates
        .iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
        .cloned()
        .or_else(|| candidates.into_iter().next())
        .unwrap_or_else(|| "python3".to_string())
}

const COMPUTER_HELPER: &str = include_str!("../computer_helper.py");

fn required_string(args: &Map<String, Value>, key: &str) -> Result<String, ToolOutcome> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| error(format!("{key} is required")))
}

fn resolve_discovery_root(raw: &str, ctx: &ToolContext) -> Result<PathBuf, ToolOutcome> {
    let normalized = raw.trim();
    if normalized == "." || normalized == "/" {
        let workspace = ctx
            .allowed_dirs
            .first()
            .ok_or_else(|| error("no allowed directories configured".to_string()))?;
        return resolve_allowed(
            workspace.to_string_lossy().as_ref(),
            &ctx.allowed_dirs,
            true,
        );
    }
    resolve_allowed(normalized, &ctx.allowed_dirs, true)
}

/// Resolve a directory entry without following its final symlink. Parent
/// components still resolve normally and remain subject to resource grants.
fn directory_entry_path(path: &Path) -> Option<PathBuf> {
    let Some(name) = path.file_name() else { return path.canonicalize().ok() };
    let parent = resolve_missing_path(path.parent()?)?;
    let mut normalized = PathBuf::new();
    for component in parent.components() {
        match component {
            std::path::Component::ParentDir => { normalized.pop(); }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Some(normalized.join(name))
}

fn resolve_allowed_entry(raw: &str, allowed_dirs: &[PathBuf]) -> Result<PathBuf, ToolOutcome> {
    let allowed = effective_allowed_dirs(allowed_dirs);
    let base = allowed.first().ok_or_else(|| error("no allowed directories configured".into()))?;
    let input = expand_home(Path::new(raw));
    let absolute = if input.is_absolute() { input } else { base.join(input) };
    let resolved = directory_entry_path(&absolute)
        .ok_or_else(|| error(format!("failed to resolve directory entry: {raw}")))?;
    Ok(resolved)
}

fn resolve_allowed(
    raw: &str,
    allowed_dirs: &[PathBuf],
    must_exist: bool,
) -> Result<PathBuf, ToolOutcome> {
    let effective_allowed_dirs = effective_allowed_dirs(allowed_dirs);
    if effective_allowed_dirs.is_empty() {
        return Err(error("no allowed directories configured".to_string()));
    }
    let input = expand_home(Path::new(raw));
    let absolute = if input.is_absolute() {
        input
    } else {
        effective_allowed_dirs[0].join(input)
    };
    let resolved = if absolute.exists() {
        absolute
            .canonicalize()
            .map_err(|err| error(format!("failed to resolve {raw}: {err}")))?
    } else {
        if must_exist {
            return Err(error(format!("path does not exist: {raw}")));
        }
        resolve_missing_path(&absolute)
            .ok_or_else(|| error(format!("failed to resolve parent for {raw}")))?
    };

    Ok(resolved)
}

fn resolve_configured_allowed(
    raw: &str,
    allowed_dirs: &[PathBuf],
    must_exist: bool,
) -> Result<PathBuf, ToolOutcome> {
    if allowed_dirs.is_empty() {
        return Err(error("no allowed directories configured".to_string()));
    }
    let input = expand_home(Path::new(raw));
    let absolute = if input.is_absolute() {
        input
    } else {
        allowed_dirs[0].join(input)
    };
    let resolved = if absolute.exists() {
        absolute
            .canonicalize()
            .map_err(|err| error(format!("failed to resolve {raw}: {err}")))?
    } else {
        if must_exist {
            return Err(error(format!("path does not exist: {raw}")));
        }
        resolve_missing_path(&absolute)
            .ok_or_else(|| error(format!("failed to resolve parent for {raw}")))?
    };
    let configured = allowed_dirs
        .iter()
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .collect::<Vec<_>>();
    if configured.iter().any(|base| resolved.starts_with(base)) {
        Ok(resolved)
    } else {
        Err(error(format!(
            "path is outside configured capability directories: {}",
            resolved.display()
        )))
    }
}

fn effective_allowed_dirs(allowed_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = allowed_dirs
        .iter()
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .collect::<Vec<_>>();
    with_permission_cache(|cache| {
        for path in cache.allowed_dirs.iter() {
            let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !out.iter().any(|existing| existing == &resolved) {
                out.push(resolved);
            }
        }
    });
    // The workspace supplies the default cwd, not a filesystem sandbox.
    // Resource sensitivity and operation approval are resolved separately by Core.
    if !out.is_empty() {
        if let Some(root) = out[0].ancestors().last().map(Path::to_path_buf) {
            if !out.contains(&root) { out.push(root); }
        }
    }
    out
}

fn unrestricted_resource_policy() -> bool {
    if let Some(profile) = CURRENT_EXECUTION_PERMISSION_PROFILE.with(|scope|
        scope.borrow().current().copied().flatten()) {
        return profile == nola_domain::PermissionProfile::AllowAll;
    }
    with_permission_cache(|cache| cache.unrestricted_resources).unwrap_or(false)
}

fn workspace_access(args: &Map<String, Value>, approved: bool) -> ToolOutcome {
    if !approved {
        return error("workspace_access requires explicit approval of this invocation".to_string());
    }
    let raw = tool_try!(required_string(args, "root"));
    let path = Path::new(&raw);
    if !path.is_absolute() {
        return error("workspace_access root must be an absolute directory".to_string());
    }
    let root = match path.canonicalize() {
        Ok(root) if root.is_dir() => root,
        _ => return error("workspace_access root is not an accessible existing directory".to_string()),
    };
    let grant = tool_try!(nola_host_protocol::workspace_access::WorkspaceAccessGrant::for_directory(&root).map_err(error));
    let data = serde_json::to_value(grant).expect("workspace access data serializes");
    // A lossy path is useful to people, but only the reversible resource URI
    // in the typed result owns the granted filesystem identity.
    ok(json!({
        "type": "workspace_access_granted",
        "root": root.to_string_lossy(),
        "default_working_directory_changed": false,
        "mutation_authorized": false
    }).to_string(), "Workspace access authorized".to_string())
        .with_data(data)
        .with_execution(nola_host_protocol::tool_result::ExecutionReceipt {
            state: nola_host_protocol::tool_result::ExecutionState::Completed,
            ..Default::default()
        })
}

fn current_session_permission_scope() -> Option<String> {
    with_permission_cache(|cache| cache.scope.clone()).flatten()
}

fn permission_dir_for_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    }
}

fn normalize_allowed_dir(raw: &str) -> Result<PathBuf, String> {
    let input = expand_home(Path::new(raw.trim()));
    let absolute = if input.is_absolute() {
        input
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(input)
    };
    absolute
        .canonicalize()
        .map_err(|err| format!("failed to resolve {}: {err}", absolute.display()))
        .and_then(|path| {
            if path.is_dir() {
                Ok(path)
            } else {
                Err(format!("not a directory: {}", path.display()))
            }
        })
}

fn add_session_allowed_dir(dir: PathBuf) {
    with_permission_cache_mut(|cache| {
        push_unique_path(&mut cache.allowed_dirs, dir);
    });
}

#[allow(dead_code)]
fn write_session_allowed_dir_config(dir: &Path) -> Result<(), String> {
    let config_path = with_permission_cache(|cache| cache.config_path.clone())
        .flatten()
        .ok_or_else(|| "session config path is unavailable".to_string())?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("create {} failed: {err}", parent.display()))?;
    }
    let mut root = match fs::read_to_string(&config_path) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(err) => return Err(format!("read {} failed: {err}", config_path.display())),
    };
    let mut items = root
        .remove("allowed_dirs")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    let dir_text = dir.display().to_string();
    if !items.iter().any(|item| item == &dir_text) {
        items.push(dir_text);
    }
    root.insert(
        "allowed_dirs".to_string(),
        Value::Array(items.into_iter().map(Value::String).collect()),
    );
    let text = serde_json::to_string_pretty(&Value::Object(root))
        .map_err(|err| format!("serialize config failed: {err}"))?;
    fs::write(&config_path, format!("{text}\n"))
        .map_err(|err| format!("write {} failed: {err}", config_path.display()))
}

fn write_session_allowed_dir_grant(dir: &Path) -> Result<(), String> {
    if current_session_permission_scope().is_none() {
        add_session_allowed_dir(dir.to_path_buf());
        return Ok(());
    }
    write_session_permission_grant("file", "allowed_dir", &dir.display().to_string(), "allow")
}

fn resolve_missing_path(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    let mut missing = Vec::new();
    while !current.exists() {
        // A dangling symlink is an existing entry, not a missing directory.
        if fs::symlink_metadata(current).is_ok() { return None; }
        missing.push(current.file_name()?.to_os_string());
        current = current.parent()?;
    }
    let mut resolved = current.canonicalize().ok()?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Some(resolved)
}

fn walk_files<F>(root: &Path, limit: usize, callback: &mut F)
where F: FnMut(&Path) -> bool {
    walk_files_controlled(root, limit, callback, &|| false);
}

fn walk_files_controlled<F>(root: &Path, limit: usize, callback: &mut F, stop: &dyn Fn() -> bool)
where F: FnMut(&Path) -> bool {
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        if stop() { return; }
        let Ok(read_dir) = fs::read_dir(&dir) else { continue; };
        for entry in read_dir.flatten() {
            if stop() { return; }
            let Ok(kind) = entry.file_type() else { continue; };
            // Never follow directory cycles or leave the admitted root via a link.
            if kind.is_symlink() { continue; }
            let path = entry.path();
            if kind.is_dir() {
                if !should_skip_dir(&entry.file_name().to_string_lossy()) { stack.push(path); }
            } else if kind.is_file() {
                seen += 1;
                if !callback(&path) || seen >= limit { return; }
            }
        }
    }
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".build"
            | "build"
            | "dist"
            | ".cache"
            | "DerivedData"
            | "node_modules"
            | "target"
            | ".gradle"
            | ".venv"
            | "venv"
            | "vendor"
            | "Pods"
            | "__pycache__"
    )
}

fn glob_regex(pattern: &str) -> Result<Regex, String> {
    let patterns = expand_glob_braces(pattern)?;
    let mut alternatives = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        alternatives.push(glob_regex_fragment(&pattern));
    }
    Regex::new(&format!("^(?:{})$", alternatives.join("|")))
        .map_err(|err| format!("invalid glob pattern: {err}"))
}

fn glob_regex_fragment(pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        out.push_str("(?:.*/)?");
                    } else {
                        out.push_str(".*");
                    }
                } else {
                    out.push_str("[^/]*");
                }
            }
            '?' => out.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            other => out.push(other),
        }
    }
    out
}

fn expand_glob_braces(pattern: &str) -> Result<Vec<String>, String> {
    let Some((open, _)) = pattern
        .char_indices()
        .find(|(_, character)| *character == '{')
    else {
        return Ok(vec![pattern.to_string()]);
    };
    let suffix = &pattern[open + 1..];
    let Some(relative_close) = suffix.find('}') else {
        return Ok(vec![pattern.to_string()]);
    };
    let close = open + 1 + relative_close;
    let options = pattern[open + 1..close].split(',').collect::<Vec<_>>();
    if options.len() < 2 {
        return Ok(vec![pattern.to_string()]);
    }
    let mut expanded = Vec::new();
    for option in options {
        let candidate = format!("{}{}{}", &pattern[..open], option, &pattern[close + 1..]);
        expanded.extend(expand_glob_braces(&candidate)?);
        if expanded.len() > 64 {
            return Err("glob brace expansion exceeds 64 patterns".to_string());
        }
    }
    Ok(expanded)
}

#[cfg(test)]
mod glob_regex_tests {
    use super::glob_regex;

    #[test]
    fn recursive_glob_includes_files_at_the_root() {
        let all = glob_regex("**/*").expect("glob");
        assert!(all.is_match("note.txt"));
        assert!(all.is_match("nested/note.txt"));
        let rust = glob_regex("**/*.rs").expect("glob");
        assert!(rust.is_match("main.rs"));
        assert!(rust.is_match("src/main.rs"));
    }

    #[test]
    fn brace_expansion_supports_common_model_generated_patterns() {
        let files = glob_regex("**/{architecture,arch}.{md,txt}").expect("glob");
        assert!(files.is_match("architecture.md"));
        assert!(files.is_match("docs/arch.txt"));
        assert!(!files.is_match("sample.swift"));
    }
}

fn path_slash(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Debug, PartialEq)]
enum ToolPermissionDecision {
    TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason),
    Allow,
    AllowConversation,
    Custom(String),
    ReturnToComposer,
    Deny,
}

/// Presents a permission request that Core has already evaluated. This path
/// deliberately performs no command-risk classification, rule matching, or
/// Turn-wide denial caching; the Host only gathers the user's decision.
pub fn present_agent_core_permission_with_resolver(
    request: &ToolRequest,
    resolver: Option<HostPermissionResolver>,
    instructions: &[nola_core::agent::host::PermissionInstruction],
    allow_conversation: bool,
) -> HostPermissionResolution {
    if let Some(resolver) = resolver {
        resolver(request, instructions, allow_conversation)
    } else if native_permission_mode_enabled() {
        match confirm_native_tool_with_instructions(request, instructions, allow_conversation) {
            ToolPermissionDecision::Allow => HostPermissionResolution::Allow,
            ToolPermissionDecision::AllowConversation => {
                HostPermissionResolution::AllowConversation
            }
            ToolPermissionDecision::Custom(note) => HostPermissionResolution::Custom(note),
            ToolPermissionDecision::ReturnToComposer => {
                HostPermissionResolution::ReturnToComposer
            }
            ToolPermissionDecision::TransportInterrupted(reason) => HostPermissionResolution::TransportInterrupted(reason),
            ToolPermissionDecision::Deny => HostPermissionResolution::Deny,
        }
    } else if !permission_interaction_available() {
        HostPermissionResolution::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable)
    } else {
        let mut prompt = agent_core_tool_permission_prompt(request);
        prompt.allow_conversation = allow_conversation;
        if !instructions.is_empty() {
            prompt.rows.push(("Status".into(), "Guidance saved; original arguments remain pending approval".into()));
            for entry in instructions {
                prompt.rows.push(("Guidance".into(), entry.instruction.clone()));
            }
        }
        loop {
            match prompt_permission(&prompt) {
                PermissionChoice::Once => return HostPermissionResolution::Allow,
                PermissionChoice::Session(_) => return HostPermissionResolution::AllowConversation,
                PermissionChoice::Custom(note) => {
                    match nola_core::agent::host::PermissionInstruction::validate_append(instructions, &request.id, &note) {
                        Ok(()) => return HostPermissionResolution::Custom(note),
                        Err(error) => {
                            prompt.rows.retain(|(label, _)| label != "Guidance limit");
                            prompt.rows.push(("Guidance limit".into(), error));
                        }
                    }
                }
                PermissionChoice::ReturnToComposer => return HostPermissionResolution::ReturnToComposer,
                PermissionChoice::TransportInterrupted(reason) => return HostPermissionResolution::TransportInterrupted(reason),
                PermissionChoice::Deny => return HostPermissionResolution::Deny,
            }
        }
    }
}

fn agent_core_tool_permission_prompt(request: &ToolRequest) -> PermissionPrompt {
    // Core owns reusable scope. Do not advertise the Host's independently
    // derived parent/project directory as the Core conversation grant.
    let mut prompt = tool_permission_prompt(request);
    if nola_core::agent::policy::permission_name_for_tool(&request.name) != "bash" {
        let resource = nola_core::agent::permissions::permission_resource_for_input(
            &request.name, None, &Value::Object(request.args.clone()));
        prompt.session_label = format!("{} {}: {}",
            crate::i18n::permission_text().allow_conversation_prefix,
            crate::i18n::localize_tool_display_name(&request.name), resource);
    }
    prompt.supports_instruction = true;
    prompt
}

#[cfg(test)]
mod agent_core_permission_bridge_tests {
    use super::*;

    #[test]
    fn bash_session_option_discloses_exact_pipeline_and_directory_without_widening_grant() {
        let request = ToolRequest {
            id: "pipeline-scope".into(), name: "bash".into(),
            args: Map::from_iter([
                ("command".into(), Value::String("git status --short | head -20".into())),
                ("cwd".into(), Value::String("/tmp/project".into())),
            ]),
        };
        let prompt = agent_core_tool_permission_prompt(&request);
        assert!(prompt.session_label.contains("git status --short | head -20"));
        assert!(prompt.session_label.contains("/tmp/project"));
        assert!(session_permission_rule_matches(&prompt.session_value, &request));
        let mut different = request.clone();
        different.args.insert("command".into(), Value::String("git status --short | head -50".into()));
        assert!(!session_permission_rule_matches(&prompt.session_value, &different));
        different = request;
        different.args.insert("cwd".into(), Value::String("/tmp/elsewhere".into()));
        assert!(!session_permission_rule_matches(&prompt.session_value, &different));
    }

    #[test]
    fn agent_core_prompt_displays_the_persisted_session_scope() {
        let request = ToolRequest {
            id: "call-1".to_string(),
            name: "write_file".to_string(),
            args: Map::from_iter([
                (
                    "path".to_string(),
                    Value::String("/tmp/nola-exact-resource.txt".to_string()),
                ),
                ("content".to_string(), Value::String("ok".to_string())),
            ]),
        };

        let prompt = agent_core_tool_permission_prompt(&request);
        assert!(prompt.session_label.contains("/tmp/nola-exact-resource.txt"));
        assert!(!prompt.session_label.contains("edit: /tmp"));
        assert_eq!(prompt.session_value, session_permission_rule(&request));
    }

    #[test]
    fn bash_permission_prompt_has_a_stable_title_and_one_command_detail() {
        let request = ToolRequest {
            id: "call-bash".to_string(),
            name: "bash".to_string(),
            args: Map::from_iter([(
                "command".to_string(),
                Value::String("cd /tmp/project && git status --short".to_string()),
            )]),
        };

        let prompt = agent_core_tool_permission_prompt(&request);
        assert!(!prompt.title.contains("`bash`"), "{}", prompt.title);
        assert!(!prompt.title.contains("git status --short"), "{}", prompt.title);
        assert_eq!(prompt.rows.len(), 1);
        assert!(prompt.rows[0].1.contains("git status --short"));
        assert_eq!(prompt.rows[0].1, "cd /tmp/project && git status --short");
    }

    #[test]
    fn approval_preserves_long_command_suffix_redirects_and_full_directory() {
        let command = format!("cd /workspace && printf '{}' 2>&1 && rm important.txt", "界".repeat(200));
        let directory = "/workspace/nested/project-with-a-long-name";
        let request = ToolRequest {
            id: "exact-approval".into(), name: "bash".into(),
            args: Map::from_iter([
                ("command".into(), Value::String(command.clone())),
                ("cwd".into(), Value::String(directory.into())),
            ]),
        };
        let prompt = agent_core_tool_permission_prompt(&request);
        assert_eq!(prompt.rows[0].1, command);
        assert!(prompt.rows.iter().any(|(_, value)| value == directory));
    }

    #[test]
    fn workspace_access_requires_approval_and_an_existing_absolute_directory() {
        let args = json!({"root": std::env::temp_dir()}).as_object().unwrap().clone();
        assert!(workspace_access(&args, false).error);
        let outcome = workspace_access(&args, true);
        assert!(!outcome.error, "{}", outcome.text);
        let value: Value = serde_json::from_str(&outcome.text).unwrap();
        assert_eq!(value["mutation_authorized"], false);
        assert_eq!(value["default_working_directory_changed"], false);
        let canonical_root = std::env::temp_dir().canonicalize().unwrap();
        let grant: nola_host_protocol::workspace_access::WorkspaceAccessGrant =
            serde_json::from_value(outcome.data.clone().unwrap()).unwrap();
        assert_eq!(grant.directory_for_approved_root(&canonical_root).unwrap(), canonical_root);
        assert!(outcome.data.as_ref().unwrap().get("root").is_none());
        assert_eq!(outcome.execution.as_ref().unwrap().state,
            nola_host_protocol::tool_result::ExecutionState::Completed);
        for root in ["relative", "~/Desktop", "/nola/nonexistent/access-root"] {
            assert!(workspace_access(json!({"root": root}).as_object().unwrap(), true).error);
        }
    }
}

fn persist_direct_session_decision(
    request: &ToolRequest,
    decision: ToolPermissionDecision,
) -> ToolPermissionDecision {
    if matches!(decision, ToolPermissionDecision::AllowConversation) {
        remember_session_approval(request);
        ToolPermissionDecision::Allow
    } else {
        decision
    }
}

fn confirm_tool(request: &ToolRequest) -> ToolPermissionDecision {
    if let Some(resolver) = current_host_permission_resolver() {
        return match resolver(request, &[], false) {
            HostPermissionResolution::Allow => ToolPermissionDecision::Allow,
            HostPermissionResolution::AllowConversation => {
                remember_session_approval(request);
                ToolPermissionDecision::Allow
            }
            HostPermissionResolution::Custom(note) => ToolPermissionDecision::Custom(note),
            HostPermissionResolution::ReturnToComposer => {
                ToolPermissionDecision::ReturnToComposer
            }
            HostPermissionResolution::StopRequested => ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Disconnected),
            HostPermissionResolution::TransportInterrupted(reason) => ToolPermissionDecision::TransportInterrupted(reason),
            HostPermissionResolution::Deny => ToolPermissionDecision::Deny,
        };
    }
    if native_permission_mode_enabled() {
        return persist_direct_session_decision(request, confirm_native_tool(request));
    }
    if !permission_interaction_available() {
        return ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
    }
    match prompt_permission(&tool_permission_prompt(request)) {
        PermissionChoice::Once => ToolPermissionDecision::Allow,
        PermissionChoice::Session(rule) => {
            add_session_tool_allow_rule(rule.clone());
            if let Err(err) = write_session_tool_allow_grant(&rule) {
                eprintln!("failed to write session permission grant: {err}");
            }
            ToolPermissionDecision::Allow
        }
        PermissionChoice::Custom(note) => ToolPermissionDecision::Custom(note),
        PermissionChoice::ReturnToComposer => ToolPermissionDecision::ReturnToComposer,
        PermissionChoice::TransportInterrupted(reason) => ToolPermissionDecision::TransportInterrupted(reason),
        PermissionChoice::Deny => ToolPermissionDecision::Deny,
    }
}

fn permission_return_to_composer_outcome() -> ToolOutcome {
    if let Some(cancellation) = crate::runtime_control::current_execution_control() {
        cancellation.request();
    }
    error_with_display(
        "permission_interrupted: returned to composer".to_string(),
        crate::i18n::interaction_text()
            .custom_permission_return
            .to_string(),
    )
}

fn permission_custom_outcome(note: String) -> ToolOutcome {
    PENDING_CUSTOM_PERMISSION.with(|pending| {
        *pending.borrow_mut() = Some(note.clone());
    });
    error_with_display(
        format!("permission_custom: user provided custom instruction: {note}"),
        "User provided a custom permission response.".to_string(),
    )
}

fn suspend_for_custom_permission(note: String, ctx: &ToolContext) -> ToolOutcome {
    if !note.trim().is_empty() {
        if let Err(error) = ctx.cancellation.handoff_user_instruction(note.clone()) {
            return error_with_display(format!("permission instruction handoff failed: {error}"), note);
        }
    }
    ctx.cancellation.request();
    error_with_display(
        if note.trim().is_empty() {
            "permission_custom: return to the main composer".to_string()
        } else {
            format!("permission_custom: user provided custom instruction: {note}")
        },
        crate::i18n::interaction_text()
            .custom_permission_return
            .to_string(),
    )
}

fn suspend_for_return_to_composer(ctx: &ToolContext) -> ToolOutcome {
    ctx.cancellation.request();
    error_with_display(
        "permission_interrupted: returned to composer".to_string(),
        crate::i18n::interaction_text()
            .custom_permission_return
            .to_string(),
    )
}

fn native_permission_mode_enabled() -> bool {
    std::env::var("NOLA_NATIVE_PERMISSION_MODE")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

/// Native Emitter frames and authenticated Host challenges share an ordered
/// stdio writer. Resolver-backed surfaces (Web, tests, recovery) deliver the
/// challenge synchronously through their own channel and must not reserve a
/// native output slot they cannot fill.
pub(crate) fn permission_trace_uses_direct_native_transport() -> bool {
    native_permission_mode_enabled() && current_host_permission_resolver().is_none()
}

// Bound a single control frame before JSON decoding. EOF and I/O failure
// describe the transport, never a user permission decision.
fn read_native_permission_response(
    reader: impl io::BufRead,
) -> Result<Value, nola_host_protocol::PermissionTransportInterruptionReason> {
    use nola_host_protocol::PermissionTransportInterruptionReason::{Disconnected, Unavailable};
    const MAXIMUM_FRAME_BYTES: usize = 32 * 1024;
    let mut bounded = io::Read::take(reader, (MAXIMUM_FRAME_BYTES + 1) as u64);
    let mut line = String::new();
    let count = io::BufRead::read_line(&mut bounded, &mut line).map_err(|_| Disconnected)?;
    if count == 0 {
        return Err(Disconnected);
    }
    if count > MAXIMUM_FRAME_BYTES || line.trim().is_empty() {
        return Err(Unavailable);
    }
    serde_json::from_str(&line).map_err(|_| Unavailable)
}

fn permission_transport_interrupted_outcome(
    reason: nola_host_protocol::PermissionTransportInterruptionReason,
) -> ToolOutcome {
    error(format!("Permission transport interrupted ({reason:?}); no user decision was received"))
}

fn confirm_native_tool(request: &ToolRequest) -> ToolPermissionDecision {
    confirm_native_tool_with_instructions(request, &[], false)
}

fn confirm_native_tool_with_instructions(
    request: &ToolRequest,
    instructions: &[nola_core::agent::host::PermissionInstruction],
    allow_conversation: bool,
) -> ToolPermissionDecision {
    let Some(session_id) = scoped_host_permission_session_id()
    else {
        return ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let expires_at = now.as_secs().saturating_add(300);
    let args = Value::Object(request.args.clone());
    let nonce = format!(
        "native:{}:{}:{}:{}",
        session_id,
        request.id,
        std::process::id(),
        now.as_nanos()
    );
    let challenge = nola_core::host_auth::HostPermissionChallenge {
        allow_conversation,
        protocol_version: nola_core::host_auth::HOST_PERMISSION_PROTOCOL_VERSION,
        nonce: nonce.clone(),
        request_id: request.id.clone(),
        host_id: "local".to_string(),
        client_id: "mac".to_string(),
        session_id: session_id.clone(),
        tool_name: request.name.clone(),
        expires_at,
        permission_instructions: instructions.to_vec(),
    };
    let policy_digest = std::env::var("NOLA_CORE_TOOL_POLICY_DIGEST").ok();
    let manifest_digest = std::env::var("NOLA_CORE_TOOL_MANIFEST_DIGEST").ok();
    let localized = crate::i18n::permission_text();
    let localized_tool_name = crate::i18n::localize_tool_display_name(&request.name);
    let approval_reason = format!(
        "{} `{}` {}",
        tr(Message::ToolRequiresApprovalPrefix),
        localized_tool_name,
        tr(Message::ToolRequiresApprovalSuffix)
    );
    let Some(trace) = take_pending_host_permission_trace(&session_id, request, &args, &approval_reason) else {
        return ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
    };
    let Some(call_id) = trace.pointer("/payload/call_id").and_then(Value::as_str)
        .filter(|id| !id.is_empty()).map(str::to_string) else {
        return ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
    };
    let event = json!({
        "type": "permission_request",
        "allow_conversation": allow_conversation,
        "id": request.id,
        "request_id": request.id,
        "tool_call_id": call_id,
        "title": localized_tool_name,
        "tool_name": request.name,
        "args": args,
        "reason": approval_reason,
        "protocol_version": challenge.protocol_version,
        "nonce": nonce,
        "host_id": challenge.host_id,
        "client_id": challenge.client_id,
        "session_id": challenge.session_id,
        "expires_at": expires_at,
        "permission_instructions": instructions,
        "policy_digest": policy_digest,
        "manifest_digest": manifest_digest,
        "mcp_descriptor": crate::host_tool_permission_evidence::current_mcp_descriptor(),
        "message": localized.keyboard_hint,
        "trace": trace
    });
    if !emit_native_host_event(event) {
        return ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Disconnected);
    }

    let raw_decision = if crate::desktop_host_control::is_active() {
        match crate::desktop_host_control::wait_for_permission_decision(
            &request.id,
            std::time::Duration::from_secs(300),
        ) {
            Ok(decision) => decision,
            Err(reason) => return ToolPermissionDecision::TransportInterrupted(reason),
        }
    } else {
        match read_native_permission_response(io::stdin().lock()) {
            Ok(decision) => decision,
            Err(reason) => return ToolPermissionDecision::TransportInterrupted(reason),
        }
    };
    let Ok(decision) =
        serde_json::from_value::<nola_core::host_auth::HostPermissionResponse>(raw_decision)
    else {
        return ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
    };
    let resolution = decision.resolution.clone();
    if std::env::var("NOLA_PERMISSION_DEBUG").ok().as_deref() == Some("1") {
        eprintln!(
            "[permission-native-decision] tool={} resolution={:?}",
            request.name, resolution
        );
    }
    let validation_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if challenge.expires_at <= validation_now {
        return ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Timeout);
    }
    if nola_core::host_auth::validate_host_permission_decision(
        &challenge,
        &decision,
        validation_now,
    )
    .is_err()
    {
        return ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
    }
    match resolution {
        nola_core::host_auth::HostPermissionResolution::Authorization {
            decision: nola_core::host_auth::HostPermissionAuthorizationDecision::AllowOnce,
            ..
        } => ToolPermissionDecision::Allow,
        nola_core::host_auth::HostPermissionResolution::Authorization {
            decision:
                nola_core::host_auth::HostPermissionAuthorizationDecision::AllowConversation,
            ..
        } => {
            ToolPermissionDecision::AllowConversation
        }
        nola_core::host_auth::HostPermissionResolution::Authorization {
            decision: nola_core::host_auth::HostPermissionAuthorizationDecision::Deny,
            ..
        } => ToolPermissionDecision::Deny,
        nola_core::host_auth::HostPermissionResolution::Intervention {
            intervention:
                nola_core::host_auth::HostPermissionIntervention::ProvideInstruction {
                    instruction,
                },
        } => ToolPermissionDecision::Custom(instruction),
        nola_core::host_auth::HostPermissionResolution::Intervention {
            intervention: nola_core::host_auth::HostPermissionIntervention::ReturnToComposer,
        } => {
            ToolPermissionDecision::ReturnToComposer
        }
    }
}

fn emit_native_host_event(event: Value) -> bool {
    match crate::native_host_output::emit(event) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("native interaction delivery failed: {error}");
            false
        }
    }
}

pub(crate) fn remember_session_approval(request: &ToolRequest) {
    let rule = session_permission_rule(request);
    add_session_tool_allow_rule(rule.clone());
    if let Err(err) = write_session_tool_allow_grant(&rule) {
        eprintln!("failed to write session tool permission grant: {err}");
    }
}

#[allow(dead_code)]
fn tool_confirmation_prompt(request: &ToolRequest) -> String {
    permission_summary(&tool_permission_prompt(request))
}

fn tool_permission_prompt(request: &ToolRequest) -> PermissionPrompt {
    let localized = crate::i18n::permission_text();
    let is_bash = matches!(request.name.as_str(), "bash" | "bash_command_review");
    let kind = if is_network_sensitive_tool(request) {
        PermissionPromptKind::Network
    } else {
        PermissionPromptKind::Tool
    };
    let display_name = crate::i18n::localize_tool_display_name(&request.name);
    PermissionPrompt {
        kind,
        title: format!(
            "{} `{}` {}",
            tr(Message::ToolRequiresApprovalPrefix),
            display_name,
            tr(Message::ToolRequiresApprovalSuffix)
        ),
        rows: if is_bash {
            // Exact approval data is independent of terminal preview width.
            // Client renderers wrap it; shell prefixes and redirects remain.
            let mut rows = vec![(
                localized.command.to_string(),
                request.args.get("command").or_else(|| request.args.get("cmd"))
                    .and_then(Value::as_str).unwrap_or_default().to_owned(),
            )];
            if let Some(cwd) = request.args.get("cwd").and_then(Value::as_str) {
                rows.push((localized.cwd.to_string(), cwd.to_owned()));
            }
            if let Some(reason) = request.args.get("sensitive_read_reason").and_then(Value::as_str) {
                rows.push(("Sensitive read".to_string(), reason.to_owned()));
            }
            rows
        } else {
            vec![
                (
                    localized.tool.to_string(),
                    crate::i18n::localize_tool_display_name(&request.name),
                ),
                (
                    tr(Message::ArgsLabel)
                        .trim()
                        .trim_end_matches(':')
                        .to_string(),
                    Value::Object(sanitized_approval_args(request)).to_string(),
                ),
            ]
        },
        session_label: format!(
            "{} {}",
            localized.allow_conversation_prefix,
            session_permission_scope_summary(request)
        ),
        session_value: session_permission_rule(request),
        custom_label: localized.custom_response.to_string(),
        supports_instruction: false,
        allow_conversation: true,
    }
}

pub(crate) fn permission_prompt_for_tool_request(request: &ToolRequest) -> PermissionPrompt {
    tool_permission_prompt(request)
}

/// Reuse the registered tool's terminal review for metadata-only approvals.
/// Hidden argument bodies require a tool-specific review instead.
pub(crate) fn builtin_permission_review_text(request: &ToolRequest) -> Option<String> {
    let spec = specs().into_iter().find(|spec| spec.name == request.name)?;
    if required_tool_argument_error(&spec, request).is_some()
        || sanitized_approval_args(request) != request.args { return None; }
    let prompt = tool_permission_prompt(request);
    let text = prompt.rows.into_iter().map(|(label, value)| format!("{label}: {value}"))
        .collect::<Vec<_>>().join("\n");
    if text.len() > 8192 || crate::session::redact_sensitive_text(&text) != text { return None; }
    Some(text)
}

pub(crate) fn session_tool_allow_rule_for(request: &ToolRequest) -> Option<String> {
    with_permission_cache(|cache| {
        cache
            .tool_rules
            .iter()
            .find(|rule| session_permission_rule_matches(rule, request))
            .cloned()
    })
    .flatten()
}

fn add_session_tool_allow_rule(rule: String) {
    with_permission_cache_mut(|cache| {
        push_unique_string(&mut cache.tool_rules, &rule);
    });
}

fn write_session_tool_allow_grant(rule: &str) -> Result<(), String> {
    write_session_permission_grant("tool", "allow_rule", rule.trim(), "allow")
}

fn tool_allow_rule_matches(rule: &str, tool_name: &str) -> bool {
    let rule = rule.trim();
    if rule.is_empty() {
        return false;
    }
    if rule == "*" || rule == tool_name {
        return true;
    }
    if let Some(prefix) = rule.strip_suffix('*') {
        return tool_name.starts_with(prefix);
    }
    false
}

pub(crate) fn session_permission_rule(request: &ToolRequest) -> String {
    let permission = nola_core::agent::policy::permission_name_for_tool(&request.name);
    let resource = session_permission_resource(request);
    let mut rule = json!({
        "version": 1,
        "tool": request.name,
        "permission": permission.clone(),
        "resource": resource,
    });
    if permission == "bash" && !bash_request_command(request).is_empty() {
        let command = bash_request_command(request);
        rule["command_scope"] = Value::String(bash_session_command_scope(&command));
        rule["cwd"] =
            Value::String(arg_string(&request.args, "cwd").unwrap_or_else(|| ".".to_string()));
    }
    serde_json::to_string(&rule).unwrap_or_default()
}

pub(crate) fn session_permission_rule_matches(rule: &str, request: &ToolRequest) -> bool {
    let Ok(rule) = serde_json::from_str::<Value>(rule) else {
        return false;
    };
    // Version 3 binds every rule to its exact capability. Old family-wide
    // rules must not silently acquire this narrower meaning on replay.
    if rule.get("version").and_then(Value::as_u64) != Some(1)
        || rule.get("tool").and_then(Value::as_str) != Some(request.name.as_str()) {
        return false;
    }
    let permission = nola_core::agent::policy::permission_name_for_tool(&request.name);
    if rule.get("permission").and_then(Value::as_str) != Some(permission.as_str()) {
        return false;
    }
    let Some(granted_resource) = rule
        .get("resource")
        .and_then(Value::as_str)
        .filter(|resource| !resource.is_empty()) else {
            return false;
        };
    if !session_permission_resource_matches(
        granted_resource,
        &session_permission_resource(request),
        &permission,
    ) {
        return false;
    }
    if permission == "bash"
        && (rule.get("command_scope").is_some() || !bash_request_command(request).is_empty()) {
        let command = bash_request_command(request);
        let cwd = arg_string(&request.args, "cwd").unwrap_or_else(|| ".".to_string());
        return rule.get("cwd").and_then(Value::as_str) == Some(cwd.as_str())
            && rule
                .get("command_scope")
                .and_then(Value::as_str)
                .is_some_and(|scope| command.trim() == scope);
    }
    true
}

fn session_permission_scope_summary(request: &ToolRequest) -> String {
    if nola_core::agent::policy::permission_name_for_tool(&request.name) == "bash"
        && !bash_request_command(request).is_empty()
    {
        let command = bash_request_command(request);
        let localized = crate::i18n::permission_text();
        let cwd = arg_string(&request.args, "cwd").unwrap_or_else(|| ".".to_string());
        return format!("{}: {} · {}: {}", localized.command,
            command.trim(), localized.cwd, cwd);
    }
    format!(
        "{}: {}",
        nola_core::agent::policy::permission_name_for_tool(&request.name),
        session_permission_resource(request)
    )
}

pub(crate) fn session_permission_resource(request: &ToolRequest) -> String {
    let permission = nola_core::agent::policy::permission_name_for_tool(&request.name);
    if permission == "bash" {
        return arg_string(&request.args, "cwd").unwrap_or_else(|| ".".to_string());
    }
    if permission == "browser" {
        return "browser".to_string();
    }
    let raw = ["destination", "path", "root", "cwd", "source"]
        .iter()
        .find_map(|key| {
            request
                .args
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| ((*key).to_string(), value.to_string()))
        });
    if matches!(
        permission.as_str(),
        "read" | "edit" | "files_read" | "files_write"
    ) {
        if let Some((key, raw)) = raw.as_ref() {
            let path = PathBuf::from(raw);
            let requested_directory = if matches!(key.as_str(), "root" | "cwd") || path.is_dir() {
                path.as_path()
            } else {
                path.parent().unwrap_or_else(|| Path::new("."))
            };
            return requested_directory.to_string_lossy().into_owned();
        }
    }
    if let Some((_, raw)) = raw {
        return raw;
    }
    let fallback = ["url", "host", "domain", "query", "pattern", "name"]
        .iter()
        .find_map(|key| {
            request
                .args
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or(request.name.as_str());
    if permission == "web" {
        return network_permission_scope(fallback);
    }
    if matches!(
        permission.as_str(),
        "camera"
            | "microphone"
            | "photos"
            | "location"
            | "calendar"
            | "reminders"
            | "contacts"
            | "health"
            | "notifications"
            | "clipboard"
            | "computer"
    ) {
        return permission;
    }
    fallback.to_string()
}

fn session_permission_resource_matches(granted: &str, requested: &str, permission: &str) -> bool {
    if matches!(permission, "read" | "edit" | "files_read" | "files_write") {
        let granted = Path::new(granted);
        let requested = Path::new(requested);
        return requested == granted || requested.starts_with(granted);
    }
    granted == requested
}

fn bash_request_command(request: &ToolRequest) -> String {
    arg_string(&request.args, "command")
        .or_else(|| arg_string(&request.args, "cmd"))
        .unwrap_or_default()
}

fn network_permission_scope(value: &str) -> String {
    let value = value.trim();
    let Some((scheme, rest)) = value.split_once("://") else {
        return value.to_ascii_lowercase();
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if authority.is_empty() {
        value.to_ascii_lowercase()
    } else {
        format!("{}://{authority}", scheme.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod scoped_permission_tests {
    use super::*;

    fn request(name: &str, args: Value) -> ToolRequest {
        ToolRequest {
            id: "permission-test".to_string(),
            name: name.to_string(),
            args: args.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn bash_prefixes_match_token_boundaries_not_string_prefixes_or_chains() {
        assert!(bash_command_matches_scope(
            "git status --short",
            "git status"
        ));
        assert!(!bash_command_matches_scope("git statusfoo", "git status"));
        assert!(!bash_command_matches_scope(
            "git status && rm -rf build",
            "git status"
        ));
        assert!(!bash_command_matches_scope(
            "git status > /tmp/status",
            "git status"
        ));
    }

    #[test]
    fn bash_session_grant_is_scoped_to_exact_command_and_working_directory() {
        let approved = request(
            "bash",
            json!({"command": "cargo test --quiet", "cwd": "/workspace/project"}),
        );
        let rule = session_permission_rule(&approved);
        assert!(session_permission_rule_matches(&rule, &approved));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "bash",
                json!({"command": "cargo test package_name", "cwd": "/workspace/project"})
            )
        ));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "bash",
                json!({"command": "cargo build", "cwd": "/workspace/project"})
            )
        ));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "bash",
                json!({"command": "cargo test", "cwd": "/workspace/other"})
            )
        ));
    }

    #[test]
    fn bash_session_grant_does_not_widen_to_other_script_arguments() {
        let approved = request(
            "bash",
            json!({
                "command": "/usr/bin/python3 '/Applications/Nola Skills/create_document.py' --spec first.json --output first.docx",
                "cwd": "/workspace/project"
            }),
        );
        let rule = session_permission_rule(&approved);
        assert!(session_permission_rule_matches(&rule, &approved));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "bash",
                json!({
                    "command": "/usr/bin/python3 '/Applications/Nola Skills/create_document.py' --spec second.json --output second.docx",
                    "cwd": "/workspace/project"
                })
            )
        ));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "bash",
                json!({
                    "command": "/usr/bin/python3 '/Applications/Nola Skills/other.py' --output second.docx",
                    "cwd": "/workspace/project"
                })
            )
        ));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "bash",
                json!({
                    "command": "/usr/bin/python3 '/Applications/Nola Skills/create_document.py' --spec second.json --output second.docx",
                    "cwd": "/workspace/other"
                })
            )
        ));
    }

    #[test]
    fn textutil_session_grant_does_not_widen_to_other_conversion_files() {
        let approved = request(
            "bash",
            json!({
                "command": "/usr/bin/textutil -convert docx first.html -output first.docx",
                "cwd": "/workspace/project"
            }),
        );
        let rule = session_permission_rule(&approved);
        assert!(session_permission_rule_matches(&rule, &approved));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "bash",
                json!({
                    "command": "/usr/bin/textutil -convert docx second.html -output second.docx",
                    "cwd": "/workspace/project"
                })
            )
        ));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "bash",
                json!({
                    "command": "/usr/bin/textutil -convert txt second.docx -stdout",
                    "cwd": "/workspace/project"
                })
            )
        ));
    }

    #[test]
    fn file_session_grant_requires_exact_tool_inside_directory() {
        let root = std::env::temp_dir().join(format!(
            "nola-permission-scope-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let other = root.with_extension("other");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&other).unwrap();
        let file = root.join("a.txt");
        fs::write(&file, "a").unwrap();
        let rule = session_permission_rule(&request(
            "read_file",
            json!({"path": file.display().to_string()}),
        ));
        assert!(session_permission_rule_matches(
            &rule,
            &request("read_file", json!({"path": root.join("b.txt").display().to_string()}))
        ));
        assert!(!session_permission_rule_matches(
            &rule,
            &request("list_dir", json!({"path": root.display().to_string()}))
        ));
        assert!(!session_permission_rule_matches(
            &rule,
            &request("read_file", json!({"path": other.join("b.txt").display().to_string()}))
        ));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(other);
    }

    #[test]
    fn write_session_grant_does_not_expand_to_project_markers() {
        let root = std::env::temp_dir().join(format!(
            "nola-write-permission-scope-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let project = root.join("project");
        let nested = project.join("src/nested");
        let sibling = root.join("sibling");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let rule = session_permission_rule(&request(
            "edit_file_lines",
            json!({"path": project.join("src/lib.rs").display().to_string()}),
        ));
        assert!(session_permission_rule_matches(
            &rule,
            &request(
                "edit_file_lines",
                json!({"path": nested.join("module.rs").display().to_string()})
            )
        ));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "edit_file_lines",
                json!({"path": project.join("Cargo.toml").display().to_string()})
            )
        ));
        assert!(!session_permission_rule_matches(
            &rule,
            &request(
                "edit_file_lines",
                json!({"path": sibling.join("outside.rs").display().to_string()})
            )
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn session_permission_rules_reject_missing_tool_and_old_family_rules() {
        let approved = request("bash", json!({"command": "cargo test", "cwd": "/workspace"}));
        let rule = session_permission_rule(&approved);
        assert!(session_permission_rule_matches(&rule, &approved));
        // Same process permission domain and command do not imply the same
        // execution capability (foreground versus persistent background).
        assert!(!session_permission_rule_matches(&rule,
            &request("bash_background_start", json!({"command": "cargo test", "cwd": "/workspace"}))));
        let mut value: Value = serde_json::from_str(&rule).unwrap();
        value["version"] = json!(2);
        assert!(!session_permission_rule_matches(&value.to_string(), &approved));
        value["version"] = json!(1);
        value.as_object_mut().unwrap().remove("tool");
        assert!(!session_permission_rule_matches(&value.to_string(), &approved));
        assert!(!session_permission_rule_matches(&rule,
            &request("bash", json!({"cwd": "/workspace"}))));
    }

    #[test]
    fn session_permission_rules_reject_missing_or_empty_resource() {
        let approved = request("read_file", json!({"path": "/workspace/file.txt"}));
        let mut value: Value = serde_json::from_str(&session_permission_rule(&approved)).unwrap();
        value.as_object_mut().unwrap().remove("resource");
        assert!(!session_permission_rule_matches(&value.to_string(), &approved));
        value["resource"] = json!("");
        assert!(!session_permission_rule_matches(&value.to_string(), &approved));
    }

    #[test]
    fn malformed_bare_tool_rules_and_implicit_name_prefixes_do_not_match() {
        let bash = request("bash", json!({"command": "cargo test", "cwd": "."}));
        assert!(!session_permission_rule_matches("bash", &bash));
        assert!(!tool_allow_rule_matches("read", "read_file"));
        assert!(tool_allow_rule_matches("read*", "read_file"));
    }

    #[test]
    fn network_scope_uses_origin_not_unsafe_string_prefix() {
        assert_eq!(
            network_permission_scope("https://example.com/a"),
            "https://example.com"
        );
        assert_ne!(
            network_permission_scope("https://example.com.evil.test/a"),
            network_permission_scope("https://example.com/a")
        );
    }
}

pub fn session_bash_allow_prefix_for(command: &str) -> Option<String> {
    with_permission_cache(|cache| {
        cache
            .bash_prefixes
            .iter()
            .find(|prefix| bash_command_matches_scope(command, prefix))
            .cloned()
    })
    .flatten()
}

fn add_session_bash_allow_prefix(prefix: String) {
    with_permission_cache_mut(|cache| {
        push_unique_string(&mut cache.bash_prefixes, &prefix);
    });
}

pub fn allow_conversation_bash_prefix(prefix: &str) -> Result<(), String> {
    add_session_bash_allow_prefix(prefix.to_string());
    if has_session_permission_scope() {
        write_session_permission_grant("bash", "allow_prefix", prefix.trim(), "allow")?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn session_network_allow_rule_for(endpoint: &str, host: &str) -> Option<String> {
    with_permission_cache(|cache| {
        cache
            .network_rules
            .iter()
            .find(|rule| {
                network_permission_scope(endpoint) == network_permission_scope(rule)
                    || host.eq_ignore_ascii_case(rule)
            })
            .cloned()
    })
    .flatten()
}

fn add_session_network_allow_rule(rule: String) {
    with_permission_cache_mut(|cache| {
        push_unique_string(&mut cache.network_rules, &rule);
    });
}

#[allow(dead_code)]
pub fn allow_conversation_network_rule(rule: &str) -> Result<(), String> {
    add_session_network_allow_rule(rule.to_string());
    if has_session_permission_scope() {
        write_session_permission_grant("network", "allow_rule", rule.trim(), "allow")?;
    }
    Ok(())
}

pub fn session_agent_allow_rule_for(agent_type: &str) -> Option<String> {
    with_permission_cache(|cache| {
        cache
            .agent_rules
            .iter()
            .find(|rule| tool_allow_rule_matches(rule, agent_type))
            .cloned()
    })
    .flatten()
}

fn add_session_agent_allow_rule(rule: String) {
    with_permission_cache_mut(|cache| {
        push_unique_string(&mut cache.agent_rules, &rule);
    });
}

pub fn allow_conversation_agent_rule(rule: &str) -> Result<(), String> {
    add_session_agent_allow_rule(rule.to_string());
    if has_session_permission_scope() {
        write_session_permission_grant("agent", "allow_rule", rule.trim(), "allow")?;
    }
    Ok(())
}

fn approval_summary(request: &ToolRequest) -> Vec<String> {
    let args = &request.args;
    let line = match request.name.as_str() {
        "write_file" => format!(
            "write file: {} ({} chars)",
            arg_string(args, "path").unwrap_or_else(|| "(missing path)".to_string()),
            arg_char_count(args, "content").unwrap_or(0)
        ),
        "append_file" => format!(
            "append file: {} ({} chars)",
            arg_string(args, "path").unwrap_or_else(|| "(missing path)".to_string()),
            arg_char_count(args, "content").unwrap_or(0)
        ),
        "edit_file" => format!(
            "edit file: {} (replace_all={})",
            arg_string(args, "path").unwrap_or_else(|| "(missing path)".to_string()),
            args.get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        "multi_edit_file" => format!(
            "multi-edit file: {} ({} edits)",
            arg_string(args, "path").unwrap_or_else(|| "(missing path)".to_string()),
            args.get("edits")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        ),
        "apply_patch" => format!(
            "apply patch: {} ({} chars)",
            arg_string(args, "root").unwrap_or_else(|| "(missing root)".to_string()),
            arg_char_count(args, "patch").unwrap_or(0)
        ),
        "propose_agent_plan" => format!(
            "approve plan: {} ({} steps)",
            arg_string(args, "title").unwrap_or_else(|| "Execution Plan".to_string()),
            args.get("steps")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        ),
        "agent_plan_file" => format!(
            "agent plan file: {} action={}",
            arg_string(args, "root").unwrap_or_else(|| "(missing root)".to_string()),
            arg_string(args, "action").unwrap_or_else(|| "list".to_string())
        ),
        "bash" => format!(
            "run command: {}{}",
            arg_string(args, "command").unwrap_or_else(|| "(missing command)".to_string()),
            arg_string(args, "cwd")
                .map(|cwd| format!(" in {cwd}"))
                .unwrap_or_default()
        ),
        "bash_background_start" => format!(
            "start background command: {}{}",
            arg_string(args, "command").unwrap_or_else(|| "(missing command)".to_string()),
            arg_string(args, "cwd")
                .map(|cwd| format!(" in {cwd}"))
                .unwrap_or_default()
        ),
        "chrome_native_open_page" => format!(
            "browser open: {}",
            arg_string(args, "url").unwrap_or_else(|| "(missing URL)".to_string())
        ),
        "chrome_native_click" => {
            let clicks = args.get("clicks").and_then(Value::as_i64).unwrap_or(1);
            let click_suffix = if clicks > 1 {
                format!(" {clicks}x")
            } else {
                String::new()
            };
            let suffix = if args
                .get("expect_download")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                " (expect download)"
            } else if args
                .get("expect_popup")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                " (expect popup)"
            } else {
                ""
            };
            format!(
                "browser click: {}{}{}{}",
                browser_target_summary(args),
                click_suffix,
                mouse_modifier_suffix(args),
                suffix
            )
        }
        "browser_console_logs" => format!(
            "browser console logs: level={} max_entries={}",
            arg_string(args, "level").unwrap_or_else(|| "all".to_string()),
            args.get("max_entries")
                .and_then(Value::as_i64)
                .unwrap_or(50)
        ),
        "browser_network_events" => format!(
            "browser network events: level={} max_entries={}",
            arg_string(args, "level").unwrap_or_else(|| "warnings".to_string()),
            args.get("max_entries")
                .and_then(Value::as_i64)
                .unwrap_or(50)
        ),
        "browser_dom_snapshot" => format!("browser DOM snapshot: {}", browser_target_summary(args)),
        "browser_storage_state" => format!(
            "browser storage state: max_items={} values={}",
            args.get("max_items").and_then(Value::as_i64).unwrap_or(100),
            args.get("include_values")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        "browser_accessibility_snapshot" => format!(
            "browser accessibility snapshot: {} max_nodes={}",
            browser_target_summary(args),
            args.get("max_nodes").and_then(Value::as_i64).unwrap_or(120)
        ),
        "browser_drag" => format!(
            "browser drag: {} -> {}{}",
            browser_drag_source_summary(args),
            browser_drag_target_summary(args),
            mouse_modifier_suffix(args)
        ),
        "chrome_native_type_text" => format!(
            "browser type: {} ({} chars)",
            browser_target_summary(args),
            arg_char_count(args, "text").unwrap_or(0)
        ),
        "browser_select_option" => format!(
            "browser select: {} -> {}",
            browser_target_summary(args),
            browser_option_summary(args)
        ),
        "browser_set_checked" => format!(
            "browser set checked: {} -> {}",
            browser_target_summary(args),
            args.get("checked").and_then(Value::as_bool).unwrap_or(true)
        ),
        "browser_upload_file" => format!(
            "browser upload: {} <- {}",
            browser_target_summary(args),
            browser_upload_summary(args)
        ),
        "browser_press_key" => format!(
            "browser key: {}",
            arg_string(args, "key").unwrap_or_else(|| "(missing key)".to_string())
        ),
        "browser_scroll" => format!(
            "browser scroll: {} {}",
            arg_string(args, "direction").unwrap_or_else(|| "down".to_string()),
            arg_display(args, "amount")
        ),
        "browser_hover" => format!("browser hover: {}", browser_target_summary(args)),
        "chrome_native_screenshot" => {
            if args.contains_key("x")
                || args.contains_key("y")
                || args.contains_key("width")
                || args.contains_key("height")
            {
                format!(
                    "browser screenshot: clip {},{} {}x{}",
                    arg_display(args, "x"),
                    arg_display(args, "y"),
                    arg_display(args, "width"),
                    arg_display(args, "height")
                )
            } else {
                format!(
                    "browser screenshot: {}",
                    if args
                        .get("full_page")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        "full page"
                    } else {
                        "viewport"
                    }
                )
            }
        }
        "chrome_native_assert" => format!("browser assert: {}", browser_condition_summary(args)),
        "chrome_native_wait_for" => format!(
            "browser wait for: {} timeout={}ms",
            browser_condition_summary(args),
            arg_display(args, "timeout_ms")
        ),
        "computer_observe" => format!(
            "desktop observe: screenshot={} accessibility={} windows={} apps={} text={}",
            args.get("include_screenshot")
                .or_else(|| args.get("screenshot"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
            args.get("include_accessibility")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            args.get("include_windows")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            args.get("include_apps")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            args.get("include_text")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        ),
        "computer_screenshot" => {
            if args.contains_key("display_index") {
                format!(
                    "desktop screenshot display index: {}",
                    arg_display(args, "display_index")
                )
            } else if args.contains_key("display_id") {
                format!(
                    "desktop screenshot display id: {}",
                    arg_display(args, "display_id")
                )
            } else if args.contains_key("x")
                || args.contains_key("y")
                || args.contains_key("width")
                || args.contains_key("height")
            {
                format!(
                    "desktop screenshot crop: x={}, y={}, width={}, height={}",
                    arg_display(args, "x"),
                    arg_display(args, "y"),
                    arg_display(args, "width"),
                    arg_display(args, "height")
                )
            } else {
                "desktop screenshot: full screen".to_string()
            }
        }
        "computer_screenshot_element" => format!(
            "desktop screenshot element: target='{}' role='{}' padding={}",
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default(),
            arg_display(args, "padding")
        ),
        "computer_screenshot_window" => format!(
            "desktop screenshot window: {} padding={}",
            arg_string(args, "title")
                .or_else(|| arg_string(args, "app"))
                .unwrap_or_else(|| format!("index {}", arg_display(args, "index"))),
            arg_display(args, "padding")
        ),
        "computer_read_text" => format!(
            "desktop read visible text: accessibility={} ocr={} max={}",
            args.get("include_accessibility")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            args.get("include_ocr")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            arg_display(args, "max_results")
        ),
        "computer_find_text" => format!(
            "desktop find text: '{}' match_mode={} ocr={}",
            arg_string(args, "text").unwrap_or_else(|| "(missing text)".to_string()),
            arg_string(args, "match_mode").unwrap_or_else(|| "contains".to_string()),
            args.get("include_ocr")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        ),
        "computer_move_to_text" => format!(
            "desktop move to text: '{}' source={} occurrence={}",
            arg_string(args, "text").unwrap_or_else(|| "(missing text)".to_string()),
            arg_string(args, "source").unwrap_or_else(|| "any".to_string()),
            arg_display(args, "occurrence")
        ),
        "computer_click_text" => format!(
            "desktop click text: '{}' source={} occurrence={}{}",
            arg_string(args, "text").unwrap_or_else(|| "(missing text)".to_string()),
            arg_string(args, "source").unwrap_or_else(|| "any".to_string()),
            arg_display(args, "occurrence"),
            mouse_modifier_suffix(args)
        ),
        "computer_select_text" => format!(
            "desktop select text: '{}' source={} occurrence={}{}",
            arg_string(args, "text").unwrap_or_else(|| "(missing text)".to_string()),
            arg_string(args, "source").unwrap_or_else(|| "any".to_string()),
            arg_display(args, "occurrence"),
            mouse_modifier_suffix(args)
        ),
        "computer_assert_text" => format!(
            "desktop assert text: '{}' exists={} match_mode={}",
            arg_string(args, "text").unwrap_or_else(|| "(missing text)".to_string()),
            args.get("exists").and_then(Value::as_bool).unwrap_or(true),
            arg_string(args, "match_mode").unwrap_or_else(|| "contains".to_string())
        ),
        "computer_wait_for_text" => format!(
            "desktop wait for text: '{}' exists={} timeout={}ms",
            arg_string(args, "text").unwrap_or_else(|| "(missing text)".to_string()),
            args.get("exists").and_then(Value::as_bool).unwrap_or(true),
            arg_display(args, "timeout_ms")
        ),
        "computer_list_displays" => {
            "desktop read display geometry and coordinate bounds".to_string()
        }
        "computer_wait" => format!(
            "desktop wait: {}ms (stable={} screenshot={})",
            arg_display(args, "duration_ms"),
            args.get("until_stable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            args.get("screenshot")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        ),
        "computer_accessibility_snapshot" => format!(
            "desktop accessibility snapshot: up to {} elements (values={})",
            arg_display(args, "max_elements"),
            args.get("include_values")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        "computer_element_at" => format!(
            "desktop element at: {},{}",
            arg_display(args, "x"),
            arg_display(args, "y")
        ),
        "computer_press_at" => format!(
            "desktop press at: {},{}",
            arg_display(args, "x"),
            arg_display(args, "y")
        ),
        "computer_type_at" => format!(
            "desktop type at: {},{} ({} chars)",
            arg_display(args, "x"),
            arg_display(args, "y"),
            arg_string(args, "text")
                .map(|value| value.chars().count())
                .unwrap_or(0)
        ),
        "computer_scroll_at" => format!(
            "desktop scroll at: {},{} {} {}px{}",
            arg_display(args, "x"),
            arg_display(args, "y"),
            arg_string(args, "direction").unwrap_or_else(|| "down".to_string()),
            arg_display(args, "amount"),
            mouse_modifier_suffix(args)
        ),
        "computer_click_element" => format!(
            "desktop click element: target='{}' role='{}' occurrence={}{}",
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default(),
            arg_display(args, "occurrence"),
            mouse_modifier_suffix(args)
        ),
        "computer_press_element" => format!(
            "desktop press element: target='{}' role='{}' occurrence={}",
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default(),
            arg_display(args, "occurrence")
        ),
        "computer_focus_element" => format!(
            "desktop focus element: target='{}' role='{}' occurrence={}",
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default(),
            arg_display(args, "occurrence")
        ),
        "computer_perform_element_action" => format!(
            "desktop perform element action: action={} target='{}' role='{}' occurrence={}",
            arg_string(args, "action").unwrap_or_default(),
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default(),
            arg_display(args, "occurrence")
        ),
        "computer_type_element" => format!(
            "desktop type element: target='{}' role='{}' ({} chars)",
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default(),
            arg_char_count(args, "text").unwrap_or(0)
        ),
        "computer_set_element_value" => format!(
            "desktop set element value: target='{}' role='{}' ({} chars)",
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default(),
            arg_char_count(args, "value").unwrap_or(0)
        ),
        "computer_assert_element" => format!(
            "desktop assert element: target='{}' role='{}' exists={}",
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default(),
            args.get("exists").and_then(Value::as_bool).unwrap_or(true)
        ),
        "computer_wait_for_element" => format!(
            "desktop wait for element: target='{}' role='{}' exists={} timeout={}ms",
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default(),
            args.get("exists").and_then(Value::as_bool).unwrap_or(true),
            arg_display(args, "timeout_ms")
        ),
        "computer_move_to_element" => format!(
            "desktop move to element: target='{}' role='{}'",
            element_target_display(args),
            arg_string(args, "role").unwrap_or_default()
        ),
        "computer_move_mouse" => format!(
            "desktop move mouse: {},{}",
            arg_display(args, "x"),
            arg_display(args, "y")
        ),
        "computer_mouse_down" => {
            let target = if args.contains_key("x") || args.contains_key("y") {
                format!("{},{}", arg_display(args, "x"), arg_display(args, "y"))
            } else {
                "current position".to_string()
            };
            format!(
                "desktop mouse down: {} ({}){}",
                target,
                arg_string(args, "button").unwrap_or_else(|| "left".to_string()),
                mouse_modifier_suffix(args)
            )
        }
        "computer_mouse_up" => {
            let target = if args.contains_key("x") || args.contains_key("y") {
                format!("{},{}", arg_display(args, "x"), arg_display(args, "y"))
            } else {
                "current position".to_string()
            };
            format!(
                "desktop mouse up: {} ({}){}",
                target,
                arg_string(args, "button").unwrap_or_else(|| "left".to_string()),
                mouse_modifier_suffix(args)
            )
        }
        "computer_click" => format!(
            "desktop click: {},{} ({}){}",
            arg_display(args, "x"),
            arg_display(args, "y"),
            arg_string(args, "button").unwrap_or_else(|| "left".to_string()),
            mouse_modifier_suffix(args)
        ),
        "computer_drag" => format!(
            "desktop drag: {},{} -> {},{}{}",
            arg_display(args, "start_x"),
            arg_display(args, "start_y"),
            arg_display(args, "end_x"),
            arg_display(args, "end_y"),
            mouse_modifier_suffix(args)
        ),
        "computer_drag_element" => format!(
            "desktop drag element: target='{}' -> {}{}",
            element_target_display(args),
            drag_element_target_display(args),
            mouse_modifier_suffix(args)
        ),
        "computer_type_text" => format!(
            "desktop type into focused app: {} chars",
            arg_char_count(args, "text").unwrap_or(0)
        ),
        "computer_get_clipboard" => format!(
            "desktop read clipboard: up to {} chars",
            arg_display(args, "max_chars")
        ),
        "computer_set_clipboard" => format!(
            "desktop set clipboard: {} chars",
            arg_char_count(args, "text").unwrap_or(0)
        ),
        "computer_copy_selection" => format!(
            "desktop copy selected text: up to {} chars (restore_clipboard={})",
            arg_display(args, "max_chars"),
            args.get("restore_clipboard")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        ),
        "computer_assert_clipboard" => format!(
            "desktop assert clipboard: {} chars exists={} match_mode={}",
            arg_char_count(args, "text").unwrap_or(0),
            args.get("exists").and_then(Value::as_bool).unwrap_or(true),
            arg_string(args, "match_mode").unwrap_or_else(|| "contains".to_string())
        ),
        "computer_wait_for_clipboard" => format!(
            "desktop wait for clipboard: {} chars exists={} timeout={}ms",
            arg_char_count(args, "text").unwrap_or(0),
            args.get("exists").and_then(Value::as_bool).unwrap_or(true),
            arg_display(args, "timeout_ms")
        ),
        "computer_press_key" => format!(
            "desktop key: {}",
            arg_string(args, "key").unwrap_or_else(|| "(missing key)".to_string())
        ),
        "computer_hotkey" => format!(
            "desktop hotkey: {}+{}",
            modifier_list_display(args),
            arg_string(args, "key").unwrap_or_else(|| "(missing key)".to_string())
        ),
        "computer_key_down" => format!(
            "desktop key down: {}",
            arg_string(args, "key").unwrap_or_else(|| "(missing key)".to_string())
        ),
        "computer_key_up" => format!(
            "desktop key up: {}",
            arg_string(args, "key").unwrap_or_else(|| "(missing key)".to_string())
        ),
        "computer_scroll" => format!(
            "desktop scroll: {} {}{}{}",
            arg_string(args, "direction").unwrap_or_else(|| "down".to_string()),
            arg_display(args, "amount"),
            coordinate_suffix(args),
            mouse_modifier_suffix(args)
        ),
        "computer_scroll_element" => format!(
            "desktop scroll element: target='{}' {} {}{}",
            element_target_display(args),
            arg_string(args, "direction").unwrap_or_else(|| "down".to_string()),
            arg_display(args, "amount"),
            mouse_modifier_suffix(args)
        ),
        "computer_list_apps" => format!(
            "desktop read running apps (include_background={}, max_apps={})",
            args.get("include_background")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            arg_display(args, "max_apps")
        ),
        "computer_assert_app" => format!(
            "desktop assert app: {} running={}",
            arg_string(args, "bundle_id")
                .or_else(|| arg_string(args, "app"))
                .unwrap_or_else(|| "(missing app)".to_string()),
            args.get("running").and_then(Value::as_bool).unwrap_or(true)
        ),
        "computer_wait_for_app" => format!(
            "desktop wait for app: {} running={} timeout={}ms",
            arg_string(args, "bundle_id")
                .or_else(|| arg_string(args, "app"))
                .unwrap_or_else(|| "(missing app)".to_string()),
            args.get("running").and_then(Value::as_bool).unwrap_or(true),
            arg_display(args, "timeout_ms")
        ),
        "computer_list_windows" => "desktop read visible window titles and bounds".to_string(),
        "computer_focus_window" => format!(
            "desktop focus window: {}",
            arg_string(args, "title")
                .or_else(|| arg_string(args, "app"))
                .unwrap_or_else(|| format!("index {}", arg_display(args, "index")))
        ),
        "computer_set_window_bounds" => format!(
            "desktop set window bounds: {} -> {},{} {}x{}",
            arg_string(args, "title")
                .or_else(|| arg_string(args, "app"))
                .unwrap_or_else(|| format!("index {}", arg_display(args, "index"))),
            arg_display(args, "x"),
            arg_display(args, "y"),
            arg_display(args, "width"),
            arg_display(args, "height")
        ),
        "computer_perform_window_action" => format!(
            "desktop perform window action: action={} target={}",
            arg_string(args, "action").unwrap_or_default(),
            arg_string(args, "title")
                .or_else(|| arg_string(args, "app"))
                .unwrap_or_else(|| format!("index {}", arg_display(args, "index")))
        ),
        "computer_close_window" => format!(
            "desktop close window: {}",
            arg_string(args, "title")
                .or_else(|| arg_string(args, "app"))
                .unwrap_or_else(|| format!("index {}", arg_display(args, "index")))
        ),
        "computer_assert_window" => format!(
            "desktop assert window: {} exists={}",
            arg_string(args, "title")
                .or_else(|| arg_string(args, "app"))
                .unwrap_or_else(|| format!("index {}", arg_display(args, "index"))),
            args.get("exists").and_then(Value::as_bool).unwrap_or(true)
        ),
        "computer_wait_for_window" => format!(
            "desktop wait for window: {} exists={} timeout={}ms",
            arg_string(args, "title")
                .or_else(|| arg_string(args, "app"))
                .unwrap_or_else(|| format!("index {}", arg_display(args, "index"))),
            args.get("exists").and_then(Value::as_bool).unwrap_or(true),
            arg_display(args, "timeout_ms")
        ),
        "computer_list_menus" => format!(
            "desktop list menus: {}{}",
            arg_string(args, "app").unwrap_or_else(|| "(missing app)".to_string()),
            arg_string(args, "top_menu")
                .map(|menu| format!(" > {menu}"))
                .unwrap_or_default()
        ),
        "computer_select_menu" => format!(
            "desktop select menu: {} > {}",
            arg_string(args, "app").unwrap_or_else(|| "(missing app)".to_string()),
            args.get("menu_path")
                .and_then(Value::as_array)
                .map(|items| items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<_>>()
                    .join(" > "))
                .filter(|path| !path.is_empty())
                .unwrap_or_else(|| "(missing menu path)".to_string())
        ),
        "computer_activate_app" => format!(
            "desktop activate app: {}",
            arg_string(args, "app").unwrap_or_else(|| "(missing app)".to_string())
        ),
        "computer_quit_app" => format!(
            "desktop quit app: {}",
            arg_string(args, "app").unwrap_or_else(|| "(missing app)".to_string())
        ),
        "computer_open_item" => format!(
            "desktop open item: {}{}{}",
            arg_string(args, "target").unwrap_or_else(|| "(missing target)".to_string()),
            arg_string(args, "app")
                .map(|app| format!(" with {app}"))
                .unwrap_or_default(),
            if args.get("reveal").and_then(Value::as_bool).unwrap_or(false) {
                " (reveal)"
            } else {
                ""
            }
        ),
        _ => format!("run tool: {}", request.name),
    };
    vec![line]
}

fn browser_target_summary(args: &Map<String, Value>) -> String {
    if let Some(selector) = arg_string(args, "selector") {
        return format!("selector {selector}");
    }
    if let Some(label) = arg_string(args, "label") {
        return format!("label {label}");
    }
    if let Some(text) = arg_string(args, "text") {
        return format!("text {text}");
    }
    if args.contains_key("x") || args.contains_key("y") {
        return format!(
            "coordinates {},{}",
            arg_display(args, "x"),
            arg_display(args, "y")
        );
    }
    "focused element/page".to_string()
}

fn browser_drag_source_summary(args: &Map<String, Value>) -> String {
    if let Some(selector) = arg_string(args, "source_selector") {
        return format!("source selector {selector}");
    }
    if let Some(text) = arg_string(args, "source_text") {
        return format!("source text {text}");
    }
    if args.contains_key("start_x") || args.contains_key("start_y") {
        return format!(
            "source {},{}",
            arg_display(args, "start_x"),
            arg_display(args, "start_y")
        );
    }
    "missing source".to_string()
}

fn browser_drag_target_summary(args: &Map<String, Value>) -> String {
    if let Some(selector) = arg_string(args, "target_selector") {
        return format!("target selector {selector}");
    }
    if let Some(text) = arg_string(args, "target_text") {
        return format!("target text {text}");
    }
    if args.contains_key("end_x") || args.contains_key("end_y") {
        return format!(
            "target {},{}",
            arg_display(args, "end_x"),
            arg_display(args, "end_y")
        );
    }
    "missing target".to_string()
}

fn browser_condition_summary(args: &Map<String, Value>) -> String {
    for key in ["text", "selector", "url_contains", "title_contains"] {
        if let Some(value) = arg_string(args, key) {
            let exists = args.get("exists").and_then(Value::as_bool).unwrap_or(true);
            return format!("{key}='{value}' exists={exists}");
        }
    }
    "missing condition".to_string()
}

fn browser_option_summary(args: &Map<String, Value>) -> String {
    if let Some(value) = arg_string(args, "value") {
        return format!("value {value}");
    }
    if let Some(label) = arg_string(args, "option_label") {
        return format!("label {label}");
    }
    if args.contains_key("index") {
        return format!("index {}", arg_display(args, "index"));
    }
    "missing option".to_string()
}

fn browser_upload_summary(args: &Map<String, Value>) -> String {
    if let Some(path) = arg_string(args, "path") {
        return format!("path {}", truncate(&path, 160));
    }
    let count = args
        .get("paths")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if count == 0 {
        "missing paths".to_string()
    } else {
        format!("{count} path(s)")
    }
}

pub(crate) fn sanitized_approval_args(request: &ToolRequest) -> Map<String, Value> {
    let mut out = request.args.clone();
    for (key, value) in request.args.iter() {
        if request.name == "multi_edit_file" && key == "edits" {
            out.insert(key.clone(), redacted_edits_value(value));
            continue;
        }
        if should_redact_approval_arg(&request.name, key) {
            out.insert(key.clone(), redacted_value(value));
        }
    }
    out
}

fn redacted_edits_value(value: &Value) -> Value {
    let Some(edits) = value.as_array() else {
        return Value::String("[redacted]".to_string());
    };
    Value::Array(
        edits
            .iter()
            .map(|item| {
                let Some(edit) = item.as_object() else {
                    return Value::String("[redacted]".to_string());
                };
                let mut redacted = Map::new();
                if let Some(old_text) = edit.get("old_text") {
                    redacted.insert("old_text".to_string(), redacted_value(old_text));
                }
                if let Some(new_text) = edit.get("new_text") {
                    redacted.insert("new_text".to_string(), redacted_value(new_text));
                }
                if let Some(replace_all) = edit.get("replace_all") {
                    redacted.insert("replace_all".to_string(), replace_all.clone());
                }
                Value::Object(redacted)
            })
            .collect(),
    )
}

fn should_redact_approval_arg(tool_name: &str, key: &str) -> bool {
    let lower_key = key.to_ascii_lowercase();
    if lower_key.contains("password") || lower_key.contains("token") || lower_key.contains("secret")
    {
        return true;
    }
    matches!(
        (tool_name, key),
        ("write_file", "content")
            | ("write_file", "json_content")
            | ("append_file", "content")
            | ("edit_file", "old_text")
            | ("edit_file", "new_text")
            | ("edit_file_lines", "new_text")
            | ("replace_in_files", "old_text")
            | ("replace_in_files", "new_text")
            | ("apply_patch", "patch")
            | ("chrome_native_type_text", "text")
            | ("computer_type_element", "text")
            | ("computer_set_element_value", "value")
            | ("computer_type_text", "text")
            | ("computer_set_clipboard", "text")
            | ("computer_assert_clipboard", "text")
            | ("computer_wait_for_clipboard", "text")
    )
}

fn redacted_value(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        Value::String(format!("[redacted: {} chars]", text.chars().count()))
    } else {
        Value::String("[redacted]".to_string())
    }
}

fn arg_string(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn arg_char_count(args: &Map<String, Value>, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|text| text.chars().count())
}

fn element_target_display(args: &Map<String, Value>) -> String {
    if let Some(query) = arg_string(args, "query") {
        if args.get("index").is_some_and(|value| !value.is_null()) {
            return format!("{query} @ index {}", arg_display(args, "index"));
        }
        return query;
    }
    if args.get("index").is_some_and(|value| !value.is_null()) {
        return format!("index {}", arg_display(args, "index"));
    }
    "(missing query/index)".to_string()
}

fn drag_element_target_display(args: &Map<String, Value>) -> String {
    if let Some(query) = arg_string(args, "target_query") {
        if args
            .get("target_index")
            .is_some_and(|value| !value.is_null())
        {
            return format!("{query} @ index {}", arg_display(args, "target_index"));
        }
        return query;
    }
    if args
        .get("target_index")
        .is_some_and(|value| !value.is_null())
    {
        return format!("index {}", arg_display(args, "target_index"));
    }
    if args.get("target_x").is_some_and(|value| !value.is_null())
        && args.get("target_y").is_some_and(|value| !value.is_null())
    {
        return format!(
            "{},{}",
            arg_display(args, "target_x"),
            arg_display(args, "target_y")
        );
    }
    "(missing target)".to_string()
}

fn mouse_modifier_suffix(args: &Map<String, Value>) -> String {
    let Some(modifiers) = args.get("modifiers").and_then(Value::as_array) else {
        return String::new();
    };
    let labels = modifiers
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if labels.is_empty() {
        String::new()
    } else {
        format!(" modifiers=[{}]", labels.join(","))
    }
}

fn modifier_list_display(args: &Map<String, Value>) -> String {
    args.get("modifiers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join("+")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "(missing modifiers)".to_string())
}

fn coordinate_suffix(args: &Map<String, Value>) -> String {
    if args.get("x").is_some_and(|value| !value.is_null())
        && args.get("y").is_some_and(|value| !value.is_null())
    {
        format!(" at {},{}", arg_display(args, "x"), arg_display(args, "y"))
    } else {
        String::new()
    }
}

fn arg_display(args: &Map<String, Value>, key: &str) -> String {
    match args.get(key) {
        Some(Value::String(text)) if !text.trim().is_empty() => text.trim().to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Null) | None => "?".to_string(),
        Some(value) => value.to_string(),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ShellExitEvidence {
    pub exit_code: i32,
    pub pipeline_exit_codes: Vec<i32>,
}

pub(crate) fn shell_evidence_script(command: &str) -> String {
    // EXIT expands $? and PIPESTATUS together, before printf changes either.
    // The original command remains shell source, not eval'ed or reparsed.
    format!("trap 'printf \"%s\\n\" \"$?\" \"${{PIPESTATUS[*]}}\" > \"$NOLA_SHELL_EVIDENCE_PATH\"' EXIT\n{command}")
}

pub(crate) fn read_shell_exit_evidence(path: &std::path::Path, actual_exit: i32) -> Option<ShellExitEvidence> {
    use std::io::Read as _;
    let mut options=std::fs::OpenOptions::new();options.read(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK);}
    let file=options.open(path).ok()?;
    let metadata=file.metadata().ok()?;
    if !metadata.is_file() || metadata.len()>4096 {return None;}
    let mut raw=String::new();file.take(4097).read_to_string(&mut raw).ok()?;
    if raw.len()>4096 {return None;}
    let mut lines = raw.lines();
    let exit_code = lines.next()?.parse::<i32>().ok()?;
    let pipeline_exit_codes = lines.next()?.split_whitespace()
        .map(str::parse::<i32>).collect::<Result<Vec<_>, _>>().ok()?;
    (exit_code == actual_exit && !pipeline_exit_codes.is_empty()
        && pipeline_exit_codes.iter().all(|code| (0..=255).contains(code)))
        .then_some(ShellExitEvidence { exit_code, pipeline_exit_codes })
}

pub(crate) fn command_semantic(command: &str, exit_code: i32, evidence: Option<&ShellExitEvidence>) -> (bool, String) {
    if exit_code < 0 {
        return (true, "Process terminated without a numeric exit code".to_string());
    }
    if exit_code == 141 && bounded_pipeline_stage_count(command).is_some_and(|count| {
        evidence.is_some_and(|e| e.exit_code == exit_code && e.pipeline_exit_codes.len() == count
            && e.pipeline_exit_codes.last() == Some(&0)
            && e.pipeline_exit_codes.contains(&141)
            && e.pipeline_exit_codes.iter().all(|code| matches!(code, 0 | 141)))
    }) {
        return (
            false,
            "Upstream SIGPIPE after bounded pipeline consumer completed; verified PIPESTATUS, original exit code 141 retained".to_string(),
        );
    }
    // A search's exit=1 is benign only when no downstream stage failed.
    if evidence.is_some_and(|e| e.pipeline_exit_codes.iter().skip(1).any(|code| *code != 0)) {
        return (true, format!("Pipeline stage failed (exit={exit_code})"));
    }
    let head = command.split_whitespace().next().unwrap_or("");
    match head {
        "grep" | "rg" if exit_code == 1 => (false, "No matches found".to_string()),
        "grep" | "rg" => (exit_code >= 2, String::new()),
        "find" if exit_code == 1 => (false, "Some directories were inaccessible".to_string()),
        "find" => (exit_code >= 2, String::new()),
        "diff" if exit_code == 1 => (false, "Files differ".to_string()),
        "diff" => (exit_code >= 2, String::new()),
        "test" | "[" if exit_code == 1 => (false, "Condition is false".to_string()),
        _ => {
            if exit_code == 0 {
                (false, String::new())
            } else {
                (true, format!("Command failed with exit code {exit_code}"))
            }
        }
    }
}

fn bounded_pipeline_stage_count(command: &str) -> Option<usize> {
    // Deliberately conservative: only a simple pipeline with a final numeric
    // head limit. Quoted pipes are data; compounds/substitutions are not proof.
    let mut stages: Vec<Vec<String>> = vec![Vec::new()];
    let mut word = String::new();
    let mut quote = None;
    for ch in command.chars() {
        if matches!(ch, '$' | '`' | '\\' | '\n' | '\r') { return None; }
        if let Some(delimiter) = quote {
            if ch == delimiter { quote = None; } else { word.push(ch); }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ';' | '&' | '(' | ')' | '<' | '>' | '{' | '}' | '#' => return None,
            '|' => {
                if !word.is_empty() { stages.last_mut()?.push(std::mem::take(&mut word)); }
                if stages.last()?.is_empty() { return None; }
                stages.push(Vec::new());
            }
            ch if ch.is_whitespace() => {
                if !word.is_empty() { stages.last_mut()?.push(std::mem::take(&mut word)); }
            }
            _ => word.push(ch),
        }
    }
    if quote.is_some() { return None; }
    if !word.is_empty() { stages.last_mut()?.push(word); }
    let last = stages.last()?;
    let positive = |s: &str| s.parse::<u64>().is_ok_and(|n| n > 0);
    let bounded = match last.as_slice() {
        [head, limit] if head == "head" => limit.strip_prefix('-').is_some_and(positive),
        [head, flag, limit] if head == "head" && matches!(flag.as_str(), "-n" | "-c") => positive(limit),
        _ => false,
    };
    (stages.len() > 1 && bounded).then_some(stages.len())
}

#[cfg(test)]
mod command_semantic_tests {
    use super::{command_semantic, ShellExitEvidence};

    #[test]
    fn search_no_matches_does_not_hide_a_downstream_failure() {
        let evidence = ShellExitEvidence { exit_code: 1, pipeline_exit_codes: vec![0, 1] };
        assert!(command_semantic("grep pattern file | consumer", 1, Some(&evidence)).0);
        let evidence = ShellExitEvidence { exit_code: 1, pipeline_exit_codes: vec![1, 0] };
        let (failed, note) = command_semantic("grep pattern file | head -40", 1, Some(&evidence));
        assert!(!failed);
        assert_eq!(note, "No matches found");
    }


    #[test]
    fn sigpipe_from_head_is_informational() {
        let evidence = ShellExitEvidence { exit_code: 141, pipeline_exit_codes: vec![141, 0] };
        let (failed, note) = command_semantic("git diff -- src | head -160", 141, Some(&evidence));
        assert!(!failed);
        assert!(note.contains("bounded pipeline consumer"));
    }

    #[test]
    fn unrelated_sigpipe_remains_a_failure() {
        let (failed, _) = command_semantic("custom-producer | custom-consumer", 141, None);
        assert!(failed);
    }

    #[test]
    fn sigpipe_requires_actual_pipeline_evidence_not_head_in_source() {
        let evidence = ShellExitEvidence { exit_code: 141, pipeline_exit_codes: vec![141, 0] };
        for command in ["echo 'x | head -20'", "exit 141; echo hi | head -20", "producer | head -20 | false", "producer | head missing-file", "producer || head -20"] {
            assert!(command_semantic(command, 141, Some(&evidence)).0, "{command}");
        }
        assert!(command_semantic("producer | head -20", 141, None).0);
        for codes in [vec![2, 141, 0], vec![0, 141], vec![141], vec![141, 1]] {
            let e = ShellExitEvidence { exit_code: 141, pipeline_exit_codes: codes };
            assert!(command_semantic("producer | head -20", 141, Some(&e)).0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn bash_exit_evidence_preserves_pipeline_status_and_shell_exit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exit-evidence");
        for (command, code, success) in [("yes | head -20", 141, true), ("exit 141", 141, false), ("false", 1, false), ("printf ok", 0, true)] {
            let output = std::process::Command::new("bash")
                .args(["-o", "pipefail", "-c", &super::shell_evidence_script(command)])
                .env("NOLA_SHELL_EVIDENCE_PATH", &path).output().unwrap();
            assert_eq!(output.status.code(), Some(code));
            let evidence = super::read_shell_exit_evidence(&path, code).unwrap();
            assert_eq!(!command_semantic(command, code, Some(&evidence)).0, success);
            if command.contains("head") { assert_eq!(evidence.pipeline_exit_codes, vec![141, 0]); }
            assert!(super::read_shell_exit_evidence(&path, code + 1).is_none());
        }
        assert!(command_semantic("grep pattern file", -1, None).0);
    }
}

fn ok(text: String, display: String) -> ToolOutcome {
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text,
        display,
        error: false,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn ok_data(data: Value, display: String) -> ToolOutcome {
    ok(data.to_string(), display).with_data(data)
}

fn error_data(data: Value) -> ToolOutcome {
    let mut outcome = error(data.to_string()).with_data(data.clone());
    if data.get("tool_execution_skipped").and_then(Value::as_bool) == Some(true) {
        outcome.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
            state:nola_host_protocol::tool_result::ExecutionState::NotStarted,
            ..Default::default()
        });
    }
    outcome
}

fn error_data_with_display(data: Value, display: String) -> ToolOutcome {
    let mut outcome = error_data(data);
    outcome.display = display;
    outcome
}

fn system_time(_args: &Map<String, Value>) -> ToolOutcome {
    let local = Local::now();
    let utc = Utc::now();
    let offset_seconds = local.offset().local_minus_utc();
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let absolute_offset = offset_seconds.unsigned_abs();
    let offset = format!(
        "{sign}{:02}:{:02}",
        absolute_offset / 3_600,
        (absolute_offset % 3_600) / 60
    );
    let payload = json!({
        "local_time": local.to_rfc3339(),
        "utc_time": utc.to_rfc3339(),
        "utc_offset": offset,
        "unix_milliseconds": utc.timestamp_millis(),
    });
    ok_data(payload.clone(),
        format!("{} ({offset})", local.to_rfc3339()),
    )
}

fn error(text: String) -> ToolOutcome {
    let display = localize_tool_error_display(&text);
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        display: truncate(&display.replace('\n', " "), 160),
        text,
        error: true,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn error_with_display(text: String, display: String) -> ToolOutcome {
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        display: truncate(&display.replace('\n', " "), 160),
        text,
        error: true,
        artifacts: Vec::new(),
        activation: None,
    }
}

#[cfg(test)]
fn clamp_outcome(outcome: ToolOutcome) -> ToolOutcome {
    clamp_outcome_with_limit(outcome, MAX_TOOL_RESULT_CHARS)
}

#[cfg(test)]
fn clamp_outcome_with_limit(outcome: ToolOutcome, max_chars: usize) -> ToolOutcome {
    clamp_outcome_with_writer(outcome, max_chars, persist_large_tool_output)
}

fn tool_output_navigation_hint() -> String {
    // The path already lives in full_output_ref. Repeating it and the full
    // tool schema here can consume the entire bounded preview budget.
    "Partial evidence. Search full_output_ref with tool_output_search; read needed ranges with tool_output_read from preview_end_byte_offset, then next_offset. Exhaust pages only for exhaustive tasks; report gaps.".into()
}

/// Select execution diagnostics before any serialized-prefix truncation. The
/// saved payload is still exact JSON; selected fields are not a byte range of
/// that file, so they must never advertise a continuation byte offset.
fn project_shell_diagnostics(envelope: &mut Value, data: Option<&Value>, max_chars: usize) -> bool {
    let Some(data) = data.filter(|data| data["type"] == "shell_execution") else { return false; };
    let Some(object) = envelope.as_object_mut() else { return false; };
    object.remove("output_preview");
    object.remove("preview_end_byte_offset");
    object.remove("_nola_control");
    object.insert("preview_format".into(), json!("selected_shell_fields"));
    object.insert("continuation_hint".into(), json!("Selected diagnostics, not a contiguous file slice. Search full_output_ref with tool_output_search, or read JSON from offset 0 with tool_output_read. Do not rerun the command merely to recover omitted output."));
    for key in ["type", "state", "exit_code", "signal", "exit_evidence",
        "interruption_reason", "process_tree_stop", "stop_requested", "output_complete",
        "stdout_read_error", "stderr_read_error",
        "verification_status", "verification_error", "verification_message"] {
        if let Some(value) = data.get(key) { object.insert(key.into(), value.clone()); }
    }
    let stdout_unavailable = data["stdout"].is_null()
        && data.get("stdout_read_error").is_some_and(|v| !v.is_null());
    let stderr_unavailable = data["stderr"].is_null()
        && data.get("stderr_read_error").is_some_and(|v| !v.is_null());
    object.insert("stdout".into(), if stdout_unavailable { Value::Null } else { json!("") });
    object.insert("stderr".into(), if stderr_unavailable { Value::Null } else { json!("") });
    let overhead = envelope.to_string().chars().count();
    let available = max_chars.saturating_sub(overhead);
    let stdout = data["stdout"].as_str().unwrap_or_default();
    let stderr = data["stderr"].as_str().unwrap_or_default();
    fn encoded_len(text: &str) -> usize {
        serde_json::to_string(text).expect("JSON string").chars().count().saturating_sub(2)
    }
    fn bounded(text: &str, budget: usize) -> String {
        if encoded_len(text) <= budget { return text.into(); }
        let marker = "\n… omitted …\n";
        let available = budget.saturating_sub(encoded_len(marker));
        if budget < encoded_len(marker) { return String::new(); }
        let take = |reverse: bool, limit: usize| {
            let chars: Box<dyn Iterator<Item=char>> = if reverse { Box::new(text.chars().rev()) } else { Box::new(text.chars()) };
            let mut used = 0;
            let mut selected: Vec<char> = chars.take_while(|ch| {
                used += encoded_len(&ch.to_string()); used <= limit
            }).collect();
            if reverse { selected.reverse(); }
            selected.into_iter().collect::<String>()
        };
        format!("{}{marker}{}", take(false, available / 2), take(true, available - available / 2))
    }
    let stderr_budget = encoded_len(stderr).min(available / 2);
    let stdout_preview = bounded(stdout, available - stderr_budget);
    let stderr_preview = bounded(stderr, available.saturating_sub(encoded_len(&stdout_preview)));
    envelope["stdout"] = if stdout_unavailable {
        Value::Null
    } else { json!(stdout_preview) };
    envelope["stderr"] = if stderr_unavailable {
        Value::Null
    } else { json!(stderr_preview) };
    true
}

fn clamp_outcome_with_writer(
    mut outcome: ToolOutcome,
    max_chars: usize,
    persist: impl FnOnce(&str) -> std::io::Result<PathBuf>,
) -> ToolOutcome {
    // A short display string must not hide an oversized typed result. Persist
    // the actual payload before Core bounds it, so its reference remains useful.
    let ui_control = outcome.data.as_ref().and_then(|data| bounded_structured_output_preview(&data.to_string()))
        .or_else(|| bounded_structured_output_preview(&outcome.text));
    let browser_evidence = outcome.data.as_ref().and_then(|data| bounded_browser_evidence(data, max_chars / 4));
    outcome.text = nola_domain::tool_evidence::ModelToolEvidence::text(&outcome.text);
    let oversized_data = outcome.data.as_ref().map(|data|
        nola_domain::tool_evidence::ModelToolEvidence::from_host_output(data).into_value().to_string())
        .filter(|encoded| encoded.chars().count() > max_chars);
    let replace_oversized_data = oversized_data.is_some();
    if outcome.text.chars().count() > max_chars || oversized_data.is_some() {
        let original = oversized_data.unwrap_or_else(|| outcome.text.clone());
        let structured_preview = ui_control;
        let mut envelope = match persist(&original) {
            Ok(path) => {
                let path = path.to_string_lossy().into_owned();
                outcome.full_output_ref = Some(path.clone());
                let preview = original
                    .chars()
                    .take(max_chars.saturating_sub(500))
                    .collect::<String>();
                let preview_end_byte_offset = preview.len();
                json!({
                    "output_preview": preview,
                    "preview_end_byte_offset": preview_end_byte_offset,
                    "truncated": true,
                    "original_chars": original.chars().count(),
                    "original_bytes": original.len(),
                    "full_output_ref": path,
                    "_nola_control": structured_preview,
                    "continuation_hint": tool_output_navigation_hint(),
                })
            }
            Err(err) => {
                outcome.error = true;
                outcome.display = "Tool output preservation failed; execution may already have taken effect".into();
                json!({
                    "output_preview": original.chars().take(max_chars.saturating_sub(500)).collect::<String>(),
                    "truncated": true,
                    "full_output_available": false,
                    "original_chars": original.chars().count(),
                    "_nola_control": structured_preview,
                    "output_preservation_error": err.to_string(),
                    "recovery_hint": "The operation may already have taken effect. Inspect current state before retrying a mutation. Missing output is not evidence of no changes; do not claim complete coverage.",
                })
            }
        };
        if let Some(evidence) = browser_evidence { envelope["structured_preview"] = evidence; }
        // Include JSON escaping and navigation metadata in the initial preview
        // budget. Never mutate this projection on a subsequent model turn.
        if !project_shell_diagnostics(&mut envelope, outcome.data.as_ref(), max_chars) {
            let preview = envelope["output_preview"].as_str().unwrap_or_default().to_string();
            envelope["output_preview"] = json!("");
            let available = max_chars.saturating_sub(envelope.to_string().chars().count());
            let mut used = 0usize;
            let preview: String = preview.chars().take_while(|ch| {
                used = used.saturating_add(
                    serde_json::to_string(&ch.to_string()).expect("JSON character")
                        .chars().count().saturating_sub(2));
                used <= available
            }).collect();
            if envelope.get("preview_end_byte_offset").is_some() {
                envelope["preview_end_byte_offset"] = json!(preview.len());
            }
            envelope["output_preview"] = json!(preview);
        }
        // This envelope is Host-produced protocol data, not model/tool prose.
        // Keep it typed so Core sees output_preview/truncated directly instead
        // of {text:"{...}"}. Never decode arbitrary presentation text here.
        outcome.text = envelope.to_string();
        if outcome.data.is_none() || replace_oversized_data {
            outcome.data = Some(envelope);
        }
    }
    if full_output_ref_from_outcome(&outcome).is_some() {
        let navigation_capability_ids = [
            "tool_output_list",
            "tool_output_read",
            "tool_output_search",
            "tool_output_find",
        ];
        if let Some(activation) = outcome.activation.as_mut() {
            for capability_id in navigation_capability_ids {
                if !activation
                    .capability_ids
                    .iter()
                    .any(|existing| existing == capability_id)
                {
                    activation.capability_ids.push(capability_id.to_string());
                }
            }
        } else {
            outcome.activation = Some(nola_core::agent::host::HostCapabilityActivation {
                source: "full_output_ref".to_string(),
                capability_ids: navigation_capability_ids
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                capability_descriptors: Vec::new(),
            });
        }
    }
    outcome
}

#[cfg(test)]
#[test]
fn short_display_does_not_hide_large_typed_output_from_preservation() {
    let payload = json!({"text":"matches", "session_id":"fixture-session", "running":true,
        "rows":vec!["detail"; 2000]});
    let result = clamp_outcome_with_writer(ok_data(payload.clone(), "matches".into()), 4000, |text| {
        assert_eq!(serde_json::from_str::<Value>(text).unwrap(), payload);
        Ok(PathBuf::from("/fixture/saved-output.txt"))
    });
    assert_eq!(result.full_output_ref.as_deref(), Some("/fixture/saved-output.txt"));
    assert!(result.activation.is_some());
    let envelope: Value = serde_json::from_str(&result.text).unwrap();
    assert_eq!(envelope["truncated"], true);
    assert_eq!(result.data.as_ref(), Some(&envelope));
    assert!(result.data.as_ref().unwrap().get("rows").is_none());
    assert_eq!(envelope["_nola_control"]["session_id"], "fixture-session");
    assert_eq!(envelope["_nola_control"]["running"], true);
    assert!(result.text.chars().count() <= 4000);
}

#[cfg(test)]
#[test]
fn large_presentation_preserves_small_typed_control_data() {
    let data = json!({"session_id":"fixture-session", "running":true});
    let mut outcome = ok_data(data.clone(), "running".into());
    outcome.text = "large presentation".repeat(500);
    let result = clamp_outcome_with_writer(outcome, 1000, |_| {
        Ok(PathBuf::from("/fixture/saved-output.txt"))
    });
    assert_eq!(result.data, Some(data));
    assert_eq!(result.full_output_ref.as_deref(), Some("/fixture/saved-output.txt"));
}

#[cfg(test)]
#[test]
fn output_preservation_failure_cannot_masquerade_as_complete_success() {
    let result = clamp_outcome_with_writer(ok("x".repeat(3_000), "executed".into()), 1_000, |_| {
        Err(std::io::Error::other("fixture disk failure"))
    });
    assert!(result.error);
    assert!(full_output_ref_from_outcome(&result).is_none());
    assert!(result.activation.is_none());
    let output: Value = serde_json::from_str(&result.text).unwrap();
    assert_eq!(output["full_output_available"], false);
    assert_eq!(output["truncated"], true);
    assert!(output["recovery_hint"].as_str().unwrap().contains("before retrying"));
    assert_eq!(result.data, Some(output));
}

#[cfg(test)]
#[test]
fn large_text_output_is_a_typed_canonical_preview_without_double_json() {
    let original = "工作区修改\n".repeat(500);
    let outcome = clamp_outcome_with_writer(ok(original.clone(), "Workspace status".into()), 700, |text| {
        assert_eq!(text, original);
        Ok(PathBuf::from("/fixture/tool-output.txt"))
    });
    let envelope = outcome.data.as_ref().expect("typed preservation envelope");
    assert_eq!(envelope["truncated"], true);
    assert!(envelope.get("text").is_none());
    let preview = envelope["output_preview"].as_str().unwrap();
    assert!(!preview.is_empty(), "navigation metadata must leave room for evidence");
    assert!(outcome.text.chars().count() <= 700);
    let offset = envelope["preview_end_byte_offset"].as_u64().unwrap() as usize;
    assert_eq!(preview.as_bytes(), &original.as_bytes()[..offset]);
    assert_eq!(outcome.full_output_ref.as_deref(), Some("/fixture/tool-output.txt"));
    let core = nola_host_protocol::tool_result::ToolResult {
        contract: nola_host_protocol::tool_result::ToolResultContract::V1,
        presentation: nola_host_protocol::tool_result::ToolPresentation { text: outcome.text.clone(), summary: outcome.display.clone() },
        data: outcome.data.clone(),
        execution: Default::default(),
        full_output_ref: outcome.full_output_ref.clone(),
        memory_usage: None,
    }.into_core_output();
    assert_eq!(core["output_preview"], envelope["output_preview"]);
    assert_eq!(core["full_output_ref"], "/fixture/tool-output.txt");
    assert!(core.get("text").is_none());
}

#[cfg(test)]
#[test]
fn clamping_large_discovery_output_preserves_discovered_capabilities() {
    let outcome = ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: "x".repeat(2_000),
        display: "large discovery".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: Some(nola_core::agent::host::HostCapabilityActivation {
            source: "tool_search".to_string(),
            capability_ids: vec!["codebase_search".to_string()],
            capability_descriptors: Vec::new(),
        }),
    };

    let bounded = clamp_outcome_with_limit(outcome, 1_000);
    let activation = bounded.activation.expect("typed activation");

    assert_eq!(activation.source, "tool_search");
    assert!(activation
        .capability_ids
        .iter()
        .any(|capability_id| capability_id == "codebase_search"));
    assert!(activation
        .capability_ids
        .iter()
        .any(|capability_id| capability_id == "tool_output_read"));
}

/// Keeps small control-plane fields available when a large structured tool
/// result is moved out of line. UI clients must not have to parse the saved
/// full payload merely to recover a browser address or session identity.
fn bounded_structured_output_preview(text: &str) -> Option<Value> {
    let Value::Object(object) = serde_json::from_str::<Value>(text).ok()? else {
        return None;
    };
    let mut preview = Map::new();
    for key in [
        "target",
        "session_id",
        "running",
        "persistent",
        "headless",
        "preview_url",
        "url",
        "final_url",
        "title",
        "active_index",
        "page_count",
        "revision",
    ] {
        if let Some(value) = object.get(key).filter(|value| {
            matches!(value, Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
        }) {
            preview.insert(key.to_string(), value.clone());
        }
    }
    if let Some(Value::Array(pages)) = object.get("pages") {
        let pages = pages
            .iter()
            .take(20)
            .filter_map(|page| {
                let Value::Object(page) = page else { return None };
                let mut bounded = Map::new();
                for key in ["index", "title", "url", "active"] {
                    if let Some(value) = page.get(key).filter(|value| {
                        matches!(value, Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
                    }) {
                        bounded.insert(key.to_string(), value.clone());
                    }
                }
                (!bounded.is_empty()).then_some(Value::Object(bounded))
            })
            .collect::<Vec<_>>();
        if !pages.is_empty() {
            preview.insert("pages".to_string(), Value::Array(pages));
        }
    }
    (!preview.is_empty()).then_some(Value::Object(preview))
}

/// A small actionable region, not another rendering of the full document.
/// The full output reference remains the source for unreturned regions.
fn bounded_browser_evidence(output: &Value, budget: usize) -> Option<Value> {
    let fields = output.as_object()?;
    let elements_key = if fields.contains_key("elements") { "elements" }
        else if fields.contains_key("interactive_elements") { "interactive_elements" }
        else { return None };
    let elements = fields.get(elements_key)?.as_array()?;
    let mut result = json!({"type":"browser_observation_region","elements":[],"elements_total":elements.len(),
        "elements_json_pointer":format!("/{elements_key}")});
    for key in ["target", "session_id", "generation", "revision", "url", "final_url", "title"] {
        if let Some(value) = fields.get(key) {
            let value = match value { Value::String(text) if key == "title" => json!(text.chars().take(256).collect::<String>()),
                Value::String(_) => value.clone(),
                Value::Number(_) | Value::Bool(_) | Value::Null => value.clone(), _ => continue };
            let mut candidate = result.clone(); candidate[key] = value;
            if candidate.to_string().chars().count() <= budget { result = candidate; }
        }
    }
    for element in elements {
        let Some(element) = element.as_object() else { continue };
        let mut entry = Map::new();
        for key in ["ref", "selector", "role", "name", "text", "disabled", "checked"] {
            if let Some(value) = element.get(key) {
                let value = match value { Value::String(text) if matches!(key, "name" | "text") => json!(text.chars().take(160).collect::<String>()),
                    Value::String(_) => value.clone(),
                    Value::Bool(_) | Value::Number(_) => value.clone(), _ => continue };
                entry.insert(key.into(), value);
            }
        }
        let mut candidate = result.clone();
        candidate["elements"].as_array_mut()?.push(Value::Object(entry));
        if candidate.to_string().chars().count() > budget { break; }
        result = candidate;
    }
    (result.to_string().chars().count() <= budget).then_some(result)
}

pub(crate) fn full_output_ref_from_outcome(outcome: &ToolOutcome) -> Option<String> {
    outcome.full_output_ref.clone()
}

fn bounded_code_intelligence_provenance(output: &Value) -> Option<Value> {
    let provenance = output.get("_nola_code_intelligence_provenance")?;
    let encoded = serde_json::to_string(provenance).ok()?;
    if encoded.chars().count() <= 2_000 {
        return Some(provenance.clone());
    }
    let object = provenance.as_object()?;
    let mut bounded = Map::new();
    for key in [
        "operation",
        "repository_root",
        "current_repository_revision",
        "provider_repository_revision",
        "freshness",
        "requires_current_state_verification",
        "reasons",
    ] {
        if let Some(value) = object.get(key) {
            bounded.insert(key.to_string(), value.clone());
        }
    }
    bounded.insert(
        "fact_count".to_string(),
        Value::from(
            object
                .get("facts")
                .and_then(Value::as_array)
                .map_or(0, Vec::len) as u64,
        ),
    );
    bounded.insert("facts_available_in_full_output".to_string(), Value::Bool(true));
    Some(Value::Object(bounded))
}

fn clamp_external_output_preview(envelope: &mut Value) {
    loop {
        if envelope.get("preview_end_byte_offset").is_some() {
            envelope["preview_end_byte_offset"] = json!(envelope["output_preview"]
                .as_str().map_or(0, str::len));
        }
        let encoded_chars = serde_json::to_string(envelope)
            .map_or(0, |value| value.chars().count());
        if encoded_chars <= MAX_TOOL_RESULT_CHARS {
            return;
        }
        let Some(preview) = envelope
            .get("output_preview")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let preview_chars = preview.chars().count();
        if preview_chars == 0 {
            return;
        }
        let keep = preview_chars.saturating_sub(encoded_chars - MAX_TOOL_RESULT_CHARS);
        envelope["output_preview"] = Value::String(preview.chars().take(keep).collect());
    }
}

pub(crate) fn bound_external_tool_output_in_store(output: Value, store: &ToolOutputStore) -> (Value, Option<String>) {
    let code_intelligence_provenance = bounded_code_intelligence_provenance(&output);
    let evidence = nola_domain::tool_evidence::ModelToolEvidence::from_host_output(&output).into_value();
    let serialized = serde_json::to_string_pretty(&evidence).unwrap_or_else(|_| evidence.to_string());
    let character_count = serialized.chars().count();
    if character_count <= MAX_TOOL_RESULT_CHARS {
        return (output, None);
    }
    match store.persist(&serialized) {
        Ok(path) => {
            let path = path.to_string_lossy().into_owned();
            let provenance_chars = code_intelligence_provenance
                .as_ref()
                .and_then(|value| serde_json::to_string(value).ok())
                .map_or(0, |value| value.chars().count());
            let preview = serialized
                .chars()
                .take(MAX_TOOL_RESULT_CHARS.saturating_sub(500 + provenance_chars))
                .collect::<String>();
            let mut bounded = json!({
                "output_preview": preview,
                "preview_end_byte_offset": preview.len(),
                "truncated": true,
                "original_chars": character_count,
                "original_bytes": serialized.len(),
                "full_output_ref": path,
                "continuation_hint": tool_output_navigation_hint(),
            });
            if let Some(region) = bounded_browser_evidence(&evidence, MAX_TOOL_RESULT_CHARS / 4) {
                bounded["structured_preview"] = region;
            }
            if let (Some(provenance), Some(object)) =
                (code_intelligence_provenance, bounded.as_object_mut())
            {
                object.insert("_nola_code_intelligence_provenance".to_string(), provenance);
            }
            if !project_shell_diagnostics(&mut bounded, Some(&output), MAX_TOOL_RESULT_CHARS) {
                clamp_external_output_preview(&mut bounded);
            }
            (bounded, Some(path))
        }
        Err(_) => {
            let provenance_chars = code_intelligence_provenance
                .as_ref()
                .and_then(|value| serde_json::to_string(value).ok())
                .map_or(0, |value| value.chars().count());
            let mut bounded = json!({
                "output_preview": serialized
                    .chars()
                    .take(MAX_TOOL_RESULT_CHARS.saturating_sub(500 + provenance_chars))
                    .collect::<String>(),
                "truncated": true,
                "original_chars": character_count,
                "original_bytes": serialized.len(),
                "full_output_available": false,
                "output_preservation_error": "tool_output_preservation_failed",
                "recovery_hint": "The operation may already have taken effect. Inspect current state before retrying a mutation. Missing output is not evidence of no changes; do not claim complete coverage.",
            });
            if let (Some(mut provenance), Some(object)) =
                (code_intelligence_provenance, bounded.as_object_mut())
            {
                if let Some(provenance) = provenance.as_object_mut() {
                    provenance.insert(
                        "facts_available_in_full_output".to_string(),
                        Value::Bool(false),
                    );
                }
                object.insert("_nola_code_intelligence_provenance".to_string(), provenance);
            }
            if !project_shell_diagnostics(&mut bounded, Some(&output), MAX_TOOL_RESULT_CHARS) {
                clamp_external_output_preview(&mut bounded);
            }
            (bounded, None)
        }
    }
}

#[cfg(test)]
#[test]
fn shell_unknown_snapshot_preserves_read_failure_and_receipt_through_bounding() {
    let data = json!({"type":"shell_execution", "state":"unknown",
        "interruption_reason":"timeout", "process_tree_stop":"unknown",
        "stop_requested":true, "output_complete":false,
        "stdout":"中😀\n".repeat(4_000), "stderr":null,
        "stderr_read_error":"capture unavailable"});
    let mut input = error_with_display("timed out".into(), "timed out".into());
    input.data = Some(data.clone());
    input.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
        state:nola_host_protocol::tool_result::ExecutionState::Unknown,
        ..Default::default()
    });
    let result = clamp_outcome_with_writer(input, 2000, |saved| {
        assert_eq!(serde_json::from_str::<Value>(saved).unwrap(), data);
        Ok(PathBuf::from("/fixture/interrupted-command.json"))
    });
    assert!(result.error);
    assert!(result.text.chars().count() <= 2000);
    assert_eq!(result.execution.unwrap().state, nola_host_protocol::tool_result::ExecutionState::Unknown);
    let output = result.data.unwrap();
    assert_eq!(output["state"], "unknown");
    assert_eq!(output["process_tree_stop"], "unknown");
    assert_eq!(output["output_complete"], false);
    assert!(output["stderr"].is_null());
    assert_eq!(output["stderr_read_error"], "capture unavailable");
    assert_eq!(output["full_output_ref"], "/fixture/interrupted-command.json");
    assert!(output.get("preview_end_byte_offset").is_none());
}

#[cfg(test)]
#[test]
fn shell_projection_preserves_diagnostics_before_host_and_core_bounding() {
    let data = json!({"type":"shell_execution", "state":"completed", "exit_code":1,
        "exit_evidence":{"exit_code":1,"pipeline_exit_codes":[0,1]},
        "command":"irrelevant command source".repeat(2000),
        "stdout":format!("output-start\n{}\noutput-end", "中间内容\\\"\n".repeat(2000)),
        "stderr":"actual failure diagnostic", "signal":null});
    let result = clamp_outcome_with_writer(ok_data(data.clone(), "command failed".into()), 2000, |saved| {
        assert_eq!(serde_json::from_str::<Value>(saved).unwrap(), data);
        Ok(PathBuf::from("/fixture/exact-command.json"))
    });
    let output = result.data.as_ref().unwrap();
    assert!(result.text.chars().count() <= 2000);
    assert_eq!(output["exit_code"], 1);
    assert_eq!(output["exit_evidence"]["pipeline_exit_codes"], json!([0,1]));
    assert_eq!(output["stderr"], "actual failure diagnostic");
    let stdout = output["stdout"].as_str().unwrap();
    assert!(stdout.starts_with("output-start"));
    assert!(stdout.ends_with("output-end"));
    assert!(output.get("command").is_none());
    assert!(output.get("preview_end_byte_offset").is_none());
    let core = nola_host_protocol::tool_result::ToolResult {
        contract: nola_host_protocol::tool_result::ToolResultContract::V1,
        presentation: nola_host_protocol::tool_result::ToolPresentation {text:result.text, summary:result.display},
        data:result.data, execution:Default::default(), full_output_ref:result.full_output_ref, memory_usage:None,
    }.into_core_output();
    assert_eq!(core["stderr"], "actual failure diagnostic");
    assert_eq!(core["full_output_ref"], "/fixture/exact-command.json");
}

#[cfg(test)]
#[test]
fn external_shell_projection_and_preservation_failure_keep_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let data = json!({"type":"shell_execution","exit_code":0,"state":"completed",
        "command":"large script".repeat(2000),"stdout":"constraint still fails", "stderr":""});
    let store = ToolOutputStore::new(root.path().join("output"));
    let (bounded, reference) = bound_external_tool_output_in_store(data.clone(), &store);
    assert!(reference.is_some());
    assert_eq!(bounded["stdout"], "constraint still fails");
    assert!(bounded.get("preview_end_byte_offset").is_none());
    assert!(bounded.to_string().chars().count() <= MAX_TOOL_RESULT_CHARS);
    let failed = clamp_outcome_with_writer(ok_data(data, "executed".into()), 2000, |_| Err(std::io::Error::other("fixture")));
    assert!(failed.error);
    assert_eq!(failed.data.as_ref().unwrap()["stdout"], "constraint still fails");
    assert_eq!(failed.data.as_ref().unwrap()["full_output_available"], false);
}

#[cfg(test)]
#[test]
fn external_output_preservation_failure_is_not_evidence_of_no_effect() {
    // A file cannot be the store directory; this is deterministic even when
    // tests run as a privileged user and does not alter the real output store.
    let unavailable_root = tempfile::NamedTempFile::new().unwrap();
    let store = ToolOutputStore::new(unavailable_root.path().to_path_buf());
    let (bounded, reference) = bound_external_tool_output_in_store(json!({
        "content": "x".repeat(MAX_TOOL_RESULT_CHARS + 1),
        "_nola_code_intelligence_provenance": {
            "freshness": "requires_verification",
            "requires_current_state_verification": true,
            "facts_available_in_full_output": true
        }
    }), &store);

    assert!(reference.is_none());
    assert!(bounded.get("full_output_ref").is_none());
    assert!(bounded.get("preview_end_byte_offset").is_none());
    assert_eq!(bounded["truncated"], true);
    assert_eq!(bounded["full_output_available"], false);
    assert_eq!(bounded["output_preservation_error"], "tool_output_preservation_failed");
    assert_eq!(bounded["_nola_code_intelligence_provenance"]["facts_available_in_full_output"], false);
    assert_eq!(bounded["_nola_code_intelligence_provenance"]["requires_current_state_verification"], true);
    assert!(bounded["recovery_hint"].as_str().unwrap().contains("before retrying a mutation"));
    assert!(!bounded["output_preview"].as_str().unwrap().is_empty());
    assert!(bounded.to_string().chars().count() <= MAX_TOOL_RESULT_CHARS);
}

#[cfg(test)]
#[test]
fn bounded_code_intelligence_output_preserves_a_bounded_freshness_contract() {
    let facts = (0..64)
        .map(|index| json!({
            "repository_root": "/workspace",
            "path": format!("src/very/long/module/{index}/implementation.rs"),
            "symbol": format!("crate::module::{index}::implementation_symbol"),
            "path_exists": true,
            "current_content_sha256": "a".repeat(64),
            "provider_content_sha256": "a".repeat(64),
            "evidence_matches_current_source": true,
            "symbol_matches_current_source": true,
        }))
        .collect::<Vec<_>>();
    let (bounded, full_output_ref) = bound_external_tool_output(json!({
        "matches": [{"path": "src/lib.rs", "snippet": "x".repeat(MAX_TOOL_RESULT_CHARS + 1)}],
        "_nola_code_intelligence_provenance": {
            "operation": "search",
            "repository_root": "/workspace",
            "current_repository_revision": "current",
            "provider_repository_revision": "provider",
            "facts": facts,
            "freshness": "requires_verification",
            "requires_current_state_verification": true,
            "reasons": ["provider_snapshot_is_not_verifiably_current"]
        },
    }));

    let provenance = &bounded["_nola_code_intelligence_provenance"];
    assert_eq!(provenance["freshness"], "requires_verification");
    assert_eq!(provenance["requires_current_state_verification"], true);
    assert_eq!(provenance["fact_count"], 64);
    assert_eq!(provenance["facts_available_in_full_output"], true);
    assert!(provenance.get("facts").is_none());
    assert_eq!(bounded["truncated"], true);
    assert!(full_output_ref.is_some());
    assert!(serde_json::to_string(&bounded).unwrap().chars().count() <= MAX_TOOL_RESULT_CHARS);
}

#[cfg(test)]
#[test]
fn web_search_result_limit_preserves_a_model_ready_report() {
    let report = ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: "e".repeat(6_000),
        display: "search".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    };
    let web = clamp_outcome_with_limit(report.clone(), MAX_WEB_SEARCH_RESULT_CHARS);
    let generic = clamp_outcome(report);
    assert_eq!(web.text.chars().count(), 6_000);
    assert!(web.full_output_ref.is_none());
    assert!(generic.full_output_ref.is_some());
    assert_eq!(generic.data.as_ref().unwrap()["truncated"], true);
    assert_eq!(generic.data.as_ref().unwrap()["full_output_ref"].as_str(), generic.full_output_ref.as_deref());
}

#[cfg(test)]
#[test]
fn web_fetch_uses_the_inline_web_evidence_budget() {
    let fetched_page = ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: "e".repeat(6_000),
        display: "fetch".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    };
    let web = clamp_outcome_with_limit(fetched_page.clone(), MAX_WEB_SEARCH_RESULT_CHARS);
    let generic = clamp_outcome(fetched_page);
    assert_eq!(web.text.chars().count(), 6_000);
    assert!(web.full_output_ref.is_none());
    assert!(generic.full_output_ref.is_some());
    assert_eq!(generic.data.as_ref().unwrap()["truncated"], true);
    assert_eq!(generic.data.as_ref().unwrap()["full_output_ref"].as_str(), generic.full_output_ref.as_deref());
}

#[cfg(test)]
#[test]
fn large_external_tool_output_has_a_typed_full_output_reference() {
    let original = json!({"content": "中文\n\"".repeat(MAX_TOOL_RESULT_CHARS)});
    let serialized = serde_json::to_string_pretty(&original).unwrap();
    let (bounded, full_output_ref) = bound_external_tool_output(original);

    assert_eq!(bounded["truncated"], true);
    assert_eq!(
        bounded["full_output_ref"].as_str(),
        full_output_ref.as_deref()
    );
    assert!(full_output_ref.is_some());
    let preview = bounded["output_preview"].as_str().unwrap();
    let offset = bounded["preview_end_byte_offset"].as_u64().unwrap() as usize;
    assert_eq!(preview.as_bytes(), &serialized.as_bytes()[..offset]);
    assert_eq!(bounded["original_bytes"], serialized.len());
    assert!(bounded.to_string().chars().count() <= MAX_TOOL_RESULT_CHARS);
}

#[cfg(test)]
#[test]
fn external_preview_offset_tracks_post_escape_budget_trimming() {
    let original = "中文\"\n".repeat(MAX_TOOL_RESULT_CHARS);
    let mut envelope = json!({"output_preview":original,
        "preview_end_byte_offset":original.len(), "truncated":true});
    clamp_external_output_preview(&mut envelope);
    let preview = envelope["output_preview"].as_str().unwrap();
    assert!(preview.len() < original.len());
    let offset = envelope["preview_end_byte_offset"].as_u64().unwrap() as usize;
    assert_eq!(preview.as_bytes(), &original.as_bytes()[..offset]);
    assert!(envelope.to_string().chars().count() <= MAX_TOOL_RESULT_CHARS);
}

#[cfg(test)]
#[test]
fn clamped_browser_output_preserves_preview_control_plane_fields() {
    let outcome = clamp_outcome(ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: json!({
            "accessibility": "x".repeat(MAX_TOOL_RESULT_CHARS + 1_000),
            "target": "managed",
            "session_id": "nola-conversation-1",
            "preview_url": "http://127.0.0.1:4321/view?token=test",
            "final_url": "https://example.com/current",
            "title": "Current page",
            "pages": [{
                "index": 0,
                "title": "Current page",
                "url": "https://example.com/current",
                "active": true
            }]
        })
        .to_string(),
        display: "browser_managed_navigate: Current page".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    });
    let value: Value = serde_json::from_str(&outcome.text).expect("bounded JSON");

    assert_eq!(value["truncated"], true);
    assert!(
        outcome
            .text
            .chars()
            .take(1_000)
            .collect::<String>()
            .contains("http://127.0.0.1:4321/view?token=test"),
        "control-plane metadata must survive a second bounded Core preview"
    );
    assert_eq!(
        value["_nola_control"]["preview_url"],
        "http://127.0.0.1:4321/view?token=test"
    );
    assert_eq!(
        value["_nola_control"]["final_url"],
        "https://example.com/current"
    );
    assert_eq!(value["_nola_control"]["pages"][0]["active"], true);
}

#[cfg(test)]
#[test]
fn clamped_local_output_exposes_the_saved_path_structurally() {
    let outcome = clamp_outcome(ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: "x".repeat(MAX_TOOL_RESULT_CHARS + 1_000),
        display: "large".to_string(),
        error: false,
        artifacts: Vec::new(),
        activation: None,
    });

    let full_output_ref = full_output_ref_from_outcome(&outcome).expect("saved output path");
    assert!(outcome.text.contains(&full_output_ref));
    let activation = outcome.activation.expect("output continuation activation");
    assert_eq!(activation.source, "full_output_ref");
    assert!(activation
        .capability_ids
        .contains(&"tool_output_search".to_string()));
}

#[cfg(test)]
#[test]
fn capability_activation_is_structured_and_not_embedded_in_tool_text() {
    let outcome = ok(
        "model-visible tool description".to_string(),
        "1 match".to_string(),
    )
    .with_capability_activation(vec!["workspace_status".to_string()]);
    assert!(!outcome.text.contains("nola_capability_activation:"));
    let activation = outcome.activation.expect("typed capability activation");
    assert_eq!(activation.source, "tool_search");
    assert_eq!(activation.capability_ids, vec!["workspace_status"]);
}

#[cfg(test)]
fn persist_large_tool_output(text: &str) -> std::io::Result<PathBuf> {
    output_store::ToolOutputStore::new(tool_output_root()).persist(text)
}

#[cfg(test)]
fn bound_external_tool_output(output: Value) -> (Value, Option<String>) {
    bound_external_tool_output_in_store(output, &ToolOutputStore::for_test())
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
mod permission_transport_input_tests {
    use super::*;
    use nola_host_protocol::PermissionTransportInterruptionReason::{Disconnected, Unavailable};

    #[test]
    fn bounded_native_permission_reader_preserves_explicit_deny_and_reports_disconnect() {
        assert_eq!(read_native_permission_response(io::Cursor::new(b"")), Err(Disconnected));
        assert_eq!(read_native_permission_response(io::Cursor::new(b"broken json\n")), Err(Unavailable));
        let denied = read_native_permission_response(io::Cursor::new(
            b"{\"resolution\":{\"type\":\"authorization\",\"decision\":\"deny\"}}\n"
        )).unwrap();
        assert_eq!(denied.pointer("/resolution/decision").and_then(Value::as_str), Some("deny"));
        let mut reader = io::Cursor::new(vec![b'x'; 1_000_000]);
        assert_eq!(read_native_permission_response(&mut reader), Err(Unavailable));
        assert_eq!(reader.position(), 32 * 1024 + 1);
        struct BrokenReader;
        impl io::Read for BrokenReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "isolated broken transport"))
            }
        }
        assert_eq!(read_native_permission_response(io::BufReader::new(BrokenReader)), Err(Disconnected));
    }

    #[test]
    fn resolver_transport_interruption_reaches_core_and_legacy_error_is_not_denial() {
        let request = ToolRequest { id: "pending-request".into(), name: "write_file".into(), args: Default::default() };
        let resolver: HostPermissionResolver = std::sync::Arc::new(|_, _, _| HostPermissionResolution::TransportInterrupted(Disconnected));
        assert_eq!(present_agent_core_permission_with_resolver(&request, Some(resolver), &[], true),
            HostPermissionResolution::TransportInterrupted(Disconnected));
        let resolver_guard = enter_host_permission_resolver(
            std::sync::Arc::new(|_, _, _| HostPermissionResolution::TransportInterrupted(Disconnected))
        );
        let direct = confirm_tool(&request);
        drop(resolver_guard);
        assert_eq!(direct, ToolPermissionDecision::TransportInterrupted(Disconnected));
        let outcome = permission_transport_interrupted_outcome(Disconnected);
        assert!(outcome.error);
        assert!(outcome.text.contains("transport interrupted"));
        assert!(!outcome.text.contains("denied by user"));
    }
}

#[cfg(test)]
mod controlled_walk_regressions {
    use super::*;
    #[test]
    fn cancellation_stops_before_visiting_and_generated_directories_are_skipped() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("target")).unwrap();
        fs::write(root.path().join("target/generated.rs"), "x").unwrap();
        fs::write(root.path().join("source.rs"), "x").unwrap();
        let mut visited = 0;
        walk_files_controlled(root.path(), 10, &mut |_| { visited += 1; true }, &|| true);
        assert_eq!(visited, 0);
        walk_files(root.path(), 10, &mut |_| { visited += 1; true });
        assert_eq!(visited, 1);
    }
    #[test]
    fn cancelled_code_search_returns_a_failure_not_partial_success() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("source.rs"), "fn source() {}").unwrap();
        let control = crate::runtime_control::RuntimeExecutionControl::default();
        control.request();
        assert!(code_search_matches(root.path(), "source", "text", &glob_regex("**").unwrap(), "", false, false, 0, "", 10, &control).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn directory_symlink_cycle_does_not_escape_or_repeat() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("source.rs"), "x").unwrap();
        std::os::unix::fs::symlink(root.path(), root.path().join("cycle")).unwrap();
        let mut visited = 0;
        walk_files(root.path(), 10, &mut |_| { visited += 1; true });
        assert_eq!(visited, 1);
    }
}
