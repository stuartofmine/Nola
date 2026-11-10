// SPDX-License-Identifier: Apache-2.0
// Fresh resource identity/version checks for whole replacement and entry moves.
// These Host observations never populate the model's file-read coverage ledger.

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileMutationVersion {
    sha256: String,
    bytes: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
    #[cfg(windows)]
    identity: (u32, u64),
}

#[derive(Debug)]
struct FileMutationSnapshot {
    version: FileMutationVersion,
    preview: Vec<u8>,
}

#[cfg(windows)]
fn mutation_handle_identity(file: &File) -> std::io::Result<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        FILE_ATTRIBUTE_REPARSE_POINT};
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,"Mutation target is a reparse point"));
    }
    Ok((information.dwVolumeSerialNumber,
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow)))
}

fn observe_file_mutation_target(path: &Path) -> Result<Option<FileMutationSnapshot>, ToolOutcome> {
    use sha2::{Digest, Sha256};
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(error(format!("Inspect mutation target {} failed: {err}", path.display()))),
    };
    let before = file.metadata().map_err(|err| error(err.to_string()))?;
    if !before.is_file() { return Err(error("Mutation requires a regular file".into())); }
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        (before.dev(), before.ino(), before.ctime(), before.ctime_nsec())
    };
    #[cfg(windows)]
    let identity = mutation_handle_identity(&file).map_err(|err| error(format!("Inspect file identity failed: {err}")))?;
    let mut hash = Sha256::new();
    let mut preview = Vec::new();
    let mut buffer = [0_u8; 65_536];
    let mut bytes = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(|err| error(err.to_string()))?;
        if count == 0 { break; }
        hash.update(&buffer[..count]);
        bytes = bytes.checked_add(count as u64).ok_or_else(|| error("File size overflow".into()))?;
        let retained = count.min(65_536usize.saturating_sub(preview.len()));
        preview.extend_from_slice(&buffer[..retained]);
    }
    let after = file.metadata().map_err(|err| error(err.to_string()))?;
    let entry = fs::symlink_metadata(path).map_err(|err| error(err.to_string()))?;
    if !entry.is_file() || !file_read_metadata_matches(&before, &after)
        || !file_read_metadata_matches(&after, &entry) || bytes != after.len() {
        return Err(error(format!("Resource changed during inspection: {}", path.display())));
    }
    #[cfg(windows)]
    {
        // Equal length/mtime alone cannot correlate a path entry to the open
        // handle. Re-open without following reparse points and compare IDs.
        let entry = options.open(path).map_err(|err|error(err.to_string()))?;
        if mutation_handle_identity(&file).map_err(|err|error(err.to_string()))? != identity
            || mutation_handle_identity(&entry).map_err(|err|error(err.to_string()))? != identity {
            return Err(error("Resource identity changed during inspection".into()));
        }
    }
    let version = FileMutationVersion { sha256: format!("{:x}", hash.finalize()), bytes,
        modified: after.modified().map_err(|err| error(err.to_string()))?,
        #[cfg(any(unix, windows))]
        identity,
    };
    Ok(Some(FileMutationSnapshot { version, preview }))
}

fn mutation_version(snapshot: &Option<FileMutationSnapshot>) -> String {
    snapshot.as_ref().map(|value| format!("sha256:{}", value.version.sha256))
        .unwrap_or_else(|| "absent:v1".into())
}

fn same_mutation_resource(left: &FileMutationSnapshot, right: &FileMutationSnapshot) -> bool {
    #[cfg(unix)]
    { left.version.identity.0 == right.version.identity.0 && left.version.identity.1 == right.version.identity.1 }
    #[cfg(windows)]
    { left.version.identity == right.version.identity }
    #[cfg(not(any(unix, windows)))]
    { let _ = (left, right); false }
}

fn validate_mutation_expectation(snapshot: &Option<FileMutationSnapshot>, args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    if let Some(expected) = expected_sha256(args)? {
        if snapshot.as_ref().is_none_or(|value| value.version.sha256 != expected) {
            return Err(error("expected_sha256 mismatch; mutation was not started".into()));
        }
    }
    Ok(())
}

/// Called again under all affected path locks immediately before mutation.
fn revalidate_file_mutation_target(path: &Path, before: &Option<FileMutationSnapshot>,
    args: &Map<String, Value>) -> Result<Option<FileMutationSnapshot>, ToolOutcome> {
    let current = observe_file_mutation_target(path)?;
    validate_mutation_expectation(&current, args)?;
    if current.as_ref().map(|value| &value.version) != before.as_ref().map(|value| &value.version) {
        return Err(error(format!("Resource changed before mutation; operation was not started: {}", path.display())));
    }
    Ok(current)
}

/// No existence-check fallback: a destination created after preflight is never
/// overwritten unless overwrite was explicitly authorized.
fn atomic_move_file(source: &Path, destination: &Path, overwrite: bool) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};
        let source = source.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
        let destination = destination.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
        let flags = if overwrite { MOVEFILE_REPLACE_EXISTING } else { 0 };
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    if overwrite { return fs::rename(source, destination); }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let invalid = |_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in move path");
        let source = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(invalid)?;
        let destination = std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(invalid)?;
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, source.as_ptr(),
            libc::AT_FDCWD, destination.as_ptr(), libc::RENAME_NOREPLACE) };
        if result != 0 { return Err(std::io::Error::last_os_error()); }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Atomic no-clobber move is unsupported on this platform"))
}

fn with_mutation_artifact(outcome: ToolOutcome, path: &Path, operation: &str,
    before: String, after: String) -> ToolOutcome {
    let mut outcome = outcome.with_file_artifact(path, operation);
    if let Some(effect) = outcome.execution.as_mut().and_then(|receipt| receipt.effects.last_mut()) {
        effect.before_version = Some(before);
        effect.after_version = Some(after);
    }
    outcome
}

