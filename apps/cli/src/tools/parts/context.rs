fn read_file(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    if let Ok(reference) = ctx.output_store.resolve(Path::new(&path)) {
        let mut outcome = error_data_with_display(json!({
            "error": "saved_result_requires_output_reader",
            "tool_execution_skipped": true,
            "message": "This reference belongs to the current session's saved tool results, not the workspace. Do not request workspace access. Read the required byte range with tool_output_read, or locate evidence with tool_output_search.",
            "recovery": {"capability": "tool_output_read", "arguments": {"path": reference, "offset": 0, "max_bytes": MAX_TOOL_OUTPUT_SLICE_BYTES}},
        }), "Use the saved-result reader".to_string());
        outcome.activation = Some(nola_core::agent::host::HostCapabilityActivation {
            // This is a Host-owned resource boundary rejection. Preserve its
            // recovery schemas even though the requested read was rejected.
            source: "host_resource_preservation".to_string(),
            capability_ids: vec!["tool_output_read".into(), "tool_output_search".into()],
            capability_descriptors: Vec::new(),
        });
        return outcome;
    }
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_file() {
        return error(format!("Not a file: {}", target.display()));
    }
    // Resolve authorization before inspecting or snapshotting media. Retain
    // bytes, not a mutable workspace path, for subsequent turns and recovery.
    let mut header = [0u8; 12];
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut media_file = tool_try!(options.open(&target).map_err(|err| error(format!("Read failed: {err}"))));
    if !tool_try!(media_file.metadata().map_err(|err| error(err.to_string()))).is_file() {
        return error("Read requires a regular file".into());
    }
    let count = tool_try!(media_file.read(&mut header).map_err(|err| error(format!("Read failed: {err}"))));
    let header = &header[..count];
    if header.starts_with(b"%PDF-") && args.contains_key("pdf_page") {
        let page = tool_try!(args.get("pdf_page").and_then(Value::as_u64)
            .filter(|page| (1..=500).contains(page))
            .ok_or_else(|| error("pdf_page must be an integer from 1 to 500".into())));
        return render_pdf_page(&target, page, ctx);
    }
    let mime = if header.starts_with(b"\x89PNG\r\n\x1a\n") { Some("image/png") }
        else if header.starts_with(&[0xff, 0xd8, 0xff]) { Some("image/jpeg") }
        else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") { Some("image/gif") }
        else if header.starts_with(b"RIFF") && header.get(8..12) == Some(b"WEBP") { Some("image/webp") }
        else if header.starts_with(b"%PDF-") && args.get("visual").and_then(Value::as_bool) == Some(true) { Some("application/pdf") }
        else { None };
    if let Some(mime) = mime {
        const LIMIT: u64 = 30 * 1024 * 1024;
        let mut bytes = header.to_vec();
        tool_try!(media_file.take(LIMIT + 1 - count as u64).read_to_end(&mut bytes).map_err(|err| error(err.to_string())));
        if bytes.is_empty() || bytes.len() as u64 > LIMIT {
            return error("Media must be nonempty and at most 30 MiB; select a smaller image or PDF page range externally.".into());
        }
        let snapshot = tool_try!(ctx.output_store.persist_media(&bytes).map_err(|err| error(format!("Media snapshot failed: {err}"))));
        let mut outcome = ok(
            format!("Read {} as {mime} ({} bytes). Visual content is attached when the selected model supports it; text extraction is not visual inspection.", target.display(), bytes.len()),
            format!("Read {}", target.display()),
        ).with_file_artifact(&snapshot, "read");
        if let Some(artifact) = outcome.artifacts.last_mut() {
            artifact.mime_type = Some(mime.into());
            artifact.name = target.file_name().unwrap_or_default().to_string_lossy().into_owned();
        }
        return outcome;
    }
    let show_line_numbers = match args.get("show_line_numbers") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("show_line_numbers must be a boolean".to_string()),
    };
    let default_max_bytes = if args.contains_key("start_line")
        || args.contains_key("line_count")
        || args.contains_key("max_bytes")
    {
        200_000
    } else {
        LARGE_FILE_AUTO_PREVIEW_BYTES
    };
    let observed = tool_try!(read_file_text(
        &target,
        args,
        show_line_numbers,
        default_max_bytes,
        FileReadPresentationBudget::direct()
    ));
    if observed.presentation_exhausted {
        return error("File-read headers/continuation exceed the presentation budget; select a shorter authorized path or a bounded source inspection method. No file content was returned.".into());
    }
    let data = json!({
        "path": target.to_string_lossy(),
        "content": observed.raw_content,
        "content_kind": if observed.raw_content.is_some() { "utf8_source" } else { "extracted_document" },
        "next_start_line": observed.next_start_line,
        "complete": observed.observation.as_ref().is_some_and(|observation|
            matches!(observation.coverage, FileReadCoverage::Full)),
    });
    let text = observed.text;
    record_returned_file_observations(&text, observed.observation);
    ok(
        text,
        format!("{} {}", tr(Message::DisplayReadPrefix), target.display()),
    ).with_data(data)
}

fn read_many_files(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let files = match args.get("files").and_then(Value::as_array) {
        Some(value) if !value.is_empty() => value,
        _ => return error("files must be a non-empty array".to_string()),
    };
    if files.len() > MAX_READ_MANY_FILES {
        return error(format!(
            "files must contain at most {MAX_READ_MANY_FILES} items"
        ));
    }
    let show_line_numbers = match args.get("show_line_numbers") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("show_line_numbers must be a boolean".to_string()),
    };
    let max_total_bytes = tool_try!(validate_integer_arg(
        args,
        "max_total_bytes",
        MAX_READ_MANY_BYTES,
        4096,
        MAX_READ_MANY_BYTES
    )) as usize;
    let mut sections = Vec::new();
    let mut observations = Vec::new();
    let mut total_bytes = 0usize;
    let mut total_chars = 0usize;
    let mut returned_files = 0usize;
    let separator = "\n\n---\n\n";
    let omitted = |first: usize| format!(
        "...[read_many_files total byte limit reached or display budget exhausted; remaining files omitted]\nomitted_from_item: {first} of {}\nnext_action: read_file on each omitted original path with its requested start_line/line_count; omitted files have no read coverage.", files.len());
    for (index, value) in files.iter().enumerate() {
        let separator_bytes = if sections.is_empty() { 0 } else { separator.len() };
        let separator_chars = separator_bytes; // ASCII separator.
        // Reserve the final omission notice (and its join separator) before
        // accepting a section, so a full first page never loses its receipt to
        // the generic result clamp when later files are omitted.
        let reserve = if index + 1 < files.len() { omitted(files.len()) } else { String::new() };
        let reserve_bytes = if reserve.is_empty() { 0 } else { separator.len() + reserve.len() };
        let reserve_chars = if reserve.is_empty() { 0 } else { separator.len() + reserve.chars().count() };
        let budget = FileReadPresentationBudget {
            chars: MAX_TOOL_RESULT_CHARS.saturating_sub(total_chars + separator_chars + reserve_chars),
            bytes: max_total_bytes.saturating_sub(total_bytes + separator_bytes + reserve_bytes),
        };
        if budget.chars < 512 || budget.bytes < 512 {
            sections.push(omitted(index + 1));
            break;
        }
        let Some(file_args) = value.as_object() else {
            return error(format!("file {} must be an object", index + 1));
        };
        let path = match file_args.get("path").and_then(Value::as_str) {
            Some(value) if !value.trim().is_empty() => value,
            _ => return error(format!("file {} path is required", index + 1)),
        };
        let target = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
        if !target.is_file() {
            return error(format!(
                "file {} is not a file: {}",
                index + 1,
                target.display()
            ));
        }
        let observed = tool_try!(
            read_file_text(&target, file_args, show_line_numbers, 80_000, budget)
                .map_err(|outcome| error(format!("file {} {}", index + 1, outcome.text)))
        );
        if observed.presentation_exhausted {
            sections.push(omitted(index + 1));
            break;
        }
        let section = observed.text;
        let section_bytes = section.len();
        if !budget.fits(&section) {
            // Derived/binary sections have no raw-file observation; they can
            // still be omitted without making prior sections exceed budget.
            sections.push(omitted(index + 1));
            break;
        }
        total_bytes += separator_bytes + section_bytes;
        total_chars += separator_chars + section.chars().count();
        sections.push(section);
        returned_files += 1;
        observations.extend(observed.observation);
    }
    let text = sections.join(separator);
    record_returned_file_observations(&text, observations);
    ok(
        text,
        format!(
            "Read {} file{}",
            returned_files,
            if returned_files == 1 { "" } else { "s" }
        ),
    )
}

fn notebook_source_to_text(source: Option<&Value>) -> String {
    match source {
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|part| part.as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(""),
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

fn notebook_source_from_text(source: &str) -> Value {
    if source.is_empty() {
        return Value::Array(Vec::new());
    }
    Value::Array(
        source
            .split_inclusive('\n')
            .map(|part| Value::String(part.to_string()))
            .collect(),
    )
}

fn format_notebook_cell(
    cell: &Value,
    index: usize,
    include_outputs: bool,
    max_output_chars: usize,
) -> String {
    let cell_type = cell
        .get("cell_type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let cell_id = cell.get("id").and_then(Value::as_str).unwrap_or("");
    let source = notebook_source_to_text(cell.get("source"));
    let mut lines = Vec::new();
    let mut header = format!("## Cell {index} [{cell_type}]");
    if !cell_id.is_empty() {
        header.push_str(&format!(" id={cell_id}"));
    }
    lines.push(header);
    lines.push(format_numbered_lines(&source, 1, true));
    if include_outputs && cell_type == "code" {
        let mut rendered = Vec::new();
        let mut remaining = max_output_chars;
        if let Some(outputs) = cell.get("outputs").and_then(Value::as_array) {
            for output in outputs {
                if remaining == 0 {
                    rendered.push("...[outputs truncated]".to_string());
                    break;
                }
                let mut parts = Vec::new();
                for key in ["ename", "evalue", "text"] {
                    let text = notebook_source_to_text(output.get(key));
                    if !text.trim().is_empty() {
                        parts.push(text);
                    }
                }
                if let Some(data) = output.get("data").and_then(Value::as_object) {
                    let text = notebook_source_to_text(
                        data.get("text/plain").or_else(|| data.get("text/html")),
                    );
                    if !text.trim().is_empty() {
                        parts.push(text);
                    }
                }
                let text = parts.join("\n").trim().to_string();
                if !text.is_empty() {
                    let clipped = text.chars().take(remaining).collect::<String>();
                    remaining = remaining.saturating_sub(clipped.len());
                    rendered.push(clipped);
                }
            }
        }
        lines.push(String::new());
        lines.push("Outputs:".to_string());
        lines.push(if rendered.is_empty() {
            "(none)".to_string()
        } else {
            rendered.join("\n---\n")
        });
    }
    lines.join("\n").trim_end().to_string()
}

fn ensure_notebook_path(path: &Path) -> Result<(), ToolOutcome> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("ipynb") {
        return Err(error(
            "file must be a Jupyter notebook (.ipynb)".to_string(),
        ));
    }
    Ok(())
}

fn notebook_read(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_file() {
        return error(format!("Not a file: {}", target.display()));
    }
    tool_try!(ensure_notebook_path(&target));
    let max_cells = tool_try!(validate_integer_arg(
        args,
        "max_cells",
        100,
        1,
        MAX_NOTEBOOK_CELLS
    )) as usize;
    let include_outputs = match args.get("include_outputs") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => false,
        _ => return error("include_outputs must be a boolean".to_string()),
    };
    let max_output_chars = tool_try!(validate_integer_arg(
        args,
        "max_output_chars",
        4000,
        0,
        MAX_NOTEBOOK_OUTPUT_CHARS
    )) as usize;
    let content = match fs::read_to_string(&target) {
        Ok(value) => value,
        Err(err) => return error(format!("Notebook read failed: {err}")),
    };
    let notebook: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(err) => return error(format!("Notebook read failed: {err}")),
    };
    let Some(cells) = notebook.get("cells").and_then(Value::as_array) else {
        return error("Notebook read failed: notebook JSON must contain a cells array".to_string());
    };
    let mut sections = vec![
        format!("# Notebook: {}", target.display()),
        format!("sha256: {}", sha256_hex(content.as_bytes())),
        format!(
            "nbformat: {}.{}",
            notebook
                .get("nbformat")
                .and_then(Value::as_i64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string()),
            notebook
                .get("nbformat_minor")
                .and_then(Value::as_i64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string())
        ),
        format!("cells: {}", cells.len()),
    ];
    for (index, cell) in cells.iter().take(max_cells).enumerate() {
        sections.push(String::new());
        sections.push(format_notebook_cell(
            cell,
            index,
            include_outputs,
            max_output_chars,
        ));
    }
    if cells.len() > max_cells {
        sections.push(String::new());
        sections.push(format!("...[{} cells truncated]", cells.len() - max_cells));
    }
    ok(
        sections.join("\n"),
        format!(
            "Read notebook {} ({}/{}) cells",
            target.display(),
            cells.len().min(max_cells),
            cells.len()
        ),
    )
}

