// SPDX-License-Identifier: Apache-2.0
//! Bounded pool of read-only WAL `NolaStore` handles for concurrent read queries.

use nola_store::NolaStore;
use std::{
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Condvar, Mutex},
};

pub const DEFAULT_READ_POOL_MAX_IDLE: usize = 4;
pub const DEFAULT_READ_POOL_MAX_CONCURRENT: usize = 8;

struct PoolState {
    idle: Vec<NolaStore>,
    active: usize,
    generation: u64,
}

/// Thread-safe bounded pool of read-only `NolaStore` connections backed by SQLite WAL.
pub struct ReadStorePool {
    database_path: PathBuf,
    max_idle: usize,
    max_concurrent: usize,
    state: Mutex<PoolState>,
    available: Condvar,
}

impl std::fmt::Debug for ReadStorePool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadStorePool")
            .field("database_path", &self.database_path)
            .field("max_idle", &self.max_idle)
            .field("max_concurrent", &self.max_concurrent)
            .finish()
    }
}

impl ReadStorePool {
    pub fn new(database_path: impl Into<PathBuf>) -> Self {
        Self::with_limits(
            database_path,
            DEFAULT_READ_POOL_MAX_IDLE,
            DEFAULT_READ_POOL_MAX_CONCURRENT,
        )
    }

    pub fn with_limits(
        database_path: impl Into<PathBuf>,
        max_idle: usize,
        max_concurrent: usize,
    ) -> Self {
        let max_idle = max_idle.max(1);
        let max_concurrent = max_concurrent.max(max_idle);
        Self {
            database_path: database_path.into(),
            max_idle,
            max_concurrent,
            state: Mutex::new(PoolState {
                idle: Vec::with_capacity(max_idle),
                active: 0,
                generation: 0,
            }),
            available: Condvar::new(),
        }
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn is_enabled(&self) -> bool {
        !self.database_path.as_os_str().is_empty() && self.database_path != Path::new(":memory:")
    }

    pub fn checkout(&self) -> Result<PooledReadStore<'_>, String> {
        if !self.is_enabled() {
            return Err("read-only Store pool is disabled for in-memory persistence".into());
        }
        let (cached, generation) = {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while guard.idle.is_empty() && guard.active >= self.max_concurrent {
                guard = self
                    .available
                    .wait(guard)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            guard.active += 1;
            (guard.idle.pop(), guard.generation)
        };
        if let Some(store) = cached {
            return Ok(PooledReadStore {
                pool: self,
                store: Some(store),
                generation,
            });
        }
        match NolaStore::open_read_only(&self.database_path) {
            Ok(store) => Ok(PooledReadStore {
                pool: self,
                store: Some(store),
                generation,
            }),
            Err(error) => {
                let mut guard = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.active = guard.active.saturating_sub(1);
                self.available.notify_one();
                Err(format!(
                    "open read-only Application persistence failed: {error}"
                ))
            }
        }
    }

    /// Drops all idle connections and invalidates in-flight checkouts so stale
    /// file descriptors are closed as soon as the writer reopens or closes.
    pub fn clear(&self) {
        let drained = {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.generation = guard.generation.wrapping_add(1);
            std::mem::take(&mut guard.idle)
        };
        drop(drained);
    }

    pub fn idle_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .idle
            .len()
    }

    pub fn active_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active
    }
}

pub struct PooledReadStore<'pool> {
    pool: &'pool ReadStorePool,
    store: Option<NolaStore>,
    generation: u64,
}

impl Deref for PooledReadStore<'_> {
    type Target = NolaStore;

    fn deref(&self) -> &Self::Target {
        self.store
            .as_ref()
            .expect("PooledReadStore holds a live NolaStore until Drop")
    }
}

impl DerefMut for PooledReadStore<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.store
            .as_mut()
            .expect("PooledReadStore holds a live NolaStore until Drop")
    }
}

impl Drop for PooledReadStore<'_> {
    fn drop(&mut self) {
        let Some(store) = self.store.take() else {
            return;
        };
        let reusable = store.is_read_only() && !store.has_write_unit_of_work();
        let evicted = {
            let mut guard = self
                .pool
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.active = guard.active.saturating_sub(1);
            let evicted = if reusable
                && guard.generation == self.generation
                && guard.idle.len() < self.pool.max_idle
            {
                guard.idle.push(store);
                None
            } else {
                Some(store)
            };
            self.pool.available.notify_one();
            evicted
        };
        drop(evicted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn read_pool_disables_in_memory_and_reuses_up_to_max_idle_handles() {
        let disabled = ReadStorePool::new(":memory:");
        assert!(!disabled.is_enabled());
        assert!(disabled.checkout().is_err());

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pool.sqlite");
        let _writer = crate::open(&path).unwrap();

        let pool = ReadStorePool::with_limits(&path, 2, 4);
        assert!(pool.is_enabled());

        let h1 = pool.checkout().unwrap();
        let h2 = pool.checkout().unwrap();
        let h3 = pool.checkout().unwrap();
        assert_eq!(pool.active_count(), 3);
        assert_eq!(pool.idle_count(), 0);

        drop(h1);
        drop(h2);
        drop(h3);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 2);

        // Reopen/clear invalidates idle handles and prevents older generations from returning.
        let stale = pool.checkout().unwrap();
        assert_eq!(pool.idle_count(), 1);
        pool.clear();
        assert_eq!(pool.idle_count(), 0);
        drop(stale);
        assert_eq!(pool.idle_count(), 0);
    }

    #[test]
    fn read_pool_bounds_concurrent_checkouts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bounded.sqlite");
        let _writer = crate::open(&path).unwrap();

        let pool = Arc::new(ReadStorePool::with_limits(&path, 2, 2));
        let hold1 = pool.checkout().unwrap();
        let hold2 = pool.checkout().unwrap();
        assert_eq!(pool.active_count(), 2);

        let started = Arc::new(Barrier::new(2));
        let started_waiter = Arc::clone(&started);
        let pool_waiter = Arc::clone(&pool);
        let waiter = std::thread::spawn(move || {
            started_waiter.wait();
            let guard = pool_waiter.checkout().unwrap();
            assert!(guard.is_read_only());
        });

        started.wait();
        drop(hold1);
        waiter.join().unwrap();
        drop(hold2);
        assert_eq!(pool.active_count(), 0);
        assert!(pool.idle_count() <= 2);
    }
}
