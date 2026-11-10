// SPDX-License-Identifier: Apache-2.0
//! Host-owned Git inspection argv. No aliases, shell programs, hooks or network.
use std::path::{Path, PathBuf};

pub(super) struct GitReadPlan {
    pub root: PathBuf,
    pub command: String,
}

impl GitReadPlan {
    pub fn from_command(command: &str, cwd: &Path) -> Option<Self> {
        // Parse only literal pipelines, never arbitrary shell control flow.
        let mut parts = Vec::new();
        let (mut quote, mut escaped, mut start) = (None, false, 0);
        for (index, ch) in command.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && quote != Some('\'') {
                escaped = true;
                continue;
            }
            if let Some(active) = quote {
                if ch == active {
                    quote = None;
                }
            } else if matches!(ch, '\'' | '"') {
                quote = Some(ch);
            } else if ch == '|' {
                parts.push(&command[start..index]);
                start = index + 1;
            }
        }
        if escaped || quote.is_some() {
            return None;
        }
        parts.push(&command[start..]);
        let tokens = nola_core::agent::permissions::supported_shell_tokens(parts.first()?)?;
        let mut plan = Self::parse(&tokens, cwd)?;
        for part in &parts[1..] {
            let tokens = nola_core::agent::permissions::supported_shell_tokens(part)?;
            let name = tokens.first()?.as_str();
            let args = &tokens[1..];
            let allowed = match name {
                "sort" => args
                    .iter()
                    .all(|arg| matches!(arg.as_str(), "-r" | "-n" | "-rn" | "-nr" | "-u")),
                "uniq" => args
                    .iter()
                    .all(|arg| matches!(arg.as_str(), "-c" | "-d" | "-u")),
                "wc" => args.len() == 1 && matches!(args[0].as_str(), "-l" | "-c" | "-w"),
                "head" | "tail" => {
                    (args.len() == 1
                        && args[0]
                            .strip_prefix('-')
                            .and_then(|v| v.parse::<u32>().ok())
                            .is_some_and(|v| v <= 10000))
                        || (args.len() == 2
                            && args[0] == "-n"
                            && args[1].parse::<u32>().is_ok_and(|v| v <= 10000))
                }
                "cut" => {
                    args.len() == 1
                        && args[0].strip_prefix("-c").is_some_and(|v| {
                            !v.is_empty()
                                && v.chars()
                                    .all(|ch| ch.is_ascii_digit() || matches!(ch, '-' | ','))
                        })
                }
                "grep" => {
                    let mut args = args;
                    while args.first().is_some_and(|v| {
                        matches!(v.as_str(), "-E" | "-v" | "-c" | "-vE" | "-Ev" | "-F")
                    }) {
                        args = &args[1..];
                    }
                    args.len() == 1 && !args[0].starts_with('-')
                }
                // No sed/awk scripts: their execution and write primitives need
                // a real script contract, not a blacklist of suspicious words.
                _ => false,
            };
            if !allowed {
                return None;
            }
            let program = crate::platform_paths::system_utility(name)?;
            plan.command.push_str(" | ");
            plan.command.push_str(&format!(
                "'{}' '-i' 'PATH=/usr/bin:/bin' '{}'",
                crate::platform_paths::system_utility("env")?, program
            ));
            for arg in args {
                plan.command
                    .push_str(&format!(" '{}'", arg.replace('\'', "'\\''")));
            }
        }
        Some(plan)
    }

    pub fn parse(tokens: &[String], cwd: &Path) -> Option<Self> {
        if !matches!(tokens.first()?.as_str(), "git" | "/usr/bin/git")
            || !Path::new(crate::platform_paths::git_program()).is_file()
        {
            return None;
        }
        let mut args = &tokens[1..];
        let mut root = cwd.to_path_buf();
        if args.first().is_some_and(|arg| arg == "-C") {
            root = cwd.join(args.get(1)?);
            args = &args[2..];
        }
        let operation = args.first()?.as_str();
        // Limit automatic admission to metadata inspection. Patches, revisions,
        // arbitrary pathspecs, user config and executable format options require
        // the structured tools or ordinary explicit process authorization.
        let allowed: &[&str] = match operation {
            "status" => &[
                "--short",
                "-s",
                "--branch",
                "-b",
                "--porcelain",
                "--porcelain=v1",
                "--porcelain=v2",
                "--untracked-files=all",
                "--untracked-files=normal",
                "--untracked-files=no",
            ],
            "log" => &[
                "--oneline",
                "--decorate",
                "--no-decorate",
                "--graph",
                "--all",
            ],
            "diff" => &[
                "--stat",
                "--shortstat",
                "--numstat",
                "--name-only",
                "--name-status",
                "--cached",
                "--staged",
            ],
            _ => return None,
        };
        for arg in &args[1..] {
            let count = operation == "log"
                && arg
                    .strip_prefix('-')
                    .and_then(|value| value.parse::<u32>().ok())
                    .is_some_and(|count| (1..=1000).contains(&count));
            if !allowed.contains(&arg.as_str()) && !count {
                return None;
            }
        }
        if operation == "diff"
            && !args.iter().any(|arg| {
                matches!(
                    arg.as_str(),
                    "--stat" | "--shortstat" | "--numstat" | "--name-only" | "--name-status"
                )
            })
        {
            return None;
        }
        root = root.canonicalize().ok()?;
        // Do not silently follow worktree pointers or alternate metadata roots.
        let git_dir = root.join(".git");
        if !git_dir.is_dir() || git_dir.symlink_metadata().ok()?.file_type().is_symlink() {
            return None;
        }
        if git_dir.join("commondir").exists() || git_dir.join("objects/info/alternates").exists() {
            return None;
        }
        let mut argv = vec![
            crate::platform_paths::system_utility("env")?,
            "-i".into(),
            "PATH=/usr/bin:/bin".into(),
            "GIT_CONFIG_NOSYSTEM=1".into(),
            "GIT_CONFIG_GLOBAL=/dev/null".into(),
            "GIT_OPTIONAL_LOCKS=0".into(),
            "GIT_NO_LAZY_FETCH=1".into(),
            "GIT_TERMINAL_PROMPT=0".into(),
            crate::platform_paths::git_program().into(),
            "--no-pager".into(),
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "core.hooksPath=/dev/null".into(),
            "-c".into(),
            "core.untrackedCache=false".into(),
            "-c".into(),
            "protocol.allow=never".into(),
            "-C".into(),
            crate::platform_paths::shell_path(&root),
            operation.into(),
        ];
        match operation {
            "diff" => argv.extend(["--no-ext-diff".into(), "--no-textconv".into()]),
            "log" => argv.extend(["--no-show-signature".into(), "--max-count=1000".into()]),
            _ => {}
        }
        argv.extend_from_slice(&args[1..]);
        let command = argv
            .iter()
            .map(|arg| format!("'{}'", arg.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" ");
        Some(Self { root, command })
    }

    /// Call only after the repository has passed the workspace resource check.
    pub fn has_passive_configuration(&self) -> bool {
        // Even status/diff can run clean/process filters. Keep their semantics
        // intact and require explicit approval instead of disabling the filters.
        std::process::Command::new(crate::platform_paths::git_program())
            .env_clear()
            .env("SystemRoot", std::env::var_os("SystemRoot").unwrap_or_default())
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["--no-pager", "-C"])
            .arg(&self.root)
            .args([
                "config",
                "--includes",
                "--get-regexp",
                "^filter\\..*\\.(clean|process)$",
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|output| output.status.code() == Some(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(command: &str, root: &Path) -> Option<GitReadPlan> {
        GitReadPlan::parse(
            &nola_core::agent::permissions::supported_shell_tokens(command)?,
            root,
        )
        .filter(GitReadPlan::has_passive_configuration)
    }
    #[test]
    fn inspection_is_not_authority_for_mutations_or_executables() {
        let dir = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new(crate::platform_paths::git_program())
            .args(["init", "--quiet"])
            .arg(dir.path())
            .status()
            .unwrap()
            .success());
        for command in [
            "git status --porcelain=v1",
            "git log --oneline -15",
            "git diff --stat",
            "git diff --cached --name-only",
        ] {
            let plan = parse(command, dir.path()).unwrap();
            assert!(plan.command.contains("core.fsmonitor=false"));
            assert!(plan.command.contains("protocol.allow=never"));
        }
        for command in [
            "git commit -m x",
            "git add .",
            "git push",
            "git reset --hard",
            "git -c core.pager=evil log",
            "git diff --output=x",
            "git log --show-signature",
            "git log --format=%G?",
            "git status; touch x",
            "git status | sh",
            "git diff",
            "git log -999999",
        ] {
            assert!(parse(command, dir.path()).is_none(), "{command}");
        }
        for command in [
            "git status --porcelain=v1 | cut -c1-2 | sort | uniq -c",
            "git diff --stat | tail -5",
            "git status --porcelain=v1 | grep -E '^ M' | head -n 20",
        ] {
            assert!(
                GitReadPlan::from_command(command, dir.path()).is_some(),
                "{command}"
            );
        }
        for command in [
            "git status || sh",
            "git status | sort -o stolen",
            "git status | head /etc/passwd",
            "git status | grep -f /etc/passwd",
            "git status | awk '{system(\"evil\")}'",
            "git status | sed 's/x/y/e'",
            "git status | cat $(whoami)",
            "git status | wc -l; touch x",
        ] {
            assert!(
                GitReadPlan::from_command(command, dir.path()).is_none(),
                "{command}"
            );
        }
    }
    #[test]
    fn real_repository_inspection_does_not_execute_fsmonitor_or_diff_helpers() {
        let dir = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new(crate::platform_paths::git_program())
            .args(["init", "--quiet"])
            .arg(dir.path())
            .status()
            .unwrap()
            .success());
        for (key, value) in [
            ("core.fsmonitor", "touch SHOULD_NOT_EXIST"),
            ("diff.external", "touch SHOULD_NOT_EXIST"),
        ] {
            assert!(std::process::Command::new(crate::platform_paths::git_program())
                .arg("-C")
                .arg(dir.path())
                .args(["config", key, value])
                .status()
                .unwrap()
                .success());
        }
        for command in ["git status --porcelain=v1", "git diff --stat"] {
            let plan = parse(command, dir.path()).unwrap();
            let result = std::process::Command::new(crate::platform_paths::bash_program())
                .args(["-c", &plan.command])
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        assert!(!dir.path().join("SHOULD_NOT_EXIST").exists());
        assert!(std::process::Command::new(crate::platform_paths::git_program())
            .arg("-C")
            .arg(dir.path())
            .args(["config", "filter.custom.clean", "touch SHOULD_NOT_EXIST"])
            .status()
            .unwrap()
            .success());
        assert!(parse("git status", dir.path()).is_none());
        assert!(!dir.path().join("SHOULD_NOT_EXIST").exists());
    }
}
