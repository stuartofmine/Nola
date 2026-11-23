// SPDX-License-Identifier: Apache-2.0
//! Explicit local Host workspace inspection. No mutation, external diff, or network.
use std::path::Path;
use std::time::UNIX_EPOCH;
use serde_json::{json, Value};

const NON_GIT_MAX_DEPTH: usize = 3;
const NON_GIT_MAX_ENTRIES: usize = 200;
const NON_GIT_SKIP_NAMES: &[&str] = &[".git", "node_modules", "target", ".DS_Store"];

#[cfg(unix)]
pub(crate) fn is_git_repository(root: &Path) -> bool {
    let Ok(observed) = root.canonicalize() else { return false; };
    if !observed.is_dir() { return false; }
    let Ok(repository) = read_git(&observed, &["rev-parse", "--show-toplevel"], 16 * 1024) else {
        return false;
    };
    let Ok(repository) = std::str::from_utf8(&repository) else { return false; };
    let Ok(repository) = Path::new(repository.trim_end_matches(['\r', '\n'])).canonicalize() else {
        return false;
    };
    observed.starts_with(&repository)
}

#[cfg(not(unix))]
pub(crate) fn is_git_repository(root: &Path) -> bool {
    let Ok(observed) = root.canonicalize() else { return false; };
    if !observed.is_dir() { return false; }
    observed.ancestors().any(|ancestor| ancestor.join(".git").exists())
}

pub(crate) fn list_non_git_workspace_files(root: &Path) -> Result<(Vec<Value>, bool), String> {
    let observed = root.canonicalize().map_err(|e| e.to_string())?;
    if !observed.is_dir() {
        return Err("Workspace is not a directory".into());
    }
    let mut entries = Vec::new();
    let mut truncated = false;
    scan_non_git_dir(&observed, &observed, 1, &mut entries, &mut truncated)?;
    Ok((entries, truncated))
}

fn scan_non_git_dir(
    root: &Path,
    current: &Path,
    depth: usize,
    out: &mut Vec<Value>,
    truncated: &mut bool,
) -> Result<(), String> {
    if depth > NON_GIT_MAX_DEPTH || *truncated {
        return Ok(());
    }
    let mut children = std::fs::read_dir(current)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if NON_GIT_SKIP_NAMES.contains(&name_str.as_ref()) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if out.len() >= NON_GIT_MAX_ENTRIES {
            *truncated = true;
            return Ok(());
        }
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative_str = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let is_dir = file_type.is_dir();
        let metadata = std::fs::symlink_metadata(&path).ok();
        let size = if is_dir {
            0
        } else {
            metadata.as_ref().map_or(0, |m| m.len())
        };
        let modified_at_ms = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);
        out.push(json!({
            "path": relative_str,
            "size": size,
            "is_dir": is_dir,
            "modified_at_ms": modified_at_ms,
        }));
        if is_dir && depth < NON_GIT_MAX_DEPTH {
            scan_non_git_dir(root, &path, depth + 1, out, truncated)?;
            if *truncated {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn read_git(root: &Path, args: &[&str], maximum: usize) -> Result<Vec<u8>, String> {
    let mut command = super::git_operation_command(root);
    command.args(["-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null"])
        .env("GIT_OPTIONAL_LOCKS", "0").env("GIT_TERMINAL_PROMPT", "0").env("GIT_PAGER", "cat").args(args);
    let (_, output) = crate::task_runtime::bounded_process_output(command, std::time::Duration::from_secs(10), maximum)?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); }
    Ok(output.stdout)
}

