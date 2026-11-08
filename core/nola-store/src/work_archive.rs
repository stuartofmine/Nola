// SPDX-License-Identifier: Apache-2.0
//! Committed work snapshots. Files are recovery data, never execution authority.
use crate::{NolaStore, StoreError, StoreResult};
use nola_store_port::run_effect::StoredRecoveryCheckpoint;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const FILES: &[&str] = &[
    "checkpoint.json",
    "goal.json",
    "goal.md",
    "tasks.json",
    "evidence.json",
    "compactions.json",
];
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkArchiveManifest {
    pub format_version: u32,
    pub conversation_id: String,
    pub revision: i64,
    pub event_cursor: i64,
    pub generation: String,
    pub previous_generation: Option<String>,
    pub files: BTreeMap<String, String>,
    /// Readable projections describe this baseline, not an unreplayed tail.
    pub projection_revision: i64,
    pub replay_pending: bool,
    pub task_state_sha256: String,
}

#[derive(Debug)]
pub struct VerifiedWorkArchive {
    pub manifest: WorkArchiveManifest,
    pub recovery: StoredRecoveryCheckpoint,
    pub files: BTreeMap<String, Vec<u8>>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn io(error: std::io::Error) -> StoreError {
    StoreError::Invariant(format!("work archive I/O: {error}"))
}
fn hex_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
fn plain(path: &Path, directory: bool) -> StoreResult<()> {
    let meta = fs::symlink_metadata(path).map_err(io)?;
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(StoreError::Invariant(
            "work archive path must not be a symlink or special file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(StoreError::Invariant(
                "work archive must be private to its owner".into(),
            ));
        }
    }
    Ok(())
}
fn directory(path: &Path) -> StoreResult<()> {
    if !path.try_exists().map_err(io)? {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    File::open(parent).map_err(io)?.sync_all().map_err(io)?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io(e)),
        }
    }
    plain(path, true)
}
fn write_new(path: &Path, bytes: &[u8]) -> StoreResult<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}
fn read(path: &Path) -> StoreResult<Vec<u8>> {
    plain(path, false)?;
    if fs::metadata(path).map_err(io)?.len() > MAX_FILE_BYTES {
        return Err(StoreError::Invariant(
            "work archive file exceeds limit".into(),
        ));
    }
    fs::read(path).map_err(io)
}

