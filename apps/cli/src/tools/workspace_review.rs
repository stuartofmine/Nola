// SPDX-License-Identifier: Apache-2.0
//! Explicit local Host workspace inspection. No mutation, external diff, or network.
use std::path::Path;
use serde_json::{json, Value};

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
    }
}
