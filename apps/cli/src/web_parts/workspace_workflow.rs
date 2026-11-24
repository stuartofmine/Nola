// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Web Host developer workflow endpoints (Workspace Diff, Turn Checkpoints
//            Undo/Redo, and Git Worktree listing/creation).

const MAX_WORKSPACE_DIFF_BYTES: usize = 64 * 1024;
const MAX_WORKTREE_BRANCH_LEN: usize = 128;

#[derive(Debug, Default)]
struct WorkspaceCheckpointEntry {
    store: crate::turn_checkpoints::TurnCheckpointStore,
    next_turn: usize,
    redo_count: usize,
}

static WORKSPACE_CHECKPOINTS: OnceLock<Mutex<HashMap<String, WorkspaceCheckpointEntry>>> =
    OnceLock::new();

fn workspace_checkpoints() -> &'static Mutex<HashMap<String, WorkspaceCheckpointEntry>> {
    WORKSPACE_CHECKPOINTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkspaceDiffFile {
    status: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkspaceDiffResponse {
    is_git_repo: bool,
    branch: String,
    files: Vec<WorkspaceDiffFile>,
    diff_text: String,
    truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkspaceCheckpointView {
    turn: usize,
    label: String,
    tree_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkspaceWorktree {
    path: String,
    head: String,
    branch: String,
    is_main: bool,
    is_current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkspaceWorktreesResponse {
    is_git_repo: bool,
    worktrees: Vec<WorkspaceWorktree>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceCreateWorktreeInput {
    branch: String,
    #[serde(default)]
    base_ref: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
}

fn workflow_git_repo_root(path: &Path) -> Option<PathBuf> {
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
    if root.is_empty() {
        return None;
    }
    let path_buf = PathBuf::from(root);
    Some(fs::canonicalize(&path_buf).unwrap_or(path_buf))
}

fn workflow_checkpoint_key(repo_root: &Path) -> String {
    fs::canonicalize(repo_root)
        .unwrap_or_else(|_| repo_root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn resolve_workflow_workspace_root(config: &Config, conversation_id: Option<&str>) -> PathBuf {
    if let Some(conv_id) = conversation_id.map(str::trim).filter(|id| !id.is_empty()) {
        if let Ok(Some((_, settings))) = conversation_configuration_state(conv_id) {
            if let Some(configured) = settings.get("workspaceRoot").and_then(Value::as_str) {
                if let Ok(valid) = validate_configured_workspace_path(config, configured) {
                    return valid;
                }
            }
        }
        let sandbox = session_workspace_dir(config, conv_id);
        if workflow_git_repo_root(&sandbox).is_some() {
            return fs::canonicalize(&sandbox).unwrap_or(sandbox);
        }
    }
    fs::canonicalize(&config.project_root).unwrap_or_else(|_| config.project_root.clone())
}

fn resolve_workflow_git_root_for_turn(
    conversation_workspace: &Path,
    host_project_root: &Path,
) -> Option<PathBuf> {
    workflow_git_repo_root(conversation_workspace)
        .or_else(|| workflow_git_repo_root(host_project_root))
}

fn capture_workspace_turn_checkpoint(workspace_root: &Path, prompt: &str) -> Option<usize> {
    let repo_root = workflow_git_repo_root(workspace_root)?;
    let key = workflow_checkpoint_key(&repo_root);
    let mut guard = workspace_checkpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.entry(key).or_default();
    let turn = entry.next_turn.max(1);
    entry.store.capture(&repo_root, turn, prompt)?;
    entry.next_turn = turn + 1;
    entry.redo_count = 0;
    Some(turn)
}

fn list_workspace_checkpoints(workspace_root: &Path) -> (Vec<WorkspaceCheckpointView>, usize) {
    let Some(repo_root) = workflow_git_repo_root(workspace_root) else {
        return (Vec::new(), 0);
    };
    let key = workflow_checkpoint_key(&repo_root);
    let guard = workspace_checkpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(entry) = guard.get(&key) else {
        return (Vec::new(), 0);
    };
    let checkpoints = entry
        .store
        .checkpoints()
        .iter()
        .map(|cp| WorkspaceCheckpointView {
            turn: cp.turn,
            label: cp.label.clone(),
            tree_sha: cp.tree_sha.clone(),
        })
        .collect();
    (checkpoints, entry.redo_count)
}

fn undo_workspace_checkpoint(
    workspace_root: &Path,
) -> Result<crate::turn_checkpoints::RestoreSummary, String> {
    let repo_root = workflow_git_repo_root(workspace_root)
        .ok_or_else(|| "Workspace is not a Git repository.".to_string())?;
    let key = workflow_checkpoint_key(&repo_root);
    let mut guard = workspace_checkpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.entry(key).or_default();
    let summary = entry.store.undo(&repo_root)?;
    entry.redo_count = summary.remaining_redo;
    Ok(summary)
}

fn redo_workspace_checkpoint(
    workspace_root: &Path,
) -> Result<crate::turn_checkpoints::RestoreSummary, String> {
    let repo_root = workflow_git_repo_root(workspace_root)
        .ok_or_else(|| "Workspace is not a Git repository.".to_string())?;
    let key = workflow_checkpoint_key(&repo_root);
    let mut guard = workspace_checkpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.entry(key).or_default();
    let summary = entry.store.redo(&repo_root)?;
    entry.redo_count = summary.remaining_redo;
    Ok(summary)
}

fn parse_git_status_porcelain_z(stdout: &[u8]) -> Vec<WorkspaceDiffFile> {
    let mut files = Vec::new();
    let mut parts = stdout.split(|byte| *byte == 0);
    while let Some(entry) = parts.next() {
        if entry.len() < 4 || entry[2] != b' ' {
            continue;
        }
        let x = entry[0] as char;
        let y = entry[1] as char;
        let status = format!("{x}{y}").trim().to_string();
        let path = String::from_utf8_lossy(&entry[3..]).into_owned();
        if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            // In `-z` porcelain mode, renames and copies emit an extra NUL-terminated original path.
            let _ = parts.next();
        }
        if !path.is_empty() {
            files.push(WorkspaceDiffFile { status, path });
        }
    }
    files
}

fn validate_diff_path_filter(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("path filter must not be empty".into());
    }
    if trimmed.contains('\0') || trimmed.starts_with('-') {
        return Err("invalid path filter".into());
    }
    let path = Path::new(trimmed);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return Err("path filter must be a relative workspace path".into());
    }
    Ok(trimmed.to_string())
}

fn truncate_utf8_bytes(input: &str, max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (input.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    (input[..end].to_string(), true)
}

fn synthetic_untracked_diff(repo_root: &Path, rel_path: &str) -> Option<String> {
    let full = repo_root.join(rel_path);
    let metadata = fs::metadata(&full).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mut file = fs::File::open(&full).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_WORKSPACE_DIFF_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.contains(&0) {
        return Some(format!(
            "diff --git a/{rel_path} b/{rel_path}\nnew file\nBinary file {rel_path}\n"
        ));
    }
    let text = String::from_utf8(bytes).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let mut out = format!(
        "diff --git a/{rel_path} b/{rel_path}\nnew file\n--- /dev/null\n+++ b/{rel_path}\n@@ -0,0 +1,{} @@\n",
        lines.len()
    );
    for line in lines {
        out.push('+');
        out.push_str(line);
        out.push('\n');
        if out.len() > MAX_WORKSPACE_DIFF_BYTES {
            break;
        }
    }
    Some(out)
}

fn git_current_branch(repo_root: &Path) -> String {
    let abbrev = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output();
    if let Ok(output) = abbrev {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    let symbolic = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output();
    if let Ok(output) = symbolic {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
    }
    String::new()
}

fn collect_workspace_diff(
    workspace_root: &Path,
    path_filter: Option<&str>,
) -> Result<WorkspaceDiffResponse, String> {
    let Some(repo_root) = workflow_git_repo_root(workspace_root) else {
        return Ok(WorkspaceDiffResponse {
            is_git_repo: false,
            branch: String::new(),
            files: Vec::new(),
            diff_text: String::new(),
            truncated: false,
        });
    };
    let branch = git_current_branch(&repo_root);
    let mut status_cmd = Command::new("git");
    status_cmd
        .arg("-C")
        .arg(&repo_root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
    if let Some(path) = path_filter {
        status_cmd.arg("--").arg(path);
    }
    let status_out = status_cmd
        .output()
        .map_err(|err| format!("git status failed: {err}"))?;
    if !status_out.status.success() {
        return Err(format!(
            "git status failed: {}",
            String::from_utf8_lossy(&status_out.stderr).trim()
        ));
    }
    let files = parse_git_status_porcelain_z(&status_out.stdout);

    let has_head = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .is_ok_and(|out| out.status.success());

    let mut diff_cmd = Command::new("git");
    diff_cmd
        .arg("-C")
        .arg(&repo_root)
        .args(["diff", "--no-color"]);
    if has_head {
        diff_cmd.arg("HEAD");
    }
    if let Some(path) = path_filter {
        diff_cmd.arg("--").arg(path);
    }
    let diff_out = diff_cmd
        .output()
        .map_err(|err| format!("git diff failed: {err}"))?;
    if !diff_out.status.success() {
        return Err(format!(
            "git diff failed: {}",
            String::from_utf8_lossy(&diff_out.stderr).trim()
        ));
    }
    let mut raw_diff = String::from_utf8_lossy(&diff_out.stdout).into_owned();
    if raw_diff.trim().is_empty() {
        if let Some(path) = path_filter {
            if files.iter().any(|item| item.path == path && item.status == "??") {
                if let Some(untracked) = synthetic_untracked_diff(&repo_root, path) {
                    raw_diff = untracked;
                }
            }
        }
    }
    let (diff_text, truncated) = truncate_utf8_bytes(&raw_diff, MAX_WORKSPACE_DIFF_BYTES);
    Ok(WorkspaceDiffResponse {
        is_git_repo: true,
        branch,
        files,
        diff_text,
        truncated,
    })
}

fn parse_git_worktree_list_porcelain(output: &str, current_root: &Path) -> Vec<WorkspaceWorktree> {
    let canonical_current =
        fs::canonicalize(current_root).unwrap_or_else(|_| current_root.to_path_buf());
    let mut worktrees = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_head = String::new();
    let mut current_branch = String::new();

    let flush = |worktrees: &mut Vec<WorkspaceWorktree>,
                 path_slot: &mut Option<String>,
                 head_slot: &mut String,
                 branch_slot: &mut String| {
        if let Some(path) = path_slot.take() {
            let candidate = fs::canonicalize(&path).unwrap_or_else(|_| PathBuf::from(&path));
            let is_main = worktrees.is_empty();
            let is_current = candidate == canonical_current;
            worktrees.push(WorkspaceWorktree {
                path,
                head: std::mem::take(head_slot),
                branch: std::mem::take(branch_slot),
                is_main,
                is_current,
            });
        } else {
            head_slot.clear();
            branch_slot.clear();
        }
    };

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(
                &mut worktrees,
                &mut current_path,
                &mut current_head,
                &mut current_branch,
            );
            continue;
        }
        if let Some(path) = trimmed.strip_prefix("worktree ") {
            flush(
                &mut worktrees,
                &mut current_path,
                &mut current_head,
                &mut current_branch,
            );
            current_path = Some(path.trim().to_string());
        } else if let Some(head) = trimmed.strip_prefix("HEAD ") {
            current_head = head.trim().to_string();
        } else if let Some(branch) = trimmed.strip_prefix("branch ") {
            let clean = branch
                .trim()
                .strip_prefix("refs/heads/")
                .unwrap_or(branch.trim());
            current_branch = clean.to_string();
        } else if trimmed == "detached" && current_branch.is_empty() {
            current_branch = "(detached)".to_string();
        } else if trimmed == "bare" && current_branch.is_empty() {
            current_branch = "(bare)".to_string();
        }
    }
    flush(
        &mut worktrees,
        &mut current_path,
        &mut current_head,
        &mut current_branch,
    );
    worktrees
}

fn list_workspace_worktrees(workspace_root: &Path) -> Result<WorkspaceWorktreesResponse, String> {
    let Some(repo_root) = workflow_git_repo_root(workspace_root) else {
        return Ok(WorkspaceWorktreesResponse {
            is_git_repo: false,
            worktrees: Vec::new(),
        });
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|err| format!("git worktree list failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let worktrees = parse_git_worktree_list_porcelain(&text, &repo_root);
    Ok(WorkspaceWorktreesResponse {
        is_git_repo: true,
        worktrees,
    })
}

fn validate_worktree_branch_name(raw: &str) -> Result<String, String> {
    let branch = raw.trim();
    if branch.is_empty() {
        return Err("Branch name is required.".into());
    }
    if branch.len() > MAX_WORKTREE_BRANCH_LEN {
        return Err("Branch name is too long (max 128 characters).".into());
    }
    if !branch
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/' | b'.'))
    {
        return Err(
            "Branch name may only contain letters, digits, '-', '_', '/', and '.'.".into(),
        );
    }
    if branch.starts_with('-')
        || branch.starts_with('/')
        || branch.starts_with('.')
        || branch.ends_with('/')
        || branch.ends_with('.')
        || branch.ends_with(".lock")
        || branch.contains("..")
        || branch.contains("//")
        || branch.contains("/.")
        || branch.contains("./")
    {
        return Err("Branch name is not a valid Git branch reference.".into());
    }
    Ok(branch.to_string())
}

fn create_workspace_worktree(
    workspace_root: &Path,
    raw_branch: &str,
    base_ref: Option<&str>,
) -> Result<(PathBuf, String), String> {
    let branch = validate_worktree_branch_name(raw_branch)?;
    let validated_base = match base_ref.map(str::trim).filter(|value| !value.is_empty()) {
        Some(base) => Some(validate_worktree_branch_name(base)?),
        None => None,
    };
    let repo_root = workflow_git_repo_root(workspace_root)
        .ok_or_else(|| "Workspace is not a Git repository.".to_string())?;
    let listed = list_workspace_worktrees(&repo_root)?;
    let main_root = listed
        .worktrees
        .iter()
        .find(|wt| wt.is_main)
        .map(|wt| PathBuf::from(&wt.path))
        .unwrap_or_else(|| repo_root.clone());
    let main_root = fs::canonicalize(&main_root).unwrap_or(main_root);
    let main_parent = main_root
        .parent()
        .ok_or_else(|| "Cannot determine parent directory for Git worktrees.".to_string())?;
    let repo_name = main_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    let worktrees_base = main_parent.join(".nola-worktrees").join(repo_name);
    let folder_name = branch.replace('/', "-");
    if folder_name.is_empty() || folder_name == "." || folder_name == ".." {
        return Err("Branch name produced an invalid worktree directory name.".into());
    }
    let target_path = worktrees_base.join(&folder_name);
    if target_path.exists() {
        return Err(format!(
            "Worktree path already exists: {}",
            target_path.display()
        ));
    }
    fs::create_dir_all(&worktrees_base)
        .map_err(|err| format!("Failed to prepare worktrees directory: {err}"))?;
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(&repo_root)
        .args(["worktree", "add", "-b", &branch])
        .arg(&target_path);
    if let Some(base) = validated_base.as_deref() {
        cmd.arg(base);
    }
    let output = cmd
        .output()
        .map_err(|err| format!("git worktree add failed: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "git worktree add failed".to_string()
        } else {
            stderr
        });
    }
    let final_path = fs::canonicalize(&target_path).unwrap_or(target_path);
    Ok((final_path, branch))
}

fn read_optional_json_object(request: &mut Request) -> Result<Value, Response<Box<dyn Read + Send>>> {
    let mut bytes = Vec::new();
    const LIMIT: u64 = 64 * 1024;
    if request
        .as_reader()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Err(json_response(
            StatusCode(400),
            json!({"ok": false, "error": "Failed to read request body."}),
        ));
    }
    if bytes.len() as u64 > LIMIT {
        return Err(json_response(
            StatusCode(413),
            json!({"ok": false, "error": "Request body too large."}),
        ));
    }
    let trimmed = String::from_utf8_lossy(&bytes);
    if trimmed.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| {
            json_response(
                StatusCode(400),
                json!({"ok": false, "error": "Invalid JSON object payload."}),
            )
        })
}

fn workflow_conversation_id_from_query_and_body(url: &str, body: Option<&Value>) -> Option<String> {
    body.and_then(|value| {
        value
            .get("conversation_id")
            .or_else(|| value.get("conversationId"))
            .or_else(|| value.get("session_id"))
            .or_else(|| value.get("sessionId"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    })
    .or_else(|| {
        query_param(url, "conversation_id")
            .or_else(|| query_param(url, "session_id"))
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
    })
}

fn workspace_diff_response(url: &str, config: &Config) -> Response<Box<dyn Read + Send>> {
    let conv_id = workflow_conversation_id_from_query_and_body(url, None);
    let path_filter = match query_param(url, "path") {
        Some(raw) if !raw.trim().is_empty() => match validate_diff_path_filter(&raw) {
            Ok(valid) => Some(valid),
            Err(error) => {
                return json_response(StatusCode(400), json!({"error": error}));
            }
        },
        _ => None,
    };
    let root = resolve_workflow_workspace_root(config, conv_id.as_deref());
    match collect_workspace_diff(&root, path_filter.as_deref()) {
        Ok(payload) => json_response(StatusCode(200), json!(payload))
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap()),
        Err(error) => json_response(StatusCode(500), json!({"error": error})),
    }
}

fn workspace_checkpoints_response(url: &str, config: &Config) -> Response<Box<dyn Read + Send>> {
    let conv_id = workflow_conversation_id_from_query_and_body(url, None);
    let root = resolve_workflow_workspace_root(config, conv_id.as_deref());
    let (checkpoints, redo_count) = list_workspace_checkpoints(&root);
    json_response(
        StatusCode(200),
        json!({
            "checkpoints": checkpoints,
            "redo_count": redo_count,
        }),
    )
    .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
}

fn workspace_undo_redo_response(
    request: &mut Request,
    url: &str,
    config: &Config,
    is_redo: bool,
) -> Response<Box<dyn Read + Send>> {
    let body = match read_optional_json_object(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let conv_id = workflow_conversation_id_from_query_and_body(url, Some(&body));
    let root = resolve_workflow_workspace_root(config, conv_id.as_deref());
    let outcome = if is_redo {
        redo_workspace_checkpoint(&root)
    } else {
        undo_workspace_checkpoint(&root)
    };
    match outcome {
        Ok(summary) => json_response(
            StatusCode(200),
            json!({
                "ok": true,
                "checkpoint_turn": summary.checkpoint_turn,
                "checkpoint_label": summary.checkpoint_label,
                "restored_files": summary.restored_files,
                "removed_files": summary.removed_files,
                "remaining_undo": summary.remaining_undo,
                "remaining_redo": summary.remaining_redo,
            }),
        )
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap()),
        Err(error) => json_response(
            StatusCode(409),
            json!({
                "ok": false,
                "error": error,
            }),
        ),
    }
}

fn workspace_worktrees_list_response(url: &str, config: &Config) -> Response<Box<dyn Read + Send>> {
    let conv_id = workflow_conversation_id_from_query_and_body(url, None);
    let root = resolve_workflow_workspace_root(config, conv_id.as_deref());
    match list_workspace_worktrees(&root) {
        Ok(payload) => json_response(StatusCode(200), json!(payload))
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap()),
        Err(error) => json_response(StatusCode(500), json!({"error": error})),
    }
}

fn workspace_worktrees_create_response(
    request: &mut Request,
    url: &str,
    config: &Config,
) -> Response<Box<dyn Read + Send>> {
    let input = match read_json::<WorkspaceCreateWorktreeInput>(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(error) = validate_worktree_branch_name(&input.branch) {
        return json_response(
            StatusCode(400),
            json!({
                "ok": false,
                "error": error,
            }),
        );
    }
    let conv_id = input
        .conversation_id
        .or(input.session_id)
        .or_else(|| workflow_conversation_id_from_query_and_body(url, None));
    let root = resolve_workflow_workspace_root(config, conv_id.as_deref());
    match create_workspace_worktree(&root, &input.branch, input.base_ref.as_deref()) {
        Ok((path, branch)) => json_response(
            StatusCode(200),
            json!({
                "ok": true,
                "path": path.to_string_lossy(),
                "branch": branch,
            }),
        )
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap()),
        Err(error) => json_response(
            StatusCode(409),
            json!({
                "ok": false,
                "error": error,
            }),
        ),
    }
}

#[cfg(test)]
mod workspace_workflow_tests {
    use super::*;

    fn init_workflow_temp_repo(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nola-web-workflow-test-{label}-{}",
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
        run(&["config", "user.name", "Nola Workflow Test"]);
        run(&["config", "user.email", "workflow@nola.local"]);
        run(&["config", "commit.gpgsign", "false"]);
        fs::write(dir.join("app.rs"), "fn main() { println!(\"v1\"); }\n").unwrap();
        fs::write(dir.join("notes.txt"), "hello\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "initial"]);
        dir
    }

    #[test]
    fn workspace_workflow_parses_porcelain_z_status_including_renames_and_untracked() {
        let raw = b" M src/main.rs\0A  src/new file.rs\0R  src/renamed.rs\0src/old name.rs\0?? untracked.txt\0 D removed.txt\0";
        let files = parse_git_status_porcelain_z(raw);
        assert_eq!(
            files,
            vec![
                WorkspaceDiffFile {
                    status: "M".into(),
                    path: "src/main.rs".into(),
                },
                WorkspaceDiffFile {
                    status: "A".into(),
                    path: "src/new file.rs".into(),
                },
                WorkspaceDiffFile {
                    status: "R".into(),
                    path: "src/renamed.rs".into(),
                },
                WorkspaceDiffFile {
                    status: "??".into(),
                    path: "untracked.txt".into(),
                },
                WorkspaceDiffFile {
                    status: "D".into(),
                    path: "removed.txt".into(),
                },
            ]
        );
    }

    #[test]
    fn workspace_workflow_collects_clean_diff_and_truncates_at_64kb() {
        let repo = init_workflow_temp_repo("diff");
        fs::write(repo.join("app.rs"), "fn main() { println!(\"v2\"); }\n").unwrap();
        fs::write(repo.join("fresh.txt"), "brand new line\n").unwrap();

        let full = collect_workspace_diff(&repo, None).unwrap();
        assert!(full.is_git_repo);
        assert!(!full.branch.is_empty());
        assert!(!full.truncated);
        assert!(full
            .files
            .iter()
            .any(|file| file.path == "app.rs" && file.status == "M"));
        assert!(full
            .files
            .iter()
            .any(|file| file.path == "fresh.txt" && file.status == "??"));
        assert!(full.diff_text.contains("-fn main() { println!(\"v1\"); }"));
        assert!(full.diff_text.contains("+fn main() { println!(\"v2\"); }"));
        assert!(!full.diff_text.contains("\x1b["));

        let untracked_filtered = collect_workspace_diff(&repo, Some("fresh.txt")).unwrap();
        assert_eq!(untracked_filtered.files.len(), 1);
        assert!(untracked_filtered.diff_text.contains("+brand new line"));

        // Verify UTF-8 safe 64KB truncation
        let large = "a".repeat(MAX_WORKSPACE_DIFF_BYTES + 1024);
        let (bounded, was_truncated) = truncate_utf8_bytes(&large, MAX_WORKSPACE_DIFF_BYTES);
        assert!(was_truncated);
        assert_eq!(bounded.len(), MAX_WORKSPACE_DIFF_BYTES);

        assert!(validate_diff_path_filter("../escape").is_err());
        assert!(validate_diff_path_filter("--flag").is_err());
        assert!(validate_diff_path_filter("/etc/passwd").is_err());
        assert!(validate_diff_path_filter("src/app.rs").is_ok());

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn workspace_workflow_parses_worktrees_and_validates_branch_names() {
        let porcelain = "\
worktree /tmp/repo-main
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /tmp/repo-wt-feat
HEAD 2222222222222222222222222222222222222222
branch refs/heads/feat/ui-diff

worktree /tmp/repo-wt-detached
HEAD 3333333333333333333333333333333333333333
detached
";
        let list = parse_git_worktree_list_porcelain(porcelain, Path::new("/tmp/repo-wt-feat"));
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].branch, "main");
        assert!(list[0].is_main);
        assert!(!list[0].is_current);
        assert_eq!(list[1].branch, "feat/ui-diff");
        assert!(!list[1].is_main);
        assert!(list[1].is_current);
        assert_eq!(list[2].branch, "(detached)");
        assert!(!list[2].is_main);
        assert!(!list[2].is_current);

        assert!(validate_worktree_branch_name("feat/workspace-diff_1.0").is_ok());
        assert!(validate_worktree_branch_name("").is_err());
        assert!(validate_worktree_branch_name("-bad-flag").is_err());
        assert!(validate_worktree_branch_name("bad..ref").is_err());
        assert!(validate_worktree_branch_name("bad branch").is_err());
        assert!(validate_worktree_branch_name("bad;rm").is_err());
        assert!(validate_worktree_branch_name("ends.lock").is_err());
    }

    #[test]
    fn workspace_workflow_creates_isolated_worktree_in_sibling_directory() {
        let repo = init_workflow_temp_repo("worktree-create");
        let (wt_path, branch) = create_workspace_worktree(&repo, "feat/test-wt", None).unwrap();
        assert_eq!(branch, "feat/test-wt");
        assert!(wt_path.exists());
        assert!(wt_path.join("app.rs").exists());

        let listed = list_workspace_worktrees(&repo).unwrap();
        assert!(listed.is_git_repo);
        assert_eq!(listed.worktrees.len(), 2);
        assert!(listed.worktrees.iter().any(|wt| wt.branch == "feat/test-wt"));

        let _ = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "remove", "--force"])
            .arg(&wt_path)
            .output();
        let _ = fs::remove_dir_all(&wt_path);
        if let Some(parent) = wt_path.parent() {
            let _ = fs::remove_dir(parent);
        }
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn workspace_workflow_checkpoints_capture_undo_and_redo_round_trip() {
        let repo = init_workflow_temp_repo("checkpoints");
        let captured_turn = capture_workspace_turn_checkpoint(&repo, "update app.rs").unwrap();
        assert_eq!(captured_turn, 1);

        let (checkpoints, redo_count) = list_workspace_checkpoints(&repo);
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].turn, 1);
        assert_eq!(checkpoints[0].label, "update app.rs");
        assert_eq!(redo_count, 0);

        fs::write(repo.join("app.rs"), "fn main() { println!(\"v2\"); }\n").unwrap();
        fs::write(repo.join("extra.rs"), "// created in turn 1\n").unwrap();

        let undone = undo_workspace_checkpoint(&repo).unwrap();
        assert_eq!(undone.checkpoint_turn, 1);
        assert!(undone.restored_files.contains(&"app.rs".to_string()));
        assert!(undone.removed_files.contains(&"extra.rs".to_string()));
        assert_eq!(undone.remaining_undo, 0);
        assert_eq!(undone.remaining_redo, 1);
        assert_eq!(
            fs::read_to_string(repo.join("app.rs")).unwrap(),
            "fn main() { println!(\"v1\"); }\n"
        );
        assert!(!repo.join("extra.rs").exists());

        let (after_undo, redo_after_undo) = list_workspace_checkpoints(&repo);
        assert!(after_undo.is_empty());
        assert_eq!(redo_after_undo, 1);

        let redone = redo_workspace_checkpoint(&repo).unwrap();
        assert_eq!(redone.checkpoint_turn, 1);
        assert!(redone.restored_files.contains(&"app.rs".to_string()));
        assert!(redone.restored_files.contains(&"extra.rs".to_string()));
        assert_eq!(redone.remaining_undo, 1);
        assert_eq!(redone.remaining_redo, 0);
        assert_eq!(
            fs::read_to_string(repo.join("app.rs")).unwrap(),
            "fn main() { println!(\"v2\"); }\n"
        );
        assert!(repo.join("extra.rs").exists());

        let key = workflow_checkpoint_key(&workflow_git_repo_root(&repo).unwrap());
        workspace_checkpoints()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
        let _ = fs::remove_dir_all(repo);
    }
}