fn file_outline(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_file() {
        return error(format!("Not a file: {}", target.display()));
    }
    let max_symbols = tool_try!(validate_integer_arg(
        args,
        "max_symbols",
        200,
        1,
        MAX_OUTLINE_SYMBOLS
    )) as usize;
    let include_imports = match args.get("include_imports") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => false,
        _ => return error("include_imports must be boolean".to_string()),
    };
    let max_imports = tool_try!(validate_integer_arg(
        args,
        "max_imports",
        50,
        0,
        MAX_DEPENDENCY_IMPORTS
    )) as usize;
    let mut file = match File::open(&target) {
        Ok(file) => file,
        Err(err) => return error(format!("Read failed: {err}")),
    };
    let mut data = Vec::new();
    if let Err(err) = std::io::Read::by_ref(&mut file)
        .take(MAX_READ_BYTES)
        .read_to_end(&mut data)
    {
        return error(format!("Read failed: {err}"));
    }
    let text = String::from_utf8_lossy(&data).to_string();
    // Symbol signatures/imports are derived views, not the raw file body.
    // Inspecting this file internally does not grant full-file edit coverage.
    let symbols = tool_try!(extract_outline(&text, max_symbols));
    let imports = if include_imports {
        tool_try!(extract_imports(&text, max_imports))
    } else {
        Vec::new()
    };
    let count = symbols
        .iter()
        .filter(|line| !line.starts_with("..."))
        .count();
    let mut sections = vec![
        format!("# Outline: {}", target.display()),
        "## Symbols".to_string(),
        if symbols.is_empty() {
            "(no symbols found)".to_string()
        } else {
            symbols.join("\n")
        },
    ];
    if include_imports {
        sections.extend([
            String::new(),
            "## Imports".to_string(),
            if imports.is_empty() {
                "(none)".to_string()
            } else {
                imports
                    .iter()
                    .map(|item| {
                        format!(
                            "{}: {} {} | {}",
                            item.line, item.kind, item.spec, item.signature
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        ]);
    }
    ok(
        sections.join("\n"),
        format!("Outlined {} ({count} symbols)", target.display()),
    )
}

fn normalize_symbol_identifier(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('`');
    let parts = value.split_whitespace().collect::<Vec<_>>();
    let declaration_kinds = [
        "class",
        "const",
        "enum",
        "fn",
        "function",
        "interface",
        "static",
        "struct",
        "trait",
        "type",
    ];
    let candidate = if parts.len() == 1 {
        parts[0]
    } else if parts.len() == 2 && declaration_kinds.contains(&parts[0]) {
        parts[1]
    } else if parts.len() == 3 && parts[0] == "pub" && declaration_kinds.contains(&parts[1]) {
        parts[2]
    } else {
        return None;
    };
    let candidate = candidate
        .rsplit("::")
        .next()
        .unwrap_or(candidate)
        .trim_matches('`');
    if Regex::new(r"^[A-Za-z_$][A-Za-z0-9_$]*$")
        .expect("valid symbol regex")
        .is_match(candidate)
    {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn read_symbol(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = arg_string(args, "root").or_else(|| {
        ctx.allowed_dirs
            .first()
            .map(|path| path.display().to_string())
    });
    let Some(root_arg) = root_arg else {
        return error("root is required when no allowed workspace is configured".to_string());
    };
    let raw_symbol = tool_try!(required_string(args, "symbol"));
    let Some(symbol) = normalize_symbol_identifier(&raw_symbol) else {
        return error(
            "symbol must be an identifier (accepted forms include Config, struct Config, and module::Config)"
                .to_string(),
        );
    };
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let target = match args
        .get("path")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        Some(path) => {
            let target = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
            if !target.is_file() {
                return error(format!("Not a file: {}", target.display()));
            }
            if !target.starts_with(&root) {
                return error("path is not under root".to_string());
            }
            Some(target)
        }
        None => None,
    };
    let glob = match args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(glob) => match glob_regex(glob) {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
        None => None,
    };
    let max_matches = tool_try!(validate_integer_arg(args, "max_matches", 20, 1, 100)) as usize;
    let context_lines = tool_try!(validate_integer_arg(args, "context_lines", 0, 0, 20)) as usize;
    let show_line_numbers = match args.get("show_line_numbers") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("show_line_numbers must be a boolean".to_string()),
    };
    let (sections, scanned) = tool_try!(read_symbol_bodies(
        &root,
        &symbol,
        glob.as_ref(),
        max_matches,
        context_lines,
        show_line_numbers,
        target.as_ref(),
    ));
    ok(
        format!(
            "# Read symbol: {}\nscanned_files: {}\n\n{}",
            symbol,
            scanned,
            if sections.is_empty() {
                "(no matching symbol definitions found)".to_string()
            } else {
                sections.join("\n\n")
            }
        ),
        format!("Read symbol {} ({} matches)", symbol, sections.len()),
    )
}

fn repo_map(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let glob_text = args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("**/*")
        .to_string();
    let glob = match glob_regex(&glob_text) {
        Ok(regex) => regex,
        Err(err) => return error(err),
    };
    let max_files = tool_try!(validate_integer_arg(
        args,
        "max_files",
        120,
        1,
        MAX_REPO_MAP_FILES
    )) as usize;
    let max_symbols = tool_try!(validate_integer_arg(
        args,
        "max_symbols_per_file",
        40,
        1,
        MAX_REPO_MAP_SYMBOLS_PER_FILE
    )) as usize;
    let (files, scanned) = repo_map_files(&root, &glob, max_files);
    let mut sections = vec![
        format!("# Repo map: {}", root.display()),
        format!("glob: {glob_text}"),
        format!("scanned_files: {scanned}"),
        String::new(),
    ];
    let mut mapped = 0usize;
    for path in &files {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let symbols = tool_try!(extract_outline(&text, max_symbols))
            .into_iter()
            .filter(|line| !line.starts_with("..."))
            .collect::<Vec<_>>();
        if symbols.is_empty() {
            continue;
        }
        mapped += 1;
        sections.push(format!("## {}", relative_display(path, &root)));
        sections.extend(symbols.into_iter().map(|symbol| format!("- {symbol}")));
        sections.push(String::new());
    }
    if mapped == 0 {
        sections.push("(no symbols found)".to_string());
    }
    ok(
        sections.join("\n").trim_end().to_string(),
        format!("Repo map {} ({mapped} files with symbols)", root.display()),
    )
}

fn code_search(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let query = tool_try!(required_string(args, "query")).trim().to_string();
    if query.is_empty() {
        return error("query is required".to_string());
    }
    if query.chars().count() > 200 {
        return error("query must be at most 200 characters".to_string());
    }
    let enclosing_symbol = args
        .get("enclosing_symbol")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if enclosing_symbol.chars().count() > 200 {
        return error("enclosing_symbol must be at most 200 characters".to_string());
    }
    let mode = match args.get("mode").and_then(Value::as_str).unwrap_or("text") {
        "definition" => "definition",
        "call" => "call",
        "reference" => "reference",
        "text" => "text",
        _ => return error("mode must be definition, call, reference, or text".to_string()),
    };
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let glob_text = args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("**/*")
        .to_string();
    let glob = match glob_regex(&glob_text) {
        Ok(regex) => regex,
        Err(err) => return error(err),
    };
    let language = args
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let exact = match args.get("exact") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => false,
        _ => return error("exact must be boolean".to_string()),
    };
    let case_sensitive = match args.get("case_sensitive") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => false,
        _ => return error("case_sensitive must be boolean".to_string()),
    };
    let context_lines = tool_try!(validate_integer_arg(args, "context_lines", 0, 0, 20)) as usize;
    let limit = tool_try!(validate_integer_arg(
        args,
        "max_results",
        200,
        1,
        MAX_CODE_SEARCH_MATCHES,
    )) as usize;
    let (matches, scanned) = tool_try!(code_search_matches(
        &root,
        &query,
        mode,
        &glob,
        &language,
        exact,
        case_sensitive,
        context_lines,
        &enclosing_symbol,
        limit,
        &ctx.cancellation,
    ));
    ok(
        format!(
            "# Code search: {}\nquery: {}\nmode: {}\nglob: {}\nlanguage: {}\nscanned_files: {}\nmatches: {}\n\n{}",
            root.display(),
            query,
            mode,
            glob_text,
            if language.is_empty() { "(any)" } else { &language },
            scanned,
            matches.len(),
            if matches.is_empty() {
                "(no code matches found)".to_string()
            } else {
                matches.join("\n")
            }
        ),
        format!("Code search {} ({} matches)", query, matches.len()),
    )
}

fn workspace_symbols(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let query = tool_try!(required_string(args, "query"));
    let query = query.trim().to_string();
    if query.is_empty() {
        return error("query is required".to_string());
    }
    if query.chars().count() > 128 {
        return error("query must be at most 128 characters".to_string());
    }
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let glob_text = args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("**/*")
        .to_string();
    let glob = match glob_regex(&glob_text) {
        Ok(regex) => regex,
        Err(err) => return error(err),
    };
    let kinds = match args.get("kinds") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(kind) = value.as_str() else {
                    return error("kinds must be known symbol kind strings".to_string());
                };
                if !is_known_symbol_kind(kind) {
                    return error("kinds must be known symbol kind strings".to_string());
                }
                out.push(kind.to_string());
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("kinds must be an array".to_string()),
    };
    let limit = tool_try!(validate_integer_arg(
        args,
        "max_results",
        200,
        1,
        MAX_WORKSPACE_SYMBOL_MATCHES
    )) as usize;
    let (matches, scanned) = tool_try!(workspace_symbol_matches(
        &root, &query, &glob, &kinds, limit
    ));
    ok(
        format!(
            "# Workspace symbols: {}\nquery: {}\nglob: {}\nscanned_files: {}\nmatches: {}\n\n{}",
            root.display(),
            query,
            glob_text,
            scanned,
            matches.len(),
            if matches.is_empty() {
                "(no matching symbols found)".to_string()
            } else {
                matches.join("\n")
            }
        ),
        format!("Workspace symbols {} ({} matches)", query, matches.len()),
    )
}

fn symbol_search(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let symbol = tool_try!(required_string(args, "symbol"));
    if !Regex::new(r"^[A-Za-z_$][A-Za-z0-9_$]*$")
        .expect("valid symbol regex")
        .is_match(&symbol)
    {
        return error("symbol must be an identifier".to_string());
    }
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let glob = match args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(glob) => match glob_regex(glob) {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
        None => None,
    };
    let include_references = match args.get("include_references") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_references must be a boolean".to_string()),
    };
    let limit = tool_try!(validate_integer_arg(
        args,
        "max_matches",
        200,
        1,
        MAX_SYMBOL_SEARCH_MATCHES
    )) as usize;
    let (definitions, references) = tool_try!(scan_symbol_matches(
        &root,
        &symbol,
        glob.as_ref(),
        include_references,
        limit,
    ));
    let mut sections = vec![
        format!("# Symbol search: {symbol}"),
        "## Definitions".to_string(),
        if definitions.is_empty() {
            "(none)".to_string()
        } else {
            definitions.join("\n")
        },
    ];
    if include_references {
        sections.extend([
            String::new(),
            "## References".to_string(),
            if references.is_empty() {
                "(none)".to_string()
            } else {
                references.join("\n")
            },
        ]);
    }
    ok(
        sections.join("\n"),
        format!(
            "Symbol search {} ({} definitions, {} references)",
            symbol,
            definitions.len(),
            references.len()
        ),
    )
}

fn implementation_search(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let symbol = tool_try!(required_string(args, "symbol"));
    if !Regex::new(r"^[A-Za-z_$][A-Za-z0-9_$]*$")
        .expect("valid symbol regex")
        .is_match(&symbol)
    {
        return error("symbol must be an identifier".to_string());
    }
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let glob = match args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(glob) => match glob_regex(glob) {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
        None => None,
    };
    let limit = tool_try!(validate_integer_arg(
        args,
        "max_matches",
        200,
        1,
        MAX_IMPLEMENTATION_MATCHES
    )) as usize;
    let (type_implementations, same_name_definitions, scanned) = tool_try!(
        scan_implementation_matches(&root, &symbol, glob.as_ref(), limit)
    );
    ok(
        format!(
            "# Implementation search: {}\nglob: {}\nscanned_files: {}\nmatches: {}\n\n## Type implementations\n{}\n\n## Same-name definitions\n{}",
            symbol,
            args.get("glob").and_then(Value::as_str).unwrap_or("**/*"),
            scanned,
            type_implementations.len() + same_name_definitions.len(),
            if type_implementations.is_empty() {
                "(none)".to_string()
            } else {
                type_implementations.join("\n")
            },
            if same_name_definitions.is_empty() {
                "(none)".to_string()
            } else {
                same_name_definitions.join("\n")
            }
        ),
        format!(
            "Implementation search {} ({} type implementations, {} same-name definitions)",
            symbol,
            type_implementations.len(),
            same_name_definitions.len()
        ),
    )
}

fn code_navigation(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_file() {
        return error(format!("Not a file: {}", target.display()));
    }
    let root = match args
        .get("root")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        Some(root) => tool_try!(resolve_allowed(root, &ctx.allowed_dirs, true)),
        None => target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| target.clone()),
    };
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let line = tool_try!(validate_integer_arg(args, "line", 1, 1, 1_000_000_000)) as usize;
    let character =
        tool_try!(validate_integer_arg(args, "character", 1, 1, 1_000_000_000)) as usize;
    let operation = match args
        .get("operation")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some("symbol_context") | None => "symbol_context",
        Some("definitions") => "definitions",
        Some("references") => "references",
        Some("implementations") => "implementations",
        Some(_) => {
            return error(
                "operation must be symbol_context, definitions, references, or implementations"
                    .to_string(),
            )
        }
    };
    let glob = match args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(glob) => match glob_regex(glob) {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
        None => None,
    };
    let limit = tool_try!(validate_integer_arg(
        args,
        "max_matches",
        200,
        1,
        MAX_CODE_NAVIGATION_MATCHES
    )) as usize;
    let text = match fs::read_to_string(&target) {
        Ok(text) => text,
        Err(err) => return error(format!("Read failed: {err}")),
    };
    let Some((symbol, source_line)) = tool_try!(symbol_at_position(&text, line, character)) else {
        return error(format!(
            "No identifier at {}:{}:{}",
            target.display(),
            line,
            character
        ));
    };
    if operation == "implementations" {
        let (type_implementations, same_name_definitions, scanned) = tool_try!(
            scan_implementation_matches(&root, &symbol, glob.as_ref(), limit)
        );
        return ok(
            format!(
                "# Code navigation: {}\nposition: {}:{}:{}\nline: {}\nscanned_files: {}\n\n## Type implementations\n{}\n\n## Same-name definitions\n{}",
                symbol,
                target.display(),
                line,
                character,
                outline_signature(&source_line),
                scanned,
                if type_implementations.is_empty() {
                    "(none)".to_string()
                } else {
                    type_implementations.join("\n")
                },
                if same_name_definitions.is_empty() {
                    "(none)".to_string()
                } else {
                    same_name_definitions.join("\n")
                }
            ),
            format!(
                "Code navigation {} ({} type implementations, {} same-name definitions)",
                symbol,
                type_implementations.len(),
                same_name_definitions.len()
            ),
        );
    }
    let include_references = matches!(operation, "symbol_context" | "references");
    let (definitions, references) = tool_try!(scan_symbol_matches(
        &root,
        &symbol,
        glob.as_ref(),
        include_references,
        limit,
    ));
    let docs = if operation == "symbol_context" {
        tool_try!(symbol_definition_context(&root, &symbol, glob.as_ref(), 3))
    } else {
        Vec::new()
    };
    let mut sections = vec![
        format!("# Code navigation: {symbol}"),
        format!("position: {}:{}:{}", target.display(), line, character),
        format!("line: {}", outline_signature(&source_line)),
        String::new(),
        "## Definitions".to_string(),
        if operation == "references" {
            "(not requested)".to_string()
        } else if definitions.is_empty() {
            "(none)".to_string()
        } else {
            definitions.join("\n")
        },
    ];
    if operation == "symbol_context" {
        sections.extend([
            String::new(),
            "## Symbol docs".to_string(),
            if docs.is_empty() {
                "(none)".to_string()
            } else {
                docs.join("\n\n")
            },
        ]);
    }
    if operation != "definitions" {
        sections.extend([
            String::new(),
            "## References".to_string(),
            if references.is_empty() {
                "(none)".to_string()
            } else {
                references.join("\n")
            },
        ]);
    }
    ok(
        sections.join("\n"),
        format!(
            "Code navigation {} ({} definitions, {} references)",
            symbol,
            definitions.len(),
            references.len()
        ),
    )
}

