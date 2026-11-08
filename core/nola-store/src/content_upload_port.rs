// SPDX-License-Identifier: Apache-2.0
//! SQLite upload adapter; transaction ownership stays inside each Store operation.
use crate::{content_upload::ContentUploadStore, StoreError};
use nola_store_port::content_upload::{ContentUploadRepository, UploadOwner, UploadProgress};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

/// Owns SQLite connection lifecycle, not upload use-case decisions. Construction
/// performs no IO. Only `begin` may create the staging database.
pub struct LazyContentUploadStore {
    path: PathBuf,
    maximum_upload_bytes: u64,
    canonical_path: PathBuf,
    store: RefCell<Option<ContentUploadStore>>,
}

impl LazyContentUploadStore {
    pub fn for_application(path: &Path, maximum_upload_bytes: u64) -> Self {
        let canonical_path = path.to_path_buf();
        let path = if path == Path::new(":memory:") {
            path.to_path_buf()
        } else {
            let mut name = path.as_os_str().to_os_string();
            name.push(".uploads.sqlite");
            PathBuf::from(name)
        };
        Self {
            path,
            maximum_upload_bytes,
            canonical_path,
            store: RefCell::new(None),
        }
    }

    fn with_store<T>(
        &self,
        may_create: bool,
        action: impl FnOnce(&mut ContentUploadStore) -> Result<T, StoreError>,
    ) -> Result<T, String> {
        let mut slot = self
            .store
            .try_borrow_mut()
            .map_err(|_| "upload store is already borrowed")?;
        if slot.is_none() {
            // `try_exists` preserves filesystem errors rather than mistaking
            // an inaccessible existing file for a missing database.
            let exists = self.path.try_exists().map_err(|error| error.to_string())?;
            let store = if exists {
                ContentUploadStore::open_existing(&self.path, self.maximum_upload_bytes)
            } else if may_create {
                ContentUploadStore::open(&self.path, self.maximum_upload_bytes)
            } else {
                return Err("unknown upload".into());
            }
            .map_err(|error| error.to_string())?;
            *slot = Some(store);
        }
        action(slot.as_mut().ok_or("upload store unavailable")?).map_err(|error| error.to_string())
    }
}

impl ContentUploadRepository for LazyContentUploadStore {
    type Error = String;

    fn begin(
        &mut self,
        id: &str,
        owner: &UploadOwner,
        expected: u64,
    ) -> Result<UploadProgress, Self::Error> {
        self.with_store(true, |store| {
            store.retire_durable_uploads(&self.canonical_path)?;
            store.begin(id, owner, expected)
        })
    }
    fn progress(&self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error> {
        self.with_store(false, |store| store.progress(id, owner))
    }
    fn append(
        &mut self,
        id: &str,
        owner: &UploadOwner,
        offset: u64,
        text: &str,
    ) -> Result<UploadProgress, Self::Error> {
        self.with_store(false, |store| store.append(id, owner, offset, text))
    }
    fn seal(&mut self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error> {
        self.with_store(false, |store| store.seal(id, owner))
    }
    fn cancel(&mut self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error> {
        self.with_store(false, |store| store.cancel(id, owner))
    }
    fn read_chunk(
        &self,
        id: &str,
        owner: &UploadOwner,
        offset: u64,
    ) -> Result<Option<String>, Self::Error> {
        self.with_store(false, |store| {
            store.read_chunk_with_canonical(id, owner, offset, &self.canonical_path)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> UploadOwner {
        UploadOwner {
            conversation_id: "c".into(),
            import_id: "i".into(),
            part_id: "p".into(),
        }
    }

    #[test]
    fn only_begin_creates_staging_and_restart_retains_sealed_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nola.db");
        let staging = directory.path().join("nola.db.uploads.sqlite");
        let owner = owner();
        let mut store = LazyContentUploadStore::for_application(&path, 1024);
        assert!(!staging.exists());
        assert!(store.progress("u", &owner).is_err());
        assert!(store.append("u", &owner, 0, "text").is_err());
        assert!(store.seal("u", &owner).is_err());
        assert!(store.cancel("u", &owner).is_err());
        assert!(store.read_chunk("u", &owner, 0).is_err());
        assert!(!staging.exists());
        store.begin("u", &owner, 4).unwrap();
        store.append("u", &owner, 0, "text").unwrap();
        drop(store);
        let mut store = LazyContentUploadStore::for_application(&path, 1024);
        assert_eq!(store.progress("u", &owner).unwrap().committed_bytes, 4);
        assert!(store.seal("u", &owner).unwrap().sealed);
        drop(store);
        let store = LazyContentUploadStore::for_application(&path, 1024);
        assert_eq!(
            store.read_chunk("u", &owner, 0).unwrap().as_deref(),
            Some("text")
        );
        let mut other = owner.clone();
        other.part_id = "foreign".into();
        assert!(store.progress("u", &other).is_err());
    }

    #[test]
    fn existing_corrupt_staging_is_not_recreated() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nola.db");
        let staging = directory.path().join("nola.db.uploads.sqlite");
        std::fs::write(&staging, b"not a database").unwrap();
        let mut store = LazyContentUploadStore::for_application(&path, 1024);
        assert!(store.progress("u", &owner()).is_err());
        assert!(store.begin("u", &owner(), 4).is_err());
        assert_eq!(std::fs::read(staging).unwrap(), b"not a database");
    }

    #[test]
    fn in_memory_uploads_are_isolated_between_adapters() {
        let mut first = LazyContentUploadStore::for_application(Path::new(":memory:"), 1024);
        let second = LazyContentUploadStore::for_application(Path::new(":memory:"), 1024);
        first.begin("u", &owner(), 4).unwrap();
        assert!(first.progress("u", &owner()).is_ok());
        assert!(second.progress("u", &owner()).is_err());
    }
}

impl ContentUploadRepository for ContentUploadStore {
    type Error = StoreError;

    fn begin(
        &mut self,
        id: &str,
        owner: &UploadOwner,
        expected: u64,
    ) -> Result<UploadProgress, Self::Error> {
        ContentUploadStore::begin(self, id, owner, expected)
    }
    fn progress(&self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error> {
        ContentUploadStore::progress(self, id, owner)
    }
    fn append(
        &mut self,
        id: &str,
        owner: &UploadOwner,
        offset: u64,
        text: &str,
    ) -> Result<UploadProgress, Self::Error> {
        ContentUploadStore::append(self, id, owner, offset, text)
    }
    fn seal(&mut self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error> {
        ContentUploadStore::seal(self, id, owner)
    }
    fn cancel(&mut self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error> {
        ContentUploadStore::cancel(self, id, owner)
    }
    fn read_chunk(
        &self,
        id: &str,
        owner: &UploadOwner,
        offset: u64,
    ) -> Result<Option<String>, Self::Error> {
        ContentUploadStore::read_chunk(self, id, owner, offset)
    }
}
