// SPDX-License-Identifier: Apache-2.0
//! Durable session process observations shared by terminal and native control planes.
use crate::store::{DbSessionRunLock, NolaStore};

pub fn session_running_process(session_id: &str) -> Option<u32> {
    let store = NolaStore::open_default().ok()?;
    let lock = store.session_run_lock(session_id).ok().flatten();
    match lock {
        Some(lock) if session_lock_owner_is_alive(&lock) => Some(lock.pid),
        Some(_) => None,
        None => {
            // Absence of the legacy session lock is not owner-death evidence.
            // Current Runtime ownership lives in the effect-attempt ledger.
            crate::core_agent_runtime::observed_session_delivery_owner(session_id)
                .ok()
                .flatten()
        }
    }
}

pub fn cleanup_stale_session_run_locks() {
    let Ok(store) = NolaStore::open_default() else {
        return;
    };
    cleanup_stale_session_run_locks_in_store(&store);
}

fn cleanup_stale_session_run_locks_in_store(store: &NolaStore) {
    let Ok(locks) = store.session_run_locks() else {
        return;
    };
    for lock in locks {
        // Source admission owns a typed process identity and no conversation Run.
        // Its Host fence performs exact liveness and owner-qualified cleanup.
        if lock.session_id.starts_with("source-admission/") { continue; }
        if session_lock_owner_is_alive(&lock) {
            continue;
        }
        let _ = store.release_session_run_lock(&lock.session_id, &lock.owner);
        let _ = store.interrupt_orphaned_runs_for_session(&lock.session_id);
    }
}

#[cfg(test)]
mod stale_session_run_lock_tests {
    use super::cleanup_stale_session_run_locks_in_store;
    use crate::store::{DbSessionRunLock, NolaStore};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn startup_recovery_removes_dead_owner_and_keeps_live_owner() -> Result<(), String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nola-stale-session-lock-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).map_err(|err| err.to_string())?;
        let store = NolaStore::open(&root.join("nola.db"))?;
        let live = DbSessionRunLock {
            session_id: "live-session".to_string(),
            owner: "live-owner".to_string(),
            pid: std::process::id(),
            process_start: String::new(),
            acquired_at_ms: 1,
        };
        let dead = DbSessionRunLock {
            session_id: "dead-session".to_string(),
            owner: "dead-owner".to_string(),
            pid: u32::MAX,
            process_start: String::new(),
            acquired_at_ms: 2,
        };
        assert!(store.try_acquire_session_run_lock(&live)?);
        assert!(store.try_acquire_session_run_lock(&dead)?);

        cleanup_stale_session_run_locks_in_store(&store);

        assert!(store.session_run_lock(&live.session_id)?.is_some());
        assert!(store.session_run_lock(&dead.session_id)?.is_none());
        drop(store);
        let _ = fs::remove_dir_all(root);
        Ok(())
    }
}

fn session_lock_owner_is_alive(lock: &DbSessionRunLock) -> bool {
    if lock.pid == 0 {
        return false;
    }
    if !lock.process_start.is_empty() {
        if let Some(current) = process_start_identity(lock.pid) {
            return current == lock.process_start;
        }
    }
    crate::process_observation::process_id_is_alive(lock.pid)
}

#[cfg(target_os = "linux")]
fn process_start_identity(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat
        .rsplit_once(") ")?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    fields.get(19).map(|value| (*value).to_string())
}

#[cfg(not(target_os = "linux"))]
fn process_start_identity(_pid: u32) -> Option<String> {
    None
}
