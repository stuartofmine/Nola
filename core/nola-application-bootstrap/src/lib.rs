// SPDX-License-Identifier: Apache-2.0
//! Persistence composition for the owning Application Service (including embedded service).
//! This is not a client SDK. The caller must acquire its service writer lock first.
//! Application owns transactions; this crate only opens concrete storage adapters.

/// Concrete runtime selected only by the Service composition boundary.
pub type ApplicationRuntime = nola_application::ApplicationRuntime<nola_store::NolaStore>;
mod read_pool;
mod runtime_owner;
pub use read_pool::{
    PooledReadStore, ReadStorePool, DEFAULT_READ_POOL_MAX_CONCURRENT, DEFAULT_READ_POOL_MAX_IDLE,
};
pub use runtime_owner::{acquire_owned_runtime, RuntimeOwnerGuard};
/// Names a fresh development database without exposing the persistence
/// adapter itself to thin clients. Incrementing the Store epoch therefore
/// selects a new default database instead of reopening an incompatible one.
pub const APPLICATION_STORE_SCHEMA_EPOCH: i64 = nola_store::STORE_SCHEMA_EPOCH;
use nola_store::NolaStore;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationPersistencePolicy {
    RequireCurrent,
    /// Only an explicit user-requested recreate operation may select this policy.
    RecreateIncompatible,
    /// Preserve the complete incompatible SQLite family before creating a new Store.
    ArchiveAndRecreateIncompatible {
        archive_directory: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationOpenFailureKind {
    IncompatibleStore,
    Other,
}

#[derive(Debug)]
pub struct ApplicationOpenFailure {
    pub kind: ApplicationOpenFailureKind,
    pub detail: String,
}

impl std::fmt::Display for ApplicationOpenFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

/// Production default: never explicitly removes or recreates an existing database.
pub fn open(path: impl AsRef<Path>) -> Result<ApplicationRuntime, String> {
    open_with_policy(path, ApplicationPersistencePolicy::RequireCurrent)
}

pub fn open_with_policy(
    path: impl AsRef<Path>,
    policy: ApplicationPersistencePolicy,
) -> Result<ApplicationRuntime, String> {
    open_with_policy_diagnosed(path, policy).map_err(|error| error.detail)
}

pub fn open_with_policy_diagnosed(
    path: impl AsRef<Path>,
    policy: ApplicationPersistencePolicy,
) -> Result<ApplicationRuntime, ApplicationOpenFailure> {
    let path = path.as_ref();
    if path != Path::new(":memory:") {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| ApplicationOpenFailure {
                kind: ApplicationOpenFailureKind::Other,
                detail: format!("create Application persistence directory failed: {error}"),
            })?;
        }
    }
    reject_obviously_non_sqlite_file(path).map_err(other_failure)?;
    let store = match NolaStore::open(path) {
        Ok(store) => store,
        Err(error) if error.is_schema_incompatible() => match &policy {
            ApplicationPersistencePolicy::RequireCurrent => {
                return Err(ApplicationOpenFailure {
                    kind: ApplicationOpenFailureKind::IncompatibleStore,
                    detail: format!("open Application persistence failed: {error}"),
                });
            }
            ApplicationPersistencePolicy::RecreateIncompatible => {
                remove_persistence_family(path).map_err(other_failure)?;
                NolaStore::open(path).map_err(|error| ApplicationOpenFailure {
                    kind: ApplicationOpenFailureKind::Other,
                    detail: format!("recreate Application persistence failed: {error}"),
                })?
            }
            ApplicationPersistencePolicy::ArchiveAndRecreateIncompatible { archive_directory } => {
                archive_persistence_family(path, archive_directory).map_err(other_failure)?;
                NolaStore::open(path).map_err(|error| ApplicationOpenFailure {
                    kind: ApplicationOpenFailureKind::Other,
                    detail: format!("create fresh Application persistence failed; original store is preserved in {}: {error}", archive_directory.display()),
                })?
            }
        },
        Err(error) => {
            return Err(ApplicationOpenFailure {
                kind: ApplicationOpenFailureKind::Other,
                detail: format!("open Application persistence failed: {error}"),
            })
        }
    };
    let uploads = Box::new(
        nola_store::content_upload_port::LazyContentUploadStore::for_application(
            path,
            nola_application::MAXIMUM_CONTENT_UPLOAD_BYTES,
        ),
    );
    Ok(ApplicationRuntime::from_store(store, uploads))
}

