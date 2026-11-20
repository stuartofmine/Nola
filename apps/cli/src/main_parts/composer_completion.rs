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
}

fn matching_slash_commands_with_config(input: &str, config: &Config) -> Vec<SlashCompletion> {
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
    let command = normalize_interactive_command(raw_command).unwrap_or(raw_command);

    if command == "/room" && !rest.is_empty() {
        return interactive_room_completions(rest);
    }

    if command == "/model" {
        let (prefix, query) = interactive_model_completion_parts(rest);
        return available_interactive_models(config, query)
            .into_iter()
            .map(|model| SlashCompletion {
                name: format!("{prefix}{model}"),
            })
            .take(4)
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
            .map(|backend| SlashCompletion {
                name: format!("/executor {}", backend.id()),
            })
            .collect();
    }
    if command == "/goal" && rest.is_empty() {
        return interactive_goal_subcommand_matches(rest);
    }
    if command == "/goal" {
        return interactive_goal_subcommand_matches(rest);
    }
    if command == "/debug" {
        return interactive_toggle_matches(rest, true)
            .into_iter()
            .map(|value| SlashCompletion {
                name: format!("/debug {value}"),
            })
            .collect();
    }
    if command == "/skills" {
        return ["list", "enable", "disable"]
            .into_iter()
            .filter(|sub| rest.is_empty() || sub.starts_with(&rest.to_ascii_lowercase()))
            .map(|sub| SlashCompletion {
                name: format!("/skills {sub}"),
            })
            .collect();
    }
    if command == "/mcp" {
        return ["list", "enable", "disable"]
            .into_iter()
            .filter(|sub| rest.is_empty() || sub.starts_with(&rest.to_ascii_lowercase()))
            .map(|sub| SlashCompletion {
                name: format!("/mcp {sub}"),
            })
            .collect();
    }
    if command == "/memory" {
        return ["list", "search", "add"]
            .into_iter()
            .filter(|sub| rest.is_empty() || sub.starts_with(&rest.to_ascii_lowercase()))
            .map(|sub| SlashCompletion {
                name: format!("/memory {sub}"),
            })
            .collect();
    }
    if command == "/allow" {
        return Vec::new();
    }

    let command = if command == raw_command {
        trimmed
    } else {
        command
    };

    INTERACTIVE_SLASH_COMMANDS
        .iter()
        .filter(|item| item.name.starts_with(command))
        .map(|command| SlashCompletion {
            name: command.name.to_string(),
        })
        .collect()
}

fn first_matching_slash_completion(input: &str, config: &Config) -> Option<SlashCompletion> {
    // The visible candidate list and Tab must use the same contract; independent
    // resolvers previously showed /executor candidates that Tab could not insert.
    matching_slash_commands_with_config(input, config).into_iter().next()
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

fn workspace_file_completions(project_root: &std::path::Path, prefix: &str) -> Vec<String> {
    let mut stack = vec![project_root.to_path_buf()];
    let mut collected = Vec::new();
    let max_entries = 400usize;
    let query = prefix.trim().to_ascii_lowercase();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if collected.len() >= max_entries {
                break;
            }
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.')
                || matches!(
                    name_str.as_ref(),
                    "target" | "node_modules" | "dist" | "build" | ".git"
                )
            {
                continue;
            }
            let Ok(rel) = path.strip_prefix(project_root) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                stack.push(path);
                let dir_entry = format!("{rel_str}/");
                if query.is_empty() || dir_entry.to_ascii_lowercase().contains(&query) {
                    collected.push(dir_entry);
                }
            } else if query.is_empty() || rel_str.to_ascii_lowercase().contains(&query) {
                collected.push(rel_str);
            }
        }
        if collected.len() >= max_entries {
            break;
        }
    }
    collected.sort_by(|a, b| {
        let a_pref = !query.is_empty() && a.to_ascii_lowercase().starts_with(&query);
        let b_pref = !query.is_empty() && b.to_ascii_lowercase().starts_with(&query);
        b_pref
            .cmp(&a_pref)
            .then_with(|| a.len().cmp(&b.len()))
            .then_with(|| a.cmp(b))
    });
    collected.truncate(12);
    collected
}

fn apply_composer_completion_cycle(
    line: &mut String,
    cursor: &mut usize,
    config: &Config,
    cycle: &mut Option<ComposerCompletionCycle>,
    reverse: bool,
    mentions: &mut Vec<crate::cli_expert_mentions::Occurrence>,
) -> bool {
    let mut insert = |line: &mut String, start: usize, end: usize, token: &str| {
        let selected = crate::cli_expert_mentions::completion(token, start);
        let display = selected.as_ref().map(|r| r.text.as_str()).unwrap_or(token);
        line.replace_range(start..end, display);
        crate::cli_expert_mentions::replace(mentions, start, end, display.len());
        let at = start + display.len();
        if let Some(run) = selected {
            let index = mentions.partition_point(|r| r.start < start);
            mentions.insert(index, run);
        }
        at
    };
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
            let new_end = insert(line, existing.span_start, existing.last_inserted_end, &next);
            existing.last_inserted_end = new_end;
            *cursor = new_end;
            return true;
        }
    }

    let trimmed = line.trim_start();
    let (span_start, span_end, candidates) = if trimmed.starts_with('/') {
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
    let new_end = insert(line, span_start, span_end, &replacement);
    *cursor = new_end;
    *cycle = Some(ComposerCompletionCycle {
        span_start,
        candidates,
        index: idx,
        last_inserted_end: new_end,
    });
    true
}
