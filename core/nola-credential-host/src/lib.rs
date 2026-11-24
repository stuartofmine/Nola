// SPDX-License-Identifier: Apache-2.0
//! Host-private endpoint credentials. Secrets never enter canonical product
//! state, Application commands, receipts, exports, or sync. SQLite stays private.

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;

#[cfg(windows)]
pub use nola_windows_permissions as windows_permissions;

pub struct HostCredentialVault {
    connection: Connection,
    cipher: VaultCipher,
}

const VAULT_ENVELOPE_PREFIX: &str = "nola-enc-v1:";
const VAULT_ENVELOPE_DOMAIN: &[u8] = b"nola-enc-v1";
#[cfg(any(unix, windows))]
const VAULT_ENC_KEY_DOMAIN: &[u8] = b"nola-vault-enc-v1";
#[cfg(any(unix, windows))]
const VAULT_MAC_KEY_DOMAIN: &[u8] = b"nola-vault-mac-v1";
#[cfg(any(unix, windows))]
const VAULT_MASTER_KEY_LEN: usize = 32;
const VAULT_NONCE_LEN: usize = 16;
const VAULT_MAC_LEN: usize = 32;
#[cfg(any(unix, windows))]
const VAULT_KEY_FILENAME: &str = ".vault.key";

fn zeroize_bytes(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: `byte` is a valid, aligned, uniquely borrowed mutable reference.
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&a, &b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    // SAFETY: `diff` is a valid local stack variable.
    let equal = unsafe { std::ptr::read_volatile(&diff) } == 0;
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    equal
}

fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let mut digest: [u8; 32] = Sha256::digest(key).into();
        key_block[..32].copy_from_slice(&digest);
        zeroize_bytes(&mut digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for index in 0..BLOCK_SIZE {
        ipad[index] ^= key_block[index];
        opad[index] ^= key_block[index];
    }

    let mut inner_hasher = Sha256::new();
    inner_hasher.update(ipad);
    for part in parts {
        inner_hasher.update(part);
    }
    let mut inner_digest: [u8; 32] = inner_hasher.finalize().into();

    let mut outer_hasher = Sha256::new();
    outer_hasher.update(opad);
    outer_hasher.update(inner_digest);
    let mac: [u8; 32] = outer_hasher.finalize().into();

    zeroize_bytes(&mut key_block);
    zeroize_bytes(&mut ipad);
    zeroize_bytes(&mut opad);
    zeroize_bytes(&mut inner_digest);
    mac
}

fn apply_ctr_keystream(
    enc_key: &[u8; 32],
    nonce: &[u8; VAULT_NONCE_LEN],
    data: &mut [u8],
) -> Result<(), String> {
    let mut counter: u32 = 0;
    for chunk in data.chunks_mut(32) {
        let mut hasher = Sha256::new();
        hasher.update(enc_key);
        hasher.update(nonce);
        hasher.update(counter.to_be_bytes());
        let mut block: [u8; 32] = hasher.finalize().into();
        for (dst, keystream_byte) in chunk.iter_mut().zip(block.iter()) {
            *dst ^= *keystream_byte;
        }
        zeroize_bytes(&mut block);
        counter = counter
            .checked_add(1)
            .ok_or_else(|| "Credential payload too large".to_string())?;
    }
    Ok(())
}

#[cfg(unix)]
fn fill_random_bytes(dest: &mut [u8]) -> Result<(), String> {
    use std::io::Read;
    let mut file =
        std::fs::File::open("/dev/urandom").map_err(|_| "Cannot access system randomness")?;
    file.read_exact(dest)
        .map_err(|_| "Cannot read system randomness".to_string())
}

#[cfg(not(unix))]
fn fill_random_bytes(dest: &mut [u8]) -> Result<(), String> {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut offset = 0usize;
    while offset < dest.len() {
        let mut hasher = Sha256::new();
        hasher.update(b"nola-vault-rng-v1");
        hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_be_bytes());
        hasher.update(std::process::id().to_be_bytes());
        let now_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        hasher.update(now_nanos.to_be_bytes());
        hasher.update(
            format!("{:?}{:?}", Instant::now(), std::thread::current().id()).as_bytes(),
        );
        for round in 0u8..4 {
            let state = RandomState::new();
            let mut state_hasher = state.build_hasher();
            state_hasher.write_u8(round);
            hasher.update(state_hasher.finish().to_be_bytes());
        }
        let mut block: [u8; 32] = hasher.finalize().into();
        let take = (dest.len() - offset).min(block.len());
        dest[offset..offset + take].copy_from_slice(&block[..take]);
        zeroize_bytes(&mut block);
        offset += take;
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("Corrupted encrypted credential".into()),
    }
}