fn call_graph(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let symbol = tool_try!(required_string(args, "symbol"));
    if !Regex::new(r"^[A-Za-z_$][A-Za-z0-9_$]*$")
        .expect("valid symbol regex")
        .is_match(&symbol)
    {
        return error("symbol must be an identifier".to_string());
    }
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let glob = match args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(glob) => match glob_regex(glob) {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
        None => None,
    };
    let max_entries = tool_try!(validate_integer_arg(
        args,
        "max_entries",
        200,
        1,
        MAX_SYMBOL_SEARCH_MATCHES
    )) as usize;
    let (definitions, incoming, outgoing, scanned) =
        tool_try!(scan_call_graph(&root, &symbol, glob.as_ref(), max_entries));
    ok(
        [
            format!("# Call graph: {symbol}"),
            format!("scanned_files: {scanned}"),
            String::new(),
            "## Definitions".to_string(),
            if definitions.is_empty() {
                "(none)".to_string()
            } else {
                definitions.join("\n")
            },
            String::new(),
            "## Incoming calls".to_string(),
            if incoming.is_empty() {
                "(none)".to_string()
            } else {
                incoming.join("\n")
            },
            String::new(),
            "## Outgoing calls".to_string(),
            if outgoing.is_empty() {
                "(none)".to_string()
            } else {
                outgoing.join("\n")
            },
        ]
        .join("\n"),
        format!(
            "Call graph {} ({} incoming, {} outgoing)",
            symbol,
            incoming.len(),
            outgoing.len()
        ),
    )
}

fn code_diagnostics(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let glob = match args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(glob) => match glob_regex(glob) {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
        None => None,
    };
    let max_entries = tool_try!(validate_integer_arg(
        args,
        "max_entries",
        200,
        1,
        MAX_CODE_DIAGNOSTICS
    )) as usize;
    let paths = match args.get("paths") {
        Some(Value::Array(paths)) => {
            let mut out = Vec::new();
            for item in paths {
                let Some(path) = item.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
                if resolved.is_file() && !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            Some(out)
        }
        Some(Value::Null) | None => None,
        _ => return error("paths must be a list".to_string()),
    };
    let (files, scan_scope) = diagnostic_files(&root, paths, glob.as_ref());
    let (issues, scanned) = scan_code_diagnostics(&files, max_entries);
    ok(
        format!(
            "# Code diagnostics: {}\nscan_scope: {}\nscanned_files: {}\n\n## Issues\n{}",
            root.display(),
            scan_scope,
            scanned,
            if issues.is_empty() {
                "(none)".to_string()
            } else {
                issues.join("\n")
            }
        ),
        format!("Code diagnostics ({} issues)", issues.len()),
    )
}

fn failure_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    failure_context_with_observations(args, ctx, true)
}

fn failure_context_with_observations(args: &Map<String, Value>, ctx: &ToolContext, record_observations: bool) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let output_storage;
    let output = match args.get("output").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value,
        _ => {
            let Some(path_arg) = args.get("output_path").and_then(Value::as_str) else {
                return error("output or output_path is required".to_string());
            };
            let path = match ctx.output_store.resolve(Path::new(path_arg)) {
                Ok(path) => path,
                Err(msg) => return error(msg.to_string()),
            };
            output_storage = match fs::read_to_string(&path) {
                Ok(content) => content,
                Err(err) => return error(format!("failed to read tool output: {err}")),
            };
            &output_storage
        }
    };
    let context_lines = tool_try!(validate_integer_arg(args, "context_lines", 4, 0, 40)) as usize;
    let max_entries = tool_try!(validate_integer_arg(args, "max_entries", 20, 1, 100)) as usize;
    let locations = tool_try!(failure_locations(&root, output, max_entries));
    let mut sections = vec![
        format!("# Failure context: {}", root.display()),
        format!(
            "source: {}",
            args.get("output_path")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("output_path: {value}"))
                .unwrap_or_else(|| "inline output".to_string())
        ),
        format!("locations: {}", locations.len()),
        String::new(),
        "## Parsed locations".to_string(),
    ];
    let mut nearest_symbols = HashMap::new();
    if locations.is_empty() {
        sections.push("(none)".to_string());
    } else {
        for (path, line_no, message) in &locations {
            let lines = fs::read_to_string(path)
                .map(|text| text.lines().map(ToOwned::to_owned).collect::<Vec<_>>())
                .unwrap_or_default();
            let nearest = if lines.is_empty() {
                "module".to_string()
            } else {
                nearest_enclosing_symbol(&lines, *line_no)
            };
            nearest_symbols.insert((path.clone(), *line_no), nearest.clone());
            sections.push(format!(
                "- {}:{} | {} | {}",
                relative_display(path, &root),
                line_no,
                nearest,
                message
            ));
        }
    }
    sections.extend([String::new(), "## Snippets".to_string()]);
    let mut observations = Vec::new();
    for (path, line_no, message) in &locations {
        let start_line = (*line_no).saturating_sub(context_lines).max(1);
        let snippet_args = Map::from_iter([
            ("start_line".into(), json!(start_line)),
            ("line_count".into(), json!(context_lines.saturating_mul(2).saturating_add(1))),
        ]);
        let observed = match read_file_text(path, &snippet_args, true, MAX_READ_BYTES as i64, FileReadPresentationBudget::direct()) {
            Ok(observed) => observed,
            Err(outcome) => {
                sections.extend([
                    format!("### {}:{}", path.display(), line_no),
                    format!("error: unable to read file: {}", outcome.text),
                    String::new(),
                ]);
                continue;
            }
        };
        if observed.presentation_exhausted {
            sections.extend([format!("### {}:{}", path.display(), line_no),
                "error: source snippet could not fit the presentation budget; no source content returned".into(), String::new()]);
            continue;
        }
        observations.extend(observed.observation);
        sections.extend([
            format!("### {}:{}", path.display(), line_no),
            format!("nearest_symbol: {}", nearest_symbols.get(&(path.clone(), *line_no))
                .map(String::as_str).unwrap_or("module")),
            format!("message: {message}"),
            String::new(),
            observed.text,
            String::new(),
        ]);
    }
    let text = sections.join("\n").trim_end_matches('\n').to_string();
    if record_observations { record_returned_file_observations(&text, observations); }
    ok(
        text,
        format!(
            "Failure context ({} location{})",
            locations.len(),
            if locations.len() == 1 { "" } else { "s" }
        ),
    )
}

fn compact_head_tail_text(text: &str, max_chars: usize) -> (String, bool) {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return (text.to_string(), false);
    }
    let head_chars = max_chars.saturating_mul(3) / 4;
    let tail_chars = max_chars.saturating_sub(head_chars);
    let head = text.chars().take(head_chars).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    (
        format!(
            "{head}\n...[{} chars omitted]...\n{tail}",
            char_count.saturating_sub(max_chars)
        ),
        true,
    )
}

fn automatic_failure_context(root: &Path, output: &str, ctx: &ToolContext) -> Option<String> {
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.to_string_lossy().to_string()),
    );
    args.insert("output".to_string(), Value::String(output.to_string()));
    args.insert("context_lines".to_string(), Value::from(3));
    args.insert("max_entries".to_string(), Value::from(8));
    // This is later compacted and embedded in another tool's bounded output;
    // internal source inspection must not claim unseen raw-file coverage.
    let outcome = failure_context_with_observations(&args, ctx, false);
    if outcome.error || outcome.text.contains("locations: 0") {
        return None;
    }
    Some(compact_head_tail_text(&outcome.text, 10_000).0)
}

#[cfg(test)]
mod command_failure_context_tests {
    use super::*;

    #[test]
    fn command_output_compaction_preserves_head_and_tail() {
        let input = format!("HEAD{}TAIL", "x".repeat(200));
        let (compacted, changed) = compact_head_tail_text(&input, 80);

        assert!(changed);
        assert!(compacted.starts_with("HEAD"));
        assert!(compacted.ends_with("TAIL"));
        assert!(compacted.contains("chars omitted"));
    }

    #[test]
    fn rust_panic_location_excludes_the_message_prefix() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nola-rust-panic-context-{nonce}"));
        let source = root.join("src/main.rs");
        fs::create_dir_all(source.parent().expect("source parent")).expect("create source parent");
        fs::write(&source, "fn first() {}\nfn failing() {}\n").expect("write source");

        let locations = failure_locations(
            &root,
            "thread 'case' panicked at src/main.rs:2:5:\nassertion failed",
            8,
        )
        .expect("parse Rust panic location");

        assert_eq!(locations.len(), 1);
        assert_eq!(
            locations[0].0,
            source.canonicalize().expect("canonical source")
        );
        assert_eq!(locations[0].1, 2);
        let _ = fs::remove_dir_all(root);
    }
}

