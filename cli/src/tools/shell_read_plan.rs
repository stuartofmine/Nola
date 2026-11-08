// SPDX-License-Identifier: Apache-2.0
//! Literal read pipelines: inspect argv/resources, then execute the pinned plan.
use std::path::{Path, PathBuf};

pub(super) struct ShellReadPlan {
    pub command: String,
    pub paths: Vec<PathBuf>,
}

struct Segment {
    source: String,
    separator: &'static str,
    stderr_null: bool,
}

// This is deliberately a bounded literal grammar, not a shell-risk keyword list.
// Quotes protect regex syntax; expansions, scripts and arbitrary redirects have
// no read-only contract. Reconstruct argv instead of executing the source again.
fn segments(command: &str) -> Option<Vec<Segment>> {
    if command.len() > 32_768 {
        return None;
    }
    let mut result = Vec::new();
    let mut source = String::new();
    let (mut quote, mut escaped, mut stderr_null) = (None, false, false);
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\0' || ch == '\n' || ch == '\r' {
            return None;
        }
        if escaped {
            source.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            source.push(ch);
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if active == '"' && matches!(ch, '$' | '`') {
                return None;
            }
            source.push(ch);
            if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                source.push(ch);
                quote = Some(ch);
            }
            '$' | '`' | '<' | '>' | '(' | ')' | '*' | '?' | '[' | ']' | '{' | '}' | '~' | '#' => {
                return None
            }
            '2' if source.ends_with(char::is_whitespace) && chars.peek() == Some(&'>') => {
                for expected in ">/dev/null".chars() {
                    if chars.next() != Some(expected) {
                        return None;
                    }
                }
                if chars
                    .peek()
                    .is_some_and(|c| !c.is_whitespace() && !matches!(c, ';' | '|' | '&'))
                {
                    return None;
                }
                if stderr_null {
                    return None;
                }
                stderr_null = true;
            }
            ';' | '|' | '&' => {
                let separator = match ch {
                    ';' => ";",
                    '|' if chars.next_if_eq(&'|').is_some() => "||",
                    '|' => "|",
                    '&' if chars.next_if_eq(&'&').is_some() => "&&",
                    _ => return None,
                };
                if source.trim().is_empty() {
                    return None;
                }
                result.push(Segment {
                    source: std::mem::take(&mut source),
                    separator,
                    stderr_null,
                });
                stderr_null = false;
                if result.len() >= 32 {
                    return None;
                }
            }
            _ => source.push(ch),
        }
    }
    if escaped || quote.is_some() || source.trim().is_empty() {
        return None;
    }
    result.push(Segment {
        source,
        separator: "",
        stderr_null,
    });
    Some(result)
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