/// Read-only preparation; these paths join the mutation lock set before any
/// source/target or derived-plan validation can authorize directory creation.
fn missing_mutation_parents(target: &Path) -> Result<Vec<PathBuf>, String> {
    let mut missing = Vec::new();
    let mut cursor = target.parent();
    while let Some(path) = cursor {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => break,
            Ok(_) => return Err(format!("Mutation parent is not a regular directory: {}",path.display())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => missing.push(path.to_path_buf()),
            Err(error) => return Err(format!("Inspect mutation parent failed: {error}")),
        }
        cursor = path.parent();
    }
    missing.reverse();
    Ok(missing)
}

fn created_directory_version(path: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Created directory changed its resource type".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(Some(format!("directory:v1:unix:{}:{}",metadata.dev(),metadata.ino())))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS,FILE_FLAG_OPEN_REPARSE_POINT};
        let directory = fs::OpenOptions::new().read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path).map_err(|error|error.to_string())?;
        let (volume,index) = mutation_handle_identity(&directory).map_err(|error|error.to_string())?;
        Ok(Some(format!("directory:v1:windows:{volume}:{index}")))
    }
    #[cfg(not(any(unix, windows)))]
    Err("Directory identity observation is unsupported on this platform".into())
}

/// Called only after all lock-local invocation/plan checks. Each successful
/// mkdir is already a physical effect even if observation or the move fails.
fn create_mutation_parents(target: &Path, cancellation: &RuntimeExecutionControl,
    locked_parents: &[PathBuf], outcome: &mut ToolOutcome) -> Result<(), String> {
    let parents = missing_mutation_parents(target)?;
    if parents.iter().any(|path| !locked_parents.contains(path)) {
        return Err("Mutation parent lock set changed; re-plan before creating any directory".into());
    }
    // Resolve every receipt URI before the first mkdir, not after an effect.
    let parents = parents.into_iter().map(|path| {
        let uri = url::Url::from_file_path(&path).map_err(|_| "Directory has no exact file URI".to_string())?;
        Ok((path,uri.to_string()))
    }).collect::<Result<Vec<_>,String>>()?;
    for (path,uri) in parents {
        if cancellation.is_requested() { return Err("Directory preparation cancelled".into()); }
        match fs::create_dir(&path) {
            Ok(()) => {
                let receipt = outcome.execution.get_or_insert_with(|| nola_host_protocol::tool_result::ExecutionReceipt {
                    local_stop: None,
                    state:nola_host_protocol::tool_result::ExecutionState::Completed,effects:Vec::new(),checks:Vec::new(),
                });
                receipt.effects.push(nola_host_protocol::tool_result::ResourceEffect {
                    resource_uri:uri,operation:"create_directory".into(),before_version:Some("absent:v1".into()),after_version:None,
                });
                let version = created_directory_version(&path)?
                    .ok_or_else(|| "Created directory disappeared before observation".to_string())?;
                receipt.effects.last_mut().expect("created directory effect").after_version = Some(version);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // Another creator owns this directory's effect. Accept only
                // a real directory at the exact entry, never follow a symlink.
                created_directory_version(&path)?
                    .ok_or_else(|| "Concurrent directory disappeared".to_string())?;
            }
            Err(error) => return Err(format!("Create mutation parent {} failed: {error}",path.display())),
        }
    }
    Ok(())
}

/// The caller already owns every affected path lock and has validated the
/// complete derived plan. Injecting the entry operation keeps I/O failure
/// tests at this real directory/effect boundary without bypassing admission.
fn commit_move_entry_with_parents(source: &Path, destination: &Path,
    source_before: &Option<FileMutationSnapshot>, destination_before: &Option<FileMutationSnapshot>,
    args: &Map<String,Value>, cancellation: &RuntimeExecutionControl, locked_parents: &[PathBuf],
    move_entry: impl FnOnce(&Path,&Path) -> io::Result<()>) -> ToolOutcome {
    let mut outcome = ok(format!("Move target: {} -> {}",source.display(),destination.display()),
        format!("Move {} -> {}",source.display(),destination.display()));
    let validate = || revalidate_file_mutation_target(source,source_before,args)
        .and_then(|_|revalidate_file_mutation_target(destination,destination_before,&Map::new()));
    if let Err(failure) = validate() {
        return partial_file_mutation_failure(outcome,&failure.text);
    }
    if let Err(message) = create_mutation_parents(destination,cancellation,locked_parents,&mut outcome) {
        return partial_file_mutation_failure(outcome,&message);
    }
    if cancellation.is_requested() {
        return partial_file_mutation_failure(outcome,"Move cancelled after directory preparation");
    }
    if let Err(failure) = validate() {
        return partial_file_mutation_failure(outcome,&format!("File phase refused after directory preparation: {}",failure.text));
    }
    if let Err(err) = move_entry(source,destination) {
        return partial_file_mutation_failure(outcome,&format!("Move failed without deleting the destination: {err}"));
    }
    forget_file_access_state(source);
    refresh_file_access_state(destination);
    outcome.text = format!("Moved {} -> {}",source.display(),destination.display());
    outcome.display = outcome.text.clone();
    outcome = with_mutation_artifact(outcome,source,"delete",mutation_version(source_before),"absent:v1".into());
    with_mutation_artifact(outcome,destination,"move",mutation_version(destination_before),mutation_version(source_before))
}

fn partial_file_mutation_failure(mut outcome: ToolOutcome, message: &str) -> ToolOutcome {
    if outcome.execution.as_ref().is_none_or(|receipt| receipt.effects.is_empty()) {
        let mut failure = error(format!("{message}\ntool_execution_skipped: true"));
        failure.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
            local_stop: None,
            state:nola_host_protocol::tool_result::ExecutionState::NotStarted,effects:Vec::new(),checks:Vec::new(),
        });
        return failure;
    }
    if let Some(receipt) = &mut outcome.execution {
        receipt.state = nola_host_protocol::tool_result::ExecutionState::Partial;
    }
    outcome.error = true;
    outcome.text.push_str(&format!("\npartial_execution: true\n{message}\nDo not replay the mutation; inspect the committed effect receipts before continuing."));
    outcome.display = "File operation partially completed; recovery requires its exact effect receipts".into();
    outcome
}