fn hex_decode(input: &str) -> Result<Vec<u8>, String> {
    let raw = input.as_bytes();
    if raw.len() % 2 != 0 {
        return Err("Corrupted encrypted credential".into());
    }
    let mut out = Vec::with_capacity(raw.len() / 2);
    for pair in raw.chunks_exact(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

struct VaultCipher {
    enc_key: [u8; 32],
    mac_key: [u8; 32],
}

impl Drop for VaultCipher {
    fn drop(&mut self) {
        zeroize_bytes(&mut self.enc_key);
        zeroize_bytes(&mut self.mac_key);
    }
}

impl VaultCipher {
    #[cfg(any(unix, windows))]
    fn from_master_key(master_key: &[u8; VAULT_MASTER_KEY_LEN]) -> Self {
        let enc_key = hmac_sha256(master_key, &[VAULT_ENC_KEY_DOMAIN]);
        let mac_key = hmac_sha256(master_key, &[VAULT_MAC_KEY_DOMAIN]);
        Self { enc_key, mac_key }
    }

    fn encrypt(&self, plaintext: &str, aad_identity: &str) -> Result<String, String> {
        let mut nonce = [0u8; VAULT_NONCE_LEN];
        fill_random_bytes(&mut nonce)?;
        let mut buffer = plaintext.as_bytes().to_vec();
        if let Err(error) = apply_ctr_keystream(&self.enc_key, &nonce, &mut buffer) {
            zeroize_bytes(&mut buffer);
            zeroize_bytes(&mut nonce);
            return Err(error);
        }
        let mut mac = hmac_sha256(
            &self.mac_key,
            &[
                VAULT_ENVELOPE_DOMAIN,
                aad_identity.as_bytes(),
                &nonce,
                &buffer,
            ],
        );
        let encoded = format!(
            "{VAULT_ENVELOPE_PREFIX}{}:{}:{}",
            hex_encode(&nonce),
            hex_encode(&buffer),
            hex_encode(&mac),
        );
        zeroize_bytes(&mut buffer);
        zeroize_bytes(&mut nonce);
        zeroize_bytes(&mut mac);
        Ok(encoded)
    }

    fn decrypt(&self, stored: &str, aad_identity: &str) -> Result<String, String> {
        let Some(rest) = stored.strip_prefix(VAULT_ENVELOPE_PREFIX) else {
            // Backward compatibility: existing legacy plaintext rows remain readable
            // and are upgraded to encrypted envelopes on the next write.
            return Ok(stored.to_owned());
        };
        let mut parts = rest.split(':');
        let (Some(nonce_hex), Some(ct_hex), Some(mac_hex), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err("Corrupted encrypted credential".into());
        };
        let mut nonce_vec = hex_decode(nonce_hex)?;
        if nonce_vec.len() != VAULT_NONCE_LEN {
            zeroize_bytes(&mut nonce_vec);
            return Err("Corrupted encrypted credential".into());
        }
        let mut nonce = [0u8; VAULT_NONCE_LEN];
        nonce.copy_from_slice(&nonce_vec);
        zeroize_bytes(&mut nonce_vec);

        let mut ciphertext = hex_decode(ct_hex)?;
        let mut provided_mac = hex_decode(mac_hex)?;
        if provided_mac.len() != VAULT_MAC_LEN {
            zeroize_bytes(&mut nonce);
            zeroize_bytes(&mut ciphertext);
            zeroize_bytes(&mut provided_mac);
            return Err("Corrupted encrypted credential".into());
        }

        let mut expected_mac = hmac_sha256(
            &self.mac_key,
            &[
                VAULT_ENVELOPE_DOMAIN,
                aad_identity.as_bytes(),
                &nonce,
                &ciphertext,
            ],
        );
        let valid = constant_time_eq(&expected_mac, &provided_mac);
        zeroize_bytes(&mut expected_mac);
        zeroize_bytes(&mut provided_mac);
        if !valid {
            zeroize_bytes(&mut nonce);
            zeroize_bytes(&mut ciphertext);
            return Err("Credential integrity verification failed".into());
        }

        if let Err(error) = apply_ctr_keystream(&self.enc_key, &nonce, &mut ciphertext) {
            zeroize_bytes(&mut nonce);
            zeroize_bytes(&mut ciphertext);
            return Err(error);
        }
        zeroize_bytes(&mut nonce);

        match String::from_utf8(ciphertext) {
            Ok(plaintext) => Ok(plaintext),
            Err(error) => {
                let mut bytes = error.into_bytes();
                zeroize_bytes(&mut bytes);
                Err("Corrupted encrypted credential".into())
            }
        }
    }
}

fn endpoint_aad(endpoint_id: &str, identity: &str) -> String {
    format!(
        "endpoint_credentials:{}:{endpoint_id}:{identity}",
        endpoint_id.len()
    )
}

fn plugin_aad(plugin_id: &str, reference: &str, package_digest: &str) -> String {
    format!(
        "plugin_credentials:{}:{}:{plugin_id}:{reference}:{package_digest}",
        plugin_id.len(),
        reference.len()
    )
}

fn runtime_descriptor_aad(handle: &str) -> String {
    format!("runtime_descriptors:{handle}")
}

fn client_receipt_aad(handle: &str) -> String {
    format!("client_configuration_receipts:{handle}")
}

#[cfg(any(unix, windows))]
enum KeyFileRead {
    Missing,
    Empty,
    Key([u8; VAULT_MASTER_KEY_LEN]),
}

#[cfg(any(unix, windows))]
fn read_master_key_once(key_path: &Path) -> Result<KeyFileRead, String> {
    use std::io::Read;

    if let Ok(metadata) = std::fs::symlink_metadata(key_path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Unsafe credential vault key file".into());
        }
    }

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }

    let mut file = match options.open(key_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(KeyFileRead::Missing);
        }
        Err(_) => return Err("Cannot open credential vault key file".into()),
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| "Cannot protect credential vault key file")?;
    }
    #[cfg(windows)]
    windows_permissions::protect_private_path(key_path)?;

    let mut raw = Vec::with_capacity(VAULT_MASTER_KEY_LEN);
    file.read_to_end(&mut raw)
        .map_err(|_| "Cannot read credential vault key file")?;
    if raw.is_empty() {
        return Ok(KeyFileRead::Empty);
    }
    if raw.len() != VAULT_MASTER_KEY_LEN {
        zeroize_bytes(&mut raw);
        return Err("Corrupted credential vault key file".into());
    }
    let mut key = [0u8; VAULT_MASTER_KEY_LEN];
    key.copy_from_slice(&raw);
    zeroize_bytes(&mut raw);
    Ok(KeyFileRead::Key(key))
}

