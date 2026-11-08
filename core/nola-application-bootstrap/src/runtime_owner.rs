// SPDX-License-Identifier: Apache-2.0
//! Shared in-place owner retirement for service and embedded Host handles.
//! The caller retains its process writer lock throughout this guard's lifetime.
use crate::ApplicationRuntime;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

pub struct RuntimeOwnerGuard<'a> {
    runtime: MutexGuard<'a, Option<ApplicationRuntime>>,
    database_path: &'a Path,
}

pub fn acquire_owned_runtime<'a>(
    runtime: &'a Mutex<Option<ApplicationRuntime>>,
    database_path: &'a Path,
) -> Result<RuntimeOwnerGuard<'a>, String> {
    let mut owner = runtime
        .lock()
        .map_err(|_| "application runtime lock is poisoned".to_string())?;
    if owner.is_none() {
        *owner = Some(crate::open(database_path)?);
    }
    Ok(RuntimeOwnerGuard {
        runtime: owner,
        database_path,
    })
}

impl std::ops::Deref for RuntimeOwnerGuard<'_> {
    type Target = ApplicationRuntime;
    fn deref(&self) -> &Self::Target {
        self.runtime
            .as_ref()
            .expect("validated Application runtime owner")
    }
}

impl std::ops::DerefMut for RuntimeOwnerGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.runtime
            .as_mut()
            .expect("validated Application runtime owner")
    }
}

impl RuntimeOwnerGuard<'_> {
    /// Drop the failed composition before opening its successor. Other handles
    /// still share this same mutex and the caller's existing writer ownership.
    /// Failed open leaves the owner absent; the next acquisition can retry.
    pub fn reopen(&mut self) -> Result<(), String> {
        if self.database_path == Path::new(":memory:") {
            return Err(
                "an in-memory Application owner cannot be reopened without losing its Store".into(),
            );
        }
        drop(self.runtime.take());
        *self.runtime = Some(crate::open(self.database_path)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_application::ApplicationCommand;

    fn put(runtime: &mut ApplicationRuntime) {
        runtime
            .execute(ApplicationCommand::PutProductSetting {
                key: "test.owner-reopen".into(),
                value_json: "{\"retained\":true}".into(),
                updated_at_ms: 1,
            })
            .unwrap();
    }

    #[test]
    fn reopened_owner_keeps_committed_store_and_is_seen_by_the_next_handle() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("owner.db");
        let owner = Mutex::new(Some(crate::open(&path).unwrap()));
        {
            let mut guard = acquire_owned_runtime(&owner, &path).unwrap();
            put(&mut guard);
            guard.reopen().unwrap();
        }
        let result = acquire_owned_runtime(&owner, &path)
            .unwrap()
            .execute(ApplicationCommand::GetProductSetting {
                key: "test.owner-reopen".into(),
            })
            .unwrap();
        assert_eq!(result["key"], "test.owner-reopen");
        assert_eq!(result["value_json"], "{\"retained\":true}");
    }

    #[test]
    fn failed_reopen_leaves_owner_absent_and_preserves_original_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("owner.db");
        let mut runtime = crate::open(&path).unwrap();
        put(&mut runtime);
        let owner = Mutex::new(Some(runtime));
        // A directory cannot be an SQLite Store. No file is deleted or reset.
        {
            let mut guard = acquire_owned_runtime(&owner, directory.path()).unwrap();
            assert!(guard.reopen().is_err());
        }
        assert!(owner.lock().unwrap().is_none());
        assert!(acquire_owned_runtime(&owner, directory.path()).is_err());
        let result = crate::open(&path)
            .unwrap()
            .execute(ApplicationCommand::GetProductSetting {
                key: "test.owner-reopen".into(),
            })
            .unwrap();
        assert_eq!(result["key"], "test.owner-reopen");
        assert_eq!(result["value_json"], "{\"retained\":true}");
    }

    #[test]
    fn memory_owner_is_not_replaced_with_an_empty_store() {
        let owner = Mutex::new(Some(crate::open(":memory:").unwrap()));
        let mut guard = acquire_owned_runtime(&owner, Path::new(":memory:")).unwrap();
        put(&mut guard);
        assert!(guard.reopen().is_err());
        let result = guard
            .execute(ApplicationCommand::GetProductSetting {
                key: "test.owner-reopen".into(),
            })
            .unwrap();
        assert_eq!(result["key"], "test.owner-reopen");
        assert_eq!(result["value_json"], "{\"retained\":true}");
    }
}