fn failure_triage(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let output_storage;
    let (output, source_note) = match args.get("output").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => (value, "inline output".to_string()),
        _ => {
            let Some(path_arg) = args.get("output_path").and_then(Value::as_str) else {
                return error("output or output_path is required".to_string());
            };
            let path = match ctx.output_store.resolve(Path::new(path_arg)) {
                Ok(path) => path,
                Err(msg) => return error(msg.to_string()),
            };
            output_storage = match fs::read_to_string(&path) {
                Ok(content) => content,
                Err(err) => return error(format!("failed to read tool output: {err}")),
            };
            (
                &output_storage[..],
                format!("output_path: {}", path.display()),
            )
        }
    };
    let max_entries = tool_try!(validate_integer_arg(args, "max_entries", 12, 1, 100)) as usize;
    let locations = tool_try!(failure_locations(&root, output, max_entries));
    let (counts, signals) = failure_triage_summary(output);
    let primary_category = primary_failure_category(&counts, &locations);
    let mut files = Vec::new();
    for (path, _line_no, _message) in &locations {
        if !files.contains(path) {
            files.push(path.clone());
        }
    }
    let (diagnostics, commands, related_tests, _target_paths) =
        validation_plan_items(&root, &files, true);
    let mut queue = Vec::new();
    for (index, (path, line_no, message)) in locations.iter().enumerate() {
        let lines = fs::read_to_string(path)
            .map(|text| text.lines().map(ToOwned::to_owned).collect::<Vec<_>>())
            .unwrap_or_default();
        let nearest = if lines.is_empty() {
            "module".to_string()
        } else {
            nearest_enclosing_symbol(&lines, *line_no)
        };
        queue.push(format!(
            "{}. {}: {}:{} | {} | {}",
            index + 1,
            failure_category(message),
            relative_display(path, &root),
            line_no,
            nearest,
            message
        ));
    }
    if queue.is_empty() && counts.contains_key("environment/dependency") {
        queue.push(
            "1. environment/dependency: inspect missing command/module/network error before changing source code"
                .to_string(),
        );
    }
    let mut count_lines = counts.iter().collect::<Vec<_>>();
    count_lines.sort_by(|a, b| a.0.cmp(b.0));
    let mut next_steps = vec![if source_note.starts_with("output_path:") {
        "failure_context root=<root> output_path=<saved-output> for source snippets around parsed locations".to_string()
    } else {
        "failure_context root=<root> output=<failed-output> for source snippets around parsed locations".to_string()
    }];
    next_steps.extend(diagnostics);
    next_steps.extend(commands);
    ok(
        [
            format!("# Failure triage: {}", root.display()),
            format!("source: {source_note}"),
            format!("primary_category: {primary_category}"),
            format!("locations: {}", locations.len()),
            String::new(),
            "## Category counts".to_string(),
            if count_lines.is_empty() {
                "(none detected)".to_string()
            } else {
                count_lines
                    .into_iter()
                    .map(|(category, count)| format!("- {category}: {count}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Failure signals".to_string(),
            if signals.is_empty() {
                "(none detected)".to_string()
            } else {
                signals.join("\n")
            },
            String::new(),
            "## Fix queue".to_string(),
            if queue.is_empty() {
                "(no source locations parsed; inspect raw output or rerun with fuller logs)"
                    .to_string()
            } else {
                queue.join("\n")
            },
            String::new(),
            "## Related tests".to_string(),
            if related_tests.is_empty() {
                "(none inferred)".to_string()
            } else {
                related_tests
                    .iter()
                    .map(|path| format!("- {}", relative_display(path, &root)))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Suggested next steps".to_string(),
            next_steps
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ]
        .join("\n"),
        format!(
            "Failure triage ({primary_category}, {} locations)",
            locations.len()
        ),
    )
}

fn file_dependencies(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_file() {
        return error(format!("Not a file: {}", target.display()));
    }
    let root = match args
        .get("root")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        Some(root) => tool_try!(resolve_allowed(root, &ctx.allowed_dirs, true)),
        None => target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| target.clone()),
    };
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_imports = tool_try!(validate_integer_arg(
        args,
        "max_imports",
        200,
        0,
        MAX_DEPENDENCY_IMPORTS
    )) as usize;
    let max_importers = tool_try!(validate_integer_arg(
        args,
        "max_importers",
        100,
        0,
        MAX_DEPENDENCY_IMPORTS
    )) as usize;
    let mut file = match File::open(&target) {
        Ok(file) => file,
        Err(err) => return error(format!("Read failed: {err}")),
    };
    let mut data = Vec::new();
    if let Err(err) = std::io::Read::by_ref(&mut file)
        .take(MAX_READ_BYTES)
        .read_to_end(&mut data)
    {
        return error(format!("Read failed: {err}"));
    }
    let text = String::from_utf8_lossy(&data).to_string();
    let imports = if max_imports == 0 {
        Vec::new()
    } else {
        tool_try!(extract_imports(&text, max_imports))
    };
    let mut resolved = Vec::<PathBuf>::new();
    let mut import_lines = Vec::new();
    for item in &imports {
        let local = resolve_dependency_path(&target, &root, &item.spec);
        if let Some(local) = &local {
            if !resolved.contains(local) {
                resolved.push(local.clone());
            }
        }
        import_lines.push(format!(
            "{}: {} {}{} | {}",
            item.line,
            item.kind,
            item.spec,
            local
                .as_ref()
                .map(|path| format!(" -> {}", path.display()))
                .unwrap_or_default(),
            item.signature
        ));
    }
    let (importers, scanned_importers) = if max_importers == 0 {
        (Vec::new(), 0)
    } else {
        scan_importers(&root, &target, max_importers)
    };
    ok(
        format!(
            "# Dependencies: {}\nroot: {}\n\n## Imports\n{}\n\n## Resolved local files\n{}\n\n## Local importers\nscanned_files: {}\n{}",
            target.display(),
            root.display(),
            if import_lines.is_empty() {
                "(none)".to_string()
            } else {
                import_lines.join("\n")
            },
            if resolved.is_empty() {
                "(none)".to_string()
            } else {
                resolved
                    .iter()
                    .map(|path| format!("- {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            scanned_importers,
            if importers.is_empty() {
                "(none)".to_string()
            } else {
                importers.join("\n")
            }
        ),
        format!(
            "Dependencies {} ({} imports, {} local files, {} importers)",
            target.display(),
            imports.len(),
            resolved.len(),
            importers.len()
        ),
    )
}

fn coding_conventions(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let paths = match args.get("paths") {
        Some(Value::Array(paths)) => {
            let mut out = Vec::new();
            for item in paths {
                let Some(path) = item.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
                if resolved.is_file() && is_code_source(&resolved) && !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            Some(out)
        }
        Some(Value::Null) | None => None,
        _ => return error("paths must be an array".to_string()),
    };
    let glob = match args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(glob) => match glob_regex(glob) {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
        None => None,
    };
    let max_files = tool_try!(validate_integer_arg(
        args,
        "max_files",
        80,
        1,
        MAX_CONVENTION_FILES
    )) as usize;
    let (files, scope, scanned) = coding_convention_files(&root, paths, glob.as_ref(), max_files);
    let (configs, config_scanned) = scan_convention_config_files(&root, 40);
    let sections = coding_convention_summary(&root, &files, &configs);
    ok(
        format!(
            "# Coding conventions: {}\nscan_scope: {}\nscanned_files: {}\nscanned_config_files: {}\n\n{}",
            root.display(),
            scope,
            scanned,
            config_scanned,
            sections.join("\n")
        ),
        format!(
            "Coding conventions ({} sampled files, {} config files)",
            files.len(),
            configs.len()
        ),
    )
}

fn project_overview(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_files = tool_try!(validate_integer_arg(
        args,
        "max_files",
        5000,
        100,
        MAX_PROJECT_SCAN_FILES
    )) as usize;
    let (manifests, languages, docs, scanned, truncated) = scan_project(&root, max_files);
    let top_level_entries = project_top_level_entries(&root, 40);
    let commands = suggest_project_commands(&root, &manifests);
    let manifest_lines = if manifests.is_empty() {
        "(none)".to_string()
    } else {
        manifests
            .iter()
            .map(|path| format!("- {}", relative_display(path, &root)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let language_lines = if languages.is_empty() {
        "(none)".to_string()
    } else {
        let mut items = languages.into_iter().collect::<Vec<_>>();
        items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        items
            .into_iter()
            .map(|(language, count)| format!("- {language}: {count}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let doc_lines = if docs.is_empty() {
        "(none)".to_string()
    } else {
        docs.iter()
            .map(|path| format!("- {}", relative_display(path, &root)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let command_lines = if commands.is_empty() {
        "(none inferred)".to_string()
    } else {
        commands
            .iter()
            .map(|command| format!("- {command}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let top_level_lines = if top_level_entries.is_empty() {
        "(none)".to_string()
    } else {
        top_level_entries
            .iter()
            .map(|entry| format!("- {entry}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    ok(
        format!(
            "# Project overview: {}\nscanned_files: {}\ntruncated: {}{}\n\n## Top-level entries\n{}\n\n## Manifests\n{}\n\n## Languages\n{}\n\n## Key docs\n{}\n\n## Likely validation commands\n{}",
            root.display(),
            scanned,
            truncated,
            if truncated {
                "\n...[scan limit reached; continue with narrower paths or patterns]"
            } else {
                ""
            },
            top_level_lines,
            manifest_lines,
            language_lines,
            doc_lines,
            command_lines
        ),
        format!(
            "Project overview {} ({} manifests, {} languages)",
            root.display(),
            manifests.len(),
            if language_lines == "(none)" {
                0
            } else {
                language_lines.lines().count()
            }
        ),
    )
}

fn instruction_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    instruction_context_with_allowed_dirs(args, &ctx.allowed_dirs)
}

fn instruction_context_with_allowed_dirs(
    args: &Map<String, Value>,
    allowed_dirs: &[PathBuf],
) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_bytes = tool_try!(validate_integer_arg(
        args,
        "max_bytes_per_file",
        20_000,
        1,
        MAX_INSTRUCTION_BYTES
    )) as usize;
    let names = match args.get("names") {
        Some(Value::Array(values)) if !values.is_empty() => {
            if values.len() > 20 {
                return error("names may contain at most 20 entries".to_string());
            }
            let mut out = Vec::new();
            for value in values {
                let Some(name) = value.as_str() else {
                    return error("names must contain strings".to_string());
                };
                let trimmed = name.trim();
                if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') {
                    return error("names must be plain file names".to_string());
                }
                out.push(trimmed.to_string());
            }
            out
        }
        Some(Value::Array(_)) | Some(Value::Null) | None => INSTRUCTION_FILE_NAMES
            .iter()
            .map(|name| name.to_string())
            .collect(),
        _ => return error("names must be a list".to_string()),
    };
    let targets = match args.get("paths") {
        Some(Value::Array(values)) if !values.is_empty() => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let requested = Path::new(path);
                let requested = if requested.is_absolute() {
                    requested.to_path_buf()
                } else {
                    root.join(requested)
                };
                let resolved = tool_try!(resolve_allowed(
                    requested.to_string_lossy().as_ref(),
                    allowed_dirs,
                    true
                ));
                if !resolved.starts_with(&root) {
                    return error(format!(
                        "path is outside instruction_context root: {}",
                        resolved.display()
                    ));
                }
                out.push(resolved);
            }
            out
        }
        Some(Value::Array(_)) | Some(Value::Null) | None => vec![root.clone()],
        _ => return error("paths must be a list".to_string()),
    };

    let files = collect_instruction_files(&root, &targets, &names);

    let mut sections = vec![format!("# Instruction context: {}", root.display())];
    if files.is_empty() {
        sections.push("(no instruction files found)".to_string());
    }
    for path in &files {
        let rel = relative_display(path, &root);
        match read_instruction_file(path, max_bytes) {
            Ok((content, byte_count, truncated)) => {
                sections.extend([
                    String::new(),
                    format!("## {rel}"),
                    format!("bytes: {byte_count}"),
                    content.trim_end().to_string(),
                ]);
                if truncated {
                    sections.push(format!("...[truncated to {max_bytes} bytes]"));
                }
            }
            Err(err) => sections.extend([
                String::new(),
                format!("## {rel}"),
                format!("error: unable to read file: {err}"),
            ]),
        }
    }

    ok(
        sections.join("\n"),
        format!(
            "Instruction context ({} file{})",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        ),
    )
}

pub(crate) fn root_repository_instruction_context(
    allowed_dirs: &[PathBuf],
) -> Result<Option<(PathBuf, String)>, String> {
    let Some(root) = allowed_dirs.first() else {
        return Ok(None);
    };
    let root = root.canonicalize().unwrap_or_else(|_| root.clone());
    if !root.is_dir() {
        return Ok(None);
    }
    let names = INSTRUCTION_FILE_NAMES
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    if collect_instruction_files(&root, std::slice::from_ref(&root), &names).is_empty() {
        return Ok(None);
    }
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.to_string_lossy().into_owned()),
    );
    args.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String(root.to_string_lossy().into_owned())]),
    );
    let outcome = instruction_context_with_allowed_dirs(&args, allowed_dirs);
    if outcome.error {
        return Err(outcome.text);
    }
    Ok(Some((root, outcome.text)))
}

fn skill_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    // Skills are advertised capabilities, not arbitrary readable files.
    // Filesystem grants must not independently enable Skills.
    skill_context_with_dirs(args, ctx.skills.configured_dirs().unwrap_or(&[]))
}

#[cfg(test)]
#[test]
fn legacy_skill_context_reads_only_the_runtime_owned_roots() {
    let fixture = tempfile::tempdir().expect("isolated Skill root");
    let root = fixture.path().to_path_buf();
    fs::write(
        root.join("SKILL.md"),
        "---\nname: owned-context\ndescription: owned root\n---\n\nOWNED_CONTEXT_BODY\n",
    )
    .expect("Skill fixture");
    let args = Map::from_iter([
        (
            "root".to_string(),
            Value::String(root.to_string_lossy().into_owned()),
        ),
        ("name".to_string(), Value::String("owned-context".to_string())),
        ("include_files".to_string(), Value::Bool(false)),
    ]);
    let mut ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: SkillRuntimeContext::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: SessionPermissionScope::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(root.join("browser")),
    };
    for skills in [
        SkillRuntimeContext::default(),
        SkillRuntimeContext::from_configured_dirs(Some(Vec::new())),
    ] {
        ctx.skills = skills;
        let outcome = skill_context(&args, &ctx);
        assert!(
            outcome.error,
            "filesystem grants must not become Skill roots"
        );
        assert!(!outcome.text.contains("OWNED_CONTEXT_BODY"));
    }
    ctx.skills = SkillRuntimeContext::from_configured_dirs(Some(vec![root]));
    let outcome = skill_context(&args, &ctx);
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("OWNED_CONTEXT_BODY"));
}

fn skill_update_artifact(
    artifact_ref: &str,
    ctx: &ToolContext,
) -> Result<nola_core::agent::host::HostArtifact, String> {
    let parsed = url::Url::parse(artifact_ref)
        .map_err(|_| "artifact_ref must be a canonical absolute URI".to_string())?;
    if parsed.as_str() != artifact_ref {
        return Err("artifact_ref must be normalized and percent-encoded".to_string());
    }
    let (uri, name, mime_type) = if parsed.scheme() == "file" {
        let path = parsed
            .to_file_path()
            .map_err(|_| "artifact_ref is not an absolute file URI".to_string())?;
        let path = path
            .canonicalize()
            .map_err(|error| format!("Skill artifact does not exist: {error}"))?;
        if !path.is_file() {
            return Err("Skill artifact must identify an existing file".to_string());
        }
        let allowed = ctx.allowed_dirs.iter().any(|root| {
            let root = root.canonicalize().unwrap_or_else(|_| root.clone());
            path.starts_with(root)
        });
        if !allowed {
            return Err("Skill artifact is outside the active workspace".to_string());
        }
        (
            url::Url::from_file_path(&path)
                .map_err(|_| "Skill artifact path cannot be represented as a file URI".to_string())?
                .to_string(),
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("artifact")
                .to_string(),
            mime_guess::from_path(&path).first_raw().map(str::to_string),
        )
    } else {
        (
            parsed.to_string(),
            parsed
                .path_segments()
                .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
                .unwrap_or("artifact")
                .to_string(),
            None,
        )
    };
    let artifact = nola_core::agent::host::HostArtifact {
        uri,
        name,
        mime_type,
        operation: None,
    };
    artifact.validate()?;
    Ok(artifact)
}

fn skill_update(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let required_string = |key: &str| -> Result<String, ToolOutcome> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| error(format!("{key} is required for this skill_update action")))
    };
    let action = match args.get("action").and_then(Value::as_str) {
        Some(action) => action,
        None => return error("action is required".to_string()),
    };
    let run_id = match required_string("run_id") {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    let mut registered_artifact = None;
    let payload = match action {
        "progress" => {
            let step_id = match required_string("step_id") {
                Ok(value) => value,
                Err(outcome) => return outcome,
            };
            let Some(completed) = args.get("completed").and_then(Value::as_bool) else {
                return error("completed is required for progress".to_string());
            };
            json!({"type": "skill_runtime_update", "run_id": run_id, "action": "progress", "step_id": step_id, "completed": completed})
        }
        "artifact" => {
            let artifact_id = match required_string("artifact_id") {
                Ok(value) => value,
                Err(outcome) => return outcome,
            };
            let artifact_ref = match required_string("artifact_ref") {
                Ok(value) => value,
                Err(outcome) => return outcome,
            };
            let artifact = match skill_update_artifact(&artifact_ref, ctx) {
                Ok(artifact) => artifact,
                Err(message) => {
                    return error_data_with_display(json!({
                            "error": "invalid_skill_artifact_ref",
                            "message": message,
                            "tool_execution_skipped": true,
                            "next_action": "Use the canonical artifact_uri returned by the successful producing tool. If no such URI exists, stop the Skill as failed instead of retrying an invented reference."
                        }),
                        "Skill artifact was not registered".to_string(),
                    )
                }
            };
            let artifact_ref = artifact.uri.clone();
            registered_artifact = Some(artifact);
            json!({"type": "skill_runtime_update", "run_id": run_id, "action": "artifact", "artifact_id": artifact_id, "artifact_ref": artifact_ref})
        }
        "verification" => {
            let verifier_id = match required_string("verifier_id") {
                Ok(value) => value,
                Err(outcome) => return outcome,
            };
            let evidence = match args.get("evidence").cloned()
                .ok_or_else(|| "evidence is required for verification".to_string())
                .and_then(|value| serde_json::from_value::<nola_core::agent::capability::SkillVerificationEvidence>(value).map_err(|error| error.to_string())) {
                Ok(value) => value,
                Err(message) => return error(message),
            };
            if let Err(message) = evidence.validate() { return error(message); }
            let replaces_evidence = match args.get("replaces_evidence").cloned()
                .map(serde_json::from_value::<nola_core::agent::capability::SkillVerificationEvidence>).transpose() {
                Ok(value) => value,
                Err(message) => return error(message.to_string()),
            };
            let Some(passed) = args.get("passed").and_then(Value::as_bool) else {
                return error("passed is required for verification".to_string());
            };
            let summary = args
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            json!({"type": "skill_runtime_update", "run_id": run_id, "action": "verification", "verifier_id": verifier_id, "passed": passed, "evidence": evidence, "replaces_evidence": replaces_evidence, "summary": summary})
        }
        "resume_point" => {
            let resume_point = match required_string("resume_point") {
                Ok(value) => value,
                Err(outcome) => return outcome,
            };
            json!({"type": "skill_runtime_update", "run_id": run_id, "action": "resume_point", "resume_point": resume_point})
        }
        _ => return error("unsupported skill_update action".to_string()),
    };
    let mut outcome = ok_data(payload.clone(), "Skill progress recorded".to_string());
    if let Some(artifact) = registered_artifact {
        outcome.artifacts.push(artifact);
    }
    outcome
}

#[cfg(test)]
#[test]
fn skill_update_emits_strict_runtime_protocol_payloads() {
    let root = std::env::temp_dir().join(format!("nola-skill-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("Skill update workspace");
    let artifact_path = root.join("report.docx");
    fs::write(&artifact_path, "document").expect("Skill artifact fixture");
    let artifact_ref = url::Url::from_file_path(&artifact_path)
        .expect("Skill artifact URI")
        .to_string();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: SkillRuntimeContext::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: SessionPermissionScope::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(root.join("browser")),
    };
    let cases = vec![
        json!({"run_id": "skill-run", "action": "progress", "step_id": "render", "completed": true}),
        json!({"run_id": "skill-run", "action": "artifact", "artifact_id": "document", "artifact_ref": artifact_ref}),
        json!({"run_id": "skill-run", "action": "verification", "verifier_id": "render-check", "passed": true, "evidence": {"kind":"model_judgment","sources":[]}, "summary": "3 pages"}),
        json!({"run_id": "skill-run", "action": "resume_point", "resume_point": "after-render"}),
    ];
    for arguments in cases {
        let outcome = skill_update(arguments.as_object().expect("arguments"), &ctx);
        assert!(!outcome.error, "{}", outcome.text);
        let payload: Value = serde_json::from_str(&outcome.text).expect("protocol JSON");
        assert_eq!(
            payload.get("type").and_then(Value::as_str),
            Some("skill_runtime_update")
        );
        assert_eq!(
            payload.get("action").and_then(Value::as_str),
            arguments.get("action").and_then(Value::as_str)
        );
        assert_eq!(payload.get("run_id").and_then(Value::as_str), Some("skill-run"));
        assert_eq!(
            outcome.artifacts.len(),
            usize::from(arguments.get("action").and_then(Value::as_str) == Some("artifact"))
        );
    }
    assert!(
        skill_update(
            json!({"run_id": "skill-run", "action": "artifact", "artifact_id": "document"})
                .as_object()
                .expect("arguments"),
            &ctx,
        )
        .error
    );
    assert!(
        skill_update(
            json!({"run_id": "skill-run", "action": "verification", "verifier_id": "render-check", "passed": true})
                .as_object()
                .expect("arguments"),
            &ctx,
        )
        .error
    );
    assert!(skill_update(
        json!({"run_id":"skill-run","action":"artifact","artifact_id":"document","artifact_ref":"file:///outside.docx"})
            .as_object()
            .expect("arguments"),
        &ctx,
    )
    .error);
    let invalid_reference = skill_update(
        json!({"run_id":"skill-run","action":"artifact","artifact_id":"document","artifact_ref":"record-42"})
            .as_object()
            .expect("arguments"),
        &ctx,
    );
    assert!(invalid_reference.error);
    let recovery: Value =
        serde_json::from_str(&invalid_reference.text).expect("structured recovery payload");
    assert_eq!(recovery["error"], "invalid_skill_artifact_ref");
    assert_eq!(recovery["tool_execution_skipped"], true);
    assert!(recovery["next_action"]
        .as_str()
        .expect("next action")
        .contains("canonical artifact_uri"));
    let _ = fs::remove_dir_all(root);
}

fn skill_invoke(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let skill_id = tool_try!(required_string(args, "skill")).trim().to_string();
    if !host_skill_enabled(ctx, &skill_id) {
        return error(format!("Unknown or disabled Skill id: {skill_id}"));
    }
    let plugin_skills = match ctx.skills.plugin_skills() {
        Ok(skills) => skills,
        Err(error_message) => return error(error_message),
    };
    let base_directory = if let Some(plugin) = plugin_skills.into_iter().find(|skill| skill.resource_id == skill_id) {
        plugin.directory
    } else if skill_id.starts_with("plugin:") {
        return error(format!("Unknown or disabled plugin Skill id: {skill_id}"));
    } else if let Some(skill_dirs) = ctx.skills.configured_dirs() {
        match configured_skill_directory_for_id(&skill_id, skill_dirs) {
            Ok(Some(directory)) => directory,
            Ok(None) => return error(format!("Unknown or disabled Skill id: {skill_id}")),
            Err(resolve_error) => return error(resolve_error),
        }
    } else {
        match crate::builtin_skills::materialize_managed_directory_for_id(&skill_id) {
            Ok(Some(directory)) => directory,
            Ok(None) => {
                let custom_root = match crate::builtin_skills::managed_custom_root() {
                    Ok(root) => root,
                    Err(resolve_error) => return error(resolve_error),
                };
                let resolved = optional_custom_skill_context_entries(&custom_root)
                    .map(|entries| skill_directory_for_id_from_entries(&skill_id, entries));
                match resolved {
                    Ok(Some(directory)) => directory,
                    Ok(None) => return error(format!("Unknown or disabled Skill id: {skill_id}")),
                    Err(resolve_error) => return error(resolve_error),
                }
            }
            Err(resolve_error) => return error(resolve_error),
        }
    };
    let resource_directory_uri = match url::Url::from_directory_path(&base_directory) {
        Ok(uri) => uri.to_string(),
        Err(_) => {
            return error("Unable to encode the exact Skill resource directory URI".to_string())
        }
    };
    let skill_file = base_directory.join("SKILL.md");
    let instructions = match read_configured_skill_index_file(&skill_file) {
        Ok(content) => content,
        Err(read_error) => {
            return error(format!(
                "Failed to read {}: {read_error}",
                skill_file.display()
            ))
        }
    };
    let manifest = fs::read_to_string(base_directory.join("NOLA_MANIFEST.json"))
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .unwrap_or(Value::Null);
    let instructions_digest = sha256_hex(instructions.as_bytes());
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("sha256-{}", &instructions_digest[..12]));
    let instructions_ref =
        format!("skill://{skill_id}/{version}/SKILL.md#sha256={instructions_digest}");
    let validation_steps = manifest
        .get("validation")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|step| step.as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    let required_tools = manifest
        .get("required_tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|tool| match tool {
            // The string form is an exact Host capability id declared by the
            // Skill author. It is never inferred from the user's prompt.
            Value::String(tool_name) => json!({
                "provider": "builtin",
                "tool_name": tool_name,
            }),
            structured => structured,
        })
        .collect::<Vec<_>>();
    if let Some(decode_error) = required_tools.iter().find_map(|tool| {
        serde_json::from_value::<nola_core::agent::capability::CapabilityToolReference>(
            tool.clone(),
        )
        .err()
    }) {
        return error(format!(
            "Invalid required_tools entry in Skill manifest: {decode_error}"
        ));
    }
    let availability = HostCapabilityAvailability::capture(&ctx.skills);
    let unavailable = required_tools.iter().filter_map(|tool| {
        if tool.get("provider").and_then(Value::as_str) != Some("builtin") { return None; }
        let name = tool.get("tool_name")?.as_str()?;
        (!availability.enabled(name)).then(|| name.to_string())
    }).collect::<Vec<_>>();
    if !unavailable.is_empty() {
        return error_data_with_display(json!({
            "error": "This Skill requires capabilities disabled by the current Host configuration. No workflow was started. Do not search repeatedly or substitute another storage mechanism. Ask the user to review the relevant conversation settings.",
            "code": "skill_dependencies_disabled",
            "unavailable_capabilities": unavailable,
            "tool_execution_skipped": true
        }), "Skill unavailable: a required capability is disabled in conversation settings.".to_string());
    }
    let display_name = manifest
        .get("display_name")
        .or_else(|| manifest.get("name"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&skill_id)
        .to_string();
    let mode = manifest
        .get("execution_mode")
        .or_else(|| manifest.get("default_mode"))
        .and_then(Value::as_str)
        .unwrap_or("inline");
    if !matches!(mode, "inline" | "fork") {
        return error(format!("Invalid Skill execution mode: {mode}"));
    }
    let execution_context = json!({
        "workspace_root": ctx.allowed_dirs.first().map(|path| path.to_string_lossy()),
        "base_directory": base_directory.to_string_lossy(),
        "path_resolution": "Resolve deliverables and user files from workspace_root. Resolve Skill-owned scripts, templates, and references from base_directory.",
    });
    let payload = json!({
        // This compact field deliberately sorts before the potentially long
        // instructions body. Model-facing output previews may be bounded, but
        // the paths required to execute the Skill must never disappear behind
        // that boundary.
        "_nola_execution_context": execution_context,
        "type": "invoked_skill",
        "skill_id": skill_id,
        "version": version,
        "display_name": display_name,
        "instructions_ref": instructions_ref,
        "instructions_digest": instructions_digest,
        "mode": mode,
        // A Host may relay a Skill-authored explicit budget, but it must not
        // invent one. Core owns the default lifecycle policy and interprets a
        // missing/null budget as SkillRunBudget::default().
        "budget": materialized_skill_budget(&manifest),
        "execution_contract": manifest
            .get("execution_contract")
            .cloned()
            .unwrap_or_else(|| json!({"schemaVersion": 1})),
        "base_directory": base_directory.to_string_lossy(),
        "resource_directory_uri": resource_directory_uri,
        "workspace_root": ctx.allowed_dirs.first().map(|path| path.to_string_lossy()),
        "path_resolution": "Use base_directory only for Skill-owned scripts, templates, and reference files. Resolve user-provided paths, workspace files, and deliverable destinations from workspace_root.",
        "arguments": args.get("args").and_then(Value::as_str).unwrap_or_default(),
        "instructions": instructions,
        "manifest": manifest,
        "required_tools": required_tools,
        "validation_steps": validation_steps,
    });
    ok_data(payload.clone(),
        format!(
            "Skill · {}",
            payload["skill_id"].as_str().unwrap_or_default()
        ),
    )
}

fn materialized_skill_budget(manifest: &Value) -> Value {
    manifest.get("budget").cloned().unwrap_or(Value::Null)
}

fn configured_skill_directory_for_id(
    skill_id: &str,
    skill_dirs: &[PathBuf],
) -> Result<Option<PathBuf>, String> {
    Ok(skill_directory_for_id_from_entries(
        skill_id,
        configured_skill_context_entries(skill_dirs)?,
    ))
}

fn skill_directory_for_id_from_entries(
    skill_id: &str,
    entries: Vec<(PathBuf, Value)>,
) -> Option<PathBuf> {
    // Exact paths originate from the same strict scanner as the model index.
    // Display-only, possibly lossy JSON paths never select execution sources.
    entries.into_iter().find_map(|(directory, metadata)| {
        (metadata.get("id").and_then(Value::as_str) == Some(skill_id)).then_some(directory)
    })
}

fn sop_invoke(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let sop_id = tool_try!(required_string(args, "sop")).trim().to_string();
    let Some(definition) = crate::host_prompts::current_host_sops()
        .into_iter()
        .find(|definition| definition.id == sop_id)
    else {
        return error(format!("Unknown or unavailable SOP id: {sop_id}"));
    };
    let mut start = Map::new();
    start.insert("action".to_string(), Value::String("start".to_string()));
    start.insert("sop_id".to_string(), Value::String(sop_id.clone()));
    start.insert(
        "inputs".to_string(),
        args.get("inputs")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new())),
    );
    let started = sop_run_update(&start, ctx);
    if started.error {
        return started;
    }
    let run = match serde_json::from_str::<Value>(&started.text) {
        Ok(value) => value,
        Err(decode_error) => return error(format!("Decode started SOP run failed: {decode_error}")),
    };
    let payload = json!({
        "type": "invoked_sop",
        "sop_id": sop_id,
        "version": definition.version,
        "display_name": definition.title,
        "run": run,
    });
    ok_data(payload.clone(), format!("SOP · {}", definition.title))
}

fn sop_run_update(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let action = tool_try!(required_string(args, "action"));
    let owner_session_id = ctx.permission_scope.owner_session_id();
    let project_root = ctx
        .allowed_dirs
        .first()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(error_message) => return error(format!("Open SOP Store failed: {error_message}")),
    };

    // The model-facing protocol follows the exact owner boundary used by the
    // active SOP context. Explicit CLI Work controls remain the recovery path
    // for an operator who intentionally resumes a run from another session.
    let visible = |projection: &nola_application::SopRunProjection| {
        crate::sop_context::roots_refer_to_same_directory(&projection.project_root, &project_root)
            && sop_owner_is_visible_to_session(
                projection.owner_session_id.as_deref(),
                owner_session_id.as_deref(),
            )
    };
    let result = match action.as_str() {
        "start" => {
            let sop_id = tool_try!(required_string(args, "sop_id"));
            let Some(definition) = crate::host_prompts::current_host_sops()
                .into_iter()
                .find(|definition| definition.id == sop_id)
            else {
                return error(format!("Unknown or unavailable SOP id: {sop_id}"));
            };
            // Repeated model calls within one owner/workspace resume the
            // durable workflow already in progress. Creating parallel copies
            // loses step evidence and gives the model two competing protocol
            // obligations for the same request.
            match store.list_sop_runs(
                Some(project_root.clone()),
                owner_session_id.clone(),
                100,
            ) {
                Ok(existing) => {
                    if let Some(existing) = existing.into_iter().find(|projection| {
                        visible(projection)
                            && projection.definition.id == sop_id
                            && sop_run_status_is_active(projection.record.status)
                    }) {
                        return ok(
                            serde_json::to_string(&existing).unwrap_or_else(|_| "{}".into()),
                            format!(
                                "SOP · {} · {:?} · resumed",
                                existing.record.run_id, existing.record.status
                            ),
                        );
                    }
                }
                Err(error_message) => return error(error_message),
            }
            store.create_sop_run(
                definition,
                uuid::Uuid::new_v4().to_string(),
                args.get("inputs")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new())),
                owner_session_id,
                project_root,
            )
        }
        "apply" => {
            let run_id = tool_try!(required_string(args, "run_id"));
            let Some(existing) = (match store.sop_run(run_id.clone()) {
                Ok(value) => value,
                Err(error_message) => return error(error_message),
            }) else {
                return error(format!("Unknown SOP run: {run_id}"));
            };
            if !visible(&existing) {
                return error("Unknown or unavailable SOP run".into());
            }
            let expected_revision = match args.get("expected_revision").and_then(Value::as_u64) {
                Some(value) => value,
                None => return error("apply requires expected_revision".into()),
            };
            let event = match args
                .get("event")
                .cloned()
                .ok_or_else(|| "apply requires event".to_string())
                .and_then(|value| {
                    serde_json::from_value::<nola_core::agent::SopRunEvent>(value)
                        .map_err(|decode_error| format!("Invalid SOP event: {decode_error}"))
                }) {
                Ok(event) => event,
                Err(error_message) => return error(error_message),
            };
            store.apply_sop_run_event(run_id, expected_revision, event)
        }
        "load" => {
            let run_id = tool_try!(required_string(args, "run_id"));
            match store.sop_run(run_id.clone()) {
                Ok(Some(projection)) if visible(&projection) => Ok(projection),
                Ok(Some(_)) => return error("Unknown or unavailable SOP run".into()),
                Ok(None) => return error(format!("Unknown SOP run: {run_id}")),
                Err(error_message) => return error(error_message),
            }
        }
        "list" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 100);
            return match store.list_sop_runs(
                Some(project_root.clone()),
                None,
                limit as usize,
            ) {
                Ok(projections) => {
                    let projections = projections
                        .into_iter()
                        .filter(visible)
                        .collect::<Vec<_>>();
                    ok(
                        serde_json::to_string(&projections).unwrap_or_else(|_| "[]".into()),
                        format!("SOP · {} runs", projections.len()),
                    )
                }
                Err(error_message) => error(error_message),
            };
        }
        _ => return error(format!("Unknown SOP action: {action}")),
    };

    match result {
        Ok(projection) => ok(
            serde_json::to_string(&projection).unwrap_or_else(|_| "{}".into()),
            format!(
                "SOP · {} · {:?}",
                projection.record.run_id, projection.record.status
            ),
        ),
        Err(error_message) => error(error_message),
    }
}