#[cfg(any(unix, windows))]
fn read_existing_master_key(key_path: &Path) -> Result<Option<[u8; VAULT_MASTER_KEY_LEN]>, String> {
    for attempt in 0..25 {
        match read_master_key_once(key_path)? {
            KeyFileRead::Key(key) => return Ok(Some(key)),
            KeyFileRead::Missing => return Ok(None),
            KeyFileRead::Empty if attempt < 24 => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            KeyFileRead::Empty => return Err("Corrupted credential vault key file".into()),
        }
    }
    Err("Cannot read credential vault key file".into())
}

#[cfg(any(unix, windows))]
fn load_or_create_master_key(directory: &Path) -> Result<[u8; VAULT_MASTER_KEY_LEN], String> {
    use std::io::Write;

    let key_path = directory.join(VAULT_KEY_FILENAME);
    if let Some(existing) = read_existing_master_key(&key_path)? {
        return Ok(existing);
    }

    let mut generated = [0u8; VAULT_MASTER_KEY_LEN];
    fill_random_bytes(&mut generated)?;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }

    match options.open(&key_path) {
        Ok(mut file) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if file
                    .set_permissions(std::fs::Permissions::from_mode(0o600))
                    .is_err()
                {
                    zeroize_bytes(&mut generated);
                    let _ = std::fs::remove_file(&key_path);
                    return Err("Cannot protect credential vault key file".into());
                }
            }
            if file.write_all(&generated).is_err() || file.sync_all().is_err() {
                zeroize_bytes(&mut generated);
                let _ = std::fs::remove_file(&key_path);
                return Err("Cannot persist credential vault key file".into());
            }
            #[cfg(windows)]
            if let Err(error) = windows_permissions::protect_private_path(&key_path) {
                zeroize_bytes(&mut generated);
                let _ = std::fs::remove_file(&key_path);
                return Err(error);
            }
            Ok(generated)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            zeroize_bytes(&mut generated);
            read_existing_master_key(&key_path)?
                .ok_or_else(|| "Cannot read credential vault key file".to_string())
        }
        Err(_) => {
            zeroize_bytes(&mut generated);
            Err("Cannot create credential vault key file".into())
        }
    }
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

