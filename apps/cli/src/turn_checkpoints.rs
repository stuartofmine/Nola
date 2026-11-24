// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Zero-pollution Git shadow-tree turn checkpoints (`/undo`, `/redo`, `/checkpoints`)
//            and instant workspace diff view (`/diff`) for interactive CLI sessions.
// - Boundary: Uses an ephemeral `GIT_INDEX_FILE` so the user's real Git staging area,
//             branches, and commit history are never modified.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_CHECKPOINTS: usize = 32;
const MAX_DIFF_DISPLAY_CHARS: usize = 12_000;

#[derive(Debug, Clone)]
pub(crate) struct TurnCheckpoint {
    pub turn: usize,
    pub label: String,
    pub tree_sha: String,
    pub repo_root: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct RestoreSummary {
    pub checkpoint_turn: usize,
    pub checkpoint_label: String,
    pub restored_files: Vec<String>,
    pub removed_files: Vec<String>,
    pub remaining_undo: usize,
    pub remaining_redo: usize,
}

#[derive(Debug, Default)]
pub(crate) struct TurnCheckpointStore {
    undo_stack: Vec<TurnCheckpoint>,
    redo_stack: Vec<TurnCheckpoint>,
}

impl TurnCheckpointStore {
    pub(crate) fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    pub(crate) fn checkpoints(&self) -> &[TurnCheckpoint] {
        &self.undo_stack
    }

    /// Captures a shadow-index Git tree snapshot of the workspace before a turn.
    /// Silently returns `None` if `project_root` is not inside a Git repository.
    pub(crate) fn capture(&mut self, project_root: &Path, turn: usize, prompt: &str) -> Option<&TurnCheckpoint> {
        let repo_root = git_repo_root(project_root)?;
        let tree_sha = capture_shadow_tree(&repo_root).ok()?;
        let label = compact_prompt_label(prompt);
        self.redo_stack.clear();
        self.undo_stack.push(TurnCheckpoint {
            turn,
            label,
            tree_sha,
            repo_root,
        });
        if self.undo_stack.len() > MAX_CHECKPOINTS {
            self.undo_stack.remove(0);
        }
        self.undo_stack.last()
    }

    /// Restores the workspace to the latest captured turn checkpoint, saving the
    /// current state onto the redo stack so `/redo` can invert it losslessly.
    pub(crate) fn undo(&mut self, project_root: &Path) -> Result<RestoreSummary, String> {
        let Some(target) = self.undo_stack.pop() else {
            return Err("No turn checkpoint available to undo in this session.".to_string());
        };
        let repo_root = git_repo_root(project_root).unwrap_or_else(|| target.repo_root.clone());
        let current_tree = capture_shadow_tree(&repo_root)?;
        let (restored_files, removed_files) =
            restore_tree_diff(&repo_root, &current_tree, &target.tree_sha)?;
        self.redo_stack.push(TurnCheckpoint {
            turn: target.turn,
            label: target.label.clone(),
            tree_sha: current_tree,
            repo_root,
        });
        Ok(RestoreSummary {
            checkpoint_turn: target.turn,
            checkpoint_label: target.label,
            restored_files,
            removed_files,
            remaining_undo: self.undo_stack.len(),
            remaining_redo: self.redo_stack.len(),
        })
    }