#[cfg(unix)]
pub(crate) fn read_host_workspace_review(root: &Path) -> Result<Value, String> {
    let observed = root.canonicalize().map_err(|e| e.to_string())?;
    if !observed.is_dir() { return Err("Workspace is not a directory".into()); }
    let repository = read_git(&observed, &["rev-parse", "--show-toplevel"], 16 * 1024)?;
    let repository = std::str::from_utf8(&repository).map_err(|_| "Non-UTF8 repository path")?.trim_end_matches(['\r', '\n']);
    let repository = Path::new(repository).canonicalize().map_err(|e| e.to_string())?;
    if !observed.starts_with(&repository) { return Err("Workspace is outside the returned repository".into()); }
    let scope = observed.to_str().ok_or("Non-UTF8 workspace")?;
    let status = read_git(&repository, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--no-renames", "--", scope], 1024 * 1024)?;
    let inventory = super::git_inventory_from_status(&status, &repository, &observed)?;
    let changes = read_git(&repository, &["diff", "--no-ext-diff", "--no-textconv", "--", scope], 2 * 1024 * 1024);
    let staged = read_git(&repository, &["diff", "--cached", "--no-ext-diff", "--no-textconv", "--", scope], 2 * 1024 * 1024);
    let text = |result: Result<Vec<u8>, String>| match result {
        Ok(bytes) => json!({"text":String::from_utf8_lossy(&bytes),"error":Value::Null}),
        Err(error) => json!({"text":Value::Null,"error":error}),
    };
    Ok(json!({"inventory":inventory,"unstaged":text(changes),"staged":text(staged)}))
}

#[cfg(not(unix))]
pub(crate) fn read_host_workspace_review(_root: &Path) -> Result<Value, String> {
    Err("This Host does not provide a workspace Git reader".into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git").current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
    #[test]
    fn actual_workspace_reader_keeps_staged_and_unstaged_separate_and_never_invokes_external_diff() {
        let directory = tempfile::tempdir().unwrap(); let root = directory.path();
        git(root, &["init", "--quiet"]);
        std::fs::write(root.join("note.txt"), "first\n").unwrap(); git(root, &["add", "note.txt"]);
        std::fs::write(root.join("note.txt"), "second\n").unwrap();
        let marker = root.join("external-diff-ran");
        git(root, &["config", "diff.external", &format!("touch '{}'", marker.display())]);
        let review = read_host_workspace_review(root).unwrap();
        assert_eq!(review["inventory"]["entries"][0]["path"], "note.txt");
        assert!(review["staged"]["text"].as_str().unwrap().contains("+first"));
        assert!(review["unstaged"]["text"].as_str().unwrap().contains("+second"));
        assert!(!marker.exists());
        assert_eq!(std::fs::read_to_string(root.join("note.txt")).unwrap(), "second\n");
        assert!(is_git_repository(root));
    }

    #[test]
    fn non_git_workspace_listing_respects_depth_skips_ignored_and_symlinks_and_truncates() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        assert!(!is_git_repository(root));
        std::fs::write(root.join("hello.txt"), "abc").unwrap();
        std::fs::write(root.join(".DS_Store"), "ignored").unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "ignored").unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("target/debug/bin"), "ignored").unwrap();
        std::fs::create_dir_all(root.join("a/b/c")).unwrap();
        std::fs::write(root.join("a/b/file_at_depth3.txt"), "ok").unwrap();
        std::fs::write(root.join("a/b/c/too_deep.txt"), "no").unwrap();
        std::os::unix::fs::symlink(root.join("hello.txt"), root.join("link.txt")).unwrap();

        let (files, truncated) = list_non_git_workspace_files(root).unwrap();
        assert!(!truncated);
        let paths: Vec<&str> = files.iter().filter_map(|v| v["path"].as_str()).collect();
        assert!(paths.contains(&"hello.txt"));
        assert!(paths.contains(&"a"));
        assert!(paths.contains(&"a/b"));
        assert!(paths.contains(&"a/b/file_at_depth3.txt"));
        assert!(!paths.iter().any(|p| p.contains("too_deep.txt")));
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
        assert!(!paths.iter().any(|p| p.contains("target")));
        assert!(!paths.iter().any(|p| p.contains(".DS_Store")));
        assert!(!paths.iter().any(|p| p.contains("link.txt")));
        let hello = files.iter().find(|v| v["path"] == "hello.txt").unwrap();
        assert_eq!(hello["size"], 3);
        assert_eq!(hello["is_dir"], false);
        assert!(hello["modified_at_ms"].as_u64().is_some());

        // Truncation at 200 entries
        let many = tempfile::tempdir().unwrap();
        for i in 0..215 {
            std::fs::write(many.path().join(format!("f_{i:03}.txt")), "x").unwrap();
        }
        let (many_files, many_truncated) = list_non_git_workspace_files(many.path()).unwrap();
        assert_eq!(many_files.len(), NON_GIT_MAX_ENTRIES);
        assert!(many_truncated);
    }
}