/// A formatter can fail after changing a file. Observe its actual final state;
/// an unreadable result is unknown, never the planned or original body digest.
fn refresh_mutation_effect_versions(outcome: &mut ToolOutcome) -> bool {
    let Some(receipt) = &mut outcome.execution else { return false; };
    let mut complete = true;
    for effect in &mut receipt.effects {
        let path = url::Url::parse(&effect.resource_uri).ok().and_then(|uri|uri.to_file_path().ok());
        if effect.operation == "create_directory" {
            let observed = path.and_then(|path|created_directory_version(&path).ok());
            complete &= observed.as_ref().is_some_and(Option::is_some);
            effect.after_version = observed.map(|version|version.unwrap_or_else(||"absent:v1".into()));
            continue;
        }
        let observed = path.and_then(|path| observe_file_mutation_target(&path).ok());
        complete &= observed.as_ref().is_some_and(Option::is_some);
        effect.after_version = observed.as_ref().map(mutation_version);
    }
    complete
}

fn capture_exact_mutation_preimage(path: &Path, original: &str) -> Result<FileMutationVersion, String> {
    let observed = observe_file_mutation_target(path).map_err(|outcome| outcome.text)?
        .ok_or("Exact edit resource disappeared")?;
    if observed.version.sha256 != sha256_hex(original.as_bytes()) {
        return Err("Exact edit resource changed during planning".into());
    }
    Ok(observed.version)
}

/// Exact preimages do not require a redundant model full read. Every concrete
/// file still has a fresh identity/version and lock-local OCC before any edit.
fn commit_exact_file_batch(root: &Path, updates: &[(PathBuf, String, usize)],
    versions: &HashMap<PathBuf, FileMutationVersion>, args: &Map<String, Value>, ctx: &ToolContext,
    format: bool) -> (ToolOutcome, Vec<String>) {
    let paths = updates.iter().map(|(path, _, _)| path.clone()).collect::<Vec<_>>();
    let mut sections = Vec::new();
    let outcome = with_file_write_locks(&paths, || {
        for path in &paths {
            tool_try!(check_expected_sha256_by_path(root, path, args));
            let current = tool_try!(observe_file_mutation_target(path));
            if current.as_ref().map(|value| &value.version) != versions.get(path) {
                return error(format!("Exact edit preimage changed before commit: {}", path.display()));
            }
        }
        let mut outcome = ok(String::new(), String::new());
        for (path, updated, _) in updates {
            if ctx.cancellation.is_requested() {
                return partial_file_mutation_failure(outcome, "Exact edit batch cancelled; already recorded effects were not rolled back");
            }
            // External writers do not acquire these process-local locks.
            let current = match observe_file_mutation_target(path) {
                Ok(Some(value)) if Some(&value.version) == versions.get(path) => value,
                _ => return partial_file_mutation_failure(outcome, "Exact edit preimage changed during the batch; no unknown action was replayed"),
            };
            if let Err(err) = atomic_write_text_file(path, updated) {
                return partial_file_mutation_failure(outcome, &format!("Exact edit failed: {err}; prior effects were not rolled back"));
            }
            refresh_file_access_state(path);
            outcome = with_mutation_artifact(outcome, path, "edit", format!("sha256:{}", current.version.sha256),
                format!("sha256:{}", sha256_hex(updated.as_bytes())));
        }
        if format {
            match formatting_sections(root, &paths, &ctx.cancellation,ctx.policy.process_sandbox()) {
                Ok(value) => sections = value,
                Err(failure) => {
                    refresh_mutation_effect_versions(&mut outcome);
                    return partial_file_mutation_failure(outcome, &failure.text);
                }
            }
            for path in &paths { refresh_file_access_state(path); }
            if !refresh_mutation_effect_versions(&mut outcome) {
                return partial_file_mutation_failure(outcome, "Formatted resource could not be observed; prior effects remain recorded with unknown versions where necessary");
            }
        }
        outcome
    });
    (outcome, sections)
}

fn move_import_candidates(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut stack = vec![root.to_path_buf()];
    let mut candidates = Vec::new();
    let mut inspected = 0usize;
    while let Some(directory) = stack.pop() {
        let entries = fs::read_dir(&directory).map_err(|err| format!("Cannot inspect import directory: {err}"))?;
        for entry in entries {
            let entry = entry.map_err(|err| err.to_string())?;
            let kind = entry.file_type().map_err(|err| err.to_string())?;
            // A collection never follows a symlink into another resource tree.
            if kind.is_symlink() { continue; }
            if kind.is_dir() {
                if !should_skip_dir(&entry.file_name().to_string_lossy()) { stack.push(entry.path()); }
            } else if kind.is_file() {
                inspected += 1;
                if inspected > MAX_PROJECT_SCAN_FILES as usize { return Err("Import discovery capacity exceeded".into()); }
                candidates.push(entry.path());
            }
        }
    }
    candidates.sort();
    Ok(candidates)
}

struct MoveImportPlan {
    root: PathBuf,
    updates: Vec<MoveImportUpdate>,
    scanned: usize,
}

fn move_import_plan(ctx: &ToolContext, input: &Value) -> Result<MoveImportPlan, String> {
    let resolve = |raw: &str| {
        let path = expand_home(Path::new(raw));
        let path = if path.is_absolute() { path } else {
            ctx.allowed_dirs.first().ok_or("No import resolution base")?.join(path)
        };
        resolve_missing_path(&path).ok_or_else(|| "Cannot resolve import resource".to_string())
    };
    let source = resolve(input.get("source").and_then(Value::as_str).ok_or("Missing move source")?)?;
    let destination = resolve(input.get("destination").and_then(Value::as_str).ok_or("Missing move destination")?)?;
    if input.get("update_imports").and_then(Value::as_bool) != Some(true) {
        let root = resolve(input.get("root").and_then(Value::as_str).filter(|root| !root.trim().is_empty()).unwrap_or("."))?;
        if !root.is_dir() { return Err("Import discovery boundary is not a directory".into()); }
        return Ok(MoveImportPlan { root, updates: Vec::new(), scanned: 0 });
    }
    let root = if let Some(root) = input.get("root").and_then(Value::as_str).filter(|root| !root.trim().is_empty()) {
        resolve(root)?
    } else {
        ctx.allowed_dirs.iter().filter_map(|root| root.canonicalize().ok())
            .filter(|root| source.starts_with(root) && destination.starts_with(root))
            .max_by_key(|root| root.components().count()).ok_or("Move import root does not contain both resources")?
    };
    if !root.is_dir() || !source.starts_with(&root) || !destination.starts_with(&root) {
        return Err("Move import root does not contain both resources".into());
    }
    let max = input.get("max_import_updates").and_then(Value::as_u64).unwrap_or(100);
    if max > MAX_DEPENDENCY_IMPORTS as u64 { return Err("Import member capacity exceeds the declared limit".into()); }
    let (updates, scanned) = move_file_import_updates(&root, &source, &destination, max as usize)?;
    Ok(MoveImportPlan { root, updates, scanned })
}