impl ShellReadPlan {
    pub fn from_command(command: &str, cwd: &Path) -> Option<Self> {
        let mut plan = Self {
            command: String::new(),
            paths: Vec::new(),
        };
        let mut piped_input = false;
        for segment in segments(command)? {
            // The shared tokenizer drops empty argv. Decline these rather than
            // changing the invocation's semantics during reconstruction.
            if segment.source.contains("''") || segment.source.contains("\"\"") {
                return None;
            }
            let tokens = nola_core::agent::permissions::supported_shell_tokens(&segment.source)?;
            let executable = tokens.first()?;
            let name = Path::new(executable).file_name()?.to_str()?;
            let program = [format!("/usr/bin/{name}"), format!("/bin/{name}")]
                .into_iter()
                .find(|p| Path::new(p).is_file())?;
            if executable.contains('/') && executable != &program {
                // /bin and /usr/bin can both contain the system program.
                if ![format!("/bin/{name}"), format!("/usr/bin/{name}")].contains(executable)
                    || !Path::new(executable).is_file()
                {
                    return None;
                }
            }
            let args = &tokens[1..];
            let mut files = Vec::new();
            let mut recursive = false;
            let mut directories = false;
            match name {
                "ls" => {
                    directories = true;
                    for arg in args {
                        if arg.starts_with('-') {
                            if arg.len() < 2 || !arg[1..].chars().all(|c| "alh1d".contains(c)) {
                                return None;
                            }
                        } else {
                            files.push(arg);
                        }
                    }
                    if files.is_empty() {
                        plan.paths.push(cwd.to_path_buf());
                    }
                }
                "sed" => {
                    // Only a numeric print range; no sed program execution,
                    // file commands, substitution flags or in-place writes.
                    if args.len() < 3 || args[0] != "-n" {
                        return None;
                    }
                    let range = args[1].strip_suffix('p')?;
                    let numbers = range.split(',').collect::<Vec<_>>();
                    if numbers.is_empty()
                        || numbers.len() > 2
                        || numbers
                            .iter()
                            .any(|n| n.parse::<u32>().ok().filter(|n| *n > 0).is_none())
                    {
                        return None;
                    }
                    for arg in &args[2..] {
                        if arg.starts_with('-') {
                            return None;
                        }
                        files.push(arg);
                    }
                }
                "echo" => {
                    // No implementation-dependent escape or option semantics.
                    if args.iter().any(|s| s.contains('\\'))
                        || args.first().is_some_and(|s| {
                            s.strip_prefix('-').is_some_and(|flags| {
                                !flags.is_empty() && flags.chars().all(|c| "neE".contains(c))
                            })
                        })
                    {
                        return None;
                    }
                }
                "grep" => {
                    let mut i = 0;
                    let mut pattern = false;
                    let mut options = true;
                    while i < args.len() {
                        let arg = &args[i];
                        if options && arg == "--" {
                            options = false;
                            i += 1;
                            continue;
                        }
                        if options && arg.starts_with('-') && arg != "-" {
                            match arg.as_str() {
                                "-e" => {
                                    i += 1;
                                    args.get(i)?;
                                    pattern = true;
                                }
                                "-A" | "-B" | "-C" | "-m" => {
                                    i += 1;
                                    args.get(i)?.parse::<u32>().ok()?;
                                }
                                // Pattern files are not covered by this argv contract.
                                _ if arg.len() > 1
                                    && arg[1..]
                                        .chars()
                                        .all(|c| "nHhivwxcqlLEFGosr".contains(c)) =>
                                {
                                    recursive |= arg.contains('r');
                                }
                                _ if arg.starts_with("--include=") && arg.len() > 10 => {}
                                _ => return None,
                            }
                        } else if !pattern {
                            pattern = true;
                        } else {
                            files.push(arg);
                        }
                        i += 1;
                    }
                    if !pattern || (files.is_empty() && !piped_input) {
                        return None;
                    }
                }
                "head" | "tail" | "cat" | "wc" => {
                    let mut i = 0;
                    let mut options = true;
                    while i < args.len() {
                        let arg = &args[i];
                        if options && arg == "--" {
                            options = false;
                            i += 1;
                            continue;
                        }
                        if options && arg.starts_with('-') && arg != "-" {
                            match name {
                                "head" | "tail" if matches!(arg.as_str(), "-n" | "-c") => {
                                    i += 1;
                                    args.get(i)?.parse::<u32>().ok()?;
                                }
                                "head" | "tail" if arg[1..].parse::<u32>().is_ok() => {}
                                "cat" if arg[1..].chars().all(|c| "nbsvet".contains(c)) => {}
                                "wc" if arg[1..].chars().all(|c| "lwcm".contains(c)) => {}
                                _ => return None,
                            }
                        } else {
                            files.push(arg);
                        }
                        i += 1;
                    }
                    if files.is_empty() && !piped_input {
                        return None;
                    }
                }
                _ => return None,
            }
            for file in files {
                if file == "-" {
                    if !piped_input {
                        return None;
                    }
                    continue;
                }
                let path = cwd.join(file);
                // Resolve even absent leaves through their real parent, so a
                // missing grep operand can report exit 2 without asking consent.
                let path = super::resolve_missing_path(&path)?;
                if path.is_dir() && recursive {
                    let mut pending = vec![path.clone()];
                    let mut visited = std::collections::BTreeSet::new();
                    let mut entries = 0usize;
                    while let Some(directory) = pending.pop() {
                        if !visited.insert(directory.clone()) {
                            continue;
                        }
                        plan.paths.push(directory.clone());
                        for entry in std::fs::read_dir(directory).ok()? {
                            entries += 1;
                            if entries > 10_000 {
                                return None;
                            }
                            let entry = entry.ok()?;
                            let original = entry.path();
                            let resolved = original.canonicalize().ok()?;
                            plan.paths.push(original);
                            if resolved.is_dir() {
                                pending.push(resolved);
                            } else if resolved.is_file() {
                                plan.paths.push(resolved);
                            } else {
                                return None;
                            }
                        }
                    }
                } else if path.exists() && !path.is_file() && !(directories && path.is_dir()) {
                    return None;
                }
                plan.paths.push(cwd.join(file));
                plan.paths.push(path);
            }
            plan.command
                .push_str("'/usr/bin/env' '-i' 'PATH=/usr/bin:/bin' ");
            plan.command.push_str(&quote(&program));
            for arg in args {
                plan.command.push(' ');
                plan.command.push_str(&quote(arg));
            }
            if segment.stderr_null {
                plan.command.push_str(" 2>/dev/null");
            }
            if !segment.separator.is_empty() {
                plan.command.push(' ');
                plan.command.push_str(segment.separator);
                plan.command.push(' ');
            }
            piped_input = segment.separator == "|";
        }
        Some(plan)
    }
}