fn sop_run_status_is_active(status: nola_core::agent::SopRunStatus) -> bool {
    !matches!(
        status,
        nola_core::agent::SopRunStatus::Completed
            | nola_core::agent::SopRunStatus::Failed
            | nola_core::agent::SopRunStatus::Cancelled
    )
}

#[cfg(test)]
#[test]
fn terminal_sop_reuse_excludes_only_terminal_runs() {
    use nola_core::agent::SopRunStatus;

    for status in [
        SopRunStatus::Queued,
        SopRunStatus::Running,
        SopRunStatus::WaitingPermission,
        SopRunStatus::WaitingUser,
        SopRunStatus::WaitingExternal,
        SopRunStatus::Paused,
        SopRunStatus::Verifying,
        SopRunStatus::Compensating,
    ] {
        assert!(sop_run_status_is_active(status), "{status:?}");
    }
    for status in [
        SopRunStatus::Completed,
        SopRunStatus::Failed,
        SopRunStatus::Cancelled,
    ] {
        assert!(!sop_run_status_is_active(status), "{status:?}");
    }
}

fn sop_owner_is_visible_to_session(
    owner_session_id: Option<&str>,
    active_session_id: Option<&str>,
) -> bool {
    owner_session_id.is_none() || owner_session_id == active_session_id
}

