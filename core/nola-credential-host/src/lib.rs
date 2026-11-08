// SPDX-License-Identifier: Apache-2.0
//! Host-private endpoint credentials. Secrets never enter canonical product
//! state, Application commands, receipts, exports, or sync. SQLite stays private.

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct HostCredentialVault {
    connection: Connection,
}

pub struct CredentialRecord {
    pub identity: String,
    pub revision: i64,
    // Intentionally neither Debug nor Serialize.
    pub secret: Option<String>,
}

pub fn identity(base_url: &str, protocol: &str) -> Result<String, String> {
    let mut url = url::Url::parse(base_url.trim()).map_err(|_| "Invalid endpoint identity")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(protocol, "responses" | "chat_completions" | "anthropic")
    {
        return Err("Invalid endpoint identity".into());
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(format!("{protocol}\n{url}"))
}

impl HostCredentialVault {
    /// Separate namespace from model endpoints. Package replacement must not
    /// inherit credentials approved for different executable bytes.
    pub fn plugin_record(
        &self,
        plugin_id: &str,
        reference: &str,
    ) -> Result<Option<CredentialRecord>, String> {
        self.plugin_table()?;
        self.connection.query_row(
            "SELECT package_digest,revision,secret FROM plugin_credentials WHERE plugin_id=?1 AND reference=?2",
            params![plugin_id, reference], |row| Ok(CredentialRecord {
                identity: row.get(0)?, revision: row.get(1)?, secret: row.get(2)?,
            }),
        ).optional().map_err(|_| "Cannot read plugin credential".into())
    }

    fn plugin_table(&self) -> Result<(), String> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS plugin_credentials(plugin_id TEXT NOT NULL,reference TEXT NOT NULL,package_digest TEXT NOT NULL,revision INTEGER NOT NULL,secret TEXT,PRIMARY KEY(plugin_id,reference));")
            .map_err(|_| "Cannot initialize plugin credentials".into())
    }

    pub fn write_plugin(
        &mut self,
        plugin_id: &str,
        reference: &str,
        package_digest: &str,
        expected_revision: i64,
        secret: Option<&str>,
        validate: impl Fn() -> Result<(), String>,
    ) -> Result<i64, String> {
        self.plugin_table()?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "Cannot lock plugin credentials")?;
        let revision: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM plugin_credentials WHERE plugin_id=?1 AND reference=?2",
                params![plugin_id, reference],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Cannot read plugin credential revision")?;
        if expected_revision < 0 || revision.unwrap_or(0) != expected_revision {
            return Err("Plugin credential revision changed".into());
        }
        let next = expected_revision
            .checked_add(1)
            .ok_or("Plugin credential revision exhausted")?;
        validate()?;
        transaction.execute("INSERT INTO plugin_credentials(plugin_id,reference,package_digest,revision,secret) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(plugin_id,reference) DO UPDATE SET package_digest=excluded.package_digest,revision=excluded.revision,secret=excluded.secret",
            params![plugin_id, reference, package_digest, next, secret]).map_err(|_| "Cannot save plugin credential")?;
        validate()?;
        transaction
            .commit()
            .map_err(|_| "Cannot commit plugin credential")?;
        Ok(next)
    }

    #[cfg(not(unix))]
    pub fn open(_: &Path) -> Result<Self, String> {
        Err(
            "Private endpoint credentials require a host with protected Unix file permissions"
                .into(),
        )
    }

    #[cfg(unix)]
    pub fn open(canonical_database: &Path) -> Result<Self, String> {
        let canonical = std::fs::canonicalize(canonical_database)
            .map_err(|_| "Cannot locate credential host")?;
        let directory = canonical
            .parent()
            .ok_or("Cannot locate credential host")?
            .join(".nola-host-secrets");
        if let Ok(metadata) = std::fs::symlink_metadata(&directory) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("Unsafe credential directory".into());
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            if let Err(error) = builder.create(&directory) {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err("Cannot create credential directory".into());
                }
            }
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "Cannot protect credential directory")?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&directory).map_err(|_| "Cannot create credential directory")?;
        let path = directory.join(format!(
            "{:x}.sqlite",
            Sha256::digest(canonical.as_os_str().as_encoded_bytes())
        ));
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err("Unsafe credential file".into());
            }
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            let file = options
                .open(&path)
                .map_err(|_| "Cannot open credential file")?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "Cannot protect credential file")?;
        }
        #[cfg(not(unix))]
        let _file = options
            .open(&path)
            .map_err(|_| "Cannot open credential file")?;
        let connection = Connection::open(&path).map_err(|_| "Cannot open credential vault")?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "Cannot configure credential vault")?;
        connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA secure_delete=ON; CREATE TABLE IF NOT EXISTS endpoint_credentials(endpoint_id TEXT PRIMARY KEY, identity TEXT NOT NULL, revision INTEGER NOT NULL, secret TEXT);")
            .map_err(|_| "Cannot initialize credential vault")?;
        Ok(Self { connection })
    }

    pub fn record(&self, endpoint_id: &str) -> Result<Option<CredentialRecord>, String> {
        self.connection
            .query_row(
                "SELECT identity,revision,secret FROM endpoint_credentials WHERE endpoint_id=?1",
                [endpoint_id],
                |row| {
                    Ok(CredentialRecord {
                        identity: row.get(0)?,
                        revision: row.get(1)?,
                        secret: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|_| "Cannot read endpoint credential".into())
    }

    /// Immutable Host reconstruction material. It is separate from endpoint
    /// credentials: clearing a credential cannot overwrite a Run descriptor.
    pub fn save_runtime_descriptor(&self, handle: &str, descriptor: &str) -> Result<(), String> {
        if handle.len() != 32 || !handle.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid runtime descriptor handle".into());
        }
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS runtime_descriptors(handle TEXT PRIMARY KEY, descriptor TEXT NOT NULL)")
            .map_err(|_| "Cannot initialize runtime descriptors")?;
        self.connection
            .execute(
                "INSERT INTO runtime_descriptors(handle,descriptor) VALUES(?1,?2)",
                params![handle, descriptor],
            )
            .map_err(|_| "Cannot persist immutable runtime descriptor")?;
        Ok(())
    }

    pub fn runtime_descriptor(&self, handle: &str) -> Result<Option<String>, String> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS runtime_descriptors(handle TEXT PRIMARY KEY, descriptor TEXT NOT NULL)")
            .map_err(|_| "Cannot initialize runtime descriptors")?;
        self.connection
            .query_row(
                "SELECT descriptor FROM runtime_descriptors WHERE handle=?1",
                [handle],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Cannot read runtime descriptor".into())
    }

    /// Host-private ownership receipts may contain the gateway token written to
    /// an explicitly selected client config. They never enter canonical state.
    pub fn save_client_configuration_receipt(
        &self,
        handle: &str,
        receipt: &str,
    ) -> Result<(), String> {
        if handle.len() != 32 || !handle.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid client configuration receipt handle".into());
        }
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS client_configuration_receipts(handle TEXT PRIMARY KEY, receipt TEXT NOT NULL)")
            .map_err(|_| "Cannot initialize client configuration receipts")?;
        self.connection
            .execute(
                "INSERT INTO client_configuration_receipts(handle,receipt) VALUES(?1,?2)",
                params![handle, receipt],
            )
            .map_err(|_| "Cannot save client configuration receipt")?;
        Ok(())
    }

    pub fn client_configuration_receipts(&self) -> Result<Vec<(String, String)>, String> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS client_configuration_receipts(handle TEXT PRIMARY KEY, receipt TEXT NOT NULL)")
            .map_err(|_| "Cannot initialize client configuration receipts")?;
        let mut statement = self
            .connection
            .prepare("SELECT handle,receipt FROM client_configuration_receipts ORDER BY handle")
            .map_err(|_| "Cannot read client configuration receipts")?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|_| "Cannot read client configuration receipts")?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Cannot read client configuration receipts".into())
    }

    pub fn remove_client_configuration_receipt(&self, handle: &str) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM client_configuration_receipts WHERE handle=?1",
                [handle],
            )
            .map_err(|_| "Cannot remove client configuration receipt")?;
        Ok(())
    }

    /// Remove a credential only after its canonical endpoint was deleted.
    /// The caller must verify endpoint absence before invoking this method.
    pub fn purge_deleted_endpoint(&mut self, endpoint_id: &str) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "Cannot lock credential vault")?;
        transaction
            .execute(
                "DELETE FROM endpoint_credentials WHERE endpoint_id=?1",
                [endpoint_id],
            )
            .map_err(|_| "Cannot purge deleted endpoint credential")?;
        transaction
            .commit()
            .map_err(|_| "Cannot commit credential purge".to_string())
    }

    /// Core and the vault are separate stores. Check the full canonical baseline
    /// while holding the vault write lock; binding identity is checked again on
    /// every inference, so a later URL/protocol change cannot carry this key.
    pub fn write(
        &mut self,
        endpoint_id: &str,
        identity: &str,
        expected_revision: i64,
        secret: Option<&str>,
        validate_identity: impl Fn() -> Result<(), String>,
    ) -> Result<i64, String> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "Cannot lock credential vault")?;
        let revision: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM endpoint_credentials WHERE endpoint_id=?1",
                [endpoint_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Cannot read credential revision")?;
        if expected_revision < 0 || revision.unwrap_or(0) != expected_revision {
            return Err("Credential revision changed; reload before retrying".into());
        }
        validate_identity()?;
        let next = expected_revision
            .checked_add(1)
            .ok_or("Credential revision exhausted")?;
        transaction.execute("INSERT INTO endpoint_credentials(endpoint_id,identity,revision,secret) VALUES(?1,?2,?3,?4) ON CONFLICT(endpoint_id) DO UPDATE SET identity=excluded.identity,revision=excluded.revision,secret=excluded.secret", params![endpoint_id, identity, next, secret]).map_err(|_| "Cannot save endpoint credential")?;
        validate_identity()?;
        transaction
            .commit()
            .map_err(|_| "Cannot commit endpoint credential")?;
        Ok(next)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn plugin_credentials_are_package_bound_and_separate_from_endpoints() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("canonical.sqlite");
        std::fs::File::create(&database).unwrap();
        let mut vault = HostCredentialVault::open(&database).unwrap();
        vault
            .write("same", "endpoint", 0, Some("model-secret"), || Ok(()))
            .unwrap();
        vault
            .write_plugin(
                "same",
                "token",
                "digest-a",
                0,
                Some("plugin-secret"),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            vault.record("same").unwrap().unwrap().secret.as_deref(),
            Some("model-secret")
        );
        let plugin = vault.plugin_record("same", "token").unwrap().unwrap();
        assert_eq!(plugin.identity, "digest-a");
        assert_eq!(plugin.secret.as_deref(), Some("plugin-secret"));
        assert!(vault.plugin_record("other", "token").unwrap().is_none());
        assert!(vault
            .write_plugin("same", "token", "digest-b", 0, Some("bad"), || Ok(()))
            .is_err());
        assert!(vault
            .write_plugin("same", "token", "digest-b", 1, Some("bad"), || Err(
                "changed".into()
            ))
            .is_err());
        assert_eq!(
            vault
                .plugin_record("same", "token")
                .unwrap()
                .unwrap()
                .identity,
            "digest-a"
        );
        vault
            .write_plugin("same", "token", "digest-a", 1, None, || Ok(()))
            .unwrap();
        assert!(vault
            .plugin_record("same", "token")
            .unwrap()
            .unwrap()
            .secret
            .is_none());
    }

    #[test]
    fn private_vault_cas_clear_and_failed_identity_check_are_atomic() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("canonical.sqlite");
        std::fs::File::create(&database).unwrap();
        let mut vault = HostCredentialVault::open(&database).unwrap();
        let a = identity("https://a.invalid/v1/", "responses").unwrap();
        let b = identity("https://b.invalid/v1", "responses").unwrap();
        assert_eq!(a, identity("https://a.invalid/v1", "responses").unwrap());
        assert!(vault.record("endpoint-a").unwrap().is_none());
        assert_eq!(
            vault
                .write("endpoint-a", &a, 0, Some("private-key-a"), || Ok(()))
                .unwrap(),
            1
        );
        vault
            .write("endpoint-b", &b, 0, Some("private-key-b"), || Ok(()))
            .unwrap();
        assert!(vault
            .write("endpoint-a", &a, 0, Some("stale-key"), || Ok(()))
            .is_err());
        let checks = std::cell::Cell::new(0);
        assert!(vault
            .write("endpoint-a", &a, 1, Some("racing-key"), || {
                checks.set(checks.get() + 1);
                if checks.get() == 2 {
                    Err("canonical identity changed".into())
                } else {
                    Ok(())
                }
            })
            .is_err());
        assert_eq!(
            vault
                .record("endpoint-a")
                .unwrap()
                .unwrap()
                .secret
                .as_deref(),
            Some("private-key-a")
        );
        assert_eq!(
            vault.write("endpoint-a", &a, 1, None, || Ok(())).unwrap(),
            2
        );
        let cleared = vault.record("endpoint-a").unwrap().unwrap();
        assert_eq!(cleared.revision, 2);
        assert!(cleared.secret.is_none());
        assert_eq!(
            vault
                .record("endpoint-b")
                .unwrap()
                .unwrap()
                .secret
                .as_deref(),
            Some("private-key-b")
        );
        use std::os::unix::fs::PermissionsExt;
        let directory = root.path().join(".nola-host-secrets");
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for file in std::fs::read_dir(directory).unwrap() {
            assert_eq!(
                file.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn identity_rejects_credentials_fragments_and_protocol_changes() {
        for url in [
            "file:///tmp/a",
            "https://user:secret@a.invalid",
            "https://a.invalid/#fragment",
        ] {
            assert!(identity(url, "responses").is_err());
        }
        assert_ne!(
            identity("https://a.invalid/v1", "responses").unwrap(),
            identity("https://a.invalid/v1", "anthropic").unwrap()
        );
        assert_ne!(
            identity("https://a.invalid/v1?a=1", "responses").unwrap(),
            identity("https://a.invalid/v1?a=2", "responses").unwrap()
        );
    }

    #[test]
    fn vault_refuses_symlinked_private_directory() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("canonical.sqlite");
        std::fs::File::create(&database).unwrap();
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), root.path().join(".nola-host-secrets")).unwrap();
        assert!(HostCredentialVault::open(&database).is_err());
    }
}