// Released vaults have no SQLite user_version and always contain endpoint_credentials.
// Check before switching journaling modes or seeding a table in an unrelated file.
#[cfg(any(unix, windows))]
fn recognize_vault(connection: &Connection) -> Result<(), String> {
    let (version, application, empty, endpoints): (i64, i64, bool, bool) = connection.query_row(
        "SELECT (SELECT user_version FROM pragma_user_version),
                (SELECT application_id FROM pragma_application_id),
                NOT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'),
                EXISTS(SELECT 1 FROM sqlite_schema WHERE name='endpoint_credentials' AND type='table')",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).map_err(|_| "Cannot inspect credential vault")?;
    if version == 0 && application == 0 && (empty || (endpoints && connection
        .prepare("SELECT endpoint_id, identity, revision, secret FROM endpoint_credentials LIMIT 0").is_ok())) {
        return Ok(());
    }
    Err("Unsupported credential vault format".into())
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
        let row: Option<(String, i64, Option<String>)> = self
            .connection
            .query_row(
                "SELECT package_digest,revision,secret FROM plugin_credentials WHERE plugin_id=?1 AND reference=?2",
                params![plugin_id, reference],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| "Cannot read plugin credential".to_string())?;
        match row {
            Some((package_digest, revision, stored_secret)) => {
                let aad = plugin_aad(plugin_id, reference, &package_digest);
                let secret = match stored_secret {
                    Some(stored) => Some(self.cipher.decrypt(&stored, &aad)?),
                    None => None,
                };
                Ok(Some(CredentialRecord {
                    identity: package_digest,
                    revision,
                    secret,
                }))
            }
            None => Ok(None),
        }
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
        let aad = plugin_aad(plugin_id, reference, package_digest);
        let encrypted_secret = match secret {
            Some(plaintext) => Some(self.cipher.encrypt(plaintext, &aad)?),
            None => None,
        };
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
            params![plugin_id, reference, package_digest, next, encrypted_secret.as_deref()]).map_err(|_| "Cannot save plugin credential")?;
        validate()?;
        transaction
            .commit()
            .map_err(|_| "Cannot commit plugin credential")?;
        Ok(next)
    }

    #[cfg(not(any(unix, windows)))]
    pub fn open(_: &Path) -> Result<Self, String> {
        Err(
            "Private endpoint credentials require a host with protected Unix file permissions"
                .into(),
        )
    }

    #[cfg(any(unix, windows))]
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
        #[cfg(windows)]
        windows_permissions::protect_private_path(&directory)?;
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
        #[cfg(windows)]
        windows_permissions::protect_private_path(&path)?;
        let connection = Connection::open(&path).map_err(|_| "Cannot open credential vault")?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "Cannot configure credential vault")?;
        recognize_vault(&connection)?;
        let mut master_key = load_or_create_master_key(&directory)?;
        let cipher = VaultCipher::from_master_key(&master_key);
        zeroize_bytes(&mut master_key);
        connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA secure_delete=ON; CREATE TABLE IF NOT EXISTS endpoint_credentials(endpoint_id TEXT PRIMARY KEY, identity TEXT NOT NULL, revision INTEGER NOT NULL, secret TEXT);")
            .map_err(|_| "Cannot initialize credential vault")?;
        Ok(Self { connection, cipher })
    }

    pub fn record(&self, endpoint_id: &str) -> Result<Option<CredentialRecord>, String> {
        let row: Option<(String, i64, Option<String>)> = self
            .connection
            .query_row(
                "SELECT identity,revision,secret FROM endpoint_credentials WHERE endpoint_id=?1",
                [endpoint_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| "Cannot read endpoint credential".to_string())?;
        match row {
            Some((identity, revision, stored_secret)) => {
                let aad = endpoint_aad(endpoint_id, &identity);
                let secret = match stored_secret {
                    Some(stored) => Some(self.cipher.decrypt(&stored, &aad)?),
                    None => None,
                };
                Ok(Some(CredentialRecord {
                    identity,
                    revision,
                    secret,
                }))
            }
            None => Ok(None),
        }
    }

    /// Immutable Host reconstruction material. It is separate from endpoint
    /// credentials: clearing a credential cannot overwrite a Run descriptor.
    pub fn save_runtime_descriptor(&self, handle: &str, descriptor: &str) -> Result<(), String> {
        if handle.len() != 32 || !handle.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid runtime descriptor handle".into());
        }
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS runtime_descriptors(handle TEXT PRIMARY KEY, descriptor TEXT NOT NULL)")
            .map_err(|_| "Cannot initialize runtime descriptors")?;
        let encrypted = self
            .cipher
            .encrypt(descriptor, &runtime_descriptor_aad(handle))?;
        self.connection
            .execute(
                "INSERT INTO runtime_descriptors(handle,descriptor) VALUES(?1,?2)",
                params![handle, encrypted],
            )
            .map_err(|_| "Cannot persist immutable runtime descriptor")?;
        Ok(())
    }

    pub fn runtime_descriptor(&self, handle: &str) -> Result<Option<String>, String> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS runtime_descriptors(handle TEXT PRIMARY KEY, descriptor TEXT NOT NULL)")
            .map_err(|_| "Cannot initialize runtime descriptors")?;
        let stored: Option<String> = self
            .connection
            .query_row(
                "SELECT descriptor FROM runtime_descriptors WHERE handle=?1",
                [handle],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Cannot read runtime descriptor".to_string())?;
        match stored {
            Some(value) => self
                .cipher
                .decrypt(&value, &runtime_descriptor_aad(handle))
                .map(Some),
            None => Ok(None),
        }
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
        let encrypted = self.cipher.encrypt(receipt, &client_receipt_aad(handle))?;
        self.connection
            .execute(
                "INSERT INTO client_configuration_receipts(handle,receipt) VALUES(?1,?2)",
                params![handle, encrypted],
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
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|_| "Cannot read client configuration receipts")?;
        let raw_entries = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Cannot read client configuration receipts".to_string())?;
        let mut decrypted = Vec::with_capacity(raw_entries.len());
        for (handle, stored_receipt) in raw_entries {
            let receipt = self
                .cipher
                .decrypt(&stored_receipt, &client_receipt_aad(&handle))?;
            decrypted.push((handle, receipt));
        }
        Ok(decrypted)
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
        let aad = endpoint_aad(endpoint_id, identity);
        let encrypted_secret = match secret {
            Some(plaintext) => Some(self.cipher.encrypt(plaintext, &aad)?),
            None => None,
        };
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
        transaction.execute("INSERT INTO endpoint_credentials(endpoint_id,identity,revision,secret) VALUES(?1,?2,?3,?4) ON CONFLICT(endpoint_id) DO UPDATE SET identity=excluded.identity,revision=excluded.revision,secret=excluded.secret", params![endpoint_id, identity, next, encrypted_secret.as_deref()]).map_err(|_| "Cannot save endpoint credential")?;
        validate_identity()?;
        transaction
            .commit()
            .map_err(|_| "Cannot commit endpoint credential")?;
        Ok(next)
    }
}