#[cfg(test)]
mod sop_model_owner_boundary_tests {
    use super::sop_owner_is_visible_to_session;

    #[test]
    fn model_protocol_exposes_only_workspace_or_exact_session_owners() {
        assert!(sop_owner_is_visible_to_session(None, Some("session-current")));
        assert!(sop_owner_is_visible_to_session(
            Some("session-current"),
            Some("session-current")
        ));
        for unrelated_prompt in [
            "继续当前流程",
            "resume the current work",
            "reanudar el trabajo actual",
        ] {
            assert!(!unrelated_prompt.is_empty());
            assert!(!sop_owner_is_visible_to_session(
                Some("session-foreign"),
                Some("session-current")
            ));
        }
        assert!(!sop_owner_is_visible_to_session(Some("session-owned"), None));
    }
}

fn skill_skip(args: &Map<String, Value>) -> ToolOutcome {
    let closest_skill = tool_try!(required_string(args, "closest_skill"))
        .trim()
        .to_string();
    if !crate::builtin_skills::is_user_facing_id(&closest_skill) {
        return error(format!(
            "closest_skill must be an exact advertised Skill id: {closest_skill}"
        ));
    }
    let requested_outcome = tool_try!(required_string(args, "requested_outcome"))
        .trim()
        .to_string();
    let uncovered_requirement = tool_try!(required_string(args, "uncovered_requirement"))
        .trim()
        .to_string();
    if requested_outcome.is_empty() || uncovered_requirement.is_empty() {
        return error(
            "Skill skip requires both the requested outcome and a concrete uncovered requirement"
                .to_string(),
        );
    }
    ok_data(json!({
            "type": "skill_decision",
            "decision": "none",
            "closest_skill": closest_skill,
            "requested_outcome": requested_outcome,
            "uncovered_requirement": uncovered_requirement,
            "next_action": "continue_with_tool_catalog",
            "tool_discovery": "A missing Skill does not mean a requested tool is unavailable. Continue with the normal tool catalog; use tool_search with a semantic query or select:<exact_tool_name> when the required schema is deferred."
        }),
        "Skill · none".to_string(),
    )
}

#[cfg(test)]
#[test]
fn skill_skip_requires_a_structured_semantic_gap() {
    let obsolete_input = json!({
        "closest_skill": "nature-reviewer",
        "reason": "The task is small"
    });
    let obsolete_input = obsolete_input.as_object().expect("obsolete arguments");
    assert!(skill_skip(obsolete_input).error);

    let structured = json!({
        "closest_skill": "nature-reviewer",
        "requested_outcome": "Translate a manuscript into Spanish",
        "uncovered_requirement": "The advertised reviewer workflow evaluates a manuscript but does not translate it"
    });
    let outcome = skill_skip(structured.as_object().expect("structured arguments"));
    assert!(!outcome.error);
    assert!(outcome.text.contains("continue_with_tool_catalog"));
    assert!(outcome.text.contains("tool_search"));
    let structured = structured.as_object().expect("structured arguments");
    assert!(!skill_skip(structured).error);
}