fn goal_markdown(goal: &Value, revision: &Value, replay_pending: bool) -> String {
    let active = &goal["active"];
    let mut text = format!("# Goal 工作记录\n\n自动生成的可读视图；编辑此文件不会改变运行状态或权限。\n\n基线版本：{revision}；待重放更新：{replay_pending}。\n\n## 目标\n\n{}\n\n## 用户约束\n\n",active["objective"].as_str().unwrap_or("无活动 Goal"));
    for constraint in active["constraints"].as_array().into_iter().flatten() {
        text.push_str(&format!("- {}\n", constraint.as_str().unwrap_or("")));
    }
    text.push_str("\n## 验收进度\n\n");
    if let Some(requirements) = active["requirements"].as_object() {
        for (id, requirement) in requirements {
            text.push_str(&format!(
                "- [{}] {} — {}\n",
                if requirement["status"] == "completed" {
                    "x"
                } else {
                    " "
                },
                id,
                requirement["description"].as_str().unwrap_or("")
            ));
        }
    }
    text.push_str("\n## 用户补充（原文）\n\n");
    for update in active
        .pointer("/work/user_updates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        text.push_str(&format!("- {}\n", update["content"].as_str().unwrap_or("")));
    }
    text.push_str("\n## 决策、进度、阻塞与下一步\n\n");
    let entries = active.pointer("/work/entries").and_then(Value::as_array);
    let replaced = entries
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["supersedes"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for entry in entries
        .into_iter()
        .flatten()
        .filter(|entry| !entry["id"].as_str().is_some_and(|id| replaced.contains(id)))
    {
        text.push_str(&format!(
            "- [{} / {}] {}\n",
            entry["origin"].as_str().unwrap_or("unknown"),
            entry["kind"].as_str().unwrap_or("unknown"),
            entry["content"].as_str().unwrap_or("")
        ));
    }
    text.push_str("\n完整历史与证据引用见 goal.json；重放记录见 checkpoint.json。模型报告的进度不等于已验证完成。\n");
    text
}

/// Checks every member before exposing any recovered state. No DB mutation,
/// permission restoration, worker restart, or Host effect is performed.
pub fn read_verified(directory: &Path) -> StoreResult<VerifiedWorkArchive> {
    let lock_path = directory.parent().map(|parent| parent.join("publish.lock"));
    let _lock = match lock_path.filter(|path| path.exists()) {
        Some(path) => {
            plain(&path, false)?;
            let file = File::open(path).map_err(io)?;
            file.lock_shared().map_err(io)?;
            Some(file)
        }
        None => None,
    };
    read_verified_unlocked(directory)
}

fn read_verified_unlocked(directory: &Path) -> StoreResult<VerifiedWorkArchive> {
    plain(directory, true)?;
    if directory.join("retired.json").exists() {
        return Err(StoreError::Invariant("archive owner retired".into()));
    }
    let manifest: WorkArchiveManifest =
        serde_json::from_slice(&read(&directory.join("manifest.json"))?)?;
    if manifest.format_version != 1
        || !hex_id(&manifest.generation)
        || manifest.files.len() != FILES.len()
        || FILES.iter().any(|name| !manifest.files.contains_key(*name))
    {
        return Err(StoreError::Invariant(
            "invalid work archive manifest".into(),
        ));
    }
    let generations = directory.join("generations");
    plain(&generations, true)?;
    let generation = generations.join(&manifest.generation);
    plain(&generation, true)?;
    let mut files = BTreeMap::new();
    for (name, checksum) in &manifest.files {
        let bytes = read(&generation.join(name))?;
        if digest(&bytes) != *checksum {
            return Err(StoreError::Invariant(format!(
                "work archive checksum mismatch: {name}"
            )));
        }
        files.insert(name.clone(), bytes);
    }
    if digest(&serde_json::to_vec(&manifest.files)?) != manifest.generation {
        return Err(StoreError::Invariant(
            "work archive generation mismatch".into(),
        ));
    }
    let recovery: StoredRecoveryCheckpoint = serde_json::from_slice(&files["checkpoint.json"])?;
    let baseline: Value = serde_json::from_str(&recovery.checkpoint.checkpoint_json)?;
    let tasks: Value = serde_json::from_slice(&files["tasks.json"])?;
    if recovery.checkpoint.conversation_id != manifest.conversation_id
        || recovery.checkpoint.revision != manifest.revision
        || recovery.checkpoint.event_cursor != manifest.event_cursor
        || manifest.replay_pending != !recovery.replay_inputs.is_empty()
        || manifest.projection_revision != baseline["revision"].as_i64().unwrap_or(0)
        || manifest.task_state_sha256 != digest(&serde_json::to_vec(&tasks["durable_tasks"])?)
    {
        return Err(StoreError::Invariant(
            "work archive identity mismatch".into(),
        ));
    }
    Ok(VerifiedWorkArchive {
        manifest,
        recovery,
        files,
    })
}

fn publish(directory: &Path, recovery: StoredRecoveryCheckpoint, tasks: Value) -> StoreResult<()> {
    let task_state_sha256 = digest(&serde_json::to_vec(&tasks)?);
    let baseline: Value = serde_json::from_str(&recovery.checkpoint.checkpoint_json)?;
    let state = &baseline["state"];
    let goal = &state["goal_projection"];
    let transcript = state
        .pointer("/conversation/transcript")
        .and_then(Value::as_array);
    let compactions = transcript
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("compaction"))
        .cloned()
        .collect::<Vec<_>>();
    let evidence = transcript.into_iter().flatten().filter(|item| {
        item.get("type").and_then(Value::as_str) == Some("tool_result") && item.get("is_error").and_then(Value::as_bool) == Some(false)
    }).map(|item| json!({"item_id":item["id"],"call_id":item["call_id"],"name":item["name"],"full_output_ref":item["full_output_ref"],"source":"checkpoint.json baseline transcript"})).collect::<Vec<_>>();
    let goal_json = serde_json::to_vec_pretty(goal)?;
    let mut files = BTreeMap::<String, Vec<u8>>::from([
        ("checkpoint.json".into(), serde_json::to_vec(&recovery)?),
        ("goal.json".into(), goal_json.clone()),
        (
            "goal.md".into(),
            goal_markdown(
                goal,
                &baseline["revision"],
                !recovery.replay_inputs.is_empty(),
            )
            .into_bytes(),
        ),
        (
            "tasks.json".into(),
            serde_json::to_vec_pretty(
                &json!({"projection":state["task_projection"],"durable_tasks":tasks}),
            )?,
        ),
        (
            "evidence.json".into(),
            serde_json::to_vec_pretty(&evidence)?,
        ),
        (
            "compactions.json".into(),
            serde_json::to_vec_pretty(&compactions)?,
        ),
    ]);
    if files
        .values()
        .any(|bytes| bytes.len() as u64 > MAX_FILE_BYTES)
    {
        return Err(StoreError::Invariant(
            "work archive exceeds file limit".into(),
        ));
    }
    let checksums: BTreeMap<String, String> = files
        .iter()
        .map(|(name, bytes)| (name.clone(), digest(bytes)))
        .collect();
    let generation = digest(&serde_json::to_vec(&checksums)?);
    directory_checked(directory)?;
    let previous = if directory.join("manifest.json").exists() && !directory.join("retired.json").exists() {
        Some(read_verified_unlocked(directory)?.manifest)
    } else {
        None
    };
    if previous
        .as_ref()
        .is_some_and(|previous| previous.generation == generation)
    {
        return Ok(());
    }
    let generations = directory.join("generations");
    directory_checked(&generations)?;
    let target = generations.join(&generation);
    if target.exists() {
        plain(&target, true)?;
        for (name, expected) in &checksums {
            if digest(&read(&target.join(name))?) != *expected {
                return Err(StoreError::Invariant(
                    "existing archive generation is corrupt".into(),
                ));
            }
        }
    } else {
        let staging = generations.join(format!("pending-{}", uuid::Uuid::new_v4()));
        directory_checked(&staging)?;
        for (name, bytes) in std::mem::take(&mut files) {
            write_new(&staging.join(name), &bytes)?;
        }
        File::open(&staging).map_err(io)?.sync_all().map_err(io)?;
        fs::rename(&staging, &target).map_err(io)?;
        File::open(&generations)
            .map_err(io)?
            .sync_all()
            .map_err(io)?;
    }
    let manifest = WorkArchiveManifest {
        format_version: 1,
        conversation_id: recovery.checkpoint.conversation_id,
        revision: recovery.checkpoint.revision,
        event_cursor: recovery.checkpoint.event_cursor,
        generation,
        previous_generation: previous.as_ref().map(|m| m.generation.clone()),
        files: checksums,
        projection_revision: baseline["revision"].as_i64().unwrap_or(0),
        replay_pending: !recovery.replay_inputs.is_empty(),
        task_state_sha256,
    };
    let temporary = directory.join(format!("manifest-{}.tmp", uuid::Uuid::new_v4()));
    write_new(&temporary, &serde_json::to_vec_pretty(&manifest)?)?;
    fs::rename(&temporary, directory.join("manifest.json")).map_err(io)?;
    if directory.join("retired.json").exists() {
        fs::remove_file(directory.join("retired.json")).map_err(io)?;
    }
    File::open(directory).map_err(io)?.sync_all().map_err(io)?;
    // Only our immutable generated members are collected; never recursive deletion.
    if let Some(old) = previous.and_then(|m| m.previous_generation).filter(|id| {
        hex_id(id)
            && *id != manifest.generation
            && Some(id) != manifest.previous_generation.as_ref()
    }) {
        let obsolete = generations.join(old);
        if plain(&obsolete, true).is_ok() {
            for name in FILES {
                let path = obsolete.join(name);
                if plain(&path, false).is_ok() {
                    let _ = fs::remove_file(path);
                }
            }
            let _ = fs::remove_dir(obsolete);
        }
    }
    Ok(())
}
fn directory_checked(path: &Path) -> StoreResult<()> {
    directory(path)
}

