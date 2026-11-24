// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Zero-config, fast post-edit syntax & fatal-error diagnostics attached to file edits.
// - Boundary: Read-only advisory feedback; never modifies workspace files or fails a committed edit.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_DIAGNOSTIC_CHARS: usize = 1500;
const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(2);

/// Returns a formatted `## Diagnostics (auto-check)` markdown section if the
/// edited file has syntax or fatal static errors, or `None` when clean (0 extra
/// tokens on clean edits).
pub(crate) fn diagnostics_section_for_file(path: &Path) -> Option<String> {
    let raw = diagnose_file(path)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bounded = if trimmed.chars().count() > MAX_DIAGNOSTIC_CHARS {
        let mut s: String = trimmed.chars().take(MAX_DIAGNOSTIC_CHARS).collect();
        s.push_str("\n...[diagnostics truncated]");
        s
    } else {
        trimmed.to_string()
    };
    Some(format!("\n\n## Diagnostics (auto-check)\n{bounded}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Python,
    JavaScript,
    Go,
    Shell,
    Json,
    Yaml,
    Toml,
}

fn detect_file_kind(path: &Path) -> Option<FileKind> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("py" | "pyi") => return Some(FileKind::Python),
        Some("js" | "mjs" | "cjs") => return Some(FileKind::JavaScript),
        Some("go") => return Some(FileKind::Go),
        Some("sh" | "bash") => return Some(FileKind::Shell),
        Some("json") => return Some(FileKind::Json),
        Some("yaml" | "yml") => return Some(FileKind::Yaml),
        Some("toml") => return Some(FileKind::Toml),
        Some(_) => return None,
        None => {}
    }
    // Support extensionless executables (e.g. `/workspace/dispatch`) via shebang.
    let mut file = fs::File::open(path).ok()?;
    let mut header = [0u8; 160];
    let n = file.read(&mut header).ok()?;
    let first_line = std::str::from_utf8(&header[..n])
        .ok()?
        .lines()
        .next()?
        .trim();
    if !first_line.starts_with("#!") {
        return None;
    }
    let lower = first_line.to_ascii_lowercase();
    if lower.contains("python") {
        Some(FileKind::Python)
    } else if lower.contains("bash") || lower.ends_with("/sh") || lower.ends_with(" sh") {
        Some(FileKind::Shell)
    } else if lower.contains("node") {
        Some(FileKind::JavaScript)
    } else {
        None
    }
}

fn diagnose_file(path: &Path) -> Option<String> {
    if let Some(kind) = detect_file_kind(path) {
        let fast = match kind {
            FileKind::Json => {
                let content = fs::read_to_string(path).ok()?;
                serde_json::from_str::<serde_json::Value>(&content)
                    .err()
                    .map(|err| format!("JSON syntax error: {err}"))
            }
            FileKind::Yaml => {
                let path_str = path.to_str()?;
                let yaml_script = "import sys\ntry:\n    import yaml\nexcept ImportError:\n    sys.exit(0)\ntry:\n    yaml.safe_load(open(sys.argv[1], 'rb'))\nexcept Exception as e:\n    sys.stderr.write(f'YAML syntax error: {e}\\n')\n    sys.exit(1)\n";
                run_command_with_timeout("python3", &["-B", "-c", yaml_script, path_str])
            }
            FileKind::Toml => {
                let content = fs::read_to_string(path).ok()?;
                toml::from_str::<toml::Value>(&content)
                    .err()
                    .map(|err| format!("TOML syntax error: {err}"))
            }
            FileKind::Python => diagnose_python(path),
            FileKind::JavaScript => run_checker_with_timeout("node", &["--check"], path),
            FileKind::Go => run_checker_with_timeout("gofmt", &["-e"], path),
            FileKind::Shell => run_checker_with_timeout("bash", &["-n"], path),
        };
        if fast.is_some() {
            return fast;
        }
    }
    // Phase 2: query persistent, auto-detected LSP server (if installed in PATH)
    // for type/symbol diagnostics on Rust, TypeScript, Python, Go, C/C++.
    super::lsp_diagnostics::query_lsp_diagnostics(path)
}