#[cfg(test)]
#[test]
fn configured_skill_directory_resolves_frontmatter_identity_instead_of_embedded_copy() {
    let root = std::env::temp_dir().join(format!(
        "nola-configured-skill-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let directory = root.join("custom-folder-name");
    fs::create_dir_all(&directory).expect("create configured Skill directory");
    fs::write(
        directory.join("SKILL.md"),
        "---\nname: organize-files\ndescription: configured override\n---\n\nOVERRIDE_MARKER\n",
    )
    .expect("write configured Skill");

    let resolved = configured_skill_directory_for_id("organize-files", &[root.clone()])
        .expect("resolve configured Skill")
        .expect("configured Skill exists");
    assert_eq!(
        resolved,
        directory.canonicalize().expect("canonical directory")
    );
    assert!(
        configured_skill_directory_for_id("analyze-data", &[root.clone()])
            .expect("missing configured Skill is valid")
            .is_none()
    );

    let duplicate_root = std::env::temp_dir().join(format!(
        "nola-configured-skill-duplicate-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let duplicate = duplicate_root.join("another-folder");
    fs::create_dir_all(&duplicate).expect("create duplicate configured Skill directory");
    fs::write(
        duplicate.join("SKILL.md"),
        "---\nname: organize-files\ndescription: conflicting override\n---\n",
    )
    .expect("write duplicate configured Skill");
    let ambiguity = configured_skill_directory_for_id(
        "organize-files",
        &[root.clone(), duplicate_root.clone()],
    )
    .expect_err("duplicate exact ids must be rejected");
    assert!(ambiguity.contains("Ambiguous configured Skill id"));

    fs::remove_dir_all(root).expect("remove configured Skill fixture");
    fs::remove_dir_all(duplicate_root).expect("remove duplicate configured Skill fixture");
}

#[cfg(test)]
mod configured_skill_invocation_admission_tests {
    use super::*;

    fn write_skill(directory: &Path, id: &str, marker: &str) {
        fs::create_dir_all(directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {id}\ndescription: invocation fixture\n---\n\n{marker}\n"),
        )
        .unwrap();
    }

    fn context(roots: Vec<PathBuf>, workspace: &Path) -> ToolContext {
        ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::from_configured_dirs(Some(roots)),
            allowed_dirs: vec![workspace.to_path_buf()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "deny".to_string(), Vec::new()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(workspace.join("browser")),
        }
    }

    fn invoke(context: &ToolContext, id: &str) -> ToolOutcome {
        skill_invoke(
            &Map::from_iter([("skill".to_string(), Value::String(id.to_string()))]),
            context,
        )
    }

    #[test]
    fn actual_invocation_rejects_missing_or_invalid_roots_without_partial_fallback() {
        let fixture = tempfile::tempdir().unwrap();
        let valid = fixture.path().join("valid");
        write_skill(&valid, "requested-skill", "VALID_BODY");
        let missing = fixture.path().join("missing");
        let mixed = context(vec![valid, missing.clone()], fixture.path());
        let outcome = invoke(&mixed, "requested-skill");
        assert!(outcome.error);
        assert!(
            outcome.text.contains("configured Skill root"),
            "{}",
            outcome.text
        );
        assert!(outcome.data.is_none());
        assert!(!outcome.text.contains("VALID_BODY"));
        assert!(configured_skill_directory_for_id("requested-skill", &[missing]).is_err());

        let ordinary_file = fixture.path().join("ordinary-file");
        fs::write(&ordinary_file, "not a Skill directory").unwrap();
        let invalid = context(vec![ordinary_file], fixture.path());
        let outcome = invoke(&invalid, "requested-skill");
        assert!(outcome.error);
        assert!(outcome.text.contains("not a directory"), "{}", outcome.text);
        assert!(outcome.data.is_none());
    }

    #[test]
    fn actual_invocation_propagates_duplicate_ids_and_full_instruction_limit() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("configured");
        write_skill(&root.join("a"), "duplicate-skill", "BODY_A");
        write_skill(&root.join("b"), "duplicate-skill", "BODY_B");
        let duplicate = context(vec![root], fixture.path());
        let outcome = invoke(&duplicate, "duplicate-skill");
        assert!(outcome.error);
        assert!(outcome.text.contains("Ambiguous configured Skill id"));
        assert!(outcome.data.is_none());

        let oversized = fixture.path().join("oversized");
        write_skill(
            &oversized,
            "oversized-skill",
            &"x".repeat(MAX_SKILL_BYTES as usize),
        );
        let oversized_context = context(vec![oversized], fixture.path());
        let outcome = invoke(&oversized_context, "oversized-skill");
        assert!(outcome.error);
        assert!(outcome.text.contains("100000-byte instruction limit"));
        assert!(outcome.data.is_none());
    }

    #[test]
    fn optional_managed_custom_absence_does_not_relax_explicit_roots() {
        let fixture = tempfile::tempdir().unwrap();
        let absent = fixture.path().join("absent-custom");
        let entries = optional_custom_skill_context_entries(&absent).unwrap();
        assert!(skill_directory_for_id_from_entries("unknown-id", entries).is_none());
        assert!(configured_skill_directory_for_id("unknown-id", &[absent]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn actual_invocation_checks_directory_and_file_containment_before_loading_body() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().unwrap();
        let outside = fixture.path().join("outside");
        write_skill(&outside, "outside-skill", "OUTSIDE_SECRET_BODY");
        let trusted_directory = fixture.path().join("trusted-directory");
        fs::create_dir(&trusted_directory).unwrap();
        symlink(&outside, trusted_directory.join("linked-directory")).unwrap();
        let linked_directory = context(vec![trusted_directory], fixture.path());
        let outcome = invoke(&linked_directory, "outside-skill");
        assert!(outcome.error);
        assert!(outcome
            .text
            .contains("directory resolved outside its authorized root"));
        assert!(!outcome.text.contains("OUTSIDE_SECRET_BODY"));
        assert!(outcome.data.is_none());

        let trusted_file = fixture.path().join("trusted-file");
        fs::create_dir(&trusted_file).unwrap();
        symlink(outside.join("SKILL.md"), trusted_file.join("SKILL.md")).unwrap();
        let linked_file = context(vec![trusted_file], fixture.path());
        let outcome = invoke(&linked_file, "outside-skill");
        assert!(outcome.error);
        assert!(outcome.text.contains("resolved outside its authorized root"));
        assert!(!outcome.text.contains("OUTSIDE_SECRET_BODY"));
        assert!(outcome.data.is_none());
    }

    // macOS APFS rejects these byte names before a Skill can exist. Its URI
    // identity is covered by the pure Unix driver regression; real-file
    // invocation of an invalid UTF-8 name is exercised on Linux.
    #[cfg(target_os = "linux")]
    #[test]
    fn actual_invocation_uses_exact_non_utf8_path_and_reversible_resource_uri() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let fixture = tempfile::tempdir().unwrap();
        let trusted = fixture.path().join("trusted");
        let raw_directory = trusted.join(OsString::from_vec(b"collision-\xff".to_vec()));
        let replacement_directory = trusted.join("collision-\u{fffd}");
        write_skill(&raw_directory, "raw-path-skill", "RAW_PATH_BODY");
        write_skill(
            &replacement_directory,
            "replacement-path-skill",
            "REPLACEMENT_PATH_BODY",
        );
        let raw_directory = raw_directory.canonicalize().unwrap();
        let replacement_directory = replacement_directory.canonicalize().unwrap();
        assert_ne!(raw_directory, replacement_directory);
        assert_eq!(
            raw_directory.to_string_lossy(),
            replacement_directory.to_string_lossy()
        );

        let runtime = context(vec![trusted], fixture.path());
        assert_eq!(
            configured_skill_directory_for_id(
                "raw-path-skill",
                runtime.skills.configured_dirs().unwrap(),
            )
            .unwrap(),
            Some(raw_directory.clone())
        );
        let outcome = invoke(&runtime, "raw-path-skill");
        assert!(!outcome.error, "{}", outcome.text);
        let data = outcome.data.expect("explicit Skill result data");
        assert!(data["instructions"]
            .as_str()
            .unwrap()
            .contains("RAW_PATH_BODY"));
        assert!(!data["instructions"]
            .as_str()
            .unwrap()
            .contains("REPLACEMENT_PATH_BODY"));
        let resource_uri =
            url::Url::parse(data["resource_directory_uri"].as_str().unwrap()).unwrap();
        assert_eq!(resource_uri.to_file_path().unwrap(), raw_directory);
        assert_ne!(resource_uri.to_file_path().unwrap(), replacement_directory);
    }
}

#[cfg(test)]
#[test]
fn host_does_not_override_core_skill_budget_without_an_explicit_manifest_budget() {
    assert_eq!(materialized_skill_budget(&json!({})), Value::Null);

    let explicit = json!({
        "budget": {
            "max_model_turns": 3,
            "max_tool_calls": 7,
            "max_elapsed_millis": 9000
        }
    });
    assert_eq!(
        materialized_skill_budget(&explicit),
        json!({
            "max_model_turns": 3,
            "max_tool_calls": 7,
            "max_elapsed_millis": 9000
        })
    );
}

#[cfg(test)]
#[test]
fn validated_task_status_is_a_typed_validation_attestation() {
    use nola_core::agent::host::HostCapabilityEffect;

    let declaration = tool_effect_declaration_with_read_only("agent_task_status", true);
    assert_eq!(
        declaration.resolve(&json!({"status": "validated"})),
        HostCapabilityEffect::Validate
    );
    assert_eq!(
        declaration.resolve(&json!({"status": "in_progress"})),
        HostCapabilityEffect::Investigate
    );
}

fn skill_context_with_dirs(args: &Map<String, Value>, skill_dirs: &[PathBuf]) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_configured_allowed(&root_arg, skill_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_skills = tool_try!(validate_integer_arg(args, "max_skills", 20, 1, 100)) as usize;
    let max_bytes = tool_try!(validate_integer_arg(
        args,
        "max_bytes_per_skill",
        40_000,
        1,
        MAX_SKILL_BYTES
    )) as usize;
    let max_files = tool_try!(validate_integer_arg(
        args,
        "max_files_per_skill",
        10,
        0,
        MAX_SKILL_FILES
    )) as usize;
    let include_files = match args.get("include_files") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_files must be a boolean".to_string()),
    };
    let name_filter = args
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let exact_query_filter = (name_filter.is_empty()
        && query.contains('-')
        && query
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')))
    .then(|| query.clone());
    let exact_filter = if name_filter.is_empty() {
        exact_query_filter.as_deref().unwrap_or("")
    } else {
        name_filter.as_str()
    };
    let loading = !name_filter.is_empty() || !query.is_empty();
    let mut search_roots = vec![root.clone()];
    if !exact_filter.is_empty() {
        for candidate in skill_dirs {
            let Ok(candidate) =
                resolve_configured_allowed(candidate.to_string_lossy().as_ref(), skill_dirs, true)
            else {
                continue;
            };
            if candidate.is_dir() && !search_roots.contains(&candidate) {
                search_roots.push(candidate);
            }
        }
    }

    // Exact-name loading must search far enough to find the requested Skill;
    // max_skills limits returned matches, not how early directory iteration may
    // stop before reaching the matching directory.
    let discovery_limit = if exact_filter.is_empty() {
        max_skills
    } else {
        100
    };
    let mut scanned = 0usize;
    let mut discovered_names = Vec::<String>::new();
    let mut matches = Vec::<(PathBuf, PathBuf, String, String, String)>::new();
    'roots: for search_root in &search_roots {
        let (skill_paths, root_scanned) = discover_skill_files(search_root, discovery_limit);
        scanned += root_scanned;
        for path in skill_paths {
            if matches.iter().any(|(existing, ..)| existing == &path) {
                continue;
            }
            let Ok((content, _byte_count, _truncated)) = read_skill_file(&path, max_bytes) else {
                continue;
            };
            let title = skill_title(&path, &content);
            let description = skill_description(&content);
            if !discovered_names.contains(&title) {
                discovered_names.push(title.clone());
            }
            let rel_dir = path
                .parent()
                .map(|parent| relative_display(parent, search_root))
                .unwrap_or_default();
            let haystack = format!("{title}\n{description}\n{rel_dir}\n{content}").to_lowercase();
            if !exact_filter.is_empty()
                && title.to_lowercase() != exact_filter
                && path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_lowercase() != exact_filter)
                    .unwrap_or(true)
            {
                continue;
            }
            if exact_filter.is_empty() && !query.is_empty() && !haystack.contains(&query) {
                continue;
            }
            matches.push((path, search_root.clone(), title, description, content));
            if matches.len() >= max_skills {
                break 'roots;
            }
        }
    }

    let mut sections = vec![
        format!("# Skill context: {}", root.display()),
        format!("scanned_files: {scanned}"),
    ];
    if matches.is_empty() {
        if loading {
            return error(format!(
                "No enabled Skill matched name={:?} query={:?} across {} configured Skill director{} (discovered: {}); correct the Skill id or configuration instead of improvising the workflow.",
                name_filter,
                query,
                search_roots.len(),
                if search_roots.len() == 1 { "y" } else { "ies" },
                if discovered_names.is_empty() { "none".to_string() } else { discovered_names.join(", ") }
            ));
        }
        sections.push("(no skills found)".to_string());
    } else if loading && !exact_filter.is_empty() && matches.len() > 1 {
        return error(format!(
            "Ambiguous configured Skill id {exact_filter:?}: {} matching directories",
            matches.len()
        ));
    } else if loading {
        for (path, match_root, title, description, content) in &matches {
            let skill_dir = path.parent().unwrap_or(path);
            let rel_dir = relative_display(skill_dir, match_root);
            sections.extend([
                String::new(),
                format!("## Skill: {title}"),
                format!("path: {rel_dir}/SKILL.md"),
            ]);
            if !description.is_empty() {
                sections.push(format!("description: {description}"));
            }
            sections.extend([
                format!("base_directory: {}", skill_dir.display()),
                "relative paths in this skill are resolved from base_directory.".to_string(),
                String::new(),
                content.trim_end().to_string(),
            ]);
            if include_files {
                let files = skill_sample_files(skill_dir, max_files);
                sections.extend([String::new(), "skill_files:".to_string()]);
                if files.is_empty() {
                    sections.push("(none)".to_string());
                } else {
                    sections.extend(files.iter().map(|item| {
                        format!(
                            "- {}",
                            item.strip_prefix(skill_dir)
                                .map(path_slash)
                                .unwrap_or_else(|_| item.display().to_string())
                        )
                    }));
                }
            }
        }
    } else {
        sections.extend([String::new(), "## Available skills".to_string()]);
        for (path, match_root, title, description, _content) in &matches {
            let rel_dir = path
                .parent()
                .map(|parent| relative_display(parent, match_root))
                .unwrap_or_default();
            let suffix = if description.is_empty() {
                String::new()
            } else {
                format!(" — {description}")
            };
            sections.push(format!("- {title} ({rel_dir}/SKILL.md){suffix}"));
        }
        sections.extend([
            String::new(),
            "Call skill_context with name or query to load a matching skill.".to_string(),
        ]);
    }

    ok(
        sections.join("\n"),
        format!(
            "Skill context ({} skill{})",
            matches.len(),
            if matches.len() == 1 { "" } else { "s" }
        ),
    )
}

fn nola_orchestrate(args: &Map<String, Value>, skills: &SkillRuntimeContext) -> ToolOutcome {
    let execution_spec: nola_domain::automation_execution::AutomationDefinitionSpec = match args.get("execution_spec")
        .ok_or_else(|| "execution_spec is required".to_string())
        .and_then(|value| serde_json::from_value(value.clone()).map_err(|error|format!("invalid execution_spec: {error}"))) {
        Ok(value) => value,
        Err(message) => return error(message),
    };
    if let Err(message) = execution_spec.validate() { return error(message); }
    let spec = match execution_spec.agent() { Ok(value) => value, Err(message) => return error(message) };
    let name = tool_try!(required_string(args, "name")).trim().to_string();
    let Some(prompt) = args.get("prompt").and_then(Value::as_str).map(ToOwned::to_owned) else {
        return error("prompt is required".into());
    };
    let skill_id = tool_try!(required_string(args, "skill_id"))
        .trim()
        .to_string();
    let frequency = tool_try!(required_string(args, "frequency"))
        .trim()
        .to_string();
    if name.is_empty() {
        return error("name must not be empty".to_string());
    }
    if prompt.trim().is_empty() {
        return error("prompt must not be empty".to_string());
    }
    if spec.prompt != prompt {
        return error("prompt must match execution_spec.spec.prompt exactly".into());
    }
    let catalog = match skills.index() {
        Ok(catalog) => catalog,
        Err(message) => return error(format!("automation Skill catalog unavailable: {message}")),
    };
    if !catalog.iter().any(|item| item.get("id").and_then(Value::as_str) == Some(skill_id.as_str())) {
        return error(format!(
            "skill_id is not available in the current Host catalog: {skill_id}"
        ));
    }
    if spec.skill_refs != vec![skill_id.clone()] {
        return error("skill_id must match execution_spec.spec.skill_refs exactly".into());
    }
    if !matches!(frequency.as_str(), "hourly" | "daily" | "weekly") {
        return error("frequency must be hourly, daily, or weekly".to_string());
    }
    let hour = tool_try!(validate_integer_arg(args, "hour", 9, 0, 23));
    let minute = tool_try!(validate_integer_arg(args, "minute", 0, 0, 59));
    let weekday = tool_try!(validate_integer_arg(args, "weekday", 1, 0, 6));
    let enabled = match args.get("enabled") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("enabled must be a boolean".to_string()),
    };
    let run_when_data_changes = match args.get("run_when_data_changes") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => false,
        _ => return error("run_when_data_changes must be a boolean".to_string()),
    };
    let maximum_data_age_minutes = tool_try!(validate_integer_arg(
        args,
        "maximum_data_age_minutes",
        60,
        5,
        10_080,
    ));
    let timeout_minutes = tool_try!(validate_integer_arg(args, "timeout_minutes", 30, 1, 720));
    let retry_count = tool_try!(validate_integer_arg(args, "retry_count", 0, 0, 3));

    let context_sources = match args.get("context_sources") {
        Some(Value::Array(items)) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                let Some(value) = item.as_str() else {
                    return error("context_sources must contain strings".to_string());
                };
                if !matches!(value, "personal_data" | "web_sources") {
                    return error(format!("unsupported context source: {value}"));
                }
                let value = value.to_string();
                if !values.contains(&value) {
                    values.push(value);
                }
            }
            values
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("context_sources must be a list".to_string()),
    };

    let web_sources = match args.get("web_sources") {
        Some(Value::Array(items)) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                let Some(source) = item.as_object() else {
                    return error("web_sources must contain objects".to_string());
                };
                let source_id = tool_try!(required_string(source, "id"));
                if uuid::Uuid::parse_str(&source_id).is_err()
                    || !spec.context_refs.iter().any(|reference|
                        reference.kind == nola_domain::automation_execution::AutomationContextKind::WebSource
                        && reference.id == source_id) {
                    return error("web source id must be its frozen execution_spec reference".into());
                }
                let source_name = tool_try!(required_string(source, "name"))
                    .trim()
                    .to_string();
                let url = tool_try!(required_string(source, "url")).trim().to_string();
                if source_name.is_empty()
                    || !(url.starts_with("https://") || url.starts_with("http://"))
                {
                    return error("each web source needs a name and an http(s) URL".to_string());
                }
                let format = source
                    .get("format")
                    .and_then(Value::as_str)
                    .unwrap_or("automatic");
                if !matches!(format, "automatic" | "json" | "rss" | "text") {
                    return error(format!("unsupported web source format: {format}"));
                }
                let refresh_frequency = source
                    .get("refresh_frequency")
                    .and_then(Value::as_str)
                    .unwrap_or("manual");
                if !matches!(refresh_frequency, "manual" | "once" | "daily" | "weekly") {
                    return error(format!(
                        "unsupported web source refresh frequency: {refresh_frequency}"
                    ));
                }
                values.push(json!({
                    "id": source_id,
                    "name": source_name,
                    "url": url,
                    "format": format,
                    "refresh_frequency": refresh_frequency,
                }));
            }
            values
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("web_sources must be a list".to_string()),
    };

    let orchestration_key =
        sha256_hex(format!("{}\n{}\n{}", name.to_ascii_lowercase(), skill_id, prompt).as_bytes());
    let action_id = format!("act-agent-{}", &orchestration_key[..24]);
    let updated_at = Utc::now().to_rfc3339();
    let time = format!("{hour:02}:{minute:02}");
    let persisted_record = json!({
        "executionSpec":execution_spec,
        "id": action_id,
        "name": name,
        "url": format!("agent://skill/{skill_id}"),
        "format": "agent",
        "sourceKind": "agent",
        "sourceKindLabel": "Agent",
        "localOnly": true,
        "userPrompt": prompt,
        "frequency": frequency,
        "weekday": if frequency == "weekly" { Some(weekday) } else { None },
        "time": time,
        "timezone": "local",
        "enabled": enabled,
        "nextScheduledAt": Value::Null,
        "updatedAt": updated_at,
        "latestResult": Value::Null,
        "scriptApproved": Value::Null,
        "scriptRuntime": Value::Null,
        "scriptCwd": Value::Null,
        "scriptPreview": Value::Null,
        "skillId": skill_id,
        "runWhenDataChanges": run_when_data_changes,
        "contextSources": context_sources,
        "webSources": web_sources,
        "maximumDataAgeMinutes": maximum_data_age_minutes,
        "timeoutMinutes": timeout_minutes,
        "retryCount": retry_count,
        "orchestrationKey": orchestration_key,
    });
    let mut store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return error(format!("open automation store failed: {err}")),
    };
    let (committed, verification) = match store.upsert_web_scheduled_action_with_readback(&persisted_record) {
        Ok(record) => record,
        Err(err) => return error(format!("persist automation failed: {err}")),
    };
    automation_mutation_outcome(committed, verification)
}

fn automation_mutation_outcome(
    committed: nola_application::AutomationDefinition,
    verification: Result<Value, String>,
) -> ToolOutcome {
    use nola_host_protocol::tool_result::{ExecutionReceipt, ExecutionState, PostconditionCheck, ResourceEffect};
    let resource_uri = format!("nola://automation/{}", committed.id);
    let version = format!("revision:{}", committed.revision);
    let verified = verification.is_ok();
    let receipt = ExecutionReceipt {
        local_stop: None,
        state: if verified { ExecutionState::Completed } else { ExecutionState::Partial },
        effects: vec![ResourceEffect {
            resource_uri: resource_uri.clone(),
            operation: "automation.upsert".into(),
            before_version: None,
            after_version: Some(version.clone()),
        }],
        checks: vec![PostconditionCheck {
            kind: nola_host_protocol::tool_result::ExecutionCheckKind::Postcondition,
            resource_uri: resource_uri.clone(),
            version,
            predicate: "durable_definition_matches".into(),
            passed: verified,
        }],
    };
    let persisted = match verification {
        Ok(value) => value,
        Err(reason) => {
            return error(format!("Automation was committed, but verification failed: {reason}"))
                .with_data(json!({"id": committed.id, "revision": committed.revision,
                    "persisted": true, "read_back_verified": false, "error": reason}))
                .with_execution(receipt);
        }
    };
    let canonical = json!({
        "automation": persisted,
        "persisted": true,
        "read_back_verified": true,
        "native_commit_required": false,
        "status": "persisted",
    });
    let mut outcome = ok(canonical.to_string(), format!("Automation saved: {}", committed.name))
        .with_data(canonical)
        .with_execution(receipt);
    outcome.artifacts.push(nola_core::agent::host::HostArtifact {
        uri: resource_uri,
        name: format!("{}.json", committed.id),
        mime_type: Some("application/json".to_string()),
        operation: Some("upsert".to_string()),
    });
    outcome
}

#[cfg(test)]
mod automation_execution_receipt_tests {
    use super::*;
    use nola_host_protocol::tool_result::ExecutionState;