#[cfg(all(test, any(unix, windows)))]
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
        #[cfg(unix)]
        {
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
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn foreign_or_future_vault_is_rejected_without_seeding_tables() {
        for sql in ["CREATE TABLE notes(body TEXT)", "PRAGMA user_version=42"] {
            let dir = tempfile::tempdir().unwrap();
            let database = dir.path().join("fixture.sqlite");
            std::fs::File::create(&database).unwrap();
            drop(HostCredentialVault::open(&database).unwrap());
            let file = vault_sqlite_path(dir.path());
            std::fs::remove_file(&file).unwrap();
            Connection::open(&file).unwrap().execute_batch(sql).unwrap();
            let original = std::fs::read(&file).unwrap();
            assert!(matches!(HostCredentialVault::open(&database), Err(error) if error == "Unsupported credential vault format"));
            assert_eq!(std::fs::read(&file).unwrap(), original);
        }
    }

    fn vault_sqlite_path(root: &Path) -> std::path::PathBuf {
        std::fs::read_dir(root.join(".nola-host-secrets"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sqlite"))
            .expect("vault sqlite file should exist")
    }

    fn flip_first_hex_nibble(hex: &str) -> String {
        let mut chars: Vec<char> = hex.chars().collect();
        assert!(!chars.is_empty());
        chars[0] = if chars[0] == '0' { '1' } else { '0' };
        chars.into_iter().collect()
    }

    #[test]
    fn secrets_are_encrypted_at_rest_and_round_trip_across_reopen() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("canonical.sqlite");
        std::fs::File::create(&database).unwrap();
        let mut vault = HostCredentialVault::open(&database).unwrap();
        let endpoint_identity = identity("https://api.example.invalid/v1", "responses").unwrap();
        let endpoint_secret = "sk-nola-super-secret-endpoint-token-9876543210";
        let plugin_secret = "plg-secret-value-abcdef123456";
        let descriptor_handle = "0123456789abcdef0123456789abcdef";
        let descriptor_body = "{\"runtime\":\"secret-runtime-material\"}";
        let receipt_handle = "fedcba9876543210fedcba9876543210";
        let receipt_body = "{\"gatewayToken\":\"nola-gateway-token-xyz\"}";

        vault
            .write("ep-1", &endpoint_identity, 0, Some(endpoint_secret), || {
                Ok(())
            })
            .unwrap();
        vault
            .write("ep-empty", &endpoint_identity, 0, Some(""), || Ok(()))
            .unwrap();
        vault
            .write_plugin("plug-1", "api_key", "sha256:pkg1", 0, Some(plugin_secret), || {
                Ok(())
            })
            .unwrap();
        vault
            .save_runtime_descriptor(descriptor_handle, descriptor_body)
            .unwrap();
        vault
            .save_client_configuration_receipt(receipt_handle, receipt_body)
            .unwrap();

        let key_path = root.path().join(".nola-host-secrets").join(VAULT_KEY_FILENAME);
        let key_bytes = std::fs::read(&key_path).unwrap();
        assert_eq!(key_bytes.len(), VAULT_MASTER_KEY_LEN);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let sqlite_path = vault_sqlite_path(root.path());
        let raw_conn = Connection::open(&sqlite_path).unwrap();
        let raw_endpoint_secret: String = raw_conn
            .query_row(
                "SELECT secret FROM endpoint_credentials WHERE endpoint_id = 'ep-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(raw_endpoint_secret.starts_with(VAULT_ENVELOPE_PREFIX));
        assert!(!raw_endpoint_secret.contains(endpoint_secret));

        let raw_plugin_secret: String = raw_conn
            .query_row(
                "SELECT secret FROM plugin_credentials WHERE plugin_id = 'plug-1' AND reference = 'api_key'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(raw_plugin_secret.starts_with(VAULT_ENVELOPE_PREFIX));
        assert!(!raw_plugin_secret.contains(plugin_secret));

        let raw_descriptor: String = raw_conn
            .query_row(
                "SELECT descriptor FROM runtime_descriptors WHERE handle = ?1",
                [descriptor_handle],
                |row| row.get(0),
            )
            .unwrap();
        assert!(raw_descriptor.starts_with(VAULT_ENVELOPE_PREFIX));
        assert!(!raw_descriptor.contains("secret-runtime-material"));

        let raw_receipt: String = raw_conn
            .query_row(
                "SELECT receipt FROM client_configuration_receipts WHERE handle = ?1",
                [receipt_handle],
                |row| row.get(0),
            )
            .unwrap();
        assert!(raw_receipt.starts_with(VAULT_ENVELOPE_PREFIX));
        assert!(!raw_receipt.contains("nola-gateway-token-xyz"));

        // Rewriting the same secret must produce a fresh ciphertext (random nonce).
        vault
            .write("ep-1", &endpoint_identity, 1, Some(endpoint_secret), || {
                Ok(())
            })
            .unwrap();
        let rewritten_raw: String = raw_conn
            .query_row(
                "SELECT secret FROM endpoint_credentials WHERE endpoint_id = 'ep-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(raw_endpoint_secret, rewritten_raw);
        drop(raw_conn);
        drop(vault);

        // Reopening the vault uses the persisted .vault.key and decrypts every record cleanly.
        let reopened = HostCredentialVault::open(&database).unwrap();
        assert_eq!(
            reopened.record("ep-1").unwrap().unwrap().secret.as_deref(),
            Some(endpoint_secret)
        );
        assert_eq!(
            reopened
                .record("ep-empty")
                .unwrap()
                .unwrap()
                .secret
                .as_deref(),
            Some("")
        );
        assert_eq!(
            reopened
                .plugin_record("plug-1", "api_key")
                .unwrap()
                .unwrap()
                .secret
                .as_deref(),
            Some(plugin_secret)
        );
        assert_eq!(
            reopened
                .runtime_descriptor(descriptor_handle)
                .unwrap()
                .as_deref(),
            Some(descriptor_body)
        );
        assert_eq!(
            reopened.client_configuration_receipts().unwrap(),
            vec![(receipt_handle.to_string(), receipt_body.to_string())]
        );
    }

    #[test]
    fn tampered_ciphertext_tag_or_swapped_aad_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("canonical.sqlite");
        std::fs::File::create(&database).unwrap();
        let mut vault = HostCredentialVault::open(&database).unwrap();
        let id_a = identity("https://a.example.invalid/v1", "responses").unwrap();
        let id_b = identity("https://b.example.invalid/v1", "responses").unwrap();

        vault
            .write("ep-a", &id_a, 0, Some("secret-for-a"), || Ok(()))
            .unwrap();
        vault
            .write("ep-b", &id_b, 0, Some("secret-for-b"), || Ok(()))
            .unwrap();
        vault
            .write_plugin("plug-a", "token", "digest-1", 0, Some("plugin-tok"), || {
                Ok(())
            })
            .unwrap();

        let sqlite_path = vault_sqlite_path(root.path());
        let raw_conn = Connection::open(&sqlite_path).unwrap();
        let raw_a: String = raw_conn
            .query_row(
                "SELECT secret FROM endpoint_credentials WHERE endpoint_id = 'ep-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let raw_b: String = raw_conn
            .query_row(
                "SELECT secret FROM endpoint_credentials WHERE endpoint_id = 'ep-b'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let parts: Vec<&str> = raw_a
            .strip_prefix(VAULT_ENVELOPE_PREFIX)
            .unwrap()
            .split(':')
            .collect();
        assert_eq!(parts.len(), 3);

        // 1. Tampered ciphertext fails closed.
        let tampered_ct = format!(
            "{VAULT_ENVELOPE_PREFIX}{}:{}:{}",
            parts[0],
            flip_first_hex_nibble(parts[1]),
            parts[2]
        );
        raw_conn
            .execute(
                "UPDATE endpoint_credentials SET secret = ?1 WHERE endpoint_id = 'ep-a'",
                [&tampered_ct],
            )
            .unwrap();
        assert!(vault.record("ep-a").is_err());

        // 2. Tampered MAC tag fails closed.
        let tampered_mac = format!(
            "{VAULT_ENVELOPE_PREFIX}{}:{}:{}",
            parts[0],
            parts[1],
            flip_first_hex_nibble(parts[2])
        );
        raw_conn
            .execute(
                "UPDATE endpoint_credentials SET secret = ?1 WHERE endpoint_id = 'ep-a'",
                [&tampered_mac],
            )
            .unwrap();
        assert!(vault.record("ep-a").is_err());

        // 3. Swapping ciphertext from ep-b into ep-a fails AAD verification.
        raw_conn
            .execute(
                "UPDATE endpoint_credentials SET secret = ?1 WHERE endpoint_id = 'ep-a'",
                [&raw_b],
            )
            .unwrap();
        assert!(vault.record("ep-a").is_err());

        // 4. Altering canonical identity on ep-a with original ciphertext fails AAD verification.
        raw_conn
            .execute(
                "UPDATE endpoint_credentials SET secret = ?1, identity = ?2 WHERE endpoint_id = 'ep-a'",
                params![&raw_a, &id_b],
            )
            .unwrap();
        assert!(vault.record("ep-a").is_err());

        // 5. Altering package_digest on a plugin row fails AAD verification.
        raw_conn
            .execute(
                "UPDATE plugin_credentials SET package_digest = 'digest-tampered' WHERE plugin_id = 'plug-a'",
                [],
            )
            .unwrap();
        assert!(vault.plugin_record("plug-a", "token").is_err());
    }

    #[test]
    fn legacy_plaintext_rows_read_cleanly_and_upgrade_on_update() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("canonical.sqlite");
        std::fs::File::create(&database).unwrap();
        let mut vault = HostCredentialVault::open(&database).unwrap();
        let ep_identity = identity("https://legacy.example.invalid/v1", "responses").unwrap();
        let sqlite_path = vault_sqlite_path(root.path());

        // Ensure plugin_credentials exists, then seed legacy unencrypted plaintext rows directly.
        assert!(vault.plugin_record("legacy-plug", "tok").unwrap().is_none());
        let raw_conn = Connection::open(&sqlite_path).unwrap();
        raw_conn
            .execute(
                "INSERT INTO endpoint_credentials(endpoint_id, identity, revision, secret) VALUES(?1, ?2, 1, ?3)",
                params!["legacy-ep", &ep_identity, "legacy-plaintext-api-key"],
            )
            .unwrap();
        raw_conn
            .execute(
                "INSERT INTO plugin_credentials(plugin_id, reference, package_digest, revision, secret) VALUES(?1, ?2, ?3, 1, ?4)",
                params!["legacy-plug", "tok", "digest-legacy", "legacy-plugin-secret"],
            )
            .unwrap();

        // Reading legacy plaintext rows succeeds transparently.
        let ep = vault.record("legacy-ep").unwrap().unwrap();
        assert_eq!(ep.revision, 1);
        assert_eq!(ep.secret.as_deref(), Some("legacy-plaintext-api-key"));
        let plug = vault.plugin_record("legacy-plug", "tok").unwrap().unwrap();
        assert_eq!(plug.revision, 1);
        assert_eq!(plug.secret.as_deref(), Some("legacy-plugin-secret"));

        // Updating upgrades both rows to authenticated ciphertext at rest.
        vault
            .write(
                "legacy-ep",
                &ep_identity,
                1,
                Some("rotated-encrypted-api-key"),
                || Ok(()),
            )
            .unwrap();
        vault
            .write_plugin(
                "legacy-plug",
                "tok",
                "digest-legacy",
                1,
                Some("rotated-plugin-secret"),
                || Ok(()),
            )
            .unwrap();

        let upgraded_ep_raw: String = raw_conn
            .query_row(
                "SELECT secret FROM endpoint_credentials WHERE endpoint_id = 'legacy-ep'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(upgraded_ep_raw.starts_with(VAULT_ENVELOPE_PREFIX));
        assert!(!upgraded_ep_raw.contains("rotated-encrypted-api-key"));

        let upgraded_plug_raw: String = raw_conn
            .query_row(
                "SELECT secret FROM plugin_credentials WHERE plugin_id = 'legacy-plug' AND reference = 'tok'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(upgraded_plug_raw.starts_with(VAULT_ENVELOPE_PREFIX));
        assert!(!upgraded_plug_raw.contains("rotated-plugin-secret"));

        assert_eq!(
            vault.record("legacy-ep").unwrap().unwrap().secret.as_deref(),
            Some("rotated-encrypted-api-key")
        );
        assert_eq!(
            vault
                .plugin_record("legacy-plug", "tok")
                .unwrap()
                .unwrap()
                .secret
                .as_deref(),
            Some("rotated-plugin-secret")
        );
    }

    #[test]
    fn zeroize_bytes_and_hmac_sha256_primitives_work_as_specified() {
        let mut sensitive = [0xabu8; 64];
        zeroize_bytes(&mut sensitive);
        assert!(sensitive.iter().all(|&byte| byte == 0));

        // RFC 4231 Test Case 2.
        let mac = hmac_sha256(b"Jefe", &[b"what do ya want for nothing?"]);
        assert_eq!(
            hex_encode(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );

        assert!(constant_time_eq(b"equal-bytes", b"equal-bytes"));
        assert!(!constant_time_eq(b"equal-bytes", b"equal-bytez"));
        assert!(!constant_time_eq(b"short", b"longer"));
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

    #[cfg(unix)]
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