fn diagnose_python(path: &Path) -> Option<String> {
    let path_str = path.to_str()?;
    // `-B` avoids writing `__pycache__` into the workspace during advisory check.
    let ast_script = "import ast, sys, traceback\ntry:\n    src = open(sys.argv[1], 'rb').read()\n    ast.parse(src, filename=sys.argv[1])\nexcept SyntaxError as e:\n    sys.stderr.write(''.join(traceback.format_exception_only(type(e), e)))\n    sys.exit(1)\n";
    if let Some(syntax_err) = run_command_with_timeout(
        "python3",
        &["-B", "-c", ast_script, path_str],
    ) {
        return Some(syntax_err);
    }
    // If syntax is valid and `ruff` is available, surface only fatal errors (undefined
    // names, syntax/parse bugs — no stylistic noise).
    run_command_with_timeout(
        "ruff",
        &[
            "check",
            "--no-cache",
            "--quiet",
            "--output-format",
            "concise",
            "--select",
            "E9,F63,F7,F82",
            path_str,
        ],
    )
}

fn run_checker_with_timeout(program: &str, args: &[&str], path: &Path) -> Option<String> {
    let path_str = path.to_str()?;
    let mut full_args = Vec::with_capacity(args.len() + 1);
    full_args.extend_from_slice(args);
    full_args.push(path_str);
    run_command_with_timeout(program, &full_args)
}

fn run_command_with_timeout(program: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + DIAGNOSTIC_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return None;
                }
                let mut stderr = String::new();
                let mut stdout = String::new();
                if let Some(mut s) = child.stderr.take() {
                    let _ = s.read_to_string(&mut stderr);
                }
                if let Some(mut s) = child.stdout.take() {
                    let _ = s.read_to_string(&mut stdout);
                }
                let combined = if !stderr.trim().is_empty() {
                    stderr
                } else {
                    stdout
                };
                return (!combined.trim().is_empty()).then_some(combined);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nola-post-edit-diag-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn json_yaml_toml_diagnostics_report_errors_and_stay_silent_when_clean() {
        let dir = temp_dir("structured");
        let valid_json = dir.join("ok.json");
        fs::write(&valid_json, r#"{"a": 1}"#).unwrap();
        assert!(diagnostics_section_for_file(&valid_json).is_none());

        let bad_json = dir.join("bad.json");
        fs::write(&bad_json, r#"{"a": }"#).unwrap();
        let diag = diagnostics_section_for_file(&bad_json).unwrap();
        assert!(diag.contains("## Diagnostics (auto-check)"));
        assert!(diag.contains("JSON syntax error"));

        let bad_toml = dir.join("bad.toml");
        fs::write(&bad_toml, "key = [1, 2\n").unwrap();
        let diag = diagnostics_section_for_file(&bad_toml).unwrap();
        assert!(diag.contains("TOML syntax error"));

        let bad_yaml = dir.join("bad.yaml");
        fs::write(&bad_yaml, "a: [1, 2\n").unwrap();
        if let Some(diag) = diagnostics_section_for_file(&bad_yaml) {
            assert!(diag.contains("YAML syntax error"));
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn python_and_shebang_diagnostics_catch_syntax_errors_without_pycache() {
        let dir = temp_dir("python");
        let clean_py = dir.join("clean.py");
        fs::write(&clean_py, "def add(a, b):\n    return a + b\n").unwrap();
        assert!(diagnostics_section_for_file(&clean_py).is_none());
        assert!(!dir.join("__pycache__").exists());

        let broken_py = dir.join("broken.py");
        fs::write(&broken_py, "def add(a, b)\n    return a + b\n").unwrap();
        let diag = diagnostics_section_for_file(&broken_py).unwrap();
        assert!(diag.contains("SyntaxError"));

        let shebang_script = dir.join("dispatch");
        fs::write(&shebang_script, "#!/usr/bin/env python3\nif True\n    pass\n").unwrap();
        let diag = diagnostics_section_for_file(&shebang_script).unwrap();
        assert!(diag.contains("SyntaxError"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn shell_diagnostics_catch_syntax_errors() {
        let dir = temp_dir("shell");
        let bad_sh = dir.join("bad.sh");
        fs::write(&bad_sh, "if [ 1 -eq 1 ]; then\n  echo hi\n").unwrap();
        let diag = diagnostics_section_for_file(&bad_sh).unwrap();
        assert!(diag.contains("## Diagnostics (auto-check)"));

        let ok_sh = dir.join("ok.sh");
        fs::write(&ok_sh, "if [ 1 -eq 1 ]; then\n  echo hi\nfi\n").unwrap();
        assert!(diagnostics_section_for_file(&ok_sh).is_none());

        let _ = fs::remove_dir_all(dir);
    }
}