    fn spec() -> Value {
        json!({"kind":"agent","spec":{"prompt":"Review","execution_scope":{"kind":"local_workspace","root":"/tmp"},
            "permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})
    }

    #[test]
    fn real_automation_upserts_attest_committed_versions_and_independent_readback() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(&directory.path().join("automations.db")).unwrap();
        let mut action = json!({"id":"agent-receipt", "name":"Daily review", "format":"agent",
            "enabled":true, "frequency":"daily", "userPrompt":"Review", "revision":999,
            "createdAt":1000, "updatedAt":1000,"executionSpec":spec()});
        for (index, (field, value)) in [
            ("enabled", json!(true)),
            ("enabled", json!(false)),
            ("frequency", json!("weekly")),
            ("userPrompt", json!("Review changes")),
        ].into_iter().enumerate() {
            action[field] = value;
            let (committed, verification) = store.upsert_web_scheduled_action_with_readback(&action).unwrap();
            assert_eq!(committed.revision, index as i64 + 1);
            let expected_version = format!("revision:{}", committed.revision);
            let outcome = automation_mutation_outcome(committed, verification);
            assert!(!outcome.error);
            assert_eq!(outcome.data.as_ref().unwrap()["read_back_verified"], true);
            let execution = outcome.execution.unwrap();
            execution.validate().unwrap();
            assert_eq!(execution.state, ExecutionState::Completed);
            assert_eq!(execution.effects.len(), 1);
            assert_eq!(execution.effects[0].resource_uri, "nola://automation/agent-receipt");
            assert_eq!(execution.effects[0].after_version.as_deref(), Some(expected_version.as_str()));
            assert_eq!(execution.checks.len(), 1);
            assert_eq!(execution.checks[0].resource_uri, execution.effects[0].resource_uri);
            assert_eq!(execution.checks[0].version, expected_version);
            assert!(execution.checks[0].passed);
        }
        // Identical replay is verified at the same canonical version, not a
        // fabricated extra mutation or metadata-provided revision.
        let (committed, verification) = store.upsert_web_scheduled_action_with_readback(&action).unwrap();
        assert_eq!(committed.revision, 4);
        assert!(verification.is_ok());
    }

    #[test]
    fn failed_verification_preserves_the_actual_committed_effect() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("automations.db");
        let mut store = NolaStore::open(&database).unwrap();
        let (committed, verification) = store.upsert_web_scheduled_action_with_readback(
            &json!({"id":"committed-before-read-error", "name":"Review", "format":"agent","executionSpec":spec()}),
        ).unwrap();
        assert!(verification.is_ok());
        let outcome = automation_mutation_outcome(committed.clone(), Err("injected read failure".into()));
        assert!(outcome.error);
        assert_eq!(outcome.data.as_ref().unwrap()["persisted"], true);
        assert_eq!(outcome.data.as_ref().unwrap()["read_back_verified"], false);
        let receipt = outcome.execution.unwrap();
        receipt.validate().unwrap();
        assert_eq!(receipt.state, ExecutionState::Partial);
        assert_eq!(receipt.effects[0].after_version.as_deref(), Some("revision:1"));
        assert!(!receipt.checks[0].passed);
        drop(store);
        let mut application = crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        let observed: Option<nola_application::AutomationDefinition> = application.execute(
            nola_application::ApplicationCommand::Automation {
                command: nola_application::AutomationCommand::Definition {
                    command: nola_application::AutomationDefinitionCommand::Get { id: committed.id.clone() },
                },
            },
        ).unwrap();
        assert_eq!(observed, Some(committed));
    }
}

fn agent_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_agents = tool_try!(validate_integer_arg(args, "max_agents", 20, 1, 100)) as usize;
    let max_bytes = tool_try!(validate_integer_arg(
        args,
        "max_bytes_per_agent",
        40_000,
        1,
        MAX_AGENT_BYTES,
    )) as usize;
    let name_filter = args
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let (agent_paths, scanned) = discover_agent_files(&root, max_agents);
    let mut matches = Vec::<(PathBuf, String, String, HashMap<String, String>, String)>::new();
    for path in agent_paths {
        let Ok(raw) = read_text_prefix(&path, max_bytes) else {
            continue;
        };
        let fields = markdown_frontmatter_fields(&raw);
        let content = strip_markdown_frontmatter(&raw).trim_end().to_string();
        let name = agent_name(&path, &fields);
        let description = agent_description(&content, &fields);
        let rel = relative_display(&path, &root);
        let mut metadata = fields
            .iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>();
        metadata.sort();
        let haystack = format!(
            "{name}\n{description}\n{rel}\n{}\n{content}",
            metadata.join("\n")
        )
        .to_lowercase();
        if !name_filter.is_empty()
            && name.to_lowercase() != name_filter
            && path
                .file_stem()
                .and_then(|name| name.to_str())
                .map(|stem| stem.to_lowercase() != name_filter)
                .unwrap_or(true)
        {
            continue;
        }
        if !query.is_empty() && !haystack.contains(&query) {
            continue;
        }
        matches.push((path, name, description, fields, content));
    }

    let loading = !name_filter.is_empty() || !query.is_empty();
    let mut sections = vec![
        format!("# Agent context: {}", root.display()),
        format!("scanned_files: {scanned}"),
    ];
    if matches.is_empty() {
        sections.push(if loading {
            "(no matching agents found)".to_string()
        } else {
            "(no custom agents found)".to_string()
        });
    } else if loading {
        for (path, name, description, fields, content) in &matches {
            sections.extend([
                String::new(),
                format!("## Agent: {name}"),
                format!("path: {}", relative_display(path, &root)),
            ]);
            if !description.is_empty() {
                sections.push(format!("description: {description}"));
            }
            let mut metadata = fields
                .iter()
                .filter(|(key, value)| {
                    key.as_str() != "name" && key.as_str() != "description" && !value.is_empty()
                })
                .map(|(key, value)| format!("{key}: {value}"))
                .collect::<Vec<_>>();
            metadata.sort();
            if !metadata.is_empty() {
                sections.push("metadata:".to_string());
                sections.extend(metadata.into_iter().map(|item| format!("- {item}")));
            }
            sections.extend([
                String::new(),
                if content.is_empty() {
                    "(empty prompt)".to_string()
                } else {
                    content.to_string()
                },
            ]);
        }
    } else {
        sections.extend([String::new(), "## Available custom agents".to_string()]);
        for (path, name, description, fields, _content) in &matches {
            let details = [
                fields.get("mode").cloned().unwrap_or_default(),
                fields.get("model").cloned().unwrap_or_default(),
            ]
            .into_iter()
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
            let details_suffix = if details.is_empty() {
                String::new()
            } else {
                format!(" [{details}]")
            };
            let suffix = if description.is_empty() {
                String::new()
            } else {
                format!(" — {description}")
            };
            sections.push(format!(
                "- {name} ({}){details_suffix}{suffix}",
                relative_display(path, &root)
            ));
        }
        sections.extend([
            String::new(),
            "Call agent_context with name or query to load a matching agent definition."
                .to_string(),
        ]);
    }

    ok(
        sections.join("\n"),
        format!(
            "Agent context ({} agent{})",
            matches.len(),
            if matches.len() == 1 { "" } else { "s" }
        ),
    )
}

fn command_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_commands = tool_try!(validate_integer_arg(args, "max_commands", 20, 1, 100)) as usize;
    let max_bytes = tool_try!(validate_integer_arg(
        args,
        "max_bytes_per_command",
        40_000,
        1,
        MAX_COMMAND_BYTES,
    )) as usize;
    let name_filter = args
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let (command_paths, scanned) = discover_command_files(&root, max_commands);
    let mut matches = Vec::<(PathBuf, String, String, HashMap<String, String>, String)>::new();
    for path in command_paths {
        let Ok(raw) = read_text_prefix(&path, max_bytes) else {
            continue;
        };
        let fields = markdown_frontmatter_fields(&raw);
        let content = strip_markdown_frontmatter(&raw).trim_end().to_string();
        let name = command_name(&path, &root, &fields);
        let description = command_description(&content, &fields);
        let rel = relative_display(&path, &root);
        let mut metadata = fields
            .iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>();
        metadata.sort();
        let haystack = format!(
            "{name}\n{description}\n{rel}\n{}\n{content}",
            metadata.join("\n")
        )
        .to_lowercase();
        if !name_filter.is_empty()
            && name.to_lowercase() != name_filter
            && path
                .file_stem()
                .and_then(|name| name.to_str())
                .map(|stem| stem.to_lowercase() != name_filter)
                .unwrap_or(true)
        {
            continue;
        }
        if !query.is_empty() && !haystack.contains(&query) {
            continue;
        }
        matches.push((path, name, description, fields, content));
    }

    let loading = !name_filter.is_empty() || !query.is_empty();
    let mut sections = vec![
        format!("# Command context: {}", root.display()),
        format!("scanned_files: {scanned}"),
    ];
    if matches.is_empty() {
        sections.push(if loading {
            "(no matching commands found)".to_string()
        } else {
            "(no custom commands found)".to_string()
        });
    } else if loading {
        for (path, name, description, fields, content) in &matches {
            sections.extend([
                String::new(),
                format!("## Command: /{name}"),
                format!("path: {}", relative_display(path, &root)),
            ]);
            if !description.is_empty() {
                sections.push(format!("description: {description}"));
            }
            let mut metadata = fields
                .iter()
                .filter(|(key, value)| {
                    key.as_str() != "name" && key.as_str() != "description" && !value.is_empty()
                })
                .map(|(key, value)| format!("{key}: {value}"))
                .collect::<Vec<_>>();
            metadata.sort();
            if !metadata.is_empty() {
                sections.push("metadata:".to_string());
                sections.extend(metadata.into_iter().map(|item| format!("- {item}")));
            }
            sections.extend([
                String::new(),
                if content.is_empty() {
                    "(empty prompt)".to_string()
                } else {
                    content.to_string()
                },
            ]);
        }
    } else {
        sections.extend([String::new(), "## Available custom commands".to_string()]);
        for (path, name, description, fields, _content) in &matches {
            let details = [
                fields.get("agent").cloned().unwrap_or_default(),
                fields.get("model").cloned().unwrap_or_default(),
                fields.get("subtask").cloned().unwrap_or_default(),
            ]
            .into_iter()
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
            let details_suffix = if details.is_empty() {
                String::new()
            } else {
                format!(" [{details}]")
            };
            let suffix = if description.is_empty() {
                String::new()
            } else {
                format!(" — {description}")
            };
            sections.push(format!(
                "- /{name} ({}){details_suffix}{suffix}",
                relative_display(path, &root)
            ));
        }
        sections.extend([
            String::new(),
            "Call command_context with name or query to load a matching command workflow."
                .to_string(),
        ]);
    }

    ok(
        sections.join("\n"),
        format!(
            "Command context ({} command{})",
            matches.len(),
            if matches.len() == 1 { "" } else { "s" }
        ),
    )
}

fn agent_config_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_files = tool_try!(validate_integer_arg(args, "max_files", 30, 1, 100)) as usize;
    let max_bytes = tool_try!(validate_integer_arg(
        args,
        "max_bytes_per_file",
        40_000,
        1,
        MAX_AGENT_CONFIG_BYTES,
    )) as usize;
    let include_content = match args.get("include_content") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return error("include_content must be a boolean".to_string()),
    };
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let (config_paths, scanned) = discover_agent_config_files(&root, max_files);
    let mut matches = Vec::<(PathBuf, String, Vec<String>, String)>::new();
    for path in config_paths {
        let Ok(raw) = read_text_prefix(&path, max_bytes) else {
            continue;
        };
        let redacted = redact_agent_config_text(&raw).trim_end().to_string();
        let kind = agent_config_kind(&path, &root);
        let signals = agent_config_signals(&redacted);
        let rel = relative_display(&path, &root);
        let haystack = format!("{rel}\n{kind}\n{}\n{redacted}", signals.join("\n")).to_lowercase();
        if !query.is_empty() && !haystack.contains(&query) {
            continue;
        }
        matches.push((path, kind, signals, redacted));
    }

    let loading = include_content || !query.is_empty();
    let mut sections = vec![
        format!("# Agent config context: {}", root.display()),
        format!("scanned_files: {scanned}"),
    ];
    if matches.is_empty() {
        sections.push(if loading {
            "(no matching agent config files found)".to_string()
        } else {
            "(no agent config files found)".to_string()
        });
    } else if loading {
        for (path, kind, signals, redacted) in &matches {
            sections.extend([
                String::new(),
                format!("## {}", relative_display(path, &root)),
                format!("kind: {kind}"),
            ]);
            if !signals.is_empty() {
                sections.push("signals:".to_string());
                sections.extend(signals.iter().map(|signal| format!("- {signal}")));
            }
            sections.extend([
                String::new(),
                "```".to_string(),
                if redacted.is_empty() {
                    "(empty file)".to_string()
                } else {
                    redacted.to_string()
                },
                "```".to_string(),
            ]);
        }
    } else {
        sections.extend([String::new(), "## Available agent config files".to_string()]);
        for (path, kind, signals, _redacted) in &matches {
            let signal_suffix = if signals.is_empty() {
                String::new()
            } else {
                format!(
                    " — {}",
                    signals
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            };
            sections.push(format!(
                "- {} [{kind}]{signal_suffix}",
                relative_display(path, &root)
            ));
        }
        sections.extend([
            String::new(),
            "Call agent_config_context with include_content=true or query to load redacted config content."
                .to_string(),
        ]);
    }

    ok(
        sections.join("\n"),
        format!(
            "Agent config context ({} file{})",
            matches.len(),
            if matches.len() == 1 { "" } else { "s" }
        ),
    )
}

fn ci_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let include_content = match args.get("include_content") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return error("include_content must be a boolean".to_string()),
    };
    let max_files = tool_try!(validate_integer_arg(args, "max_files", 40, 1, 100)) as usize;
    let max_bytes = tool_try!(validate_integer_arg(
        args,
        "max_bytes_per_file",
        30_000,
        1,
        100_000
    )) as usize;
    let (paths, scanned) = discover_ci_files(&root, max_files);
    let mut entries = Vec::<(PathBuf, String, Vec<String>, String)>::new();
    for path in paths {
        let Ok(text) = read_text_prefix(&path, max_bytes) else {
            continue;
        };
        let kind = ci_file_kind(&path, &root);
        let signals = ci_file_signals(&path, &root, &text);
        entries.push((path, kind, signals, text.trim_end().to_string()));
    }

    let mut sections = vec![
        format!("# CI context: {}", root.display()),
        format!("scanned_files: {scanned}"),
    ];
    if entries.is_empty() {
        sections.push("(no CI or task runner files found)".to_string());
    } else {
        sections.extend([String::new(), "## CI and task files".to_string()]);
        for (path, kind, signals, text) in &entries {
            sections.push(format!("- {} [{kind}]", relative_display(path, &root)));
            sections.extend(signals.iter().map(|signal| format!("  - {signal}")));
            if include_content {
                sections.extend([
                    String::new(),
                    format!("### {}", relative_display(path, &root)),
                    "```".to_string(),
                    if text.is_empty() {
                        "(empty file)".to_string()
                    } else {
                        text.to_string()
                    },
                    "```".to_string(),
                    String::new(),
                ]);
            }
        }
        let task_commands = task_file_commands(&root);
        sections.extend([String::new(), "## Local task commands".to_string()]);
        if task_commands.is_empty() {
            sections.push("(none inferred)".to_string());
        } else {
            sections.extend(task_commands.iter().map(|command| format!("- {command}")));
        }
    }

    ok(
        sections.join("\n"),
        format!(
            "CI context ({} file{})",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" }
        ),
    )
}
