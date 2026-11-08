// SPDX-License-Identifier: Apache-2.0
//! Durable derived-archive backlog. The notification channel is only a wake hint.
use crate::{NolaStore, StoreError, StoreResult};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, OnceLock, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) fn initialize_schema(connection: &Connection) -> StoreResult<()> {
    connection.execute_batch("CREATE TABLE work_archive_publication (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        target_generation INTEGER NOT NULL CHECK(typeof(target_generation)='integer' AND target_generation>=0),
        published_generation INTEGER NOT NULL CHECK(published_generation>=0 AND published_generation<=target_generation),
        consecutive_failures INTEGER NOT NULL DEFAULT 0,
        last_error_code TEXT, next_retry_at_ms INTEGER NOT NULL DEFAULT 0,
        last_attempt_at_ms INTEGER, last_published_at_ms INTEGER
    );
    INSERT INTO work_archive_publication(singleton,target_generation,published_generation)
    VALUES(1,1,0);")?;
    // The global frontier deliberately trades per-conversation precision for one
    // durable scan target. Every bump is in the exact source write transaction.
    for table in [
        "core_checkpoints",
        "core_checkpoint_bodies",
        "durable_tasks",
        "host_event_replay_order",
    ] {
        for action in ["INSERT", "UPDATE", "DELETE"] {
            connection.execute_batch(&format!("CREATE TRIGGER archive_dirty_{table}_{action}
                AFTER {action} ON {table} BEGIN
                UPDATE work_archive_publication SET target_generation=target_generation+1 WHERE singleton=1;
                END;"))?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivePublicationStatus {
    pub target_generation: i64,
    pub published_generation: i64,
    pub consecutive_failures: i64,
    pub last_error_code: Option<String>,
    pub next_retry_at_ms: i64,
    pub last_attempt_at_ms: Option<i64>,
    pub last_published_at_ms: Option<i64>,
}
pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

impl NolaStore {
    pub fn archive_publication_status(&self) -> StoreResult<ArchivePublicationStatus> {
        self.connection.query_row("SELECT target_generation,published_generation,consecutive_failures,last_error_code,
            next_retry_at_ms,last_attempt_at_ms,last_published_at_ms FROM work_archive_publication WHERE singleton=1",[],|r|Ok(ArchivePublicationStatus {
                target_generation:r.get(0)?,published_generation:r.get(1)?,consecutive_failures:r.get(2)?,last_error_code:r.get(3)?,
                next_retry_at_ms:r.get(4)?,last_attempt_at_ms:r.get(5)?,last_published_at_ms:r.get(6)?,
            })).map_err(Into::into)
    }
    pub(super) fn archive_failure(&self) -> StoreResult<()> {
        // Fixed category, never persist filesystem paths or source contents.
        self.connection.execute(
            "UPDATE work_archive_publication SET consecutive_failures=consecutive_failures+1,
            last_error_code='archive_publication_failed',last_attempt_at_ms=?1,
            next_retry_at_ms=?1 + MIN(30000,1000 * (1 << MIN(consecutive_failures,5)))
            WHERE singleton=1 AND published_generation<target_generation",
            [now()],
        )?;
        Ok(())
    }
    pub(super) fn notify_work_archive_worker(&self) {
        if let Some(worker) = &self.archive_worker {
            if let Some(sender) = &worker.sender {
                let _ = sender.try_send(());
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct Worker {
    sender: Option<mpsc::SyncSender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
    #[cfg(test)]
    finished: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Disconnect first so an idle publisher wakes immediately. Returning
        // from last-owner close must mean its SQLite connection is gone, not
        // merely that an asynchronous shutdown notification was queued.
        self.sender.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
static WORKERS: OnceLock<Mutex<HashMap<PathBuf, Weak<Worker>>>> = OnceLock::new();
pub(super) fn register(path: &Path) -> StoreResult<Arc<Worker>> {
    let path = std::fs::canonicalize(path)
        .map_err(|_| StoreError::Invariant("archive database identity unavailable".into()))?;
    let mut registry = WORKERS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| StoreError::Invariant("archive scheduler unavailable".into()))?;
    registry.retain(|_, value| value.strong_count() > 0);
    if let Some(worker) = registry.get(&path).and_then(Weak::upgrade) {
        return Ok(worker);
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    #[cfg(test)]
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    #[cfg(test)]
    let finished_in_thread = finished.clone();
    let database = path.clone();
    let handle = std::thread::Builder::new()
        .name("nola-archive".into())
        .spawn(move || {
            // No worker Arc/notification sender is retained by this thread. Last
            // owning Store Drop disconnects the channel; no permanent per-DB thread.
            let mut store = None;
            loop {
                match receiver.recv_timeout(Duration::from_secs(1)) {
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    _ => {}
                }
                if store.is_none() {
                    store = NolaStore::open_archive_mode(&database, false).ok();
                }
                let Some(store) = store.as_ref() else {
                    continue;
                };
                let Ok(status) = store.archive_publication_status() else {
                    continue;
                };
                if status.target_generation <= status.published_generation
                    || status.next_retry_at_ms > now()
                {
                    continue;
                }
                let _ = store.publish_pending_work_archives();
            }
            drop(store);
            #[cfg(test)]
            finished_in_thread.store(true, std::sync::atomic::Ordering::SeqCst);
        })
        .map_err(|_| StoreError::Invariant("archive scheduler thread unavailable".into()))?;
    let worker = Arc::new(Worker {
        sender: Some(sender),
        handle: Some(handle),
        #[cfg(test)]
        finished,
    });
    registry.insert(path, Arc::downgrade(&worker));
    Ok(worker)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publisher_lives_until_last_owner_and_is_joined_before_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("owners.sqlite");
        let first = NolaStore::open(&path).unwrap();
        let second = NolaStore::open(&path).unwrap();
        assert!(Arc::ptr_eq(first.archive_worker.as_ref().unwrap(), second.archive_worker.as_ref().unwrap()));
        let finished = first.archive_worker.as_ref().unwrap().finished.clone();
        drop(first);
        assert!(!finished.load(std::sync::atomic::Ordering::SeqCst));
        drop(second);
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
        temp.close().unwrap();
    }

    #[test]
    fn last_owner_close_stops_worker_while_external_file_lock_is_held() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lifecycle.sqlite");
        let store = NolaStore::open_archive_mode(&path, false).unwrap();
        let root = store.work_archive_root().unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("publish.lock"))
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            lock.set_permissions(std::fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        lock.lock().unwrap();
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        let finished = store.archive_worker.as_ref().unwrap().finished.clone();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while store
            .archive_publication_status()
            .unwrap()
            .consecutive_failures
            == 0
        {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(store);
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst),
            "last owner close returned before publisher released its SQLite connection");
        drop(lock);
    }
}