#[cfg(test)]
mod file_mutation_semantics_tests {
    use super::*;

    fn context(root: &Path) -> ToolContext {
        ToolContext {memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),language_services: Default::default(), skills:Default::default(),allowed_dirs:vec![root.to_path_buf()],
            policy:ToolExecutionPolicy::new(Vec::new(),false,"ask".into(),Vec::new()),
            permission_scope:Default::default(),cancellation:Default::default(),plan_state:Default::default(),
            output_store:ToolOutputStore::for_test(),browser:BrowserRuntime::new(root.join("browser")) }
    }

    struct IsolatedDatabase(Option<std::ffi::OsString>);
    impl IsolatedDatabase {
        fn new(root: &Path) -> Self {
            let before = std::env::var_os("NOLA_DB_PATH");
            std::env::set_var("NOLA_DB_PATH",root.join("state/nola.db"));
            Self(before)
        }
    }
    impl Drop for IsolatedDatabase {
        fn drop(&mut self) {
            match self.0.take() { Some(value) => std::env::set_var("NOLA_DB_PATH",value), None => std::env::remove_var("NOLA_DB_PATH") }
        }
    }

    fn import_fixture(root: &Path) -> (Value, PathBuf, PathBuf, PathBuf) {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("lib")).unwrap();
        let source = root.join("src/util.ts");
        let destination = root.join("lib/util.ts");
        let importer = root.join("src/app.ts");
        fs::write(&source,"export const util = 1\n").unwrap();
        fs::write(&importer,"import { util } from './util'\n").unwrap();
        (json!({"source":source,"destination":destination,"root":root,"update_imports":true}),source,destination,importer)
    }

    fn resolved_import_guard(ctx: &ToolContext,input: &Value) -> permission_resolution::PermissionExecutionExpectationGuard {
        let request = ToolRequest { id:"fixture-import-admission".into(),name:"move_file".into(),args:input.as_object().unwrap().clone() };
        let spec = specs_with_skills(&ctx.skills).into_iter().find(|spec| spec.name == "move_file").unwrap();
        let resources = permission_resolution::LocalPermissionResources::prepare(ctx,&spec,&request).unwrap();
        enter_permission_resource_expectations(resources.revalidate(ctx,input).unwrap())
    }

    #[test]
    fn write_creation_cannot_silently_become_replacement() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let ctx = context(&root);
        let path = root.join("note.txt");
        let args = json!({"path":path,"content":"first"});
        assert!(!write_file(args.as_object().unwrap(),&ctx).error);
        let replacement = json!({"path":path,"content":"second"});
        let refused = write_file(replacement.as_object().unwrap(),&ctx);
        assert!(refused.error);
        assert_eq!(refused.data.unwrap()["code"],"existing_file_version_required");
        assert_eq!(fs::read_to_string(&path).unwrap(),"first");
        let admitted = json!({"path":path,"content":"second","expected_sha256":sha256_hex(b"first")});
        assert!(!write_file(admitted.as_object().unwrap(),&ctx).error);
        assert_eq!(fs::read_to_string(path).unwrap(),"second");
    }

    #[test]
    fn complete_replacement_accepts_binary_preimage_without_read_credit() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let ctx = context(&root);
        let _scope = ctx.permission_scope.enter();
        let path = root.join("old.bin");
        let original = b"%PDF-1.7\n\xff\x00\xfe";
        fs::write(&path,original).unwrap();
        let rejected = write_file(json!({"path":path,"content":"not admitted","expected_sha256":"0".repeat(64)}).as_object().unwrap(),&ctx);
        assert!(rejected.error); assert_eq!(fs::read(&path).unwrap(),original);
        let args = json!({"path":path,"content":"complete replacement\n","expected_sha256":sha256_hex(original),"dry_run":true});
        let preview = write_file(args.as_object().unwrap(),&ctx);
        assert!(!preview.error,"{}",preview.text);
        assert!(preview.text.contains("binary"));
        assert_eq!(fs::read(&path).unwrap(),original);
        let mut args = args.as_object().unwrap().clone(); args.insert("dry_run".into(),json!(false));
        let result = write_file(&args,&ctx);
        assert!(!result.error,"{}",result.text);
        assert_eq!(fs::read(&path).unwrap(),b"complete replacement\n");
        let effect = &result.execution.as_ref().unwrap().effects[0];
        assert_eq!(effect.before_version,Some(format!("sha256:{}",sha256_hex(original))));
        assert_eq!(effect.after_version,Some(format!("sha256:{}",sha256_hex(b"complete replacement\n"))));
        assert!(existing_file_access_snapshot(&path,&Map::new()).is_err(),"a replacement is not a read observation");
    }

    #[test]
    fn post_write_failure_retains_the_real_effect_and_observed_final_version() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("written.txt");
        fs::write(&path,"original\n").unwrap();
        let before = observe_file_mutation_target(&path).unwrap();
        atomic_write_text_file(&path,"replacement\n").unwrap();
        let mut outcome = with_mutation_artifact(ok("Written".into(),"Written".into()),&path,"write",
            mutation_version(&before),format!("sha256:{}",sha256_hex(b"replacement\n")));
        // A failed formatter may already have changed the body. Its failure
        // must not erase the preceding write or report the planned digest.
        fs::write(&path,"formatter changed this before failing\n").unwrap();
        assert!(refresh_mutation_effect_versions(&mut outcome));
        let failure = partial_file_mutation_failure(outcome,"Formatter failed after changing the body");
        assert!(failure.error); assert!(failure.text.contains("partial_execution: true"));
        assert!(!failure.text.contains("tool_execution_skipped"));
        assert_eq!(failure.artifacts.len(),1);
        let receipt = failure.execution.as_ref().unwrap(); receipt.validate().unwrap();
        assert_eq!(receipt.state,nola_host_protocol::tool_result::ExecutionState::Partial);
        assert_eq!(receipt.effects.len(),1);
        assert_eq!(receipt.effects[0].before_version,Some(format!("sha256:{}",sha256_hex(b"original\n"))));
        assert_eq!(receipt.effects[0].after_version,Some(format!("sha256:{}",sha256_hex(b"formatter changed this before failing\n"))));
        assert_eq!(fs::read(&path).unwrap(),b"formatter changed this before failing\n");
    }

    #[test]
    fn post_write_unobservable_version_is_unknown_not_a_fabricated_old_or_planned_digest() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("written.txt");
        atomic_write_text_file(&path,"replacement\n").unwrap();
        let mut outcome = with_mutation_artifact(ok("Written".into(),"Written".into()),&path,"write",
            "absent:v1".into(),format!("sha256:{}",sha256_hex(b"replacement\n")));
        // A non-regular replacement makes the final content unobservable to
        // this observer, without relying on root-sensitive mode-bit denial.
        fs::remove_file(&path).unwrap(); fs::create_dir(&path).unwrap();
        assert!(!refresh_mutation_effect_versions(&mut outcome));
        let failure = partial_file_mutation_failure(outcome,"Written resource cannot be observed");
        assert!(failure.error); assert!(!failure.text.contains("tool_execution_skipped"));
        assert_eq!(failure.artifacts.len(),1);
        let receipt = failure.execution.as_ref().unwrap(); receipt.validate().unwrap();
        assert_eq!(receipt.effects.len(),1);
        assert_eq!(receipt.effects[0].before_version.as_deref(),Some("absent:v1"));
        assert_eq!(receipt.effects[0].after_version,None);
        assert!(path.is_dir(),"failure handling must not replay or remove the changed resource");
        fs::remove_dir(&path).unwrap();
        let mut absent = failure;
        assert!(!refresh_mutation_effect_versions(&mut absent));
        assert_eq!(absent.execution.as_ref().unwrap().effects[0].after_version.as_deref(),Some("absent:v1"),
            "a known missing resource differs from an unknown final version");
    }

    #[test]
    fn binary_move_then_delete_require_identity_not_utf8_full_read() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let ctx = context(&root);
        let _scope = ctx.permission_scope.enter();
        let source = root.join("image.bin"); let destination = root.join("moved.bin");
        let original = b"\x89PNG\xff\x00\xfe";
        fs::write(&source,original).unwrap();
        let args = json!({"source":source,"destination":destination,"expected_sha256":"0".repeat(64)});
        assert!(move_file(args.as_object().unwrap(),&ctx).error);
        assert!(source.exists()); assert!(!destination.exists());
        let args = json!({"source":source,"destination":destination,"expected_sha256":sha256_hex(original)});
        let moved = move_file(args.as_object().unwrap(),&ctx);
        assert!(!moved.error,"{}",moved.text);
        assert!(!source.exists()); assert_eq!(fs::read(&destination).unwrap(),original);
        assert_eq!(moved.artifacts.len(),2);
        assert_eq!(moved.execution.as_ref().unwrap().effects.len(),2);
        let wrong = delete_file(json!({"path":destination,"expected_sha256":"0".repeat(64)}).as_object().unwrap(),&ctx);
        assert!(wrong.error); assert_eq!(fs::read(&destination).unwrap(),original);
        let deleted = delete_file(json!({"path":destination,"expected_sha256":sha256_hex(original)}).as_object().unwrap(),&ctx);
        assert!(!deleted.error,"{}",deleted.text);
        assert!(!destination.exists());
        assert_eq!(deleted.execution.as_ref().unwrap().effects[0].before_version,Some(format!("sha256:{}",sha256_hex(original))));
        assert_eq!(deleted.execution.as_ref().unwrap().effects[0].after_version.as_deref(),Some("absent:v1"));
    }

    #[test]
    fn fresh_version_and_expected_digest_are_checked_inside_the_mutation_lock() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("raced.bin");
        fs::write(&path,b"old\xff").unwrap();
        let before = observe_file_mutation_target(&path).unwrap();
        let args = json!({"expected_sha256":sha256_hex(b"old\xff")});
        let result = with_file_write_lock(&path,|| {
            // Simulates an external writer, which does not share our mutex.
            fs::write(&path,b"new\xfe").unwrap();
            match revalidate_file_mutation_target(&path,&before,args.as_object().unwrap()) {
                Err(outcome) => outcome, Ok(_) => panic!("stale mutation was admitted"),
            }
        });
        assert!(result.error); assert_eq!(fs::read(&path).unwrap(),b"new\xfe");
        assert!(revalidate_file_mutation_target(&path,&before,&Map::new()).is_err(),"OCC is required even without expected_sha256");
        assert!(validate_mutation_expectation(&None,args.as_object().unwrap()).is_err(),"a missing file cannot satisfy a content digest");
    }

    #[cfg(any(target_os="macos",target_os="linux",windows))]
    #[test]
    fn atomic_noclobber_rejects_a_destination_created_after_preflight() {
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source"); let destination = fixture.path().join("destination");
        fs::write(&source,b"source\xff").unwrap();
        assert!(observe_file_mutation_target(&destination).unwrap().is_none());
        fs::write(&destination,b"late owner\xfe").unwrap();
        assert!(atomic_move_file(&source,&destination,false).is_err());
        assert_eq!(fs::read(&source).unwrap(),b"source\xff");
        assert_eq!(fs::read(&destination).unwrap(),b"late owner\xfe");
        atomic_move_file(&source,&destination,true).unwrap();
        assert!(!source.exists()); assert_eq!(fs::read(&destination).unwrap(),b"source\xff");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_noclobber_does_not_follow_a_late_destination_symlink() {
        use std::os::unix::fs::symlink;
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source"); let destination = fixture.path().join("destination");
        let private = fixture.path().join("private");
        fs::write(&source,b"source").unwrap(); fs::write(&private,b"private").unwrap();
        symlink(&private,&destination).unwrap();
        assert!(atomic_move_file(&source,&destination,false).is_err());
        assert!(source.exists()); assert!(fs::symlink_metadata(destination).unwrap().is_symlink());
        assert_eq!(fs::read(private).unwrap(),b"private");
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn move_does_not_claim_effects_for_same_inode_hardlink_noop() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let source = root.join("source"); let destination = root.join("alias");
        fs::write(&source,b"same inode").unwrap(); fs::hard_link(&source,&destination).unwrap();
        let result = move_file(json!({"source":source,"destination":destination,"overwrite":true}).as_object().unwrap(),&context(&root));
        assert!(result.error); assert!(source.exists()); assert!(destination.exists()); assert!(result.execution.is_none());
    }

    #[test]
    fn import_plan_requires_frozen_exact_members_and_versions_before_move() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let ctx = context(&root); let (input,source,destination,importer) = import_fixture(&root);
        let _guard = resolved_import_guard(&ctx,&input);
        fs::write(&importer,"import { util } from './util'\n// external change\n").unwrap();
        let result = move_file(input.as_object().unwrap(),&ctx);
        assert!(result.error); assert!(source.exists()); assert!(!destination.exists());
        assert!(fs::read_to_string(importer).unwrap().contains("external change"));
    }

    #[test]
    fn newly_discovered_importer_cannot_use_the_old_collection_approval() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let ctx = context(&root); let (input,source,destination,_) = import_fixture(&root);
        let _guard = resolved_import_guard(&ctx,&input);
        fs::write(root.join("src/later.ts"),"import { util } from './util'\n").unwrap();
        let result = move_file(input.as_object().unwrap(),&ctx);
        assert!(result.error); assert!(source.exists()); assert!(!destination.exists());
        assert_eq!(fs::read_to_string(root.join("src/later.ts")).unwrap(),"import { util } from './util'\n");
    }

    #[test]
    fn changed_import_plan_refuses_before_creating_any_destination_parent() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let ctx = context(&root); let (input,source,destination,importer) = import_fixture(&root);
        fs::remove_dir(root.join("lib")).unwrap();
        let _guard = resolved_import_guard(&ctx,&input);
        fs::write(&importer,"import { util } from './util'\n// new unapproved preimage\n").unwrap();
        let result = move_file(input.as_object().unwrap(),&ctx);
        assert!(result.error); assert!(source.exists()); assert!(!destination.exists());
        assert!(!root.join("lib").exists(),"a rejected plan must have zero mkdir effects");
        assert!(result.execution.as_ref().is_none_or(|receipt|receipt.effects.is_empty()));
        assert!(result.artifacts.is_empty());
    }

    #[test]
    fn lock_local_move_occ_refuses_before_creating_any_destination_parent() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let source = root.join("source.bin"); let destination = root.join("new/nested/target.bin");
        fs::write(&source,b"original\xff").unwrap();
        let before = observe_file_mutation_target(&source).unwrap();
        fs::write(&source,b"external\xfe").unwrap();
        let mut paths = missing_mutation_parents(&destination).unwrap();
        paths.extend([source.clone(),destination.clone()]);
        let result = with_file_write_locks(&paths,|| commit_move_entry_with_parents(&source,&destination,
            &before,&None,&Map::new(),&RuntimeExecutionControl::default(),&paths,|_,_|panic!("stale move was dispatched")));
        assert!(result.error); assert!(!root.join("new").exists());
        assert_eq!(fs::read(&source).unwrap(),b"external\xfe");
        let receipt = result.execution.unwrap(); receipt.validate().unwrap();
        assert_eq!(receipt.state,nola_host_protocol::tool_result::ExecutionState::NotStarted);
        assert!(receipt.effects.is_empty());
    }

    #[test]
    fn entry_failure_after_mkdir_retains_each_actual_directory_as_partial_effect() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let source = root.join("source.bin"); let destination = root.join("new/nested/target.bin");
        fs::write(&source,b"unchanged\xff").unwrap();
        let before = observe_file_mutation_target(&source).unwrap();
        let mut paths = missing_mutation_parents(&destination).unwrap();
        paths.extend([source.clone(),destination.clone()]);
        // Deterministic I/O failure at the production rename boundary (such
        // as cross-device rename), after the real directory syscalls.
        let result = with_file_write_locks(&paths,|| commit_move_entry_with_parents(&source,&destination,
            &before,&None,&Map::new(),&RuntimeExecutionControl::default(),&paths,|_,destination| {
                assert!(destination.parent().unwrap().is_dir());
                Err(io::Error::other("injected entry operation failure"))
            }));
        assert!(result.error); assert!(!result.text.contains("tool_execution_skipped"));
        assert_eq!(fs::read(&source).unwrap(),b"unchanged\xff"); assert!(!destination.exists());
        assert!(root.join("new/nested").is_dir(),"recovery must not silently remove committed directories");
        assert!(result.artifacts.is_empty(),"directories must not masquerade as completed file artifacts");
        let receipt = result.execution.unwrap(); receipt.validate().unwrap();
        assert_eq!(receipt.state,nola_host_protocol::tool_result::ExecutionState::Partial);
        assert_eq!(receipt.effects.len(),2);
        for (effect,path) in receipt.effects.iter().zip([root.join("new"),root.join("new/nested")]) {
            assert_eq!(effect.operation,"create_directory");
            assert_eq!(effect.resource_uri,url::Url::from_file_path(&path).unwrap().as_str());
            assert_eq!(effect.before_version.as_deref(),Some("absent:v1"));
            assert_eq!(effect.after_version,created_directory_version(&path).unwrap());
        }
    }

    #[test]
    fn write_and_create_append_keep_automatic_parents_and_exact_directory_receipts() {
        for append in [false,true] {
            let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
            let ctx = context(&root); let target = root.join("new/nested/output.txt");
            let mut args = json!({"path":target,"content":"actual payload\n"});
            if append { args["create"] = json!(true); }
            let result = if append { append_file(args.as_object().unwrap(),&ctx) }
                else { write_file(args.as_object().unwrap(),&ctx) };
            assert!(!result.error,"{}",result.text); assert_eq!(fs::read(&target).unwrap(),b"actual payload\n");
            assert_eq!(result.artifacts.len(),1);
            let receipt = result.execution.unwrap(); receipt.validate().unwrap();
            assert_eq!(receipt.state,nola_host_protocol::tool_result::ExecutionState::Completed);
            assert_eq!(receipt.effects.len(),3);
            assert_eq!(receipt.effects[0].operation,"create_directory");
            assert_eq!(receipt.effects[1].operation,"create_directory");
            assert_eq!(receipt.effects[2].operation,if append {"append"} else {"write"});
            assert_eq!(receipt.effects[2].before_version.as_deref(),Some("absent:v1"));
            assert_eq!(receipt.effects[2].after_version,Some(format!("sha256:{}",sha256_hex(b"actual payload\n"))));
            for (effect,path) in receipt.effects.iter().zip([root.join("new"),root.join("new/nested")]) {
                assert_eq!(effect.after_version,created_directory_version(&path).unwrap());
            }
        }
    }

    #[test]
    fn rejected_write_version_or_cancelled_creation_never_creates_parent_directories() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let ctx = context(&root); let path = root.join("new/nested/output.txt");
        let result = write_file(json!({"path":path,"content":"no","expected_sha256":"a".repeat(64)}).as_object().unwrap(),&ctx);
        assert!(result.error); assert!(!root.join("new").exists());
        ctx.cancellation.request();
        for append in [false,true] {
            let mut args = json!({"path":path,"content":"no"});
            if append { args["create"] = json!(true); }
            let result = if append { append_file(args.as_object().unwrap(),&ctx) }
                else { write_file(args.as_object().unwrap(),&ctx) };
            assert!(result.error); assert!(!root.join("new").exists());
            assert!(result.execution.as_ref().is_none_or(|receipt|receipt.effects.is_empty()));
        }
    }

    #[test]
    fn disappearing_parent_cannot_expand_the_frozen_directory_lock_set() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let source = root.join("source.bin"); let existing = root.join("existing");
        let destination = existing.join("new/target.bin");
        fs::write(&source,b"preserved").unwrap(); fs::create_dir(&existing).unwrap();
        let before = observe_file_mutation_target(&source).unwrap();
        let parents = missing_mutation_parents(&destination).unwrap();
        assert_eq!(parents,vec![existing.join("new")]);
        let mut locks = parents.clone(); locks.extend([source.clone(),destination.clone()]);
        fs::remove_dir(&existing).unwrap();
        let result = with_file_write_locks(&locks,|| commit_move_entry_with_parents(&source,&destination,
            &before,&None,&Map::new(),&RuntimeExecutionControl::default(),&parents,|_,_|panic!("unlocked ancestor mutation was admitted")));
        assert!(result.error); assert!(!existing.exists(),"re-planning must precede any mkdir");
        assert_eq!(fs::read(source).unwrap(),b"preserved");
        let receipt = result.execution.unwrap(); receipt.validate().unwrap();
        assert_eq!(receipt.state,nola_host_protocol::tool_result::ExecutionState::NotStarted);
        assert!(receipt.effects.is_empty());
    }

    #[test]
    fn import_discovery_capacity_or_parse_failure_never_returns_a_truncated_empty_plan() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let ctx = context(&root); let (mut input,source,destination,_) = import_fixture(&root);
        input["max_import_updates"] = json!(0);
        assert!(move_import_plan(&ctx,&input).is_err());
        input["max_import_updates"] = json!(100);
        fs::write(root.join("src/unreadable.ts"),b"\xff\xfe").unwrap();
        assert!(move_import_plan(&ctx,&input).is_err());
        assert!(source.exists()); assert!(!destination.exists());
    }

    #[test]
    fn all_profile_with_multiple_derived_explicit_asks_performs_zero_nola_asks() {
        let _environment_lock = crate::store::test_env_lock();
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let _database = IsolatedDatabase::new(&root);
        let mut ctx = context(&root); let (input,source,destination,importer) = import_fixture(&root);
        let second = root.join("src/second.ts"); fs::write(&second,"import { util } from './util'\n").unwrap();
        ctx.permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        ctx.policy = ToolExecutionPolicy::new(Vec::new(),false,"ask".into(),vec![crate::runtime_config::PermissionRule {
            permission:"edit".into(),pattern:"*".into(),action:crate::runtime_config::PermissionAction::Ask }]);
        let asks = Arc::new(AtomicU64::new(0)); let count = Arc::clone(&asks);
        let _resolver = enter_host_permission_resolver(Arc::new(move |_,_,_| {
            count.fetch_add(1,Ordering::SeqCst); HostPermissionResolution::Deny
        }));
        let result = execute_tool(&ToolRequest {id:"all-import-explicit-asks".into(),name:"move_file".into(),
            args:input.as_object().unwrap().clone()},&ctx);
        assert!(!result.error,"{}",result.text); assert_eq!(asks.load(Ordering::SeqCst),0);
        assert!(!source.exists()); assert!(destination.exists());
        for path in [&importer,&second] { assert!(fs::read_to_string(path).unwrap().contains("../lib/util")); }
        let receipt = result.execution.as_ref().unwrap(); receipt.validate().unwrap();
        assert_eq!(receipt.effects.len(),4);
        assert_eq!(receipt.effects.iter().filter(|effect|effect.operation=="edit").count(),2);
        assert!(receipt.effects.iter().all(|effect| effect.before_version.is_some() && effect.after_version.is_some()));
    }

    #[test]
    fn explicit_importer_deny_prevents_all_file_effects_under_both_profiles() {
        let _environment_lock = crate::store::test_env_lock();
        for profile in [crate::permissions::PermissionProfile::Cautious,crate::permissions::PermissionProfile::AllowAll] {
            let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
            let _database = IsolatedDatabase::new(&root);
            let mut ctx = context(&root); let (input,source,destination,importer) = import_fixture(&root);
            ctx.permission_scope.configure_profile(profile);
            ctx.policy = ToolExecutionPolicy::new(Vec::new(),true,"allow".into(),vec![
                crate::runtime_config::PermissionRule {permission:"edit".into(),pattern:"*".into(),action:crate::runtime_config::PermissionAction::Allow},
                crate::runtime_config::PermissionRule {permission:"edit".into(),pattern:importer.to_string_lossy().into_owned(),action:crate::runtime_config::PermissionAction::Deny},
            ]);
            let before = fs::read(&importer).unwrap();
            let result = execute_tool(&ToolRequest {id:"denied-importer".into(),name:"move_file".into(),args:input.as_object().unwrap().clone()},&ctx);
            assert!(result.error,"{profile:?}"); assert!(source.exists()); assert!(!destination.exists());
            assert_eq!(fs::read(importer).unwrap(),before); assert!(result.artifacts.is_empty());
            let receipt = result.execution.as_ref().expect("denial explicitly records that execution did not start");
            assert_eq!(receipt.state,nola_host_protocol::tool_result::ExecutionState::NotStarted);
            assert!(receipt.effects.is_empty());
            assert!(result.data.as_ref().unwrap()["next_action"].as_str().unwrap().contains("/allow"));
        }
    }

    #[test]
    fn cautious_single_destination_allow_cannot_authorize_unapproved_importers() {
        let _environment_lock = crate::store::test_env_lock();
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let _database = IsolatedDatabase::new(&root);
        let mut ctx = context(&root); let (input,source,destination,importer) = import_fixture(&root);
        ctx.permission_scope.configure_profile(crate::permissions::PermissionProfile::Cautious);
        ctx.policy = ToolExecutionPolicy::new(Vec::new(),false,"ask".into(),[&source,&destination].into_iter().map(|path|
            crate::runtime_config::PermissionRule {permission:"edit".into(),pattern:path.to_string_lossy().into_owned(),
                action:crate::runtime_config::PermissionAction::Allow}).collect());
        let asks = Arc::new(AtomicU64::new(0)); let count = Arc::clone(&asks);
        let _resolver = enter_host_permission_resolver(Arc::new(move |_,_,_| {
            count.fetch_add(1,Ordering::SeqCst); HostPermissionResolution::Deny
        }));
        let result = execute_tool(&ToolRequest {id:"not-all-members-approved".into(),name:"move_file".into(),
            args:input.as_object().unwrap().clone()},&ctx);
        assert!(result.error); assert_eq!(asks.load(Ordering::SeqCst),1);
        assert!(source.exists()); assert!(!destination.exists());
        assert_eq!(fs::read_to_string(importer).unwrap(),"import { util } from './util'\n");
    }

    #[test]
    fn whole_replacement_move_and_delete_never_override_explicit_deny() {
        let _environment_lock = crate::store::test_env_lock();
        for profile in [crate::permissions::PermissionProfile::Cautious,crate::permissions::PermissionProfile::AllowAll] {
            let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
            let _database = IsolatedDatabase::new(&root);
            let mut ctx = context(&root); let source = root.join("binary.bin"); let destination = root.join("other.bin");
            fs::write(&source,b"\xff\x00original").unwrap();
            ctx.permission_scope.configure_profile(profile);
            for (name,input) in [("write_file",json!({"path":source,"content":"replacement"})),
                ("move_file",json!({"source":source,"destination":destination})),("delete_file",json!({"path":source}))] {
                let domain = if name == "delete_file" {"delete_file"} else {"edit"};
                ctx.policy = ToolExecutionPolicy::new(Vec::new(),true,"allow".into(),vec![crate::runtime_config::PermissionRule {
                    permission:domain.into(),pattern:"*".into(),action:crate::runtime_config::PermissionAction::Deny }]);
                let result = execute_tool(&ToolRequest {id:format!("denied-{name}"),name:name.into(),args:input.as_object().unwrap().clone()},&ctx);
                assert!(result.error,"{profile:?} {name}"); assert!(result.artifacts.is_empty());
                assert_eq!(fs::read(&source).unwrap(),b"\xff\x00original"); assert!(!destination.exists());
            }
        }
    }

    #[test]
    fn edit_deny_does_not_impersonate_the_distinct_delete_file_domain() {
        let _environment_lock = crate::store::test_env_lock();
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap();
        let _database = IsolatedDatabase::new(&root);
        let mut ctx = context(&root); let source = root.join("binary.bin");
        fs::write(&source,b"\xff\x00original").unwrap();
        ctx.permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        ctx.policy = ToolExecutionPolicy::new(Vec::new(),true,"allow".into(),vec![crate::runtime_config::PermissionRule {
            permission:"edit".into(),pattern:"*".into(),action:crate::runtime_config::PermissionAction::Deny }]);
        let result = execute_tool(&ToolRequest {id:"distinct-delete-domain".into(),name:"delete_file".into(),
            args:json!({"path":source}).as_object().unwrap().clone()},&ctx);
        assert!(!result.error,"{}",result.text);
        assert!(!source.exists());
        assert!(!result.artifacts.is_empty());
    }

    #[test]
    fn exact_batch_rejects_changed_preimages_without_fabricating_read_coverage() {
        let fixture = tempfile::tempdir().unwrap(); let root = fixture.path().canonicalize().unwrap(); let ctx = context(&root);
        let _scope = ctx.permission_scope.enter(); let path = root.join("exact.txt"); fs::write(&path,"old\n").unwrap();
        let versions = HashMap::from([(path.clone(),capture_exact_mutation_preimage(&path,"old\n").unwrap())]);
        fs::write(&path,"external\n").unwrap();
        let (result,_) = commit_exact_file_batch(&root,&[(path.clone(),"new\n".into(),1)],&versions,&Map::new(),&ctx,false);
        assert!(result.error); assert!(result.artifacts.is_empty()); assert_eq!(fs::read_to_string(&path).unwrap(),"external\n");
        assert!(existing_file_access_snapshot(&path,&Map::new()).is_err());
    }
}