impl NolaStore {
    pub fn work_archive_status(&self, conversation_id: &str) -> StoreResult<Value> {
        let current = self.load_checkpoint(conversation_id)?;
        let publication=self.archive_publication_status()?;
        if current.is_none() {
            return Ok(json!({"available":false,"pending":false,"reason":"owner_checkpoint_absent","publication":publication}));
        }
        let Some(path) = self.work_archive_directory(conversation_id) else {
            return Ok(json!({"available":false,"reason":"in_memory_store","publication":publication}));
        };
        match read_verified(&path) {
            Ok(archive) => {
                let live = self
                    .list_all_runtime_tasks()?
                    .into_iter()
                    .filter(|task| task.owner_session_id.as_deref() == Some(conversation_id))
                    .collect::<Vec<_>>();
                let archived: Value = serde_json::from_slice(&archive.files["tasks.json"])?;
                let pending = current.as_ref().is_some_and(|current| {
                    current.revision != archive.manifest.revision
                        || current.event_cursor != archive.manifest.event_cursor
                }) || serde_json::to_value(live)? != archived["durable_tasks"];
                Ok(
                    json!({"available":true,"path":path,"revision":archive.manifest.revision,
                    "pending":pending,"replay_pending":archive.manifest.replay_pending,"publication":publication}),
                )
            }
            Err(_) => Ok(
                json!({"available":false,"pending":current.is_some(),"error_code":"archive_unavailable","publication":publication}),
            ),
        }
    }
    pub fn work_archive_root(&self) -> Option<PathBuf> {
        self.connection
            .path()
            .filter(|path| !path.is_empty() && *path != ":memory:")
            .map(|path| {
                let path = fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
                PathBuf::from(format!("{}.work", path.to_string_lossy()))
            })
    }
    pub fn work_archive_directory(&self, conversation_id: &str) -> Option<PathBuf> {
        self.work_archive_root()
            .map(|root| root.join(digest(conversation_id.as_bytes())))
    }
    /// Called only after an outer commit. A failed export never rewinds a
    /// committed DB transaction. Reopening or the next commit retries from DB.
    pub fn flush_work_archives(&self) -> StoreResult<()> {
        self.publish_work_archives(true)
    }
    pub(super) fn publish_pending_work_archives(&self)->StoreResult<()> {
        self.publish_work_archives(false)
    }
    fn publish_work_archives(&self, verify_existing:bool)->StoreResult<()> {
        if !self.connection.is_autocommit() {
            return Err(StoreError::Invariant("cannot archive uncommitted work".into()));
        }
        use crate::commit_diagnostics::{start,Outcome,Stage};
        let timer=start(Stage::PostCommitArchive);
        let status=self.archive_publication_status()?;
        if !verify_existing && status.target_generation==status.published_generation {
            timer.finish(Outcome::Skipped);
            return Ok(());
        }
        let result=self.flush_work_archives_impl(verify_existing);
        timer.finish(if result.is_ok(){Outcome::Succeeded}else{Outcome::Failed});
        if result.is_err() {self.archive_failure()?;}
        result
    }
    fn acknowledge_archive_generation(&self,generation:i64)->StoreResult<()> {
        self.connection.execute("UPDATE work_archive_publication SET published_generation=MAX(published_generation,?1),
            consecutive_failures=0,last_error_code=NULL,next_retry_at_ms=0,last_attempt_at_ms=?2,last_published_at_ms=?2
            WHERE singleton=1",rusqlite::params![generation,crate::work_archive_worker::now()])?;
        Ok(())
    }
    fn flush_work_archives_impl(&self, verify_existing: bool) -> StoreResult<()> {
        if !self.connection.is_autocommit() {
            return Err(StoreError::Invariant(
                "cannot archive uncommitted work".into(),
            ));
        }
        let Some(root) = self.work_archive_root() else {
            return Ok(());
        };
        // Read emptiness and its target in the same snapshot. A concurrent new
        // checkpoint must not be acknowledged without having been exported.
        let empty_snapshot=crate::read_snapshot::pin(&self.connection)?;
        let empty_generation=self.archive_publication_status()?.target_generation;
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM core_checkpoints)",
            [],
            |r| r.get(0),
        )?;
        if !exists && !root.exists() {
            drop(empty_snapshot);
            self.acknowledge_archive_generation(empty_generation)?;
            return Ok(());
        }
        drop(empty_snapshot);
        directory(&root)?;
        let lock_path = root.join("publish.lock");
        if !lock_path.exists() {
            match write_new(&lock_path, b"") {
                Ok(()) => {}
                Err(e) if !lock_path.exists() => return Err(e),
                Err(_) => {}
            }
        }
        plain(&lock_path, false)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(io)?;
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(5);
        loop {
            match lock.try_lock() {
                Ok(())=>break,
                Err(std::fs::TryLockError::WouldBlock) if verify_existing && std::time::Instant::now()<deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(std::fs::TryLockError::WouldBlock)=>return Err(StoreError::Invariant("archive publisher busy".into())),
                Err(std::fs::TryLockError::Error(error))=>return Err(io(error)),
            }
        }
        // Lock precedes snapshot acquisition, preventing a slow older writer
        // from publishing behind a newer process.
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let generation=self.archive_publication_status()?.target_generation;
        let mut query = self
            .connection
            .prepare("SELECT conversation_id,revision,event_cursor FROM core_checkpoints ORDER BY conversation_id")?;
        let ids = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let tasks = self.list_all_runtime_tasks()?;
        let live_ids=ids.iter().map(|(id,_,_)|digest(id.as_bytes())).collect::<std::collections::HashSet<_>>();
        // Retire only directories with an owned manifest. Keep all recovery
        // bytes; a durable tombstone makes old Goal snapshots non-restorable.
        for entry in fs::read_dir(&root).map_err(io)? {
            let entry=entry.map_err(io)?;
            let name=entry.file_name().to_string_lossy().into_owned();
            if !hex_id(&name) || live_ids.contains(&name) {continue;}
            let destination=entry.path();
            plain(&destination,true)?;
            if destination.join("retired.json").exists() {continue;}
            if !destination.join("manifest.json").exists() {continue;}
            let old=read_verified_unlocked(&destination)?;
            if digest(old.manifest.conversation_id.as_bytes())!=name {
                return Err(StoreError::Invariant("archive retired owner mismatch".into()));
            }
            let temporary=destination.join(format!("retire-{}.tmp",uuid::Uuid::new_v4()));
            write_new(&temporary,&serde_json::to_vec(&json!({"conversation_id":old.manifest.conversation_id,
                "retired_at_generation":generation}))?)?;
            fs::rename(temporary,destination.join("retired.json")).map_err(io)?;
            File::open(&destination).map_err(io)?.sync_all().map_err(io)?;
        }
        for (id, revision, event_cursor) in ids {
            let owned = tasks
                .iter()
                .filter(|task| task.owner_session_id.as_deref() == Some(id.as_str()))
                .collect::<Vec<_>>();
            let owned = serde_json::to_value(owned)?;
            let destination = root.join(digest(id.as_bytes()));
            if !verify_existing && !destination.join("retired.json").exists() {
                if let Ok(bytes) = read(&destination.join("manifest.json")) {
                    if let Ok(manifest) = serde_json::from_slice::<WorkArchiveManifest>(&bytes) {
                        if manifest.format_version == 1
                            && manifest.conversation_id == id
                            && manifest.revision == revision
                            && manifest.event_cursor == event_cursor
                            && manifest.task_state_sha256 == digest(&serde_json::to_vec(&owned)?)
                        {
                            continue;
                        }
                    }
                }
            }
            let Some(recovery) =
                crate::run_effect_port::read_recovery_checkpoint(&self.connection, &id)?
            else {
                continue;
            };
            publish(&destination, recovery, owned)?;
        }
        drop(query);
        drop(_snapshot);
        // Keep the publisher file lock through this durable acknowledgement.
        // Newer source writes may already exist; only this exact snapshot is published.
        self.acknowledge_archive_generation(generation)?;
        Ok(())
    }
}
