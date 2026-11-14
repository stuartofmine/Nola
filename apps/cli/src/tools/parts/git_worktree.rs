// SPDX-License-Identifier: Apache-2.0
// Explicit Host Git operations. A worktree is not an execution sandbox.

fn worktree_repository(root: &Path) -> Result<(PathBuf, String), String> {
    let common = PathBuf::from(git_commit_text(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    Ok((
        common.canonicalize().map_err(|e| e.to_string())?,
        git_commit_text(root, &["rev-parse", "--verify", "HEAD"])?,
    ))
}

fn worktree_directory_identity(path: &Path) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_dir() {
            return Err("Worktree identity is not a directory".into());
        }
        return Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::*;
        let file = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .map_err(|e| e.to_string())?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        return Ok(format!(
            "windows:{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ));
    }
    #[cfg(not(any(unix, windows)))]
    Err("Stable worktree directory identity is unavailable on this Host".into())
}

pub(crate) fn observe_task_repository(
    root: &Path,
) -> Result<nola_domain::task_workspace::TaskRepositoryIdentity, String> {
    let (common_directory, _) = worktree_repository(root)?;
    Ok(nola_domain::task_workspace::TaskRepositoryIdentity {
        common_directory: common_directory
            .to_str()
            .ok_or("Non-UTF8 Git directory")?
            .into(),
        common_directory_identity: worktree_directory_identity(&common_directory)?,
    })
}

fn observe_task_worktree(
    selection: &nola_domain::task_workspace::TaskWorktreeSelection,
) -> Result<nola_domain::task_workspace::TaskWorktreeBinding, String> {
    use nola_domain::task_workspace::TaskWorktreeBinding;
    selection.validate()?;
    let root = Path::new(&selection.root);
    let common = Path::new(&selection.expected_common_directory);
    if root.canonicalize().map_err(|e| e.to_string())? != root
        || common.canonicalize().map_err(|e| e.to_string())? != common
        || worktree_repository(root)?.0 != common
        || PathBuf::from(git_commit_text(root, &["rev-parse", "--show-toplevel"])?)
            .canonicalize()
            .map_err(|e| e.to_string())?
            != root
    {
        return Err("Worktree root or shared Git directory differs from selection".into());
    }
    let git_directory = PathBuf::from(git_commit_text(root, &["rev-parse", "--absolute-git-dir"])?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let registered = fs::read_to_string(git_directory.join("gitdir")).map_err(|e| e.to_string())?;
    let registered = PathBuf::from(registered.trim());
    let registered = if registered.is_absolute() {
        registered
    } else {
        git_directory.join(registered)
    };
    if registered.canonicalize().map_err(|e| e.to_string())?
        != root
            .join(".git")
            .canonicalize()
            .map_err(|e| e.to_string())?
    {
        return Err("Worktree registration points to a different working directory".into());
    }
    // Child commits and dirty files are allowed. Rewriting away the original
    // baseline is not silently adopted during cold recovery.
    git_command_output(
        root,
        &[
            "merge-base",
            "--is-ancestor",
            &selection.expected_head,
            "HEAD",
        ],
    )?;
    let binding = TaskWorktreeBinding {
        selection: selection.clone(),
        git_directory: git_directory
            .to_str()
            .ok_or("Non-UTF8 Git directory")?
            .into(),
        root_identity: worktree_directory_identity(root)?,
        git_directory_identity: worktree_directory_identity(&git_directory)?,
        common_directory_identity: worktree_directory_identity(common)?,
    };
    binding.validate()?;
    verify_source_snapshot(&binding,false)?;
    if selection
        .expected_directory_identity
        .as_ref()
        .is_some_and(|expected| *expected != binding.directory_identity())
    {
        return Err("Selected worktree directories changed; inspect and select again".into());
    }
    Ok(binding)
}

pub(crate) fn capture_task_worktree(
    selection: &nola_domain::task_workspace::TaskWorktreeSelection,
    roots: &[PathBuf],
) -> Result<nola_domain::task_workspace::TaskWorktreeBinding, String> {
    capture_authorized_worktree(selection,roots,true)
}

fn capture_authorized_worktree(
    selection: &nola_domain::task_workspace::TaskWorktreeSelection,
    roots: &[PathBuf], fresh_source: bool,
) -> Result<nola_domain::task_workspace::TaskWorktreeBinding,String> {
    let authorized = roots
        .iter()
        .map(|root| root.canonicalize().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    for path in [&selection.root, &selection.expected_common_directory] {
        if !Path::new(path).is_absolute()
            || !authorized
                .iter()
                .any(|root| Path::new(path).starts_with(root))
        {
            return Err("Task worktree is outside authorized workspace roots".into());
        }
    }
    let binding = observe_task_worktree(selection)?;
    binding.validate_authorized_roots(
        &authorized
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    )?;
    if fresh_source { verify_source_snapshot(&binding,true)?; }
    Ok(binding)
}

pub(crate) fn primary_execution_root(primary:&nola_domain::task_workspace::PrimaryWorktreeBinding,
    source:&Path,roots:&[PathBuf])->Result<PathBuf,String> {
    primary.validate()?;
    if source.canonicalize().map_err(|error|error.to_string())?!=Path::new(&primary.source_root)
        || observe_task_repository(source)?!=primary.source_repository {
        return Err("Primary workspace belongs to another source project".into());
    }
    let binding=capture_authorized_worktree(&primary.selection,roots,false)?;
    if binding.repository_identity()!=primary.source_repository {return Err("Primary execution repository changed".into());}
    Ok(PathBuf::from(binding.selection.root))
}

pub(crate) fn observe_parent_repository(root:&Path)->Result<Option<nola_domain::task_workspace::TaskRepositoryIdentity>,String> {
    match observe_task_repository(root) {
        Ok(repository)=>Ok(Some(repository)),
        Err(error) if root.ancestors().any(|directory|directory.join(".git").exists())=>Err(format!("Git source authority could not be observed: {error}")),
        Err(_)=>Ok(None),
    }
}

pub(crate) fn verify_task_worktree(
    binding: &nola_domain::task_workspace::TaskWorktreeBinding,
) -> Result<(), String> {
    binding.validate()?;
    if observe_task_worktree(&binding.selection)? != *binding {
        return Err("Bound worktree directories were replaced; execution is blocked".into());
    }
    Ok(())
}

// This is a trusted Host preparation, not an agent's sandboxed command.
// Reject conversion programs and gitlinks rather than executing repository code.
fn require_nonexecuting_worktree_preparation(root: &Path) -> Result<(), String> {
    let index = git_command_output(root, &["ls-files", "--stage", "-z"])?;
    let mut paths = Vec::new();
    for entry in index.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
        if entry.starts_with(b"160000 ") {
            return Err("Automatic worktree preparation does not support submodule/gitlink contents; no submodule was initialized".into());
        }
        let tab = entry.iter().position(|byte| *byte == b'\t').ok_or("Invalid Git index path")?;
        paths.extend_from_slice(&entry[tab + 1..]); paths.push(0);
    }
    // Read attributes without invoking conversion programs. Both staged and
    // unstaged attributes matter; inert global driver declarations are allowed.
    for cached in [false, true] {
        use std::io::{Write as _, Seek as _};
        let mut input = tempfile::tempfile().map_err(|error| error.to_string())?;
        input.write_all(&paths).and_then(|()| input.rewind()).map_err(|error| error.to_string())?;
        let mut command = git_operation_command(root);
        command.arg("check-attr"); if cached { command.arg("--cached"); }
        let output = command.args(["-z", "--stdin", "filter"]).stdin(input).output().map_err(|error| error.to_string())?;
        if !output.status.success() {return Err("Cannot safely inspect Git conversion attributes".into());}
        let fields = output.stdout.split(|byte| *byte == 0).filter(|value| !value.is_empty()).collect::<Vec<_>>();
        if fields.len() % 3 != 0 {return Err("Invalid Git conversion attributes".into());}
        if fields.chunks_exact(3).any(|field| field[2] != b"unspecified" && field[2] != b"unset") {
            return Err("Automatic worktree preparation refuses active clean/smudge/process filters; no conversion program was executed or configuration changed".into());
        }
    }
    Ok(())
}

pub(crate) fn inspect_git_worktrees(root: &Path) -> Result<Value, String> {
    require_nonexecuting_worktree_preparation(root)?;
    let (common, head) = worktree_repository(&root)?;
    let repository_root = PathBuf::from(git_commit_text(root,&["rev-parse","--show-toplevel"])?).
        canonicalize().map_err(|error|error.to_string())?;
    let root_is_repository_root = root.canonicalize().map_err(|error|error.to_string())? == repository_root;
    let bytes = git_command_output(&root, &["worktree", "list", "--porcelain", "-z"])?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "Non-UTF8 worktree metadata")?;
    let mut rows = Vec::new();
    let mut row = Map::new();
    for field in text.split('\0') {
        if field.is_empty() {
            if !row.is_empty() {
                rows.push(Value::Object(std::mem::take(&mut row)));
            }
            continue;
        }
        let (key, value) = field.split_once(' ').unwrap_or((field, ""));
        row.insert(
            key.into(),
            if value.is_empty() {
                Value::Bool(true)
            } else {
                Value::String(value.into())
            },
        );
    }
    if !row.is_empty() {
        rows.push(Value::Object(row));
    }
    let source=source_state(root,None,None);
    let snapshot_plan=source.as_ref().ok().filter(|state|state.dirty&&!state.untracked).map(|state|&state.plan);
    Ok(
        json!({"root":root,"repository_root":repository_root,"root_is_repository_root":root_is_repository_root,"head":head,"common_directory":common,"worktrees":rows,
            "repository_identity":observe_task_repository(root)?,
            "source_snapshot_plan":snapshot_plan,
            "snapshot_unavailable_reason":source.as_ref().err().cloned().or_else(||source.as_ref().ok().filter(|state|state.untracked).map(|_|"Untracked files require explicit source review; they were not copied".into())),
            "source_has_uncommitted_changes":!git_commit_text(root,&["status","--porcelain","--untracked-files=normal"])?.is_empty(),
            "provides_os_sandbox":false,"shared_git_metadata":true,
            "task_workspace_binding_changed":false}),
    )
}

fn git_worktree_list(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let result: Result<Value, String> = (|| {
        let raw = required_string(args, "root").map_err(|e| e.text)?;
        if !Path::new(&raw).is_absolute() {
            return Err("root must be absolute".into());
        }
        let root = resolve_configured_allowed(&raw, &ctx.allowed_dirs, true).map_err(|e| e.text)?;
        inspect_git_worktrees(&root)
    })();
    match result {
        Ok(value) => ok_data(value, "Inspected Git worktrees".into()),
        Err(message) => {
            error_data(json!({"error":"git_worktree_inspection_failed","message":message}))
        }
    }
}

fn git_worktree_create(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let mut started = false;
    let result: Result<Value, String> = (|| {
        let get = |key| required_string(args, key).map_err(|e| e.text);
        let raw = get("root")?;
        let target = get("destination")?;
        let expected_common = get("expected_common_directory")?;
        let expected_head = get("expected_head")?;
        if [&raw, &target, &expected_common]
            .iter()
            .any(|p| !Path::new(p).is_absolute())
        {
            return Err("Repository, destination and shared Git directory must be absolute".into());
        }
        if !matches!(expected_head.len(), 40 | 64)
            || !expected_head.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("expected_head must be a full inspected commit ID".into());
        }
        let root = resolve_configured_allowed(&raw, &ctx.allowed_dirs, true).map_err(|e| e.text)?;
        let common = resolve_configured_allowed(&expected_common, &ctx.allowed_dirs, true)
            .map_err(|e| e.text)?;
        let destination =
            resolve_configured_allowed(&target, &ctx.allowed_dirs, false).map_err(|e| e.text)?;
        // Check the original entry too: a dangling symlink is not a new path.
        if fs::symlink_metadata(&target).is_ok() || destination.exists() {
            return Err(
                "Destination already exists; inspect it instead of overwriting or adopting it"
                    .into(),
            );
        }
        if !destination.parent().is_some_and(Path::is_dir) || destination.starts_with(&common) {
            return Err("Destination needs an existing parent outside Git metadata".into());
        }
        let observed = worktree_repository(&root)?;
        if observed != (common.clone(), expected_head.clone()) {
            return Err("Repository HEAD or shared Git directory changed; inspect again".into());
        }
        if ctx.cancellation.is_requested() {
            return Err("Worktree creation cancelled before execution".into());
        }
        require_nonexecuting_worktree_preparation(&root)?;
        let source_has_uncommitted_changes = !git_commit_text(
            &root, &["status", "--porcelain", "--untracked-files=normal"],
        )?.is_empty();
        let source_snapshot_plan=args.get("source_snapshot").filter(|value|!value.is_null())
            .map(|value|serde_json::from_value::<nola_domain::task_workspace::SourceSnapshotPlan>(value.clone()))
            .transpose().map_err(|_|"Invalid source snapshot plan")?;
        if let Some(plan)=&source_snapshot_plan {
            plan.validate()?;
            let state=source_state(&root,None,Some(&ctx.cancellation))?;
            if plan.source_head!=expected_head || state.plan!=*plan || state.untracked || !state.dirty || destination.starts_with(&root) {
                return Err("Source snapshot is stale, has untracked files, or has no independent destination".into());
            }
        }
        // Claim the new directory atomically before Git starts. Two creators
        // must never let Git's checkout/cleanup race over the same target.
        fs::create_dir(&destination).map_err(|error| {
            format!("Destination reservation failed: {error}; no Git operation was started")
        })?;
        started = true;
        let (checkout_head,source_snapshot)=if let Some(plan)=source_snapshot_plan.as_ref() {
            let (head,snapshot)=prepare_source_snapshot(&root,plan,&ctx.cancellation)?;
            (head,Some(snapshot))
        } else {(expected_head.clone(),None)};
        let target = crate::platform_paths::shell_path(&destination);
        let no_hooks = tempfile::tempdir().map_err(|error| error.to_string())?;
        let hooks_setting = format!("core.hooksPath={}", git_path_argument(no_hooks.path())?);
        let mut checkout_args = vec!["-c".to_string(), hooks_setting, "-c".into(), "submodule.recurse=false".into()];
        // Explicitly disable all configured conversion drivers for this checkout
        // too; never rely on a global driver being absent or on mutable defaults.
        let filters = git_operation_command(&root).args(["config", "--name-only", "--get-regexp", r"^filter\..*\.(clean|smudge|process)$"])
            .stdin(Stdio::null()).output().map_err(|error| error.to_string())?;
        if !matches!(filters.status.code(), Some(0 | 1)) {return Err("Cannot inspect checkout conversion drivers".into());}
        for key in String::from_utf8(filters.stdout).map_err(|_| "Non-UTF8 Git conversion key")?.lines() {
            let (driver, _) = key.rsplit_once('.').ok_or("Invalid conversion key")?;
            for setting in [format!("{key}="), format!("{driver}.required=false")] {
                checkout_args.extend(["-c".into(), setting]);
            }
        }
        checkout_args.extend(["worktree".into(), "add".into(), "--detach".into(), "--".into(), target.into(), checkout_head.clone()]);
        git_mutation_process(
            &root,
            &checkout_args.iter().map(String::as_str).collect::<Vec<_>>(),
            &ctx.cancellation,
            "Git worktree creation",
        )?;
        if worktree_repository(&destination)? != (common.clone(), checkout_head.clone()) {
            return Err(
                "Created worktree identity did not match the requested repository and HEAD".into(),
            );
        }
        let actual_root = PathBuf::from(git_commit_text(
            &destination,
            &["rev-parse", "--show-toplevel"],
        )?);
        if actual_root.canonicalize().map_err(|e| e.to_string())? != destination
            || git_commit_text(&destination, &["rev-parse", "--abbrev-ref", "HEAD"])? != "HEAD"
        {
            return Err(
                "Created working directory or detached state changed during checkout".into(),
            );
        }
        let mut selection = nola_domain::task_workspace::TaskWorktreeSelection { source_snapshot,
            root: destination.to_string_lossy().into_owned(),
            expected_head: checkout_head.clone(),
            expected_common_directory: common.to_string_lossy().into_owned(),
            expected_directory_identity: None,
        };
        // Return a witnessed binding selector, not model-invented directory IDs.
        // Admission still re-observes it under the frozen parent's authority.
        selection.expected_directory_identity = Some(observe_task_worktree(&selection)?.directory_identity());
        if selection.source_snapshot.is_some() {
            let binding=observe_task_worktree(&selection)?;
            verify_source_snapshot(&binding,true)?;
            let state=source_state(&root,None,Some(&ctx.cancellation))?;
            for entry in &state.entries {
                let actual=source_entry(&destination,&entry.path,None,Some(&ctx.cancellation))?;
                if actual.mode!=entry.mode || actual.sha256!=entry.sha256 {return Err("Checkout could not preserve source bytes/modes; retained worktree was not admitted".into());}
            }
            if !git_commit_text(&destination,&["status","--porcelain","--untracked-files=all"])?.is_empty() {
                return Err("Snapshot cannot be represented as a clean checkout with this repository's filters or encoding".into());
            }
        }
        if worktree_repository(&root)? != (common.clone(),expected_head.clone()) {
            return Err("Source HEAD or Git directory changed during checkout; inspect the retained worktree".into());
        }
        let source_has_uncommitted_changes = source_has_uncommitted_changes || !git_commit_text(
            &root,&["status","--porcelain","--untracked-files=normal"],
        )?.is_empty();
        Ok(
            json!({"root":root,"worktree":destination,"head":checkout_head,"source_head":expected_head,"common_directory":common,
            "task_worktree_selection":selection,
            "source_has_uncommitted_changes":source_has_uncommitted_changes,
            "detached":true,"provides_os_sandbox":false,"shared_git_metadata":true,
            "checkout_source":if source_snapshot_plan.is_some(){"tracked_source_snapshot"}else{"committed_head"},"post_checkout_contents_audited":source_snapshot_plan.is_some(),
            "task_workspace_binding_changed":false,
            "retained":true,"automatic_cleanup":false}),
        )
    })();
    match result {
        Ok(value) => {
            use nola_host_protocol::tool_result::*;
            let resource = url::Url::from_directory_path(value["worktree"].as_str().unwrap())
                .unwrap()
                .to_string();
            let version = value["head"].as_str().unwrap().to_owned();
            let has_snapshot=value.pointer("/task_worktree_selection/source_snapshot").is_some();
            let mut outcome = ok_data(
                value,
                "Created detached Git worktree; retained for inspection".into(),
            );
            outcome.execution = Some(ExecutionReceipt {
                state: ExecutionState::Completed,
                effects: vec![ResourceEffect {
                    resource_uri: resource.clone(),
                    operation: "git.worktree.create".into(),
                    before_version: None,
                    after_version: Some(version.clone()),
                }],
                checks: [
                    "git.worktree.head_matches",
                    "git.worktree.common_directory_matches",
                    "git.worktree.detached",
                    "git.worktree.root_matches",
                    "git.worktree.source_snapshot_matches",
                ]
                .into_iter()
                .filter(|predicate|*predicate!="git.worktree.source_snapshot_matches"||has_snapshot)
                .map(|predicate| PostconditionCheck {
                    kind: ExecutionCheckKind::Postcondition,
                    resource_uri: resource.clone(),
                    version: version.clone(),
                    predicate: predicate.into(),
                    passed: true,
                })
                .collect(),
                ..Default::default()
            });
            outcome
        }
        Err(message) => error_data(
            json!({"error":"git_worktree_creation_failed","message":message,
            "tool_execution_skipped":!started,"effects_may_have_occurred":started,
            "recovery":"Inspect worktree registration and destination before retrying. No rollback, force or cleanup was performed."}),
        ),
    }
}

/// Apply reviewed commits without changing the source branch or its index.
/// Child must be clean; source must match the frozen clean or dirty task base.
/// New baselines/conflicts require another review, never an implicit merge.
fn git_worktree_integrate(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let mut started = false;
    let mut file_effects = Vec::new();
    let result: Result<Value,String> = (|| {
        let raw = required_string(args,"root").map_err(|error|error.text)?;
        if !Path::new(&raw).is_absolute() { return Err("Source root must be absolute".into()); }
        let root = resolve_configured_allowed(&raw,&ctx.allowed_dirs,true).map_err(|error|error.text)?;
        let selection:nola_domain::task_workspace::TaskWorktreeSelection = serde_json::from_value(
            args.get("worktree").cloned().ok_or("Missing reviewed worktree")?).map_err(|error|error.to_string())?;
        if selection.expected_directory_identity.is_none() {return Err("Integration requires witnessed worktree directory identities".into());}
        let binding = capture_task_worktree(&selection,&ctx.allowed_dirs)?;
        let child = Path::new(&selection.root);
        let child_head = required_string(args,"expected_child_head").map_err(|error|error.text)?;
        let source_head=selection.source_snapshot.as_ref().map(|snapshot|snapshot.plan.source_head.as_str()).unwrap_or(&selection.expected_head);
        let validate = || -> Result<(),String> {
            verify_task_worktree(&binding)?;
            if root == child || PathBuf::from(git_commit_text(&root,&["rev-parse","--show-toplevel"])?).canonicalize().map_err(|error|error.to_string())? != root || worktree_repository(&root)? != (PathBuf::from(&selection.expected_common_directory),source_head.into())
                || observe_task_repository(&root)? != binding.repository_identity() {
                return Err("Source repository or baseline changed; no automatic merge or overwrite".into());
            }
            if let Some(snapshot)=&selection.source_snapshot {
                if Path::new(&snapshot.source_root)!=root {return Err("Snapshot belongs to another source checkout".into());}
                verify_source_snapshot(&binding,true)?;
            }
            if git_commit_text(child,&["rev-parse","HEAD"])? != child_head {
                return Err("Child commit changed since review".into());
            }
            for directory in [root.as_path(),child] {
                if directory==root && selection.source_snapshot.is_some() {continue;}
                if !git_commit_text(directory,&["status","--porcelain","--untracked-files=all"])?.is_empty() {
                    return Err("Source and child must be clean; review and commit the child delivery first. User changes were not overwritten".into());
                }
            }
            Ok(())
        };
        validate()?;
        let patch = git_command_output(child,&["diff","--binary","--no-ext-diff","--no-textconv","--full-index",
            "--src-prefix=a/","--dst-prefix=b/",&selection.expected_head,&child_head,"--","."])?;
        if patch.is_empty() {return Err("Reviewed child has no committed changes to integrate".into());}
        if patch.len() > 8*1024*1024 {return Err("Delivery exceeds the bounded integration size; review it separately".into());}
        // Private transient patch; never staged in either checkout or retained
        // in a user configuration. Git rejects unsafe/escaping patch paths.
        let mut file = tempfile::NamedTempFile::new().map_err(|error|error.to_string())?;
        std::io::Write::write_all(&mut file,&patch).map_err(|error|error.to_string())?;
        let patch_argument = git_path_argument(file.path())?;
        let patch_path = patch_argument.as_str();
        git_command_output(&root,&["apply","--check","--binary","--",patch_path])?;
        // The repository patch identity cannot stand in for the bytes of the
        // delivered files. Observe each exact changed path, including both
        // ends of renames and deletion tombstones, before and after apply.
        let names = git_command_output(child,&["diff","--name-only","-z","--no-renames",
            "--no-ext-diff","--no-textconv",&selection.expected_head,&child_head,"--","."])?;
        let paths = names.split(|byte| *byte == 0).filter(|name| !name.is_empty())
            .map(|name| {
                let relative = Path::new(std::str::from_utf8(name).map_err(|_| "Non-UTF-8 delivery path")?);
                if !relative.components().all(|part| matches!(part,std::path::Component::Normal(_))) {
                    return Err("Unsafe delivery path".to_string());
                }
                Ok(root.join(relative))
            }).collect::<Result<Vec<_>,String>>()?;
        if paths.is_empty() || paths.len() > 4096 { return Err("Delivery exceeds the 4096-file evidence bound".into()); }
        let capture = || -> Result<Vec<String>,String> {
            let mut remaining = 256 * 1024 * 1024u64;
            paths.iter().map(|path| {
                remaining = remaining.checked_sub(fs::symlink_metadata(path).map(|metadata|metadata.len()).unwrap_or(0))
                    .ok_or("Delivery exceeds the 256 MiB version-evidence budget")?;
                resource_content_version(path)
            }).collect()
        };
        let before = capture()?;
        validate()?;
        if ctx.cancellation.is_requested() {return Err("Integration cancelled before apply".into());}
        started = true;
        git_mutation_process(&root,&["apply","--binary","--whitespace=nowarn","--",patch_path],&ctx.cancellation,"Reviewed worktree integration")?;
        git_command_output(&root,&["apply","--reverse","--check","--binary","--",patch_path])?;
        if git_commit_text(&root,&["rev-parse","HEAD"])? != source_head {
            return Err("Source HEAD changed during integration; inspect retained effects".into());
        }
        let after = capture()?;
        for ((path, before), after) in paths.iter().zip(before).zip(after) {
            let operation = if after == "absent:v1" { "delete" } else if before == "absent:v1" { "create" } else { "write" };
            file_effects.push(nola_host_protocol::tool_result::ResourceEffect {
                resource_uri:url::Url::from_file_path(path).map_err(|_| "Non-absolute delivery path")?.to_string(),
                operation:operation.into(),before_version:Some(before),after_version:Some(after),
            });
        }
        Ok(json!({"root":root,"worktree":selection.root,"base_head":selection.expected_head,"source_head":source_head,"child_head":child_head,
            "status":"applied_unstaged","patch_sha256":format!("{:x}",<sha2::Sha256 as sha2::Digest>::digest(&patch)),
            "source_branch_changed":false,"worktree_retained":true,"automatic_cleanup":false,
            "next_action":"Review and verify the applied delivery before committing. No task acceptance or test success is inferred."}))
    })();
    match result {
        Ok(data) => {
            use nola_host_protocol::tool_result::*;
            let resource = url::Url::from_directory_path(data["root"].as_str().unwrap()).unwrap().to_string();
            let version = data["patch_sha256"].as_str().unwrap().to_owned();
            let mut outcome = ok_data(data,"Applied reviewed child delivery as unstaged source changes; worktree retained".into());
            outcome.execution = Some(ExecutionReceipt {state:ExecutionState::Completed,
                effects:std::iter::once(ResourceEffect {resource_uri:resource.clone(),operation:"git.worktree.integrate".into(),before_version:None,after_version:Some(version.clone())})
                    .chain(file_effects).collect(),
                checks:vec![PostconditionCheck {kind:ExecutionCheckKind::Postcondition,resource_uri:resource,version,
                    predicate:"git.worktree.reviewed_patch_applied".into(),passed:true}],..Default::default()});
            outcome
        }
        Err(message) => error_data(json!({"error":"worktree_integration_blocked","message":message,
            "tool_execution_skipped":!started,"effects_may_have_occurred":started,
            "recovery":"Inspect retained source/worktree changes before retrying; no force, rollback, commit or cleanup was performed."})),
    }
}

#[cfg(test)]
mod git_worktree_tests {
    use super::*;

    #[test]
    fn reviewed_delivery_preserves_dirty_source_and_handles_external_bare_metadata() {
        for bare in [false,true] {
            let fixture = tempfile::tempdir().unwrap();
            let parent = fixture.path().canonicalize().unwrap();
            let seed = parent.join("seed"); fs::create_dir(&seed).unwrap();
            for args in [vec!["init","--quiet"],vec!["config","user.name","Fixture"],vec!["config","user.email","fixture@localhost"],
                vec!["config","commit.gpgsign","false"],vec!["config","core.hooksPath",".git/no-hooks"]] {
                git_command_output(&seed,&args).unwrap();
            }
            fs::write(seed.join("code.txt"),"base\n").unwrap();
            fs::write(seed.join("remove.txt"),"removed\n").unwrap();
            fs::write(seed.join("rename original.txt"),"renamed\n").unwrap();
            git_command_output(&seed,&["add","."]).unwrap();
            git_command_output(&seed,&["commit","--quiet","-m","base"]).unwrap();
            let root = if bare {
                let common = parent.join("repo.git");
                git_command_output(&seed,&["clone","--quiet","--bare","--local",&git_path_argument(&seed).unwrap(),&git_path_argument(&common).unwrap()]).unwrap();
                for args in [vec!["config","user.name","Fixture"],vec!["config","user.email","fixture@localhost"],vec!["config","commit.gpgsign","false"]] {
                    git_command_output(&common,&args).unwrap();
                }
                let source = parent.join("source");
                git_command_output(&common,&["worktree","add","--quiet","--detach",&git_path_argument(&source).unwrap(),"HEAD"]).unwrap();
                source
            } else {seed};
            let ctx = ToolContext {
                memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                allowed_dirs:vec![parent.clone()],language_services:Default::default(),
                skills:crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
                policy:ToolExecutionPolicy::new(Vec::new(),false,"ask".into(),Vec::new()),
                permission_scope:Default::default(),cancellation:Default::default(),plan_state:Default::default(),
                output_store:ToolOutputStore::for_test(),browser:BrowserRuntime::new(parent.join("browser")),
            };
            let info = inspect_git_worktrees(&root).unwrap();
            assert_eq!(info["source_has_uncommitted_changes"],false);
            let child = parent.join("child");
            let made = git_worktree_create(json!({"root":root,"destination":child,"expected_head":info["head"],
                "expected_common_directory":info["common_directory"]}).as_object().unwrap(),&ctx);
            assert!(!made.error,"{}",made.text);
            let created_receipt = made.execution.as_ref().unwrap();
            created_receipt.validate().unwrap();
            assert_eq!(created_receipt.effects.len(), 1);
            let created_effect = &created_receipt.effects[0];
            assert!(created_receipt.checks.iter().any(|check| check.passed
                && check.resource_uri == created_effect.resource_uri
                && Some(&check.version) == created_effect.after_version.as_ref()));
            let selection = made.data.unwrap()["task_worktree_selection"].clone();
            fs::write(child.join("code.txt"),"reviewed implementation\n").unwrap();
            fs::write(child.join("new.txt"),"reviewed new file\n").unwrap();
            fs::remove_file(child.join("remove.txt")).unwrap();
            fs::rename(child.join("rename original.txt"),child.join("renamed final.txt")).unwrap();
            let mut args = json!({"root":root,"worktree":selection,"expected_child_head":info["head"]});
            assert!(git_worktree_integrate(args.as_object().unwrap(),&ctx).error,"uncommitted child delivery must not be inferred");
            git_command_output(&child,&["add","."]).unwrap();
            git_command_output(&child,&["commit","--quiet","-m","reviewed delivery"]).unwrap();
            args["expected_child_head"] = json!(git_commit_text(&child,&["rev-parse","HEAD"]).unwrap());
            fs::write(root.join("code.txt"),"user dirty\n").unwrap();
            assert!(git_worktree_integrate(args.as_object().unwrap(),&ctx).error);
            assert_eq!(fs::read_to_string(root.join("code.txt")).unwrap(),"user dirty\n");
            // Restore only this synthetic fixture's marker, never user files.
            fs::write(root.join("code.txt"),"base\n").unwrap();
            let applied = git_worktree_integrate(args.as_object().unwrap(),&ctx);
            assert!(!applied.error,"{}",applied.text);
            let integration_receipt = applied.execution.as_ref().unwrap();
            integration_receipt.validate().unwrap();
            let effects = &integration_receipt.effects;
            let metadata = &effects[0];
            assert_eq!(metadata.operation, "git.worktree.integrate");
            assert!(integration_receipt.checks.iter().any(|check| check.passed
                && check.resource_uri == metadata.resource_uri
                && Some(&check.version) == metadata.after_version.as_ref()));
            // Applying the reviewed patch proves metadata postconditions, not
            // runtime correctness of the integrated file content.
            assert!(effects[1..].iter().all(|effect| integration_receipt.checks.iter()
                .all(|check| check.resource_uri != effect.resource_uri)));
            for (name, operation, before) in [("code.txt","write",Some("base\n")),("new.txt","create",None),
                ("remove.txt","delete",Some("removed\n")),("rename original.txt","delete",Some("renamed\n")),("renamed final.txt","create",None)] {
                let effect = effects.iter().find(|effect| effect.resource_uri == url::Url::from_file_path(root.join(name)).unwrap().to_string()).unwrap();
                assert_eq!(effect.operation, operation);
                assert_eq!(effect.after_version, Some(resource_content_version(&root.join(name)).unwrap()));
                if let Some(before) = before {
                    use sha2::{Digest,Sha256};
                    assert_eq!(effect.before_version, Some(format!("sha256:{:x}",Sha256::digest(before.as_bytes()))));
                } else { assert_eq!(effect.before_version.as_deref(),Some("absent:v1")); }
            }
            assert_eq!(applied.data.unwrap()["status"],"applied_unstaged");
            assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(),"reviewed new file\n");
            assert_eq!(git_commit_text(&root,&["rev-parse","HEAD"]).unwrap(),info["head"]);
            assert!(git_command_output(&root,&["diff","--cached"]).unwrap().is_empty());
            assert!(child.exists());
            assert!(git_worktree_integrate(args.as_object().unwrap(),&ctx).error,"replay must not overwrite unstaged integration");
        }
    }

    #[test]
    fn detached_worktree_preserves_dirty_source_and_never_adopts_an_existing_destination() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().canonicalize().unwrap();
        let _environment = crate::store::test_env_lock();
        struct RestoreEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for RestoreEnvironment {
            fn drop(&mut self) {
                for (name, value) in &self.0 {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
        let _restore = RestoreEnvironment(["NOLA_HOME", "NOLA_DB_PATH"].into_iter()
            .map(|name| (name, std::env::var_os(name))).collect());
        std::env::set_var("NOLA_HOME", &parent);
        std::env::set_var("NOLA_DB_PATH", parent.join("application.sqlite"));
        let root = parent.join("source");
        fs::create_dir(&root).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@localhost"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["config", "core.hooksPath", ".git/qa-no-hooks"],
            vec!["config", "core.fsmonitor", "false"],
        ] {
            git_command_output(&root, &args).unwrap();
        }
        fs::write(root.join("source.txt"), "committed").unwrap();
        git_command_output(&root, &["add", "source.txt"]).unwrap();
        git_command_output(&root, &["commit", "--quiet", "-m", "fixture"]).unwrap();
        fs::write(root.join("source.txt"), "user dirty").unwrap();
        fs::write(
            root.join("private-untracked.txt"),
            "synthetic private fixture",
        )
        .unwrap();
        let mut ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs: vec![parent.clone()],
            policy: ToolExecutionPolicy::new(Vec::new(), false, "ask".into(), Vec::new()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(parent.join("browser")),
        };
        let host_git = git_operation_command(&root);
        #[cfg(not(windows))] assert_eq!(host_git.get_program(), "/usr/bin/git");
        #[cfg(windows)] assert_eq!(host_git.get_program(), r"C:\Program Files\Git\cmd\git.exe");
        assert!(host_git.get_envs().all(|(key,value)| value.is_some() && matches!(key.to_str(),
            Some("PATH" | "SystemRoot" | "HOME" | "XDG_CONFIG_HOME" | "USERPROFILE" | "LANG" | "GIT_NO_LAZY_FETCH" | "GIT_TERMINAL_PROMPT"))));
        let marker = parent.join("repository-program-must-not-run");
        let hooks = parent.join("repository-hooks");
        fs::create_dir(&hooks).unwrap();
        let hook = hooks.join("post-checkout");
        fs::write(&hook,format!("#!/bin/sh\nprintf unsafe > '{}'\n",marker.display())).unwrap();
        #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;fs::set_permissions(&hook,fs::Permissions::from_mode(0o755)).unwrap();}
        git_command_output(&root,&["config","core.hooksPath",hooks.to_str().unwrap()]).unwrap();
        fs::write(root.join(".gitattributes"), "source.txt filter=fixture\n").unwrap();
        for kind in ["clean","smudge","process"] {
            let setting=format!("filter.fixture.{kind}");
            let program=format!("printf unsafe > '{}'",marker.display());
            git_command_output(&root,&["config",&setting,&program]).unwrap();
            let blocked=git_worktree_list(json!({"root":root}).as_object().unwrap(),&ctx);
            assert!(blocked.error,"configured {kind} must be refused before source status/checkout");
            assert!(!marker.exists());
            assert_eq!(git_commit_text(&root,&["config","--get",&setting]).unwrap(),program);
            git_command_output(&root,&["config","--unset",&setting]).unwrap();
        }
        fs::remove_file(root.join(".gitattributes")).unwrap();
        let inspected = git_worktree_list(json!({"root":root}).as_object().unwrap(), &ctx);
        assert!(!inspected.error, "{}", inspected.text);
        let info = inspected.data.unwrap();
        // A gitlink must not silently become an empty/missing task directory.
        git_command_output(&root,&["update-index","--add","--cacheinfo","160000",info["head"].as_str().unwrap(),"nested-module"]).unwrap();
        assert!(git_worktree_list(json!({"root":root}).as_object().unwrap(),&ctx).error);
        git_command_output(&root,&["update-index","--force-remove","nested-module"]).unwrap();
        let destination = parent.join("separate");
        let input = json!({"root":root,"destination":destination,"expected_head":info["head"],"expected_common_directory":info["common_directory"]});
        let mut stale = input.clone();
        stale["expected_head"] = Value::String("0".repeat(40));
        assert!(git_worktree_create(stale.as_object().unwrap(), &ctx).error);
        assert!(!destination.exists());
        let mut wrong_repository = input.clone();
        wrong_repository["expected_common_directory"] = json!(parent);
        assert!(git_worktree_create(wrong_repository.as_object().unwrap(), &ctx).error);
        assert!(!destination.exists());
        ctx.policy = ToolExecutionPolicy::new(
            Vec::new(),
            false,
            "ask".into(),
            vec![crate::runtime_config::PermissionRule {
                permission: "*".into(),
                pattern: "*".into(),
                action: crate::runtime_config::PermissionAction::Deny,
            }],
        );
        let denied = execute_tool(
            &ToolRequest {
                id: "worktree-deny".into(),
                name: "git_worktree_create".into(),
                args: input.as_object().unwrap().clone(),
            },
            &ctx,
        );
        assert!(denied.error);
        assert!(!destination.exists());
        ctx.policy = ToolExecutionPolicy::new(Vec::new(), false, "ask".into(), Vec::new());
        let result = git_worktree_create(input.as_object().unwrap(), &ctx);
        assert!(!result.error, "{}", result.text);
        assert!(!marker.exists(), "automatic checkout must not run post-checkout hooks");
        assert_eq!(git_commit_text(&root,&["config","--get","core.hooksPath"]).unwrap(),hooks.to_str().unwrap(),"only a command-local override is allowed");
        assert_eq!(result.execution.as_ref().unwrap().checks.len(), 4);
        let created = result.data.unwrap();
        assert_eq!(created["provides_os_sandbox"], false);
        assert_eq!(created["source_has_uncommitted_changes"], true);
        let selector = serde_json::from_value(created["task_worktree_selection"].clone()).unwrap();
        assert!(capture_task_worktree(&selector, &ctx.allowed_dirs).is_ok());
        assert_eq!(
            fs::read_to_string(destination.join("source.txt")).unwrap(),
            "committed"
        );
        assert!(!destination.join("private-untracked.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("source.txt")).unwrap(),
            "user dirty"
        );
        assert!(git_worktree_create(input.as_object().unwrap(), &ctx).error);
        let linked = git_worktree_list(json!({"root":destination}).as_object().unwrap(), &ctx);
        assert_eq!(
            linked.data.unwrap()["worktrees"].as_array().unwrap().len(),
            2
        );
        let contract = builtin_permission_contract("git_worktree_create", false);
        assert!(contract.risk.requires_confirmation());
        let plan = contract.resource_plan_for(&input).unwrap();
        let facts = resolve_permission_resources(&ctx, &nola_domain::permission_resource_plan::PermissionResolutionRequest {
            resolution_id:"worktree-fixture".into(),execution_target:"isolated-fixture".into(),
            invocation_digest:nola_domain::permission_resource_plan::PermissionResolutionRequest::input_digest(&input),
            capability:plan.capability.clone(),operation:plan.operation.clone(),demands:plan.extract(&input).unwrap()}, &input).unwrap();
        assert_eq!(facts.bindings.len(), 3);
        assert!(facts
            .bindings
            .iter()
            .any(|binding| binding.observation.requires_confirmation()));
        // Real concurrent Git operations; not simulated agent workers.
        let mut left = input.clone();
        left["destination"] = json!(parent.join("parallel-left"));
        let mut right = input.clone();
        right["destination"] = json!(parent.join("parallel-right"));
        let (left_result, right_result) = std::thread::scope(|scope| {
            let l = scope.spawn(|| git_worktree_create(left.as_object().unwrap(), &ctx));
            let r = scope.spawn(|| git_worktree_create(right.as_object().unwrap(), &ctx));
            (l.join().unwrap(), r.join().unwrap())
        });
        assert!(!left_result.error, "{}", left_result.text);
        assert!(!right_result.error, "{}", right_result.text);
        fs::write(
            parent.join("parallel-left/left-only.txt"),
            "synthetic child output",
        )
        .unwrap();
        assert!(!parent.join("parallel-right/left-only.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("source.txt")).unwrap(),
            "user dirty"
        );
        let mut race = input.clone();
        race["destination"] = json!(parent.join("single-owner"));
        let outcomes = std::thread::scope(|scope| {
            let first = scope.spawn(|| git_worktree_create(race.as_object().unwrap(), &ctx));
            let second = scope.spawn(|| git_worktree_create(race.as_object().unwrap(), &ctx));
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(
            outcomes.iter().filter(|outcome| !outcome.error).count(),
            1,
            "same-destination outcomes: {:?}",
            outcomes
                .iter()
                .map(|outcome| &outcome.text)
                .collect::<Vec<_>>()
        );
        assert!(outcomes.iter().any(|outcome| outcome.error
            && outcome
                .data
                .as_ref()
                .is_some_and(|data| data["tool_execution_skipped"] == true)));
        assert_eq!(
            fs::read_to_string(parent.join("single-owner/source.txt")).unwrap(),
            "committed"
        );
        let selection = nola_domain::task_workspace::TaskWorktreeSelection { source_snapshot:None,
            expected_directory_identity: None,
            root: destination.to_str().unwrap().into(),
            expected_head: info["head"].as_str().unwrap().into(),
            expected_common_directory: info["common_directory"].as_str().unwrap().into(),
        };
        let binding = capture_task_worktree(&selection, &[parent.clone()]).unwrap();
        let restored = serde_json::from_str(&serde_json::to_string(&binding).unwrap()).unwrap();
        let mut narrowed_selection=selection.clone();
        narrowed_selection.expected_directory_identity=Some(binding.directory_identity());
        let alias = parent.join("forged-index-alias");
        fs::create_dir(&alias).unwrap();
        fs::copy(destination.join(".git"), alias.join(".git")).unwrap();
        let mut forged = selection.clone();
        forged.root = alias.to_string_lossy().into_owned();
        assert!(
            capture_task_worktree(&forged, &[parent.clone()]).is_err(),
            "copied Git pointer is not an independent registered worktree"
        );

        verify_task_worktree(&restored).unwrap();
        {
            let store = crate::store::NolaStore::open(&parent.join("state.sqlite")).unwrap();
            let mut request = crate::config::Config::from_args([
                "nola", "--project", root.to_str().unwrap(), "--endpoint", "https://fixture.invalid/v1",
            ].into_iter().map(str::to_owned)).unwrap().host_invocation_request().unwrap();
            request.tool_settings.project_root = root.clone();
            request.tool_settings.allowed_dirs = vec![parent.clone()];
            request.model_settings.endpoint = "https://fixture.invalid/v1".into();
            request.model_settings.api_key = "synthetic-not-a-real-key".into();
            let profiles = crate::agent_registry::AgentRegistry::from_profiles(&Default::default());
            let mut frozen = crate::delegated_invocation::DelegatedInvocation::capture(
                &request,
                "owner",
                profiles.get("explore").unwrap(),
                &store,
            )
            .unwrap();
            frozen.0.worktree = Some(binding.clone());
            let encoded = serde_json::to_string(&frozen).unwrap();
            assert!(!encoded.contains("synthetic-not-a-real-key"));
            let frozen: crate::delegated_invocation::DelegatedInvocation =
                serde_json::from_str(&encoded).unwrap();
            request.tool_settings.project_root = parent.clone();
            let (_, child) = frozen.restore(&request, "owner", &store).unwrap();
            assert_eq!(child.tool_settings.project_root, destination);
            assert_eq!(child.tool_settings.allowed_dirs, vec![destination.clone()]);
            let metadata = json!({"delegated_invocation":frozen.0}).to_string();
            crate::delegated_invocation::validate_task_workspace_execution(&child, &metadata)
                .unwrap();
            let mut wrong = child.clone();
            wrong.tool_settings.project_root = root.clone();
            assert!(
                crate::delegated_invocation::validate_task_workspace_execution(&wrong, &metadata)
                    .is_err()
            );
            request.tool_settings.allowed_dirs = vec![root.clone()];
            assert!(frozen.restore(&request, "owner", &store).is_err());
        }

        fs::write(destination.join("child.txt"), "child work").unwrap();
        git_command_output(&destination, &["add", "child.txt"]).unwrap();
        git_command_output(&destination, &["commit", "--quiet", "-m", "child work"]).unwrap();
        verify_task_worktree(&restored).unwrap();
        assert!(capture_task_worktree(&selection, &[root.clone()]).is_err());
        ctx.policy = ctx
            .policy
            .clone()
            .with_bound_file_workspace(Some(destination.clone()));
        let write = |path: PathBuf| ToolRequest {
            id: format!("scoped-write-{}",uuid::Uuid::new_v4()),
            name: "write_file".into(),
            args: json!({"path":path,"content":"bounded synthetic output"})
                .as_object()
                .unwrap()
                .clone(),
        };
        let outside = write(root.join("must-not-appear.txt"));
        let rejected = execute_authorized_tool(
            &outside,
            &ctx,
            ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::AllowAll),
        );
        assert!(rejected.error);
        assert_eq!(rejected.data.unwrap()["tool_execution_skipped"], true);
        assert!(!root.join("must-not-appear.txt").exists());
        let git_link = fs::read(destination.join(".git")).unwrap();
        assert!(execute_authorized_tool(
            &write(destination.join(".git")), &ctx,
            ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::AllowAll),
        ).error);
        assert_eq!(fs::read(destination.join(".git")).unwrap(), git_link);
        let inside = write(destination.join("bounded-output.txt"));
        let written = execute_authorized_tool(
            &inside,
            &ctx,
            ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::AllowAll),
        );
        assert!(!written.error, "{}", written.text);
        assert_eq!(
            fs::read_to_string(destination.join("bounded-output.txt")).unwrap(),
            "bounded synthetic output"
        );
        let command = ToolRequest {
            id: "no-command".into(),
            name: "bash".into(),
            args: json!({"command":"this is not executed"})
                .as_object()
                .unwrap()
                .clone(),
        };
        assert!(
            execute_authorized_tool(
                &command,
                &ctx,
                ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::AllowAll)
            )
            .error
        );
        ctx.policy = ctx.policy.clone().with_bound_file_workspace(None);
        let moved = parent.join("retained-child");
        fs::rename(&destination, &moved).unwrap();
        assert!(verify_task_worktree(&restored).is_err());
        fs::create_dir(&destination).unwrap();
        fs::copy(moved.join(".git"), destination.join(".git")).unwrap();
        assert!(
            verify_task_worktree(&restored).is_err(),
            "replacement directory must not inherit task authority"
        );
        assert!(
            capture_task_worktree(&narrowed_selection, &[parent.clone()]).is_err(),
            "stale explicit selection cannot adopt a replacement directory"
        );
        ctx.cancellation.request();
        let mut cancelled = input.clone();
        cancelled["destination"] = json!(parent.join("cancelled-before-start"));
        let outcome = git_worktree_create(cancelled.as_object().unwrap(), &ctx);
        assert!(outcome.error);
        assert_eq!(outcome.data.unwrap()["tool_execution_skipped"], true);
        assert!(!parent.join("cancelled-before-start").exists());
        ctx.allowed_dirs = vec![root];
        assert!(git_worktree_create(input.as_object().unwrap(), &ctx).error);
    }
}
