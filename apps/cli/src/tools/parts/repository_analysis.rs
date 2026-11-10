fn context_pack(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let explicit_paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
                if !resolved.starts_with(&root) {
                    return error(format!(
                        "path is outside context_pack root: {}",
                        resolved.display()
                    ));
                }
                if resolved.is_file() && !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let include_changed = match args.get("include_changed") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_changed must be a boolean".to_string()),
    };
    let include_docs = match args.get("include_project_docs") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_project_docs must be a boolean".to_string()),
    };
    let include_instructions = match args.get("include_instructions") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_instructions must be a boolean".to_string()),
    };
    let include_skills = match args.get("include_skills") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_skills must be a boolean".to_string()),
    };
    let max_files = tool_try!(validate_integer_arg(args, "max_files", 12, 1, 50)) as usize;
    let max_skills = tool_try!(validate_integer_arg(args, "max_skills", 10, 1, 100)) as usize;
    let max_symbols = tool_try!(validate_integer_arg(
        args,
        "max_symbols_per_file",
        30,
        1,
        200
    )) as usize;
    let max_imports = tool_try!(validate_integer_arg(
        args,
        "max_imports_per_file",
        20,
        0,
        100
    )) as usize;
    let max_instruction_bytes = tool_try!(validate_integer_arg(
        args,
        "max_instruction_bytes_per_file",
        12_000,
        1,
        MAX_INSTRUCTION_BYTES
    )) as usize;

    let (manifests, languages, docs, scanned, _truncated) = scan_project(&root, 5000);
    let targets = context_pack_paths(&root, &explicit_paths, include_changed, max_files);
    let instruction_names = INSTRUCTION_FILE_NAMES
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let instruction_targets = if targets.is_empty() {
        if explicit_paths.is_empty() {
            vec![root.clone()]
        } else {
            explicit_paths.clone()
        }
    } else {
        targets.clone()
    };
    let instruction_files = if include_instructions {
        collect_instruction_files(&root, &instruction_targets, &instruction_names)
    } else {
        Vec::new()
    };
    let (skill_files, skill_scanned) = if include_skills {
        discover_skill_files(&root, max_skills)
    } else {
        (Vec::new(), 0)
    };
    let (diagnostics, commands, related_tests, target_paths) =
        validation_plan_items(&root, &targets, true);
    let mut git_root_text = "(none)".to_string();
    let mut status_text = "(not a git worktree)".to_string();
    let mut changed_symbol_lines = Vec::new();
    if let Ok(git_root_output) = run_git(&root, &["rev-parse", "--show-toplevel"]) {
        if git_root_output.status.success() {
            git_root_text = String::from_utf8_lossy(&git_root_output.stdout)
                .trim()
                .to_string();
            let git_root = PathBuf::from(&git_root_text)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&git_root_text));
            if let Ok(status) = run_git(&root, &["status", "--short", "--", "."]) {
                let text = String::from_utf8_lossy(&status.stdout).trim().to_string();
                if status.status.success() {
                    status_text = if text.is_empty() {
                        "(clean for this root)".to_string()
                    } else {
                        text
                    };
                }
            }
            let mut pathspecs = git_pathspecs(&git_root, &targets);
            if pathspecs.is_empty() {
                if let Ok(root_scope) = root.strip_prefix(&git_root) {
                    let root_scope = path_slash(root_scope);
                    if !root_scope.is_empty() {
                        pathspecs.push(root_scope);
                    }
                }
            }
            let unstaged_args = git_args_with_pathspecs(&["diff", "--unified=0"], &pathspecs);
            let staged_args =
                git_args_with_pathspecs(&["diff", "--cached", "--unified=0"], &pathspecs);
            let mut changed_lines = HashMap::new();
            if let Ok(unstaged) = run_git_strings(&git_root, &unstaged_args) {
                if unstaged.status.success() {
                    changed_lines =
                        parse_diff_changed_lines(&String::from_utf8_lossy(&unstaged.stdout));
                }
            }
            if let Ok(staged) = run_git_strings(&git_root, &staged_args) {
                if staged.status.success() {
                    merge_changed_lines(
                        &mut changed_lines,
                        parse_diff_changed_lines(&String::from_utf8_lossy(&staged.stdout)),
                    );
                }
            }
            let (entries, _scanned) = changed_symbol_entries(&git_root, &changed_lines, 80);
            changed_symbol_lines = entries;
        }
    }

    let mut language_items = languages.into_iter().collect::<Vec<_>>();
    language_items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut sections = vec![
        format!("# Context pack: {}", root.display()),
        format!("git_root: {git_root_text}"),
        format!("scanned_project_files: {scanned}"),
        String::new(),
        "## Project".to_string(),
        "manifests:".to_string(),
        if manifests.is_empty() {
            "(none)".to_string()
        } else {
            manifests
                .iter()
                .take(20)
                .map(|path| format!("- {}", relative_display(path, &root)))
                .collect::<Vec<_>>()
                .join("\n")
        },
        String::new(),
        "languages:".to_string(),
        if language_items.is_empty() {
            "(none)".to_string()
        } else {
            language_items
                .into_iter()
                .map(|(language, count)| format!("- {language}: {count}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
    ];
    if include_docs {
        sections.extend([
            String::new(),
            "docs:".to_string(),
            if docs.is_empty() {
                "(none)".to_string()
            } else {
                docs.iter()
                    .take(20)
                    .map(|path| format!("- {}", relative_display(path, &root)))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        ]);
    }
    if include_instructions {
        sections.extend([String::new(), "## Instructions".to_string()]);
        if instruction_files.is_empty() {
            sections.push("(none)".to_string());
        } else {
            for path in &instruction_files {
                let rel = relative_display(path, &root);
                match read_instruction_file(path, max_instruction_bytes) {
                    Ok((content, byte_count, truncated)) => {
                        sections.extend([
                            format!("### {rel}"),
                            format!("bytes: {byte_count}"),
                            content.trim_end().to_string(),
                        ]);
                        if truncated {
                            sections
                                .push(format!("...[truncated to {max_instruction_bytes} bytes]"));
                        }
                        sections.push(String::new());
                    }
                    Err(err) => sections.extend([
                        format!("### {rel}"),
                        format!("error: unable to read file: {err}"),
                        String::new(),
                    ]),
                }
            }
        }
    }
    if include_skills {
        sections.extend([
            String::new(),
            "## Skills".to_string(),
            format!("scanned_files: {skill_scanned}"),
        ]);
        if skill_files.is_empty() {
            sections.push("(none)".to_string());
        } else {
            for path in &skill_files {
                let Ok((content, _byte_count, _truncated)) = read_instruction_file(path, 40_000)
                else {
                    continue;
                };
                let title = skill_title(path, &content);
                let description = skill_description(&content);
                let rel_dir = path
                    .parent()
                    .map(|parent| relative_display(parent, &root))
                    .unwrap_or_default();
                let suffix = if description.is_empty() {
                    String::new()
                } else {
                    format!(" — {description}")
                };
                sections.push(format!("- {title} ({rel_dir}/SKILL.md){suffix}"));
            }
            sections.push(
                "Call skill_context with name or query to load a matching skill.".to_string(),
            );
        }
    }
    sections.extend([
        String::new(),
        "## Git changes".to_string(),
        status_text,
        String::new(),
        "## Target files".to_string(),
    ]);
    if targets.is_empty() {
        sections.push("(none)".to_string());
    } else {
        for path in &targets {
            sections.extend(context_pack_file_section(
                &root,
                path,
                max_symbols,
                max_imports,
            ));
            sections.push(String::new());
        }
    }
    sections.extend([
        "## Changed symbols".to_string(),
        if changed_symbol_lines.is_empty() {
            "(none)".to_string()
        } else {
            changed_symbol_lines.join("\n")
        },
        String::new(),
        "## Related tests".to_string(),
        if related_tests.is_empty() {
            "(none)".to_string()
        } else {
            related_tests
                .iter()
                .map(|path| format!("- {}", relative_display(path, &root)))
                .collect::<Vec<_>>()
                .join("\n")
        },
        String::new(),
        "## Suggested validation".to_string(),
        if diagnostics.is_empty() && commands.is_empty() {
            "(none inferred)".to_string()
        } else {
            diagnostics
                .iter()
                .chain(commands.iter())
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
    ]);
    ok(
        sections.join("\n"),
        format!(
            "Context pack ({} target file{})",
            target_paths.len(),
            if target_paths.len() == 1 { "" } else { "s" }
        ),
    )
}

fn focused_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let query = tool_try!(required_string(args, "query"));
    let seed_paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_project_seed_path(&root, path, &ctx.allowed_dirs));
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let max_files = tool_try!(validate_integer_arg(args, "max_files", 8, 1, 30)) as usize;
    let max_scan_files = tool_try!(validate_integer_arg(
        args,
        "max_scan_files",
        5000,
        100,
        MAX_PROJECT_SCAN_FILES
    )) as usize;
    let max_symbols = tool_try!(validate_integer_arg(
        args,
        "max_symbols_per_file",
        20,
        1,
        100
    )) as usize;
    let max_snippets = tool_try!(validate_integer_arg(
        args,
        "max_snippets_per_file",
        3,
        0,
        10
    )) as usize;
    let include_instructions = match args.get("include_instructions") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_instructions must be a boolean".to_string()),
    };
    let max_instruction_bytes = tool_try!(validate_integer_arg(
        args,
        "max_instruction_bytes_per_file",
        12_000,
        1,
        MAX_INSTRUCTION_BYTES
    )) as usize;
    let terms = focused_context_terms(&query);
    let (candidates, scanned) =
        focused_context_candidates(&root, &query, &seed_paths, max_files, max_scan_files);
    let target_paths = candidates
        .iter()
        .map(|(_score, path, _reasons)| path.clone())
        .collect::<Vec<_>>();
    let instruction_targets = if target_paths.is_empty() {
        if seed_paths.is_empty() {
            vec![root.clone()]
        } else {
            seed_paths.clone()
        }
    } else {
        target_paths.clone()
    };
    let instruction_names = INSTRUCTION_FILE_NAMES
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let instruction_files = if include_instructions {
        collect_instruction_files(&root, &instruction_targets, &instruction_names)
    } else {
        Vec::new()
    };
    let (diagnostics, commands, related_tests, _validation_targets) =
        validation_plan_items(&root, &target_paths, true);
    let mut sections = vec![
        format!("# Focused context: {}", root.display()),
        format!("query: {query}"),
        format!(
            "terms: {}",
            if terms.is_empty() {
                "(none)".to_string()
            } else {
                terms.join(", ")
            }
        ),
        format!("scanned_files: {scanned}"),
        String::new(),
        "## Ranked files".to_string(),
    ];
    if candidates.is_empty() {
        sections.push("(none)".to_string());
    } else {
        sections.extend(candidates.iter().map(|(score, path, reasons)| {
            let reason = if reasons.is_empty() {
                "matched".to_string()
            } else {
                reasons.join("; ")
            };
            format!(
                "- score {score}: {} ({reason})",
                relative_display(path, &root)
            )
        }));
    }
    if include_instructions {
        sections.extend([String::new(), "## Applicable instructions".to_string()]);
        if instruction_files.is_empty() {
            sections.push("(none)".to_string());
        } else {
            for path in &instruction_files {
                let rel = relative_display(path, &root);
                match read_instruction_file(path, max_instruction_bytes) {
                    Ok((content, byte_count, truncated)) => {
                        sections.extend([
                            format!("### {rel}"),
                            format!("bytes: {byte_count}"),
                            content.trim_end().to_string(),
                        ]);
                        if truncated {
                            sections
                                .push(format!("...[truncated to {max_instruction_bytes} bytes]"));
                        }
                        sections.push(String::new());
                    }
                    Err(err) => sections.extend([
                        format!("### {rel}"),
                        format!("error: unable to read file: {err}"),
                        String::new(),
                    ]),
                }
            }
        }
    }
    sections.extend([String::new(), "## File context".to_string()]);
    for (score, path, reasons) in &candidates {
        let rel = relative_display(path, &root);
        let Ok(text) = fs::read_to_string(path) else {
            sections.extend([
                format!("### {rel}"),
                "error: unable to read file".to_string(),
            ]);
            continue;
        };
        sections.extend([
            format!("### {rel}"),
            format!("score: {score}"),
            "reasons:".to_string(),
            if reasons.is_empty() {
                "(none)".to_string()
            } else {
                reasons
                    .iter()
                    .map(|reason| format!("- {reason}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "imports:".to_string(),
        ]);
        let imports = extract_imports(&text, 12).unwrap_or_default();
        if imports.is_empty() {
            sections.push("(none)".to_string());
        } else {
            sections.extend(imports.into_iter().map(|item| {
                format!(
                    "- {}: {} {} | {}",
                    item.line, item.kind, item.spec, item.signature
                )
            }));
        }
        sections.extend([String::new(), "outline:".to_string()]);
        let outline = extract_outline(&text, max_symbols).unwrap_or_default();
        if outline.is_empty() {
            sections.push("(none)".to_string());
        } else {
            sections.extend(outline.into_iter().map(|item| format!("- {item}")));
        }
        sections.extend([String::new(), "snippets:".to_string()]);
        let snippets = focused_context_snippets(&text, &terms, max_snippets);
        if snippets.is_empty() {
            sections.push("(none)".to_string());
        } else {
            sections.push(snippets.join("\n\n"));
        }
        sections.push(String::new());
    }
    sections.extend([
        "## Related tests".to_string(),
        if related_tests.is_empty() {
            "(none)".to_string()
        } else {
            related_tests
                .iter()
                .map(|path| format!("- {}", path.display()))
                .collect::<Vec<_>>()
                .join("\n")
        },
        String::new(),
        "## Suggested validation".to_string(),
    ]);
    let suggested = diagnostics
        .iter()
        .chain(commands.iter())
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>();
    sections.push(if suggested.is_empty() {
        "(none inferred)".to_string()
    } else {
        suggested.join("\n")
    });
    ok(
        sections.join("\n"),
        format!("Focused context ({} files)", candidates.len()),
    )
}

fn impact_analysis(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
                if !resolved.starts_with(&root) {
                    return error(format!(
                        "path is outside impact_analysis root: {}",
                        resolved.display()
                    ));
                }
                if resolved.is_file() && !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let symbols = match args.get("symbols") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(symbol) = value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return error("symbols must contain non-empty strings".to_string());
                };
                let symbol = symbol.to_string();
                if !out.contains(&symbol) {
                    out.push(symbol);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("symbols must be a list".to_string()),
    };
    let glob = match args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        Some(value) => match glob_regex(value) {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
        None => match glob_regex("**/*") {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
    };
    let max_entries = tool_try!(validate_integer_arg(args, "max_entries", 80, 1, 500)) as usize;
    let target_input = if paths.is_empty() {
        git_changed_files(&root)
    } else {
        paths
    };
    let (diagnostics, commands, _related_tests, target_paths) =
        validation_plan_items(&root, &target_input, true);
    let mut related_lines = Vec::new();
    let mut related_seen = Vec::new();
    let mut reference_lines = Vec::new();
    for path in target_paths.iter().take(max_entries) {
        let (tests, same_name, references, _scanned) = scan_related_files(path, &root, max_entries);
        for candidate in tests.into_iter().chain(same_name.into_iter()) {
            if !related_seen.contains(&candidate) {
                related_seen.push(candidate.clone());
                related_lines.push(format!("- {}", relative_display(&candidate, &root)));
            }
        }
        reference_lines.extend(references.into_iter().map(|item| format!("- {item}")));
    }
    let mut symbol_sections = Vec::new();
    for symbol in &symbols {
        let (definitions, references) = tool_try!(scan_symbol_matches(
            &root,
            symbol,
            glob.as_ref(),
            true,
            max_entries
        ));
        let (graph_definitions, incoming, outgoing, scanned) =
            tool_try!(scan_call_graph(&root, symbol, glob.as_ref(), max_entries));
        symbol_sections.extend([
            format!("### {symbol}"),
            format!("scanned_files: {scanned}"),
            "definitions:".to_string(),
            if definitions.is_empty() && graph_definitions.is_empty() {
                "(none)".to_string()
            } else {
                definitions
                    .iter()
                    .chain(graph_definitions.iter())
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "references:".to_string(),
            if references.is_empty() {
                "(none)".to_string()
            } else {
                references
                    .iter()
                    .take(max_entries)
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "incoming calls:".to_string(),
            if incoming.is_empty() {
                "(none)".to_string()
            } else {
                incoming
                    .iter()
                    .take(max_entries)
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "outgoing calls:".to_string(),
            if outgoing.is_empty() {
                "(none)".to_string()
            } else {
                outgoing
                    .iter()
                    .take(max_entries)
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
        ]);
    }
    ok(
        [
            format!("# Impact analysis: {}", root.display()),
            String::new(),
            "## Target files".to_string(),
            if target_paths.is_empty() {
                "(none inferred)".to_string()
            } else {
                target_paths
                    .iter()
                    .map(|path| format!("- {}", relative_display(path, &root)))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Related files and tests".to_string(),
            if related_lines.is_empty() {
                "(none)".to_string()
            } else {
                related_lines
                    .into_iter()
                    .take(max_entries)
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## File references".to_string(),
            if reference_lines.is_empty() {
                "(none)".to_string()
            } else {
                reference_lines
                    .into_iter()
                    .take(max_entries)
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Symbols".to_string(),
            if symbol_sections.is_empty() {
                "(none requested)".to_string()
            } else {
                symbol_sections.join("\n").trim_end().to_string()
            },
            String::new(),
            "## Suggested validation".to_string(),
            if diagnostics.is_empty() && commands.is_empty() {
                "(none inferred)".to_string()
            } else {
                diagnostics
                    .iter()
                    .chain(commands.iter())
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        ]
        .join("\n"),
        format!(
            "Impact analysis ({} files, {} symbols)",
            target_paths.len(),
            symbols.len()
        ),
    )
}

fn validation_plan(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
                if resolved.is_file() && !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let include_project_commands = match args.get("include_project_commands") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_project_commands must be a boolean".to_string()),
    };
    let (diagnostics, commands, related_tests, target_paths) =
        validation_plan_items(&root, &paths, include_project_commands);
    ok(
        format!(
            "# Validation plan: {}\n\n## Changed/target files\n{}\n\n## Tool checks\n{}\n\n## Related tests\n{}\n\n## Commands\n{}",
            root.display(),
            if target_paths.is_empty() {
                "(none inferred)".to_string()
            } else {
                target_paths
                    .iter()
                    .map(|path| format!("- {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            if diagnostics.is_empty() {
                "(none)".to_string()
            } else {
                diagnostics
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            if related_tests.is_empty() {
                "(none)".to_string()
            } else {
                related_tests
                    .iter()
                    .map(|path| format!("- {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            if commands.is_empty() {
                "(none inferred)".to_string()
            } else {
                commands
                    .iter()
                    .map(|command| format!("- {command}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        ),
        format!("Validation plan ({} commands)", commands.len()),
    )
}

fn test_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_project_file(&root, path, &ctx.allowed_dirs));
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let max_tests = tool_try!(validate_integer_arg(args, "max_tests", 8, 1, 50)) as usize;
    let max_symbols = tool_try!(validate_integer_arg(
        args,
        "max_symbols_per_test",
        20,
        1,
        100
    )) as usize;
    let (frameworks, target_paths, related_tests, commands, scanned) =
        test_context_items(&root, &paths, max_tests);
    let samples = tool_try!(test_sample_sections(
        &root,
        &related_tests,
        max_tests,
        max_symbols
    ));
    let target_lines = if target_paths.is_empty() {
        "(none inferred)".to_string()
    } else {
        target_paths
            .iter()
            .map(|path| format!("- {}", relative_display(path, &root)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let test_lines = if related_tests.is_empty() {
        "(none)".to_string()
    } else {
        related_tests
            .iter()
            .take(max_tests)
            .map(|path| format!("- {}", relative_display(path, &root)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let gaps = if target_paths.is_empty() || !related_tests.is_empty() {
        "(none)".to_string()
    } else {
        "- No likely test file found for the target paths; locate or add coverage before behavior changes.".to_string()
    };
    ok(
        format!(
            "# Test context: {}\ntargets: {}\nscanned_files: {}\n\n## Framework signals\n{}\n\n## Target files\n{}\n\n## Likely tests to read or update\n{}\n\n## Test samples\n{}\n\n## Suggested validation\n{}\n\n## Coverage gaps\n{}",
            root.display(),
            target_paths.len(),
            scanned,
            if frameworks.is_empty() {
                "(none inferred)".to_string()
            } else {
                frameworks
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            target_lines,
            test_lines,
            if samples.is_empty() {
                "(none)".to_string()
            } else {
                samples.join("\n\n")
            },
            if commands.is_empty() {
                "(none inferred)".to_string()
            } else {
                commands
                    .iter()
                    .map(|command| format!("- {command}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            gaps
        ),
        format!(
            "Test context ({} tests, {} commands)",
            related_tests.len(),
            commands.len()
        ),
    )
}

fn format_plan(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
                if resolved.is_file() && !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let (commands, notes, targets) = format_plan_items(&root, &paths);
    ok(
        format!(
            "# Format plan: {}\n\n## Target files\n{}\n\n## Commands\n{}\n\n## Notes\n{}",
            root.display(),
            if targets.is_empty() {
                "(none inferred)".to_string()
            } else {
                targets
                    .iter()
                    .map(|path| format!("- {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            if commands.is_empty() {
                "(none inferred)".to_string()
            } else {
                commands
                    .iter()
                    .map(|command| format!("- {command}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            if notes.is_empty() {
                "(none)".to_string()
            } else {
                notes
                    .iter()
                    .map(|note| format!("- {note}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        ),
        format!("Format plan ({} commands)", commands.len()),
    )
}

fn format_files(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
                if resolved.is_file() && !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let dry_run = match args.get("dry_run") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("dry_run must be a boolean".to_string()),
    };
    let (commands, notes, targets) = format_plan_items(&root, &paths);
    if commands.is_empty() {
        return ok(
            [
                format!("# Format files: {}", root.display()),
                format!("dry_run: {dry_run}"),
                String::new(),
                "## Commands".to_string(),
                "(none inferred)".to_string(),
                String::new(),
                "## Notes".to_string(),
                if notes.is_empty() {
                    "(none)".to_string()
                } else {
                    notes
                        .iter()
                        .map(|note| format!("- {note}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            ]
            .join("\n"),
            format!("Format files ({} files, 0 commands)", targets.len()),
        );
    }
    if dry_run {
        return ok(
            [
                format!("# Format files: {}", root.display()),
                "dry_run: true".to_string(),
                String::new(),
                "## Commands".to_string(),
                commands
                    .iter()
                    .map(|command| format!("- {command}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                String::new(),
                "## Notes".to_string(),
                if notes.is_empty() {
                    "(none)".to_string()
                } else {
                    notes
                        .iter()
                        .map(|note| format!("- {note}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            ]
            .join("\n"),
            format!(
                "Format files ({} files, {} commands)",
                targets.len(),
                commands.len()
            ),
        );
    }
    let before = tool_try!(capture_workspace_text_artifact_states(&root));
    let mut results = Vec::new();
    for command in &commands {
        let output = match run_shell_command(&root, command, &ctx.cancellation,ctx.policy.process_sandbox()) {
            Ok(output) => output,
            Err(outcome) => return outcome,
        };
        if !output.status.success() {
            let detail = command_output_text(&output);
            return error_with_display(
                [
                    format!("# Format files: {}", root.display()),
                    "dry_run: false".to_string(),
                    String::new(),
                    "## Commands".to_string(),
                    commands
                        .iter()
                        .map(|item| format!("- {item}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    String::new(),
                    "## Results".to_string(),
                    if results.is_empty() {
                        "(none)".to_string()
                    } else {
                        results.join("\n")
                    },
                    String::new(),
                    "## Failure".to_string(),
                    command.to_string(),
                    detail,
                ]
                .join("\n"),
                "Format files failed".to_string(),
            );
        }
        let mut snippet = command_output_text(&output);
        if snippet.is_empty() {
            snippet = "ok".to_string();
        }
        if snippet.len() > 2_000 {
            snippet.truncate(2_000);
            snippet.push_str("\n...[output truncated]");
        }
        results.push(format!("- {command}: ok\n{snippet}"));
    }
    let after = tool_try!(capture_workspace_text_artifact_states(&root));
    ok(
        [
            format!("# Format files: {}", root.display()),
            "dry_run: false".to_string(),
            String::new(),
            "## Commands".to_string(),
            commands
                .iter()
                .map(|command| format!("- {command}"))
                .collect::<Vec<_>>()
                .join("\n"),
            String::new(),
            "## Results".to_string(),
            if results.is_empty() {
                "(none)".to_string()
            } else {
                results.join("\n")
            },
            String::new(),
            "## Notes".to_string(),
            if notes.is_empty() {
                "(none)".to_string()
            } else {
                notes
                    .iter()
                    .map(|note| format!("- {note}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        ]
        .join("\n"),
        format!(
            "Formatted files ({} files, {} commands)",
            targets.len(),
            commands.len()
        ),
    )
    .with_file_artifact_changes(&before, &after)
}

fn related_files(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
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
    let max_results = tool_try!(validate_integer_arg(
        args,
        "max_results",
        200,
        1,
        MAX_RELATED_RESULTS
    )) as usize;
    let (tests, same_name, references, scanned) = scan_related_files(&target, &root, max_results);
    ok(
        format!(
            "# Related files: {}\nroot: {}\nscanned_files: {}\n\n## Likely tests\n{}\n\n## Same-name companions\n{}\n\n## Referencing files\n{}",
            target.display(),
            root.display(),
            scanned,
            if tests.is_empty() {
                "(none)".to_string()
            } else {
                tests
                    .iter()
                    .map(|path| format!("- {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            if same_name.is_empty() {
                "(none)".to_string()
            } else {
                same_name
                    .iter()
                    .map(|path| format!("- {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            if references.is_empty() {
                "(none)".to_string()
            } else {
                references.join("\n")
            }
        ),
        format!(
            "Related files {} ({} tests, {} references)",
            target.display(),
            tests.len(),
            references.len()
        ),
    )
}

fn read_file_text(
    target: &Path,
    args: &Map<String, Value>,
    show_line_numbers: bool,
    default_max_bytes: i64,
    budget: FileReadPresentationBudget,
) -> Result<ObservedFileText, ToolOutcome> {
    let explicit_range = args.contains_key("start_line") || args.contains_key("line_count");
    let explicit_max_bytes = args.contains_key("max_bytes");
    let cap = validate_integer_arg(
        args,
        "max_bytes",
        default_max_bytes,
        1024,
        MAX_READ_BYTES as i64,
    )? as u64;
    let start_line = validate_integer_arg(args, "start_line", 1, 1, 1_000_000_000)? as usize;
    let requested_line_count = if args.contains_key("line_count") {
        Some(validate_integer_arg(args, "line_count", MAX_LINE_COUNT, 1, MAX_LINE_COUNT)? as usize)
    } else {
        None
    };
    let snapshot = read_file_snapshot(target, FileReadSelection {
        first_line: start_line,
        line_count: if explicit_range { Some(requested_line_count.unwrap_or(MAX_LINE_COUNT as usize)) } else { None },
        max_bytes: cap as usize,
    })?;
    if snapshot.text.is_none() {
        if explicit_range {
            return Err(error(
                "Read failed: start_line/line_count ranges apply to UTF-8 text files; read the binary document without a line range to extract bounded PDF or workbook text"
                    .to_string(),
            ));
        }
        let (text, _) = extract_binary_document_text(target, cap as usize, show_line_numbers)?;
        return Ok(ObservedFileText { text, raw_content: None, observation: None, next_start_line: None, presentation_exhausted: false });
    }
    let auto_large_preview = !explicit_range && !explicit_max_bytes
        && snapshot.file_bytes > LARGE_FILE_PREVIEW_THRESHOLD_BYTES;
    render_file_read_snapshot(target, snapshot, start_line, show_line_numbers, auto_large_preview, budget)
}

fn outline_patterns() -> &'static [(&'static str, Regex)] {
    static OUTLINE_PATTERNS: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    OUTLINE_PATTERNS.get_or_init(|| {
        [
        (
            "class",
            r"^\s*(?:export\s+)?(?:public\s+|private\s+|protected\s+|open\s+|final\s+|sealed\s+|data\s+)*class\s+([A-Za-z_][A-Za-z0-9_]*)",
        ),
        (
            "struct",
            r"^\s*(?:export\s+)?(?:public\s+|private\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)",
        ),
        (
            "enum",
            r"^\s*(?:export\s+)?(?:public\s+|private\s+)?enum(?:\s+class)?\s+([A-Za-z_][A-Za-z0-9_]*)",
        ),
        (
            "interface",
            r"^\s*(?:export\s+)?(?:public\s+)?(?:interface|protocol|trait)\s+([A-Za-z_][A-Za-z0-9_]*)",
        ),
        ("type", r"^\s*(?:export\s+)?type\s+([A-Za-z_][A-Za-z0-9_]*)\b"),
        (
            "object",
            r"^\s*(?:public\s+|private\s+)?object\s+([A-Za-z_][A-Za-z0-9_]*)",
        ),
        (
            "function",
            r"^\s*(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][A-Za-z0-9_$]*)",
        ),
        (
            "function",
            r"^\s*(?:(?:export|pub(?:\([^)]*\))?|public|private|protected|internal|open|final|static|mutating|override|suspend|async|unsafe|const)\s+)*(?:def|func|fun|fn)\s+([A-Za-z_][A-Za-z0-9_]*)",
        ),
        (
            "function",
            r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][A-Za-z0-9_$]*)\s*=>",
        ),
        (
            "function",
            r"^\s*func\s+(?:\([^)]*\)\s*)?([A-Za-z_][A-Za-z0-9_]*)",
        ),
        (
            "method",
            r"^\s*(?:(?:public|private|protected|static|async|override|get|set)\s+)*([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]+>)?\s*\([^)]*\)\s*(?::\s*[^={]+)?\s*\{",
        ),
        (
            "impl",
            r"^\s*impl(?:<[^>]+>)?\s+([A-Za-z_][A-Za-z0-9_:<>]*)",
        ),
    ]
    .into_iter()
    .map(|(kind, pattern)| {
        Regex::new(pattern)
            .map(|regex| (kind, regex))
            .unwrap_or_else(|err| panic!("outline regex failed: {err}"))
    })
    .collect()
    })
}

fn skip_outline_symbol(kind: &str, name: &str, line: &str) -> bool {
    kind == "method"
        && (line.contains("=>")
            || matches!(
                name,
                "if" | "for"
                    | "while"
                    | "switch"
                    | "catch"
                    | "return"
                    | "throw"
                    | "new"
                    | "function"
            ))
}

fn is_symbol_definition(line: &str, symbol: &str) -> Result<bool, ToolOutcome> {
    for (kind, regex) in outline_patterns() {
        let Some(captures) = regex.captures(line) else {
            continue;
        };
        let Some(name) = captures.get(1) else {
            continue;
        };
        if !skip_outline_symbol(kind, name.as_str(), line) && name.as_str() == symbol {
            return Ok(true);
        }
    }
    Ok(false)
}

fn symbol_reference_regex(symbol: &str) -> Result<Regex, ToolOutcome> {
    Regex::new(&format!(
        r"(?P<prefix>^|[^A-Za-z0-9_$]){}(?P<suffix>$|[^A-Za-z0-9_$])",
        regex::escape(symbol)
    ))
    .map_err(|err| error(format!("symbol regex failed: {err}")))
}

fn symbol_at_position(
    text: &str,
    line: usize,
    character: usize,
) -> Result<Option<(String, String)>, ToolOutcome> {
    let Some(source_line) = text.lines().nth(line.saturating_sub(1)) else {
        return Ok(None);
    };
    let byte_index = source_line
        .char_indices()
        .nth(character.saturating_sub(1))
        .map(|(index, _)| index)
        .unwrap_or(source_line.len());
    let identifier_regex = Regex::new(r"[A-Za-z_$][A-Za-z0-9_$]*")
        .map_err(|err| error(format!("identifier regex failed: {err}")))?;
    for match_ in identifier_regex.find_iter(source_line) {
        if match_.start() <= byte_index && byte_index <= match_.end() {
            return Ok(Some((match_.as_str().to_string(), source_line.to_string())));
        }
    }
    Ok(None)
}

fn scan_symbol_matches(
    root: &Path,
    symbol: &str,
    glob: Option<&Regex>,
    include_references: bool,
    limit: usize,
) -> Result<(Vec<String>, Vec<String>), ToolOutcome> {
    let reference_regex = symbol_reference_regex(symbol)?;
    let mut definitions = Vec::new();
    let mut references = Vec::new();
    walk_files(root, usize::MAX, &mut |path| {
        if let Some(glob) = glob {
            if let Ok(rel) = path.strip_prefix(root) {
                if !glob.is_match(&path_slash(rel)) {
                    return definitions.len() + references.len() < limit;
                }
            }
        }
        let Ok(text) = fs::read_to_string(path) else {
            return definitions.len() + references.len() < limit;
        };
        for (line_no, line) in text.lines().enumerate() {
            if is_symbol_definition(line, symbol).unwrap_or(false) {
                definitions.push(format!(
                    "{}:{}:def: {}",
                    path.display(),
                    line_no + 1,
                    outline_signature(line)
                ));
            } else if include_references && reference_regex.is_match(line) {
                references.push(format!(
                    "{}:{}:ref: {}",
                    path.display(),
                    line_no + 1,
                    outline_signature(line)
                ));
            }
            if definitions.len() + references.len() >= limit {
                return false;
            }
        }
        definitions.len() + references.len() < limit
    });
    Ok((definitions, references))
}

fn line_declares_type_implementation(
    line: &str,
    kind: &str,
    name: &str,
    symbol: &str,
    reference_regex: &Regex,
) -> bool {
    if kind == "impl" {
        return line.contains(" for ") && reference_regex.is_match(line);
    }
    if !matches!(kind, "class" | "struct" | "enum" | "interface" | "object") || name == symbol {
        return false;
    }
    [" implements ", " extends ", ":", "("]
        .iter()
        .any(|marker| line.contains(marker))
        && reference_regex.is_match(line)
}

fn scan_implementation_matches(
    root: &Path,
    symbol: &str,
    glob: Option<&Regex>,
    limit: usize,
) -> Result<(Vec<String>, Vec<String>, usize), ToolOutcome> {
    let reference_regex = symbol_reference_regex(symbol)?;
    let mut type_implementations = Vec::new();
    let mut same_name_definitions = Vec::new();
    let mut scanned = 0usize;
    walk_files(root, usize::MAX, &mut |path| {
        scanned += 1;
        if !is_text_source(path) {
            return type_implementations.len() + same_name_definitions.len() < limit;
        }
        if let Some(glob) = glob {
            if let Ok(rel) = path.strip_prefix(root) {
                if !glob.is_match(&path_slash(rel)) {
                    return type_implementations.len() + same_name_definitions.len() < limit;
                }
            }
        }
        let Ok(text) = fs::read_to_string(path) else {
            return type_implementations.len() + same_name_definitions.len() < limit;
        };
        for (line_no, line) in text.lines().enumerate() {
            let Ok(Some((kind, name))) = outline_symbol(line) else {
                continue;
            };
            if line_declares_type_implementation(line, kind, &name, symbol, &reference_regex) {
                type_implementations.push(format!(
                    "{}:{}: {} {} | {}",
                    path.display(),
                    line_no + 1,
                    kind,
                    name,
                    outline_signature(line)
                ));
            } else if name == symbol && matches!(kind, "function" | "method") {
                same_name_definitions.push(format!(
                    "{}:{}: {} {} | {}",
                    path.display(),
                    line_no + 1,
                    kind,
                    name,
                    outline_signature(line)
                ));
            }
            if type_implementations.len() + same_name_definitions.len() >= limit {
                return false;
            }
        }
        type_implementations.len() + same_name_definitions.len() < limit
    });
    Ok((type_implementations, same_name_definitions, scanned))
}

fn symbol_doc_block(lines: &[String], definition_index: usize, max_lines: usize) -> Vec<String> {
    let mut docs = Vec::new();
    let mut index = definition_index;
    while index > 0 && docs.len() < max_lines {
        index -= 1;
        let stripped = lines[index].trim();
        if stripped.is_empty() {
            break;
        }
        if stripped.starts_with('#')
            || stripped.starts_with("//")
            || stripped.starts_with("///")
            || stripped.starts_with("/*")
            || stripped.starts_with('*')
            || stripped.starts_with("<!--")
        {
            docs.push(stripped.to_string());
            continue;
        }
        break;
    }
    docs.reverse();
    if !docs.is_empty() {
        return docs;
    }

    for index in (definition_index + 1)..usize::min(lines.len(), definition_index + 8) {
        let stripped = lines[index].trim();
        if stripped.is_empty() {
            continue;
        }
        if stripped.starts_with("\"\"\"") || stripped.starts_with("'''") {
            let quote = &stripped[..3];
            docs.push(stripped.to_string());
            if stripped.matches(quote).count() >= 2 && stripped.len() > 3 {
                return docs.into_iter().take(max_lines).collect();
            }
            for next_index in (index + 1)..usize::min(lines.len(), index + max_lines) {
                docs.push(lines[next_index].trim().to_string());
                if lines[next_index].contains(quote) {
                    break;
                }
            }
            return docs.into_iter().take(max_lines).collect();
        }
        break;
    }
    docs
}

fn symbol_definition_context(
    root: &Path,
    symbol: &str,
    glob: Option<&Regex>,
    max_entries: usize,
) -> Result<Vec<String>, ToolOutcome> {
    let mut entries = Vec::new();
    walk_files(root, usize::MAX, &mut |path| {
        if !is_text_source(path) {
            return entries.len() < max_entries;
        }
        if let Some(glob) = glob {
            if let Ok(rel) = path.strip_prefix(root) {
                if !glob.is_match(&path_slash(rel)) {
                    return entries.len() < max_entries;
                }
            }
        }
        let Ok(text) = fs::read_to_string(path) else {
            return entries.len() < max_entries;
        };
        let lines = text.lines().map(ToString::to_string).collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if !is_symbol_definition(line, symbol).unwrap_or(false) {
                continue;
            }
            let docs = symbol_doc_block(&lines, index, 8);
            let mut entry = format!(
                "{}:{}: {}",
                path.display(),
                index + 1,
                outline_signature(line)
            );
            if !docs.is_empty() {
                entry.push('\n');
                entry.push_str(
                    &docs
                        .into_iter()
                        .map(|doc| format!("  doc: {}", outline_signature(&doc)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            entries.push(entry);
            if entries.len() >= max_entries {
                return false;
            }
        }
        entries.len() < max_entries
    });
    Ok(entries)
}

fn is_known_symbol_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "function"
            | "method"
            | "struct"
            | "enum"
            | "interface"
            | "type"
            | "object"
            | "impl"
    )
}

fn workspace_symbol_matches(
    root: &Path,
    query: &str,
    glob: &Regex,
    kinds: &[String],
    limit: usize,
) -> Result<(Vec<String>, usize), ToolOutcome> {
    let patterns = outline_patterns();
    let needle = query.to_ascii_lowercase();
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    walk_files(root, usize::MAX, &mut |path| {
        scanned += 1;
        if !is_text_source(path) {
            return matches.len() < limit;
        }
        let Ok(rel) = path.strip_prefix(root) else {
            return matches.len() < limit;
        };
        let rel_display = path_slash(rel);
        if !glob.is_match(&rel_display) {
            return matches.len() < limit;
        }
        let Ok(text) = fs::read_to_string(path) else {
            return matches.len() < limit;
        };
        for (line_no, line) in text.lines().enumerate() {
            for (kind, regex) in patterns {
                let Some(captures) = regex.captures(line) else {
                    continue;
                };
                let Some(name) = captures.get(1) else {
                    continue;
                };
                let name = name.as_str();
                if skip_outline_symbol(kind, name, line) {
                    continue;
                }
                if !kinds.is_empty() && !kinds.iter().any(|candidate| candidate == kind) {
                    break;
                }
                if !name.to_ascii_lowercase().contains(&needle) {
                    break;
                }
                matches.push(format!(
                    "- {}:{}: {} {} | {}",
                    rel_display,
                    line_no + 1,
                    kind,
                    name,
                    outline_signature(line)
                ));
                if matches.len() >= limit {
                    return false;
                }
                break;
            }
        }
        matches.len() < limit
    });
    Ok((matches, scanned))
}

fn outline_symbol(line: &str) -> Result<Option<(&'static str, String)>, ToolOutcome> {
    for (kind, regex) in outline_patterns() {
        let Some(captures) = regex.captures(line) else {
            continue;
        };
        let Some(name) = captures.get(1) else {
            continue;
        };
        if skip_outline_symbol(kind, name.as_str(), line) {
            continue;
        }
        return Ok(Some((kind, name.as_str().to_string())));
    }
    Ok(None)
}

fn code_search_name_matches(name: &str, query: &str, exact: bool, case_sensitive: bool) -> bool {
    if case_sensitive {
        if exact {
            name == query
        } else {
            name.contains(query)
        }
    } else {
        let name = name.to_ascii_lowercase();
        let query = query.to_ascii_lowercase();
        if exact {
            name == query
        } else {
            name.contains(&query)
        }
    }
}

fn code_search_language_matches(path: &Path, language: &str) -> bool {
    if language.is_empty() {
        return true;
    }
    let expected = language.to_ascii_lowercase();
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let actual = path
        .extension()
        .and_then(|value| value.to_str())
        .and_then(language_for_ext)
        .unwrap_or("")
        .to_ascii_lowercase();
    expected == ext || expected == actual || actual.split_whitespace().any(|part| part == expected)
}

fn code_search_result(
    rel: &str,
    line_no: usize,
    match_kind: &str,
    detail: &str,
    line: &str,
    lines: &[String],
    context_lines: usize,
) -> String {
    let nearest = nearest_enclosing_symbol(lines, line_no);
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(" {detail}")
    };
    let header = format!(
        "- {rel}:{line_no}: {match_kind}{detail} | in: {nearest} | {}",
        outline_signature(line)
    );
    if context_lines == 0 {
        return header;
    }
    let start_line = line_no.saturating_sub(context_lines).max(1);
    let end_line = (line_no + context_lines).min(lines.len());
    let snippet = lines[start_line - 1..end_line].join("\n");
    let numbered = format_numbered_lines(&snippet, start_line, true);
    let indented = numbered
        .lines()
        .map(|item| format!("  {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{header}\n  context:\n{indented}")
}

fn code_search_enclosing_matches(
    lines: &[String],
    line_no: usize,
    enclosing_symbol: &str,
    case_sensitive: bool,
) -> bool {
    if enclosing_symbol.is_empty() {
        return true;
    }
    let nearest = nearest_enclosing_symbol(lines, line_no);
    if case_sensitive {
        nearest.contains(enclosing_symbol)
    } else {
        nearest
            .to_ascii_lowercase()
            .contains(&enclosing_symbol.to_ascii_lowercase())
    }
}

fn code_search_matches(
    root: &Path,
    query: &str,
    mode: &str,
    glob: &Regex,
    language: &str,
    exact: bool,
    case_sensitive: bool,
    context_lines: usize,
    enclosing_symbol: &str,
    limit: usize,
    cancellation: &crate::runtime_control::RuntimeExecutionControl,
) -> Result<(Vec<String>, usize), ToolOutcome> {
    let started = std::time::Instant::now();
    let stop = || cancellation.is_requested() || started.elapsed() >= std::time::Duration::from_secs(30);
    let call_regex = Regex::new(r"([A-Za-z_$][A-Za-z0-9_$]*)\s*\(")
        .map_err(|err| error(format!("call regex failed: {err}")))?;
    let reference_regex = if Regex::new(r"^[A-Za-z_$][A-Za-z0-9_$]*$")
        .expect("valid identifier regex")
        .is_match(query)
    {
        Some(
            RegexBuilder::new(&format!(
                r"(^|[^A-Za-z0-9_$]){}($|[^A-Za-z0-9_$])",
                regex::escape(query)
            ))
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|err| error(format!("reference regex failed: {err}")))?,
        )
    } else {
        None
    };
    let text_query = if case_sensitive {
        query.to_string()
    } else {
        query.to_ascii_lowercase()
    };
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    let mut oversized = false;
    walk_files_controlled(root, 50_000, &mut |path| {
        if stop() { return false; }
        scanned += 1;
        if !is_code_source(path) || !code_search_language_matches(path, language) {
            return matches.len() < limit;
        }
        let Ok(rel) = path.strip_prefix(root) else {
            return matches.len() < limit;
        };
        let rel_display = path_slash(rel);
        if !glob.is_match(&rel_display) {
            return matches.len() < limit;
        }
        let bytes = match fs::File::open(path).and_then(|file| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut std::io::Read::take(file, 2 * 1024 * 1024 + 1), &mut bytes)?;
            Ok(bytes)
        }) { Ok(bytes) => bytes, Err(_) => return matches.len() < limit };
        if bytes.len() > 2 * 1024 * 1024 { oversized = true; return false; }
        let Ok(text) = String::from_utf8(bytes) else { return matches.len() < limit; };
        let lines = text.lines().map(str::to_string).collect::<Vec<_>>();
        for (line_no, line) in lines.iter().enumerate() {
            if stop() { return false; }
            let line_no = line_no + 1;
            match mode {
                "definition" => {
                    if let Ok(Some((kind, name))) = outline_symbol(line) {
                        if code_search_name_matches(&name, query, exact, case_sensitive) {
                            if !code_search_enclosing_matches(
                                &lines,
                                line_no,
                                enclosing_symbol,
                                case_sensitive,
                            ) {
                                continue;
                            }
                            matches.push(code_search_result(
                                &rel_display,
                                line_no,
                                "definition",
                                &format!("{kind} {name}"),
                                line,
                                &lines,
                                context_lines,
                            ));
                        }
                    }
                }
                "call" => {
                    if outline_symbol(line).ok().flatten().is_some() {
                        continue;
                    }
                    for captures in call_regex.captures_iter(line) {
                        let Some(callee) = captures.get(1).map(|value| value.as_str()) else {
                            continue;
                        };
                        if call_exclude(callee) {
                            continue;
                        }
                        if code_search_name_matches(callee, query, exact, case_sensitive) {
                            if !code_search_enclosing_matches(
                                &lines,
                                line_no,
                                enclosing_symbol,
                                case_sensitive,
                            ) {
                                continue;
                            }
                            matches.push(code_search_result(
                                &rel_display,
                                line_no,
                                "call",
                                callee,
                                line,
                                &lines,
                                context_lines,
                            ));
                            break;
                        }
                    }
                }
                "reference" => {
                    let haystack = if case_sensitive {
                        line.to_string()
                    } else {
                        line.to_ascii_lowercase()
                    };
                    if reference_regex
                        .as_ref()
                        .is_some_and(|regex| regex.is_match(line))
                        || (!exact && haystack.contains(&text_query))
                    {
                        if !code_search_enclosing_matches(
                            &lines,
                            line_no,
                            enclosing_symbol,
                            case_sensitive,
                        ) {
                            continue;
                        }
                        matches.push(code_search_result(
                            &rel_display,
                            line_no,
                            "reference",
                            query,
                            line,
                            &lines,
                            context_lines,
                        ));
                    }
                }
                _ => {
                    let haystack = if case_sensitive {
                        line.to_string()
                    } else {
                        line.to_ascii_lowercase()
                    };
                    if haystack.contains(&text_query) {
                        if !code_search_enclosing_matches(
                            &lines,
                            line_no,
                            enclosing_symbol,
                            case_sensitive,
                        ) {
                            continue;
                        }
                        matches.push(code_search_result(
                            &rel_display,
                            line_no,
                            "text",
                            "",
                            line,
                            &lines,
                            context_lines,
                        ));
                    }
                }
            }
            if matches.len() >= limit {
                return false;
            }
        }
        matches.len() < limit
    }, &stop);
    if oversized { return Err(error("code_search file exceeds 2 MiB; narrow the glob or use bounded read_file".into())); }
    if cancellation.is_requested() { return Err(error("code_search cancelled".into())); }
    if started.elapsed() >= std::time::Duration::from_secs(30) { return Err(error("code_search timed out after 30 seconds; narrow the root or glob".into())); }
    if scanned >= 50_000 { return Err(error("code_search file budget exceeded; narrow the root or glob".into())); }
    Ok((matches, scanned))
}

fn nearest_enclosing_symbol(lines: &[String], line_no: usize) -> String {
    let mut nearest = "module".to_string();
    for (index, line) in lines.iter().take(line_no.saturating_sub(1)).enumerate() {
        if let Ok(Some((kind, name))) = outline_symbol(line) {
            nearest = format!("{} {} at line {}", kind, name, index + 1);
        }
    }
    nearest
}

fn definition_body_range(lines: &[String], start_line: usize) -> (usize, usize) {
    for index in start_line..lines.len() {
        if outline_symbol(&lines[index]).ok().flatten().is_some() {
            return (start_line, index);
        }
    }
    (start_line, lines.len())
}

fn symbol_body_range(lines: &[String], start_line: usize, context_lines: usize) -> (usize, usize) {
    let start_index = start_line.saturating_sub(1);
    if start_index >= lines.len() {
        return (start_index, start_index);
    }
    let base_indent = lines[start_index]
        .chars()
        .take_while(|value| *value == ' ' || *value == '\t')
        .count();
    let mut end_index = lines.len();
    for index in (start_index + 1)..lines.len() {
        let line = &lines[index];
        if line.trim().is_empty() {
            continue;
        }
        if outline_symbol(line).ok().flatten().is_some() {
            let indent = line
                .chars()
                .take_while(|value| *value == ' ' || *value == '\t')
                .count();
            if indent <= base_indent {
                end_index = index;
                break;
            }
        }
    }
    (
        start_index.saturating_sub(context_lines),
        end_index.saturating_add(context_lines).min(lines.len()),
    )
}

fn find_symbol_body_range(
    lines: &[String],
    symbol: &str,
    occurrence: usize,
) -> Result<Option<(&'static str, usize, usize, usize, String)>, ToolOutcome> {
    let mut seen = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let Some((kind, name)) = outline_symbol(line)? else {
            continue;
        };
        if name != symbol {
            continue;
        }
        seen += 1;
        if seen != occurrence {
            continue;
        }
        let line_no = index + 1;
        let (start_index, mut end_index) = symbol_body_range(lines, line_no, 0);
        while end_index > start_index && lines[end_index - 1].trim().is_empty() {
            end_index -= 1;
        }
        return Ok(Some((
            kind,
            line_no,
            start_index,
            end_index,
            outline_signature(line),
        )));
    }
    Ok(None)
}

fn read_symbol_bodies(
    root: &Path,
    symbol: &str,
    glob: Option<&Regex>,
    max_matches: usize,
    context_lines: usize,
    show_line_numbers: bool,
    target: Option<&PathBuf>,
) -> Result<(Vec<String>, usize), ToolOutcome> {
    let mut paths = Vec::new();
    if let Some(target) = target {
        paths.push(target.clone());
    } else {
        walk_files(root, usize::MAX, &mut |path| {
            if let Some(glob) = glob {
                if let Ok(rel) = path.strip_prefix(root) {
                    if !glob.is_match(&path_slash(rel)) {
                        return true;
                    }
                }
            }
            if is_text_source(path) {
                paths.push(path.to_path_buf());
            }
            true
        });
    }
    let mut sections = Vec::new();
    let mut scanned = 0usize;
    for path in paths {
        if sections.len() >= max_matches {
            break;
        }
        scanned += 1;
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let lines = text.lines().map(str::to_string).collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            let Some((kind, name)) = outline_symbol(line)? else {
                continue;
            };
            if name != symbol {
                continue;
            }
            let (start, end) = symbol_body_range(&lines, index + 1, context_lines);
            let body = lines[start..end].join("\n");
            sections.push(format!(
                "## {}:{}: {} {}\nlines: {}-{} of {}\n\n{}",
                path.display(),
                index + 1,
                kind,
                name,
                start + 1,
                end,
                lines.len(),
                format_numbered_lines(&body, start + 1, show_line_numbers)
            ));
            if sections.len() >= max_matches {
                break;
            }
        }
    }
    Ok((sections, scanned))
}

fn call_exclude(name: &str) -> bool {
    matches!(
        name,
        "if" | "for"
            | "while"
            | "switch"
            | "catch"
            | "return"
            | "throw"
            | "new"
            | "function"
            | "def"
            | "func"
            | "fun"
            | "fn"
            | "class"
            | "struct"
            | "enum"
            | "interface"
            | "trait"
            | "protocol"
            | "sizeof"
            | "typeof"
    )
}

type CallGraphScan = (Vec<String>, Vec<String>, Vec<String>, usize);

fn scan_call_graph(
    root: &Path,
    symbol: &str,
    glob: Option<&Regex>,
    max_entries: usize,
) -> Result<CallGraphScan, ToolOutcome> {
    let call_to_symbol = Regex::new(&format!(
        r"(^|[^A-Za-z0-9_$]){}\s*\(",
        regex::escape(symbol)
    ))
    .map_err(|err| error(format!("call graph regex failed: {err}")))?;
    let call_regex = Regex::new(r"([A-Za-z_$][A-Za-z0-9_$]*)\s*\(")
        .map_err(|err| error(format!("call regex failed: {err}")))?;
    let mut definition_locations: Vec<(PathBuf, usize, String)> = Vec::new();
    let mut incoming = Vec::new();
    let mut scanned = 0usize;
    walk_files(root, usize::MAX, &mut |path| {
        if let Some(glob) = glob {
            if let Ok(rel) = path.strip_prefix(root) {
                if !glob.is_match(&path_slash(rel)) {
                    return incoming.len() < max_entries || definition_locations.is_empty();
                }
            }
        }
        let Ok(text) = fs::read_to_string(path) else {
            return incoming.len() < max_entries || definition_locations.is_empty();
        };
        scanned += 1;
        let lines = text.lines().map(str::to_string).collect::<Vec<_>>();
        for (line_index, line) in lines.iter().enumerate() {
            if is_symbol_definition(line, symbol).unwrap_or(false) {
                definition_locations.push((
                    path.to_path_buf(),
                    line_index + 1,
                    outline_signature(line),
                ));
            } else if call_to_symbol.is_match(line) && incoming.len() < max_entries {
                incoming.push(format!(
                    "{}:{}:in: {} -> {} | {}",
                    path.display(),
                    line_index + 1,
                    nearest_enclosing_symbol(&lines, line_index + 1),
                    symbol,
                    outline_signature(line)
                ));
            }
            if incoming.len() >= max_entries && !definition_locations.is_empty() {
                return false;
            }
        }
        incoming.len() < max_entries || definition_locations.is_empty()
    });

    let mut outgoing = Vec::new();
    let mut seen_outgoing: Vec<(PathBuf, usize, String)> = Vec::new();
    for (path, line_no, _) in &definition_locations {
        if outgoing.len() >= max_entries {
            break;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let lines = text.lines().map(str::to_string).collect::<Vec<_>>();
        let (start, end) = definition_body_range(&lines, *line_no);
        for index in start..end {
            let Some(line) = lines.get(index) else {
                continue;
            };
            for captures in call_regex.captures_iter(line) {
                let Some(callee) = captures.get(1).map(|value| value.as_str()) else {
                    continue;
                };
                if call_exclude(callee) {
                    continue;
                }
                let key = (path.clone(), index + 1, callee.to_string());
                if seen_outgoing.contains(&key) {
                    continue;
                }
                seen_outgoing.push(key);
                outgoing.push(format!(
                    "{}:{}:out: {} -> {} | {}",
                    path.display(),
                    index + 1,
                    symbol,
                    callee,
                    outline_signature(line)
                ));
                if outgoing.len() >= max_entries {
                    break;
                }
            }
            if outgoing.len() >= max_entries {
                break;
            }
        }
    }
    let definitions = definition_locations
        .iter()
        .take(max_entries)
        .map(|(path, line, signature)| format!("{}:{}:def: {}", path.display(), line, signature))
        .collect();
    Ok((definitions, incoming, outgoing, scanned))
}

fn diagnostic_files(
    root: &Path,
    paths: Option<Vec<PathBuf>>,
    glob: Option<&Regex>,
) -> (Vec<PathBuf>, &'static str) {
    if let Some(paths) = paths {
        return (paths, "explicit_paths");
    }
    let changed_files: Vec<PathBuf> = git_changed_files(root)
        .into_iter()
        .filter(|path| is_text_source(path))
        .collect();
    if !changed_files.is_empty() {
        return (changed_files, "git_changed_files");
    }
    let mut files = Vec::new();
    walk_files(root, usize::MAX, &mut |path| {
        if let Some(glob) = glob {
            if let Ok(rel) = path.strip_prefix(root) {
                if !glob.is_match(&path_slash(rel)) {
                    return true;
                }
            }
        }
        if is_text_source(path) {
            files.push(path.to_path_buf());
        }
        true
    });
    (files, "glob")
}

fn scan_code_diagnostics(files: &[PathBuf], max_entries: usize) -> (Vec<String>, usize) {
    let mut issues = Vec::new();
    let mut scanned = 0usize;
    for path in files {
        if issues.len() >= max_entries {
            break;
        }
        scanned += 1;
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_READ_BYTES as usize)]);
        for (line_no, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("<<<<<<< ")
                || trimmed.starts_with("=======")
                || trimmed.starts_with(">>>>>>> ")
            {
                issues.push(format!(
                    "{}:{}:error: unresolved merge conflict marker | {}",
                    path.display(),
                    line_no + 1,
                    outline_signature(line)
                ));
            }
            let ext = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if matches!(ext, "js" | "jsx" | "ts" | "tsx") && line.contains("console.log(") {
                issues.push(format!(
                    "{}:{}:warning: console.log left in source | {}",
                    path.display(),
                    line_no + 1,
                    outline_signature(line)
                ));
            }
            if ext == "py" && line.contains("print(") {
                issues.push(format!(
                    "{}:{}:warning: print call left in source | {}",
                    path.display(),
                    line_no + 1,
                    outline_signature(line)
                ));
            }
            if let Some((label, redacted)) = secret_diagnostic(line) {
                issues.push(format!(
                    "{}:{}:warning: possible hard-coded secret ({label}) | {redacted}",
                    path.display(),
                    line_no + 1
                ));
            }
            if issues.len() >= max_entries {
                break;
            }
        }
        if issues.len() < max_entries {
            issues.extend(relative_import_diagnostics(path, &text));
        }
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            if let Err(err) = serde_json::from_str::<Value>(&text) {
                issues.push(format!("{}:1:error: invalid JSON: {err}", path.display()));
            }
        }
        if path.extension().and_then(|value| value.to_str()) == Some("toml") {
            if let Some(issue) = toml_syntax_diagnostic(path) {
                issues.push(issue);
            }
        }
        if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yaml" | "yml")
        ) {
            if let Some((line, message)) = yaml_structural_issue(&text) {
                issues.push(format!(
                    "{}:{}:error: invalid YAML: {}",
                    path.display(),
                    line,
                    message
                ));
            }
        }
        if path.extension().and_then(|value| value.to_str()) == Some("py") {
            if let Some(issue) = python_syntax_diagnostic(path) {
                issues.push(issue);
            }
        }
    }
    (issues.into_iter().take(max_entries).collect(), scanned)
}

fn strip_import_suffix(specifier: &str) -> &str {
    specifier
        .split('?')
        .next()
        .unwrap_or(specifier)
        .split('#')
        .next()
        .unwrap_or(specifier)
}

fn js_relative_import_exists(path: &Path, specifier: &str) -> bool {
    const EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "json"];
    let specifier = strip_import_suffix(specifier);
    let base = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(specifier);
    let mut candidates = vec![base.clone()];
    if base.extension().is_some() {
        for ext in EXTS {
            candidates.push(base.with_extension(ext));
        }
    } else {
        for ext in EXTS {
            candidates.push(PathBuf::from(format!("{}.{}", base.display(), ext)));
        }
    }
    for ext in EXTS {
        candidates.push(base.join(format!("index.{ext}")));
    }
    candidates.iter().any(|candidate| candidate.is_file())
}

fn python_relative_import_exists(path: &Path, module: &str) -> bool {
    let dot_count = module.chars().take_while(|value| *value == '.').count();
    let remainder = module.trim_start_matches('.');
    let mut base = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    for _ in 0..dot_count.saturating_sub(1) {
        if let Some(parent) = base.parent() {
            base = parent.to_path_buf();
        }
    }
    if remainder.is_empty() {
        return true;
    }
    let mut target = base;
    for part in remainder.split('.') {
        target.push(part);
    }
    target.with_extension("py").is_file() || target.join("__init__.py").is_file()
}

fn relative_import_diagnostics(path: &Path, text: &str) -> Vec<String> {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let js_patterns = [
        r#"^\s*import(?:\s+type)?(?:.+?\s+from\s+|\s*)["'](?P<spec>\.[^"']+)["']"#,
        r#"^\s*export(?:\s+type)?\s+.+?\s+from\s+["'](?P<spec>\.[^"']+)["']"#,
        r#"\brequire\(\s*["'](?P<spec>\.[^"']+)["']\s*\)"#,
        r#"\bimport\(\s*["'](?P<spec>\.[^"']+)["']\s*\)"#,
    ]
    .iter()
    .filter_map(|pattern| Regex::new(pattern).ok())
    .collect::<Vec<_>>();
    let python_pattern =
        Regex::new(r"^\s*from\s+(?P<module>\.+[A-Za-z_][A-Za-z0-9_.]*)\s+import\s+").ok();
    let mut issues = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        if matches!(ext, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs") {
            for pattern in &js_patterns {
                let Some(captures) = pattern.captures(line) else {
                    continue;
                };
                let Some(specifier) = captures.name("spec").map(|value| value.as_str()) else {
                    continue;
                };
                if !js_relative_import_exists(path, specifier) {
                    issues.push(format!(
                        "{}:{}:error: unresolved relative import {:?} | {}",
                        path.display(),
                        line_no,
                        specifier,
                        outline_signature(line)
                    ));
                }
                break;
            }
        } else if ext == "py" {
            let Some(pattern) = &python_pattern else {
                continue;
            };
            let Some(captures) = pattern.captures(line) else {
                continue;
            };
            let Some(module) = captures.name("module").map(|value| value.as_str()) else {
                continue;
            };
            if !python_relative_import_exists(path, module) {
                issues.push(format!(
                    "{}:{}:error: unresolved Python relative import {:?} | {}",
                    path.display(),
                    line_no,
                    module,
                    outline_signature(line)
                ));
            }
        }
    }
    issues
}

fn relative_path_string(from_dir: &Path, to: &Path) -> String {
    let from_components = from_dir.components().collect::<Vec<_>>();
    let to_components = to.components().collect::<Vec<_>>();
    let mut shared = 0usize;
    while shared < from_components.len()
        && shared < to_components.len()
        && from_components[shared] == to_components[shared]
    {
        shared += 1;
    }
    let mut rel = PathBuf::new();
    for _ in shared..from_components.len() {
        rel.push("..");
    }
    for component in &to_components[shared..] {
        rel.push(component.as_os_str());
    }
    let mut text = path_slash(&rel);
    if !text.starts_with('.') {
        text = format!("./{text}");
    }
    text
}

fn relative_js_import_spec(importer: &Path, destination: &Path, old_spec: &str) -> String {
    let split_at = old_spec.find(['?', '#']).unwrap_or(old_spec.len());
    let base_spec = &old_spec[..split_at];
    let suffix = &old_spec[split_at..];
    let mut rel = relative_path_string(
        importer.parent().unwrap_or_else(|| Path::new(".")),
        destination,
    );
    let destination_ext = destination
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if Path::new(base_spec).extension().is_none()
        && matches!(
            destination_ext,
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "json"
        )
    {
        let extension = format!(".{destination_ext}");
        if rel.ends_with(&extension) {
            rel.truncate(rel.len().saturating_sub(extension.len()));
        }
    }
    format!("{rel}{suffix}")
}

fn move_file_import_updates(
    root: &Path,
    source: &Path,
    destination: &Path,
    max_updates: usize,
) -> Result<(Vec<MoveImportUpdate>, usize), String> {
    let mut updates = Vec::new();
    let mut scanned = 0usize;
    let source_resolved = source.canonicalize().map_err(|err| err.to_string())?;
    let patterns = [
        r#"^\s*import(?:\s+type)?(?:.+?\s+from\s+|\s*)["'](?P<spec>\.[^"']+)["']"#,
        r#"^\s*export(?:\s+type)?\s+.+?\s+from\s+["'](?P<spec>\.[^"']+)["']"#,
        r#"\brequire\(\s*["'](?P<spec>\.[^"']+)["']\s*\)"#,
        r#"\bimport\(\s*["'](?P<spec>\.[^"']+)["']\s*\)"#,
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).expect("trusted import discovery pattern"))
    .collect::<Vec<_>>();
    for path in move_import_candidates(root)? {
        let path = path.as_path();
        scanned += 1;
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if path == source || path == destination || !matches!(ext, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs") {
            continue;
        }
        let metadata = fs::metadata(path).map_err(|err| err.to_string())?;
        if metadata.len() > MAX_EDITED_FILE_BYTES as u64 { return Err("Import candidate exceeds the edit capacity".into()); }
        let text = fs::read_to_string(path).map_err(|err| format!("Cannot inspect import candidate {}: {err}", path.display()))?;
        let observed = observe_file_mutation_target(path).map_err(|outcome| outcome.text)?
            .ok_or("Import candidate disappeared")?;
        if observed.version.sha256 != sha256_hex(text.as_bytes()) {
            return Err("Import candidate changed during planning".into());
        }
        let mut updated = String::new();
        let mut changed_lines = Vec::new();
        for (index, line) in text.split_inclusive('\n').enumerate() {
            let mut output_line = line.to_string();
            let line_for_match = line.trim_end_matches(['\r', '\n']);
            for pattern in &patterns {
                let Some(captures) = pattern.captures(line_for_match) else {
                    continue;
                };
                let Some(spec) = captures.name("spec").map(|value| value.as_str()) else {
                    continue;
                };
                let Some(local) = resolve_dependency_path(path, root, spec) else { break; };
                let local_resolved = local.canonicalize().map_err(|err| err.to_string())?;
                if local_resolved != source_resolved {
                    break;
                }
                let new_spec = relative_js_import_spec(path, destination, spec);
                let quoted = if line_for_match.contains(&format!("'{spec}'")) {
                    format!("'{spec}'")
                } else {
                    format!("\"{spec}\"")
                };
                let replacement = if quoted.starts_with('\'') {
                    format!("'{new_spec}'")
                } else {
                    format!("\"{new_spec}\"")
                };
                output_line = output_line.replacen(&quoted, &replacement, 1);
                let rel = path
                    .strip_prefix(root)
                    .map(path_slash)
                    .unwrap_or_else(|_| path.display().to_string());
                changed_lines.push(format!("{rel}:{}: {spec} -> {new_spec}", index + 1));
                break;
            }
            updated.push_str(&output_line);
        }
        if !changed_lines.is_empty() && updated != text {
            if updates.len() >= max_updates {
                return Err("Import update member limit exceeded; no partial collection is authorized".into());
            }
            updates.push(MoveImportUpdate {
                path: path.to_path_buf(),
                original: text,
                version: observed.version,
                updated,
                lines: changed_lines,
            });
        }
    }
    updates.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((updates, scanned))
}

fn secret_diagnostic(line: &str) -> Option<(String, String)> {
    let token_patterns = [
        ("AWS access key", r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
        ("OpenAI-style API key", r"\bsk-[A-Za-z0-9_-]{20,}\b"),
        ("GitHub token", r"\bgh[pousr]_[A-Za-z0-9_]{30,}\b"),
    ];
    for (label, pattern) in token_patterns {
        if Regex::new(pattern)
            .expect("valid secret token regex")
            .is_match(line)
        {
            return Some((
                label.to_string(),
                "[secret-like token redacted]".to_string(),
            ));
        }
    }
    let assignment = RegexBuilder::new(
        r#"\b(?P<key>api[_-]?key|secret|token|password|passwd|credential)\b\s*[:=]\s*["'](?P<value>[^"']{8,})["']"#,
    )
    .case_insensitive(true)
    .build()
    .expect("valid secret assignment regex");
    let captures = assignment.captures(line)?;
    let key = captures.name("key")?.as_str();
    let value = captures.name("value")?.as_str().trim().to_lowercase();
    if matches!(
        value.as_str(),
        "changeme" | "example" | "placeholder" | "redacted"
    ) || value.contains("your-")
    {
        return None;
    }
    Some((format!("{key} assignment"), format!("{key} = [REDACTED]")))
}

fn toml_syntax_diagnostic(path: &Path) -> Option<String> {
    let script = "import sys, tomllib\nwith open(sys.argv[1], 'rb') as f:\n    tomllib.load(f)\n";
    for candidate in ["python3", "python"] {
        let Ok(output) = Command::new(candidate)
            .arg("-c")
            .arg(script)
            .arg(path)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
        else {
            continue;
        };
        if output.status.success() {
            return None;
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("ModuleNotFoundError") && stderr.contains("tomllib") {
            continue;
        }
        let message = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .unwrap_or("invalid TOML");
        let line = Regex::new(r"line\s+(\d+)")
            .ok()
            .and_then(|regex| {
                regex
                    .captures(message)
                    .and_then(|captures| captures.get(1))
                    .and_then(|value| value.as_str().parse::<usize>().ok())
            })
            .unwrap_or(1);
        return Some(format!(
            "{}:{}:error: invalid TOML: {}",
            path.display(),
            line,
            outline_signature(message)
        ));
    }
    None
}

fn yaml_structural_issue(text: &str) -> Option<(usize, String)> {
    let mut stack: Vec<(char, usize)> = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let line_no = line_index + 1;
        let stripped = line.trim();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        let indent_len = line
            .chars()
            .take_while(|value| *value == ' ' || *value == '\t')
            .count();
        if line.chars().take(indent_len).any(|value| value == '\t') {
            return Some((line_no, "indentation contains a tab".to_string()));
        }

        let mut quote: Option<char> = None;
        let mut escaped = false;
        for value in stripped.chars() {
            if let Some(current_quote) = quote {
                if current_quote == '"' && value == '\\' && !escaped {
                    escaped = true;
                    continue;
                }
                if value == current_quote && !escaped {
                    quote = None;
                }
                escaped = false;
                continue;
            }
            match value {
                '\'' | '"' => quote = Some(value),
                '[' | '{' => stack.push((value, line_no)),
                ']' => {
                    if !matches!(stack.last(), Some(('[', _))) {
                        return Some((line_no, "unexpected closing ]".to_string()));
                    }
                    stack.pop();
                }
                '}' => {
                    if !matches!(stack.last(), Some(('{', _))) {
                        return Some((line_no, "unexpected closing }".to_string()));
                    }
                    stack.pop();
                }
                _ => {}
            }
        }
    }
    stack
        .last()
        .map(|(opener, line)| (*line, format!("unclosed {opener}")))
}

fn python_syntax_diagnostic(path: &Path) -> Option<String> {
    // py_compile writes .pyc even with PYTHONDONTWRITEBYTECODE; compile only
    // in memory, and keep the source location without a wrapper traceback.
    let syntax_script = "import sys, traceback\ntry:\n compile(open(sys.argv[1], 'rb').read(), sys.argv[1], 'exec')\nexcept SyntaxError as error:\n sys.stderr.write(''.join(traceback.format_exception_only(type(error), error)))\n sys.exit(1)";
    let output = Command::new("python3")
        .arg("-c")
        .arg(syntax_script)
        .arg(path)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .or_else(|_| {
            Command::new("python")
                .arg("-c")
                .arg(syntax_script)
                .arg(path)
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .output()
        })
        .ok()?;
    if output.status.success() {
        return None;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = Regex::new(r"line\s+(\d+)")
        .ok()
        .and_then(|regex| {
            regex
                .captures(&stderr)
                .and_then(|captures| captures.get(1))
                .and_then(|value| value.as_str().parse::<usize>().ok())
        })
        .unwrap_or(1);
    let message = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("invalid Python syntax");
    Some(format!(
        "{}:{}:error: Python syntax error: {}",
        path.display(),
        line,
        outline_signature(message)
    ))
}

#[derive(Clone, Debug)]
struct ImportHit {
    line: usize,
    kind: String,
    spec: String,
    signature: String,
}

fn import_patterns() -> Result<Vec<(&'static str, Regex)>, ToolOutcome> {
    [
        (
            "js_import",
            r#"^\s*import(?:\s+type)?(?:.+?\s+from\s+|\s*)["']([^"']+)["']"#,
        ),
        (
            "js_export",
            r#"^\s*export(?:\s+type)?\s+.+?\s+from\s+["']([^"']+)["']"#,
        ),
        ("js_require", r#"\brequire\(\s*["']([^"']+)["']\s*\)"#),
        ("js_dynamic_import", r#"\bimport\(\s*["']([^"']+)["']\s*\)"#),
        ("python_from", r"^\s*from\s+([.\w]+)\s+import\s+"),
        (
            "python_import",
            r"^\s*import\s+([A-Za-z_][A-Za-z0-9_.,\s]*)$",
        ),
        (
            "swift_import",
            r"^\s*@?\w*\s*import\s+([A-Za-z_][A-Za-z0-9_.]*)",
        ),
        ("jvm_import", r"^\s*import\s+([A-Za-z_][A-Za-z0-9_.*]*)"),
        (
            "rust_use",
            r"^\s*use\s+([A-Za-z_:][A-Za-z0-9_:{}*,\s]*)\s*;",
        ),
        (
            "rust_mod",
            r"^\s*(?:pub\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;",
        ),
        ("c_include", r#"^\s*#\s*include\s+[<"]([^>"]+)[>"]"#),
    ]
    .into_iter()
    .map(|(kind, pattern)| {
        Regex::new(pattern)
            .map(|regex| (kind, regex))
            .map_err(|err| error(format!("import regex failed: {err}")))
    })
    .collect()
}

fn extract_imports(text: &str, max_imports: usize) -> Result<Vec<ImportHit>, ToolOutcome> {
    let patterns = import_patterns()?;
    let mut imports = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        for (kind, regex) in &patterns {
            let Some(captures) = regex.captures(line) else {
                continue;
            };
            let Some(spec) = captures.get(1) else {
                continue;
            };
            let specs = if *kind == "python_import" {
                spec.as_str()
                    .split(',')
                    .filter_map(|part| part.trim().split(" as ").next())
                    .filter(|part| !part.trim().is_empty())
                    .map(|part| part.trim().to_string())
                    .collect::<Vec<_>>()
            } else {
                vec![spec.as_str().trim().to_string()]
            };
            for spec in specs {
                imports.push(ImportHit {
                    line: line_no + 1,
                    kind: kind.to_string(),
                    spec,
                    signature: outline_signature(line),
                });
                if imports.len() >= max_imports {
                    return Ok(imports);
                }
            }
            break;
        }
    }
    Ok(imports)
}

fn dependency_candidates(base: &Path, parts: &[String]) -> Vec<PathBuf> {
    let mut stem = base.to_path_buf();
    for part in parts {
        stem.push(part);
    }
    let mut candidates = Vec::new();
    for suffix in [
        "", ".py", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".swift", ".kt", ".java", ".rs",
        ".h", ".hpp",
    ] {
        candidates.push(PathBuf::from(format!("{}{}", stem.display(), suffix)));
    }
    for index_name in ["__init__.py", "index.ts", "index.tsx", "index.js", "mod.rs"] {
        candidates.push(stem.join(index_name));
    }
    candidates
}

fn resolve_dependency_path(source: &Path, root: &Path, spec: &str) -> Option<PathBuf> {
    let mut bases: Vec<(PathBuf, Vec<String>)> = Vec::new();
    if spec.starts_with("./") || spec.starts_with("../") {
        bases.push((
            source.parent()?.to_path_buf(),
            spec.split('/')
                .filter(|part| !part.is_empty() && *part != ".")
                .map(ToOwned::to_owned)
                .collect(),
        ));
    } else if spec.starts_with('.') {
        let leading = spec.chars().take_while(|ch| *ch == '.').count();
        let rest = spec.trim_start_matches('.');
        let mut base = source.parent()?.to_path_buf();
        for _ in 0..leading.saturating_sub(1) {
            base = base.parent()?.to_path_buf();
        }
        bases.push((
            base,
            rest.split('.')
                .filter(|part| !part.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        ));
    } else if let Some(rest) = spec.strip_prefix("crate::") {
        let parts = rest
            .split("::")
            .filter(|part| !part.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        bases.push((root.to_path_buf(), parts.clone()));
        bases.push((root.join("src"), parts));
    } else if spec.starts_with("super::") || spec.starts_with("self::") {
        bases.push((
            source.parent()?.to_path_buf(),
            spec.split("::")
                .skip(1)
                .filter(|part| !part.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        ));
    } else if spec.contains('.') && !spec.contains('/') {
        bases.push((
            root.to_path_buf(),
            spec.split('.')
                .filter(|part| !part.is_empty() && *part != "*")
                .map(ToOwned::to_owned)
                .collect(),
        ));
    } else if spec.contains('/') {
        bases.push((
            root.to_path_buf(),
            spec.split('/')
                .filter(|part| !part.is_empty() && *part != "." && *part != "*")
                .map(ToOwned::to_owned)
                .collect(),
        ));
    } else {
        let parts = vec![spec.to_string()];
        bases.push((source.parent()?.to_path_buf(), parts.clone()));
        bases.push((root.to_path_buf(), parts.clone()));
        bases.push((root.join("src"), parts));
    }
    for (base, parts) in bases {
        for candidate in dependency_candidates(&base, &parts) {
            let Ok(resolved) = candidate.canonicalize() else {
                continue;
            };
            if !resolved.starts_with(root) {
                continue;
            }
            if resolved.is_file() {
                return Some(resolved);
            }
        }
    }
    None
}

fn scan_importers(root: &Path, target: &Path, max_importers: usize) -> (Vec<String>, usize) {
    let mut importers = Vec::new();
    let mut scanned = 0usize;
    let Ok(target_resolved) = target.canonicalize() else {
        return (importers, scanned);
    };
    walk_files(root, usize::MAX, &mut |path| {
        scanned += 1;
        if path == target || !is_code_source(path) {
            return importers.len() < max_importers;
        }
        let Ok(text) = fs::read_to_string(path) else {
            return importers.len() < max_importers;
        };
        let imports = extract_imports(&text, 100).unwrap_or_default();
        for item in imports {
            let Some(local) = resolve_dependency_path(path, root, &item.spec) else {
                continue;
            };
            let Ok(local_resolved) = local.canonicalize() else {
                continue;
            };
            if local_resolved != target_resolved {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map(path_slash)
                .unwrap_or_else(|_| path.display().to_string());
            importers.push(format!(
                "{}:{}: {} {} | {}",
                rel, item.line, item.kind, item.spec, item.signature
            ));
            if importers.len() >= max_importers {
                return false;
            }
        }
        importers.len() < max_importers
    });
    (importers, scanned)
}

fn manifest_names() -> &'static [&'static str] {
    &[
        "package.json",
        "pyproject.toml",
        "requirements.txt",
        "Cargo.toml",
        "go.mod",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
        "pom.xml",
        "Gemfile",
        "composer.json",
        "Package.swift",
        "Podfile",
    ]
}

fn doc_names() -> &'static [&'static str] {
    &[
        "NOLA.md",
        "NOLA.local.md",
        "AGENTS.override.md",
        "AGENTS.md",
        "README.md",
        "README",
        "CONTRIBUTING.md",
        "Makefile",
        "justfile",
    ]
}

fn convention_config_names() -> &'static [&'static str] {
    &[
        ".editorconfig",
        ".prettierrc",
        ".prettierrc.json",
        ".prettierrc.yml",
        ".prettierrc.yaml",
        "prettier.config.js",
        "eslint.config.js",
        ".eslintrc",
        ".eslintrc.json",
        "ruff.toml",
        "pyproject.toml",
        "rustfmt.toml",
        ".swiftformat",
        ".swiftlint.yml",
        ".ktlint.yml",
    ]
}

fn language_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "py" => Some("Python"),
        "ts" => Some("TypeScript"),
        "tsx" => Some("TypeScript React"),
        "js" => Some("JavaScript"),
        "jsx" => Some("JavaScript React"),
        "swift" => Some("Swift"),
        "kt" => Some("Kotlin"),
        "java" => Some("Java"),
        "rs" => Some("Rust"),
        "go" => Some("Go"),
        "c" => Some("C"),
        "h" => Some("C/C++"),
        "cpp" | "hpp" => Some("C++"),
        "m" => Some("Objective-C"),
        "mm" => Some("Objective-C++"),
        "rb" => Some("Ruby"),
        "php" => Some("PHP"),
        "sh" => Some("Shell"),
        "html" => Some("HTML"),
        "css" => Some("CSS"),
        _ => None,
    }
}

fn scan_project(
    root: &Path,
    max_files: usize,
) -> (
    Vec<PathBuf>,
    HashMap<String, usize>,
    Vec<PathBuf>,
    usize,
    bool,
) {
    let mut manifests = Vec::new();
    let mut languages = HashMap::new();
    let mut docs = Vec::new();
    let mut scanned = 0usize;
    let mut truncated = false;
    walk_files(root, max_files.saturating_add(1), &mut |path| {
        if scanned >= max_files {
            truncated = true;
            return false;
        }
        scanned += 1;
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            return true;
        };
        if manifest_names().contains(&name.as_str()) {
            manifests.push(path.to_path_buf());
        }
        if doc_names().contains(&name.as_str()) {
            docs.push(path.to_path_buf());
        }
        if let Some(ext) = path.extension().and_then(|value| value.to_str()) {
            if let Some(language) = language_for_ext(ext) {
                *languages.entry(language.to_string()).or_insert(0) += 1;
            }
        }
        true
    });
    manifests.sort();
    docs.sort();
    (manifests, languages, docs, scanned, truncated)
}

fn project_top_level_entries(root: &Path, max_entries: usize) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut entries = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type().ok()?;
            if file_type.is_dir()
                && (should_skip_dir(&name)
                    || matches!(name.as_str(), "build" | ".build" | "DerivedData"))
            {
                return None;
            }
            Some(if file_type.is_dir() {
                format!("{name}/")
            } else {
                name
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.to_lowercase());
    entries.truncate(max_entries);
    entries
}

fn scan_convention_config_files(root: &Path, max_files: usize) -> (Vec<PathBuf>, usize) {
    let mut configs = Vec::new();
    let mut scanned = 0usize;
    walk_files(root, usize::MAX, &mut |path| {
        scanned += 1;
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return configs.len() < max_files;
        };
        if convention_config_names().contains(&name) {
            configs.push(path.to_path_buf());
        }
        configs.len() < max_files
    });
    configs.sort();
    (configs, scanned)
}

fn coding_convention_files(
    root: &Path,
    paths: Option<Vec<PathBuf>>,
    glob: Option<&Regex>,
    max_files: usize,
) -> (Vec<PathBuf>, &'static str, usize) {
    if let Some(paths) = paths {
        let total = paths.len();
        return (
            paths.into_iter().take(max_files).collect(),
            "explicit_paths",
            total,
        );
    }
    let changed = git_changed_files(root)
        .into_iter()
        .filter(|path| path.is_file() && is_code_source(path))
        .collect::<Vec<_>>();
    if !changed.is_empty() {
        let total = changed.len();
        return (
            changed.into_iter().take(max_files).collect(),
            "git_changed_files",
            total,
        );
    }
    let mut files = Vec::new();
    let mut scanned = 0usize;
    walk_files(root, usize::MAX, &mut |path| {
        scanned += 1;
        if !is_code_source(path) {
            return files.len() < max_files;
        }
        if let Some(glob) = glob {
            if let Ok(rel) = path.strip_prefix(root) {
                if !glob.is_match(&path_slash(rel)) {
                    return files.len() < max_files;
                }
            }
        }
        files.push(path.to_path_buf());
        files.len() < max_files
    });
    (files, "glob", scanned)
}

fn comment_header(text: &str, ext: &str) -> Option<String> {
    let mut header = Vec::new();
    for line in text.lines().take(8) {
        let stripped = line.trim();
        if stripped.is_empty() {
            if !header.is_empty() {
                break;
            }
            continue;
        }
        let mut is_comment = stripped.starts_with('#')
            || stripped.starts_with("//")
            || stripped.starts_with("/*")
            || stripped.starts_with('*')
            || stripped.starts_with("<!--");
        if matches!(ext, "py" | "sh" | "rb") {
            is_comment =
                is_comment || stripped.starts_with("\"\"\"") || stripped.starts_with("'''");
        }
        if is_comment {
            header.push(stripped);
            continue;
        }
        break;
    }
    if header.is_empty() {
        None
    } else {
        Some(outline_signature(&header.join(" ")))
    }
}

fn coding_convention_summary(root: &Path, files: &[PathBuf], configs: &[PathBuf]) -> Vec<String> {
    let mut languages: HashMap<String, usize> = HashMap::new();
    let mut space_indents: HashMap<usize, usize> = HashMap::new();
    let mut tab_indents = 0usize;
    let mut final_newline_yes = 0usize;
    let mut final_newline_no = 0usize;
    let mut headers: HashMap<String, usize> = HashMap::new();
    let mut test_signals: HashMap<String, usize> = HashMap::new();
    let mut samples = Vec::new();
    for path in files {
        if let Some(language) = path
            .extension()
            .and_then(|value| value.to_str())
            .and_then(language_for_ext)
        {
            *languages.entry(language.to_string()).or_insert(0) += 1;
        }
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let text =
            String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_READ_BYTES as usize)]).to_string();
        if text.ends_with('\n') {
            final_newline_yes += 1;
        } else {
            final_newline_no += 1;
        }
        if samples.len() < 12 {
            samples.push(relative_display(path, root));
        }
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if let Some(header) = comment_header(&text, ext) {
            *headers.entry(header).or_insert(0) += 1;
        }
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let indent = line
                .chars()
                .take_while(|value| *value == ' ' || *value == '\t')
                .collect::<String>();
            if indent.is_empty() {
                continue;
            }
            if indent.contains('\t') {
                tab_indents += 1;
            } else {
                *space_indents.entry(indent.len()).or_insert(0) += 1;
            }
        }
        let lower = text.to_ascii_lowercase();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("test_"))
            || text.contains("def test_")
            || lower.contains("pytest")
        {
            *test_signals.entry("pytest".to_string()).or_insert(0) += 1;
        }
        if lower.contains("unittest.testcase") {
            *test_signals.entry("unittest".to_string()).or_insert(0) += 1;
        }
        if lower.contains("xctest") {
            *test_signals.entry("XCTest".to_string()).or_insert(0) += 1;
        }
        if text.contains("@Test") || lower.contains("org.junit") {
            *test_signals.entry("JUnit".to_string()).or_insert(0) += 1;
        }
        if matches!(ext, "js" | "jsx" | "ts" | "tsx")
            && (text.contains("describe(") || text.contains("it(") || text.contains("test("))
        {
            *test_signals.entry("Jest/Vitest".to_string()).or_insert(0) += 1;
        }
    }
    let config_lines = if configs.is_empty() {
        "(none)".to_string()
    } else {
        configs
            .iter()
            .take(20)
            .map(|path| format!("- {}", relative_display(path, root)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let map_lines = |items: HashMap<String, usize>| -> String {
        let mut items = items.into_iter().collect::<Vec<_>>();
        items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        if items.is_empty() {
            "(none)".to_string()
        } else {
            items
                .into_iter()
                .map(|(name, count)| format!("- {name}: {count}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    let mut indent_items = space_indents.into_iter().collect::<Vec<_>>();
    indent_items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut indent_lines = indent_items
        .into_iter()
        .take(5)
        .map(|(size, count)| format!("- spaces: {size} ({count} lines)"))
        .collect::<Vec<_>>();
    if tab_indents > 0 {
        indent_lines.push(format!("- tabs: {tab_indents} lines"));
    }
    let mut header_items = headers.into_iter().collect::<Vec<_>>();
    header_items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let header_lines = if header_items.is_empty() {
        "(none)".to_string()
    } else {
        header_items
            .into_iter()
            .take(8)
            .map(|(header, count)| format!("- {header} ({count} files)"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    vec![
        "## Config files".to_string(),
        config_lines,
        String::new(),
        "## Sampled source files".to_string(),
        if samples.is_empty() {
            "(none)".to_string()
        } else {
            samples
                .into_iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
        String::new(),
        "## Languages".to_string(),
        map_lines(languages),
        String::new(),
        "## Indentation".to_string(),
        if indent_lines.is_empty() {
            "(none detected)".to_string()
        } else {
            indent_lines.join("\n")
        },
        String::new(),
        "## Final newline".to_string(),
        format!("- yes: {final_newline_yes}"),
        format!("- no: {final_newline_no}"),
        String::new(),
        "## File headers".to_string(),
        header_lines,
        String::new(),
        "## Test framework signals".to_string(),
        map_lines(test_signals),
    ]
}

fn repo_map_files(root: &Path, glob: &Regex, max_files: usize) -> (Vec<PathBuf>, usize) {
    let mut files = Vec::new();
    let mut scanned = 0usize;
    walk_files(root, usize::MAX, &mut |path| {
        scanned += 1;
        if !is_text_source(path) {
            return files.len() < max_files;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            if glob.is_match(&path_slash(rel)) {
                files.push(path.to_path_buf());
            }
        }
        files.len() < max_files
    });
    (files, scanned)
}

fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(path_slash)
        .unwrap_or_else(|_| path.display().to_string())
}
