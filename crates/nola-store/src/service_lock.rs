// SPDX-License-Identifier: Apache-2.0
//! Single-writer exclusive file lock (`flock`) and deterministic `<db>.service.json`
//! discovery synced with `apps/macos/Services/MacApplicationServiceClient.swift`.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::vault::atomic_write_restricted;
use crate::StoreError;

/// Shared UDS Application Service protocol version (matched by macOS Swift client).
pub const PROTOCOL_VERSION: u32 = 1;

/// JSON record written to `<db_path>.service.json` by the active single-writer daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDiscovery {
    pub socket_path: String,
    pub pid: u32,
    pub euid: u32,
    pub protocol_version: u32,
}

/// Canonical database path resolving nearest existing ancestor via `canonicalize`,
/// matching `MacApplicationServiceClient.databaseIdentityPath(for:)`.
pub fn database_identity_path(db_path: &Path) -> String {
    let abs = if db_path.is_absolute() {
        db_path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(db_path)
    };
    let mut ancestor = abs.as_path();
    let mut tail: Vec<String> = Vec::new();
    loop {
        if let Ok(resolved) = fs::canonicalize(ancestor) {
            let mut full = resolved;
            for seg in tail.into_iter().rev() {
                full.push(seg);
            }
            return full.to_string_lossy().into_owned();
        }
        let Some(name) = ancestor.file_name() else {
            return abs.to_string_lossy().into_owned();
        };
        tail.push(name.to_string_lossy().into_owned());
        let Some(parent) = ancestor.parent() else {
            return abs.to_string_lossy().into_owned();
        };
        ancestor = parent;
    }
}

/// Exact 64-bit FNV-1a hash matching `MacApplicationServiceClient.databaseIdentityHash`.
pub fn database_identity_hash(db_path: &Path) -> u64 {
    let mut hash: u64 = 14_695_981_039_346_656_037;
    for byte in database_identity_path(db_path).as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

/// Deterministic UDS socket path for a database file.
pub fn default_socket_path(db_path: &Path) -> PathBuf {
    let hash = database_identity_hash(db_path);
    PathBuf::from(format!("/tmp/nola-application-v{PROTOCOL_VERSION}-{hash:016x}.sock"))
}

/// Path to the authoritative service discovery JSON sidecar next to the database.
pub fn discovery_path(db_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.service.json", db_path.display()))
}

/// Read and parse `<db_path>.service.json` if present.
pub fn read_discovery(db_path: &Path) -> Result<Option<ServiceDiscovery>, StoreError> {
    let path = discovery_path(db_path);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read(&path)?;
    Ok(Some(serde_json::from_slice(&raw)?))
}

/// Exclusive lock guard held for the lifetime of a single-writer Nola daemon.
pub struct ServiceLock {
    _lock_file: File,
    discovery_file: PathBuf,
    discovery: ServiceDiscovery,
}

impl ServiceLock {
    /// Acquire non-blocking exclusive lock on `<db_path>.lock` and publish `<db_path>.service.json`.
    pub fn acquire(db_path: &Path, socket_path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let lock_path = PathBuf::from(format!("{}.lock", db_path.display()));
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;

        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let rc = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc != 0 {
                return Err(StoreError::LockBusy(format!(
                    "another Nola single-writer holds {}",
                    lock_path.display()
                )));
            }
        }

        #[cfg(unix)]
        let euid = unsafe { libc::geteuid() };
        #[cfg(not(unix))]
        let euid = 0u32;

        let discovery = ServiceDiscovery {
            socket_path: socket_path.to_string_lossy().into_owned(),
            pid: std::process::id(),
            euid,
            protocol_version: PROTOCOL_VERSION,
        };
        let discovery_file = discovery_path(db_path);
        let bytes = serde_json::to_vec_pretty(&discovery)?;
        atomic_write_restricted(&discovery_file, &bytes)?;

        Ok(Self {
            _lock_file: lock_file,
            discovery_file,
            discovery,
        })
    }

    pub fn discovery(&self) -> &ServiceDiscovery {
        &self.discovery
    }
}

impl Drop for ServiceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.discovery_file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_lock_prevents_concurrent_writers_and_publishes_discovery() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("nola.db");
        let sock = default_socket_path(&db);

        let guard = ServiceLock::acquire(&db, &sock).unwrap();
        let loaded = read_discovery(&db).unwrap().unwrap();
        assert_eq!(loaded, *guard.discovery());
        assert_eq!(loaded.socket_path, sock.to_string_lossy());
        assert_eq!(loaded.protocol_version, PROTOCOL_VERSION);

        // Second writer must fail immediately with LockBusy
        let err = ServiceLock::acquire(&db, &sock).err().unwrap();
        assert!(matches!(err, StoreError::LockBusy(_)));

        drop(guard);
        assert!(read_discovery(&db).unwrap().is_none());
    }
}