fn other_failure(detail: String) -> ApplicationOpenFailure {
    ApplicationOpenFailure {
        kind: ApplicationOpenFailureKind::Other,
        detail,
    }
}

// SQLite can discard malformed WAL/SHM files while opening a non-database.
// Check only its fixed format signature, without implementing schema/page validation.
// A zero-byte file is a legitimate new database. SQLite retains responsibility
// for recovery and validation of files with the correct signature.
fn reject_obviously_non_sqlite_file(path: &Path) -> Result<(), String> {
    if path == Path::new(":memory:") {
        return Ok(());
    }
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "read Application persistence header failed: {error}"
            ))
        }
    };
    if file
        .metadata()
        .map_err(|error| format!("read Application persistence metadata failed: {error}"))?
        .len()
        == 0
    {
        return Ok(());
    }
    let mut header = [0_u8; 16];
    file.read_exact(&mut header)
        .map_err(|error| format!("invalid Application SQLite header: {error}"))?;
    if &header != b"SQLite format 3\0" {
        return Err(
            "invalid Application SQLite header; existing files were not opened for writing".into(),
        );
    }
    Ok(())
}

fn remove_persistence_family(path: &Path) -> Result<(), String> {
    for member in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        match fs::remove_file(&member) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "remove incompatible Application persistence {} failed: {error}",
                    member.display()
                ))
            }
        }
    }
    Ok(())
}