    /// Re-applies the most recently undone checkpoint state.
    pub(crate) fn redo(&mut self, project_root: &Path) -> Result<RestoreSummary, String> {
        let Some(target) = self.redo_stack.pop() else {
            return Err("Nothing to redo.".to_string());
        };
        let repo_root = git_repo_root(project_root).unwrap_or_else(|| target.repo_root.clone());
        let current_tree = capture_shadow_tree(&repo_root)?;
        let (restored_files, removed_files) =
            restore_tree_diff(&repo_root, &current_tree, &target.tree_sha)?;
        self.undo_stack.push(TurnCheckpoint {
            turn: target.turn,
            label: target.label.clone(),
            tree_sha: current_tree,
            repo_root,
        });
        Ok(RestoreSummary {
            checkpoint_turn: target.turn,
            checkpoint_label: target.label,
            restored_files,
            removed_files,
            remaining_undo: self.undo_stack.len(),
            remaining_redo: self.redo_stack.len(),
        })
    }
}

pub(crate) fn format_workspace_diff(project_root: &Path, raw_arg: &str) -> Result<String, String> {
    let repo_root = git_repo_root(project_root)
        .ok_or_else(|| format!("Not a Git repository: {}", project_root.display()))?;
    let arg = raw_arg.trim();
    let stat_only = arg == "--stat" || arg == "-s";
    let path_filter = (!arg.is_empty() && !stat_only).then_some(arg);

    let mut sections = Vec::new();

    // 1. Short status summary (tracked + all individual untracked files)
    let mut status_cmd = Command::new("git");
    status_cmd
        .arg("-C")
        .arg(&repo_root)
        .args(["status", "--short", "-uall"]);
    if let Some(p) = path_filter {
        status_cmd.arg("--").arg(p);
    }
    let status_out = status_cmd
        .output()
        .map_err(|err| format!("git status failed: {err}"))?;
    let status_text = String::from_utf8_lossy(&status_out.stdout).trim_end().to_string();
    if status_text.is_empty() {
        return Ok("Working tree is clean (no uncommitted changes).".to_string());
    }
    sections.push(format!("Status:\n{status_text}"));

    // 2. Diff stat against HEAD (or index if no commits yet)
    let has_head = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .is_ok_and(|o| o.status.success());

    let mut diff_cmd = Command::new("git");
    diff_cmd.arg("-C").arg(&repo_root).arg("diff");
    if has_head {
        diff_cmd.arg("HEAD");
    }
    if stat_only {
        diff_cmd.arg("--stat");
    } else {
        diff_cmd.args(["--color=always", "--stat", "-p"]);
    }
    if let Some(p) = path_filter {
        diff_cmd.arg("--").arg(p);
    }
    let diff_out = diff_cmd
        .output()
        .map_err(|err| format!("git diff failed: {err}"))?;
    let diff_text = String::from_utf8_lossy(&diff_out.stdout).trim_end().to_string();
    if !diff_text.is_empty() {
        let bounded = if diff_text.chars().count() > MAX_DIFF_DISPLAY_CHARS {
            let mut truncated: String = diff_text.chars().take(MAX_DIFF_DISPLAY_CHARS).collect();
            truncated.push_str("\n\x1b[0m...[diff truncated; use `/diff --stat` or `/diff <path>` for focused output]");
            truncated
        } else {
            diff_text
        };
        sections.push(bounded);
    }

    Ok(sections.join("\n\n"))
}

fn compact_prompt_label(prompt: &str) -> String {
    let single_line = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = single_line.chars();
    let preview: String = chars.by_ref().take(60).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else if preview.is_empty() {
        "(empty prompt)".to_string()
    } else {
        preview
    }
}

fn git_repo_root(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

fn temp_shadow_index_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "nola-shadow-index-{}-{}.idx",
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

/// Captures the full working tree (tracked + non-ignored untracked files) as a
/// Git tree object using a disposable shadow index file.
fn capture_shadow_tree(repo_root: &Path) -> Result<String, String> {
    let shadow_index = temp_shadow_index_path();
    let result = (|| -> Result<String, String> {
        let add = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .env("GIT_INDEX_FILE", &shadow_index)
            .args(["add", "-A"])
            .output()
            .map_err(|err| format!("git shadow add failed: {err}"))?;
        if !add.status.success() {
            return Err(format!(
                "git shadow add failed: {}",
                String::from_utf8_lossy(&add.stderr).trim()
            ));
        }
        let write = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .env("GIT_INDEX_FILE", &shadow_index)
            .arg("write-tree")
            .output()
            .map_err(|err| format!("git write-tree failed: {err}"))?;
        if !write.status.success() {
            return Err(format!(
                "git write-tree failed: {}",
                String::from_utf8_lossy(&write.stderr).trim()
            ));
        }
        let sha = String::from_utf8_lossy(&write.stdout).trim().to_string();
        if sha.len() < 7 {
            return Err("git write-tree returned invalid tree SHA".to_string());
        }
        Ok(sha)
    })();
    let _ = fs::remove_file(&shadow_index);
    result
}

/// Applies the exact file diff from `current_tree` -> `target_tree` onto the
/// working tree using a disposable shadow index so the user's real Git index is
/// untouched. Returns `(restored_files, removed_files)`.
fn restore_tree_diff(
    repo_root: &Path,
    current_tree: &str,
    target_tree: &str,
) -> Result<(Vec<String>, Vec<String>), String> {
    if current_tree == target_tree {
        return Ok((Vec::new(), Vec::new()));
    }
    let diff = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args([
            "diff-tree",
            "-r",
            "--name-status",
            "--no-renames",
            "-z",
            current_tree,
            target_tree,
        ])
        .output()
        .map_err(|err| format!("git diff-tree failed: {err}"))?;
    if !diff.status.success() {
        return Err(format!(
            "git diff-tree failed: {}",
            String::from_utf8_lossy(&diff.stderr).trim()
        ));
    }

    let mut to_restore = Vec::new();
    let mut to_remove = Vec::new();
    let mut parts = diff.stdout.split(|b| *b == 0);
    while let Some(status_bytes) = parts.next() {
        if status_bytes.is_empty() {
            break;
        }
        let Some(path_bytes) = parts.next() else {
            break;
        };
        let status = String::from_utf8_lossy(status_bytes);
        let rel_path = String::from_utf8_lossy(path_bytes).to_string();
        if status.starts_with('D') {
            // Present in current_tree, absent in target_tree -> delete file.
            to_remove.push(rel_path);
        } else {
            // Added or modified in target_tree relative to current_tree -> restore file.
            to_restore.push(rel_path);
        }
    }

    if !to_restore.is_empty() {
        let shadow_index = temp_shadow_index_path();
        let checkout_res = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .env("GIT_INDEX_FILE", &shadow_index)
            .arg("checkout")
            .arg(target_tree)
            .arg("--")
            .args(&to_restore)
            .output();
        let _ = fs::remove_file(&shadow_index);
        let checkout = checkout_res.map_err(|err| format!("git checkout failed: {err}"))?;
        if !checkout.status.success() {
            return Err(format!(
                "git checkout failed: {}",
                String::from_utf8_lossy(&checkout.stderr).trim()
            ));
        }
    }

    for rel in &to_remove {
        let full = repo_root.join(rel);
        let _ = fs::remove_file(&full);
        // Clean empty parent directories created only for removed files (stop at repo_root).
        let mut parent = full.parent();
        while let Some(dir) = parent {
            if dir == repo_root {
                break;
            }
            if fs::remove_dir(dir).is_err() {
                break;
            }
            parent = dir.parent();
        }
    }

    Ok((to_restore, to_remove))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_temp_repo(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nola-checkpoint-test-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.name", "Nola Test"]);
        run(&["config", "user.email", "test@nola.local"]);
        run(&["config", "commit.gpgsign", "false"]);
        fs::write(dir.join("main.py"), "print('v1')\n").unwrap();
        fs::write(dir.join("keep.txt"), "keep\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        dir
    }

    #[test]
    fn undo_and_redo_restore_edits_deletions_and_new_files_without_touching_git_index() {
        let repo = init_temp_repo("undo-redo");
        // Stage a user change in the real Git index before checkpointing to verify
        // shadow-index checkpoints never clobber the user's real staged area.
        fs::write(repo.join("staged.txt"), "user staged\n").unwrap();
        assert!(Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "staged.txt"])
            .status()
            .unwrap()
            .success());

        let mut store = TurnCheckpointStore::default();
        store
            .capture(&repo, 1, "refactor main.py and add helper")
            .expect("should capture checkpoint");

        // Simulate agent turn: edit main.py, delete keep.txt, create sub/helper.py
        fs::write(repo.join("main.py"), "print('v2')\n").unwrap();
        fs::remove_file(repo.join("keep.txt")).unwrap();
        fs::create_dir_all(repo.join("sub")).unwrap();
        fs::write(repo.join("sub/helper.py"), "def hi(): return 1\n").unwrap();

        // /diff sees changes
        let diff_out = format_workspace_diff(&repo, "--stat").unwrap();
        assert!(diff_out.contains("main.py"));
        assert!(diff_out.contains("sub/helper.py"));

        // /undo restores main.py + keep.txt and removes sub/helper.py
        let undone = store.undo(&repo).unwrap();
        assert_eq!(undone.checkpoint_turn, 1);
        assert!(undone.restored_files.contains(&"main.py".to_string()));
        assert!(undone.restored_files.contains(&"keep.txt".to_string()));
        assert!(undone.removed_files.contains(&"sub/helper.py".to_string()));
        assert_eq!(fs::read_to_string(repo.join("main.py")).unwrap(), "print('v1')\n");
        assert_eq!(fs::read_to_string(repo.join("keep.txt")).unwrap(), "keep\n");
        assert!(!repo.join("sub/helper.py").exists());

        // Real git index still has staged.txt staged!
        let cached = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["diff", "--cached", "--name-only"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&cached.stdout).contains("staged.txt"));

        // /redo brings agent changes back losslessly
        let redone = store.redo(&repo).unwrap();
        assert!(redone.restored_files.contains(&"main.py".to_string()));
        assert!(redone.restored_files.contains(&"sub/helper.py".to_string()));
        assert!(redone.removed_files.contains(&"keep.txt".to_string()));
        assert_eq!(fs::read_to_string(repo.join("main.py")).unwrap(), "print('v2')\n");
        assert!(!repo.join("keep.txt").exists());
        assert_eq!(
            fs::read_to_string(repo.join("sub/helper.py")).unwrap(),
            "def hi(): return 1\n"
        );

        let _ = fs::remove_dir_all(repo);
    }
}