fn archive_persistence_family(path: &Path, archive_directory: &Path) -> Result<(), String> {
    if archive_directory.exists() {
        return Err(format!(
            "incompatible Application persistence archive already exists: {}",
            archive_directory.display()
        ));
    }
    fs::create_dir_all(archive_directory).map_err(|error| {
        format!(
            "create incompatible Application persistence archive {} failed: {error}",
            archive_directory.display()
        )
    })?;
    let uploads = PathBuf::from(format!("{}.uploads.sqlite", path.display()));
    let family = [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
        uploads.clone(),
        PathBuf::from(format!("{}-wal", uploads.display())),
        PathBuf::from(format!("{}-shm", uploads.display())),
    ];
    let mut archived = Vec::new();
    for source in family {
        if !source.exists() {
            continue;
        }
        let file_name = source
            .file_name()
            .ok_or_else(|| format!("invalid persistence family path: {}", source.display()))?;
        let destination = archive_directory.join(file_name);
        if let Err(error) = fs::copy(&source, &destination).and_then(|_| {
            fs::OpenOptions::new()
                .write(true)
                .open(&destination)?
                .sync_all()
        }) {
            let _ = fs::remove_dir_all(archive_directory);
            return Err(format!(
                "archive incompatible Application persistence {} failed: {error}",
                source.display()
            ));
        }
        archived.push((source, destination));
    }
    if archived.is_empty() {
        let _ = fs::remove_dir(archive_directory);
        return Err("incompatible Application persistence family is unavailable".into());
    }
    fs::File::open(archive_directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "sync incompatible Application persistence archive {} failed: {error}",
                archive_directory.display()
            )
        })?;
    let mut removed = Vec::new();
    for (source, destination) in &archived {
        if let Err(error) = fs::remove_file(source) {
            for (removed_source, removed_archive) in &removed {
                let _ = fs::copy(removed_archive, removed_source);
            }
            return Err(format!(
                "remove archived Application persistence {} failed; the archive remains intact: {error}",
                source.display()
            ));
        }
        removed.push((source, destination));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_application::{ApplicationCommand, ExecuteReceivedCommand};

    #[test]
    fn empty_new_and_memory_databases_remain_valid() {
        let directory = tempfile::tempdir().unwrap();
        let empty = directory.path().join("empty.db");
        fs::write(&empty, []).unwrap();
        drop(open(&empty).unwrap());
        drop(open(directory.path().join("new.db")).unwrap());
        drop(open(":memory:").unwrap());
    }

    #[test]
    fn default_open_preserves_incompatible_and_corrupt_database_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let incompatible = directory.path().join("old.db");
        let database = rusqlite::Connection::open(&incompatible).unwrap();
        database.execute_batch("CREATE TABLE valuable(value TEXT); INSERT INTO valuable VALUES('keep'); PRAGMA user_version=99;").unwrap();
        drop(database);
        let original = fs::read(&incompatible).unwrap();
        assert!(open(&incompatible).is_err());
        assert_eq!(fs::read(&incompatible).unwrap(), original);
        assert!(!directory.path().join("old.db.uploads.sqlite").exists());

        let corrupt = directory.path().join("corrupt.db");
        let members = [
            corrupt.clone(),
            directory.path().join("corrupt.db-wal"),
            directory.path().join("corrupt.db-shm"),
        ];
        for (index, path) in members.iter().enumerate() {
            fs::write(path, format!("isolated corrupt fixture {index}")).unwrap();
        }
        let originals: Vec<_> = members.iter().map(|path| fs::read(path).unwrap()).collect();
        assert!(open(&corrupt).is_err());
        // Even explicit incompatible-schema recreation does not cover corruption.
        assert!(
            open_with_policy(&corrupt, ApplicationPersistencePolicy::RecreateIncompatible).is_err()
        );
        for (path, original) in members.iter().zip(originals) {
            assert_eq!(fs::read(path).unwrap(), original);
        }
    }

    #[test]
    fn incompatible_store_has_a_typed_failure_without_changing_its_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.db");
        let database = rusqlite::Connection::open(&path).unwrap();
        database
            .execute_batch(
                "CREATE TABLE valuable(value TEXT); INSERT INTO valuable VALUES('keep'); \
                 PRAGMA application_id=1313819713; PRAGMA user_version=11;",
            )
            .unwrap();
        drop(database);
        let original = fs::read(&path).unwrap();

        let error = open_with_policy_diagnosed(&path, ApplicationPersistencePolicy::RequireCurrent)
            .err()
            .expect("old schema must be rejected");
        assert_eq!(error.kind, ApplicationOpenFailureKind::IncompatibleStore);
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn explicit_recovery_archives_the_complete_family_before_recreating() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.db");
        let database = rusqlite::Connection::open(&path).unwrap();
        database
            .execute_batch(
                "CREATE TABLE valuable(value TEXT); INSERT INTO valuable VALUES('keep'); \
                 PRAGMA application_id=1313819713; PRAGMA user_version=11;",
            )
            .unwrap();
        drop(database);
        let original = fs::read(&path).unwrap();
        let wal = PathBuf::from(format!("{}-wal", path.display()));
        let shm = PathBuf::from(format!("{}-shm", path.display()));
        let uploads = PathBuf::from(format!("{}.uploads.sqlite", path.display()));
        fs::write(&wal, b"preserved wal bytes").unwrap();
        fs::write(&shm, b"preserved shm bytes").unwrap();
        fs::write(&uploads, b"preserved upload bytes").unwrap();
        let archive = directory.path().join("archive");

        archive_persistence_family(&path, &archive).unwrap();

        assert_eq!(fs::read(archive.join("old.db")).unwrap(), original);
        assert_eq!(
            fs::read(archive.join("old.db-wal")).unwrap(),
            b"preserved wal bytes"
        );
        assert_eq!(
            fs::read(archive.join("old.db-shm")).unwrap(),
            b"preserved shm bytes"
        );
        assert_eq!(
            fs::read(archive.join("old.db.uploads.sqlite")).unwrap(),
            b"preserved upload bytes"
        );
        assert!(!path.exists());
        assert!(!wal.exists());
        assert!(!shm.exists());
        assert!(!uploads.exists());
    }

    #[test]
    fn archive_recovery_policy_opens_a_fresh_current_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.db");
        let database = rusqlite::Connection::open(&path).unwrap();
        database
            .execute_batch(
                "CREATE TABLE valuable(value TEXT); INSERT INTO valuable VALUES('keep'); \
                 PRAGMA application_id=1313819713; PRAGMA user_version=11;",
            )
            .unwrap();
        drop(database);
        let original = fs::read(&path).unwrap();
        let archive = directory.path().join("archive");

        drop(
            open_with_policy(
                &path,
                ApplicationPersistencePolicy::ArchiveAndRecreateIncompatible {
                    archive_directory: archive.clone(),
                },
            )
            .unwrap(),
        );

        assert_eq!(fs::read(archive.join("old.db")).unwrap(), original);
        assert_ne!(fs::read(&path).unwrap(), original);
        drop(NolaStore::open(&path).unwrap());
    }

    #[test]
    fn explicit_archive_does_not_convert_retired_provider_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.db");
        let database = rusqlite::Connection::open(&path).unwrap();
        database
            .execute_batch(
                "PRAGMA application_id=1313819713; PRAGMA user_version=11;
             CREATE TABLE model_profiles(id TEXT, source_json TEXT);
             INSERT INTO model_profiles VALUES('old-profile','retired format');",
            )
            .unwrap();
        drop(database);
        let original = fs::read(&path).unwrap();
        let archive = directory.path().join("archive");
        drop(
            open_with_policy(
                &path,
                ApplicationPersistencePolicy::ArchiveAndRecreateIncompatible {
                    archive_directory: archive.clone(),
                },
            )
            .unwrap(),
        );
        let current = NolaStore::open(&path).unwrap();
        assert!(current.list_model_profiles().unwrap().is_empty());
        assert_eq!(fs::read(archive.join("old.db")).unwrap(), original);
    }

    #[test]
    fn explicit_recreation_and_default_reopen_use_the_real_adapter() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.db");
        let database = rusqlite::Connection::open(&path).unwrap();
        database
            .execute_batch("CREATE TABLE legacy(value TEXT); PRAGMA user_version=99;")
            .unwrap();
        drop(database);
        drop(open_with_policy(&path, ApplicationPersistencePolicy::RecreateIncompatible).unwrap());
        drop(open(&path).unwrap());
        let database = rusqlite::Connection::open(&path).unwrap();
        let legacy: i64 = database
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy, 0);
    }

    #[test]
    fn composed_runtime_commits_and_replays_receipt_after_restart_without_opening_uploads() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/runtime.db");
        let staging = directory.path().join("nested/runtime.db.uploads.sqlite");
        let command = || ApplicationCommand::PutProductSettingChecked {
            key: "bootstrap.fixture".into(),
            value_json: "[1]".into(),
            expected_revision: None,
            updated_at_ms: 1,
        };
        let submit = |runtime: &mut ApplicationRuntime| {
            let command = command();
            runtime
                .execute_received_command(
                    ExecuteReceivedCommand {
                        client_id: "bootstrap-test".into(),
                        request_id: "once".into(),
                        command_type: "put_product_setting_checked".into(),
                        command_json: serde_json::to_string(&command).unwrap(),
                        expected_revision: None,
                        received_at: "2026-09-08T00:00:00Z".into(),
                    },
                    command,
                )
                .unwrap()
        };
        let mut runtime = open(&path).unwrap();
        assert!(!staging.exists());
        let first = submit(&mut runtime);
        assert!(!first.replayed);
        let state = runtime
            .execute(ApplicationCommand::GetProductSetting {
                key: "bootstrap.fixture".into(),
            })
            .unwrap();
        assert_eq!(state["revision"], 1);
        drop(runtime);
        let mut runtime = open(&path).unwrap();
        let replayed = submit(&mut runtime);
        assert!(replayed.replayed);
        assert_eq!(replayed.outcome, first.outcome);
        assert_eq!(
            runtime
                .execute(ApplicationCommand::GetProductSetting {
                    key: "bootstrap.fixture".into()
                })
                .unwrap(),
            state
        );
        assert!(!staging.exists());
    }
}
