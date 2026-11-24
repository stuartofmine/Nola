// SPDX-License-Identifier: Apache-2.0
//! CSPRNG + AES-256-GCM secret vault with atomic 0600 file writes and AAD key-binding.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::StoreError;

#[derive(Debug, Default, Serialize, Deserialize)]
struct VaultFile {
    version: u32,
    entries: BTreeMap<String, EncryptedEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedEntry {
    nonce_b64: String,
    ciphertext_b64: String,
}

/// Encrypted local secret store for API keys and OAuth tokens.
pub struct Vault {
    vault_path: PathBuf,
    master_key: [u8; 32],
}

impl Vault {
    /// Open or initialize a vault at `vault_path` (key stored in `<vault_path>.key` with mode 0600).
    pub fn open(vault_path: PathBuf) -> Result<Self, StoreError> {
        if let Some(parent) = vault_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let key_path = PathBuf::from(format!("{}.key", vault_path.display()));
        let master_key = if key_path.exists() {
            let bytes = fs::read(&key_path)?;
            if bytes.len() != 32 {
                return Err(StoreError::Crypto("invalid master key length".into()));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            key
        } else {
            let mut key = [0u8; 32];
            getrandom::getrandom(&mut key)
                .map_err(|e| StoreError::Crypto(format!("csprng failed: {e}")))?;
            atomic_write_restricted(&key_path, &key)?;
            key
        };
        Ok(Self {
            vault_path,
            master_key,
        })
    }

    pub fn set_secret(&self, name: &str, plaintext: &str) -> Result<(), StoreError> {
        let mut file = self.load_file()?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.master_key));
        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes)
            .map_err(|e| StoreError::Crypto(format!("nonce csprng failed: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        // Bind AAD to secret name so ciphertexts cannot be swapped across keys.
        let ct = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| StoreError::Crypto("aes-gcm encryption failed".into()))?;
        file.version = 2;
        file.entries.insert(
            name.to_string(),
            EncryptedEntry {
                nonce_b64: B64.encode(nonce_bytes),
                ciphertext_b64: B64.encode(ct),
            },
        );
        let serialized = serde_json::to_vec_pretty(&file)?;
        atomic_write_restricted(&self.vault_path, &serialized)?;
        Ok(())
    }

    pub fn get_secret(&self, name: &str) -> Result<Option<String>, StoreError> {
        let file = self.load_file()?;
        let Some(entry) = file.entries.get(name) else {
            return Ok(None);
        };
        let nonce_bytes = B64
            .decode(&entry.nonce_b64)
            .map_err(|e| StoreError::Crypto(format!("invalid nonce base64: {e}")))?;
        if nonce_bytes.len() != 12 {
            return Err(StoreError::Crypto("invalid nonce length".into()));
        }
        let ct = B64
            .decode(&entry.ciphertext_b64)
            .map_err(|e| StoreError::Crypto(format!("invalid ciphertext base64: {e}")))?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.master_key));
        let pt = cipher
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &ct,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| StoreError::Crypto("aes-gcm authentication failed".into()))?;
        let s = String::from_utf8(pt)
            .map_err(|e| StoreError::Crypto(format!("non-utf8 secret: {e}")))?;
        Ok(Some(s))
    }

    pub fn delete_secret(&self, name: &str) -> Result<bool, StoreError> {
        let mut file = self.load_file()?;
        let removed = file.entries.remove(name).is_some();
        if removed {
            let serialized = serde_json::to_vec_pretty(&file)?;
            atomic_write_restricted(&self.vault_path, &serialized)?;
        }
        Ok(removed)
    }

    pub fn list_keys(&self) -> Result<Vec<String>, StoreError> {
        let file = self.load_file()?;
        Ok(file.entries.keys().cloned().collect())
    }

    fn load_file(&self) -> Result<VaultFile, StoreError> {
        if !self.vault_path.exists() {
            return Ok(VaultFile {
                version: 2,
                entries: BTreeMap::new(),
            });
        }
        let raw = fs::read(&self.vault_path)?;
        Ok(serde_json::from_slice(&raw)?)
    }
}

/// Write `bytes` to a unique temp file with owner-only `0600` permissions BEFORE atomic rename.
pub fn atomic_write_restricted(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut suffix = [0u8; 8];
    getrandom::getrandom(&mut suffix)
        .map_err(|e| StoreError::Crypto(format!("temp suffix csprng failed: {e}")))?;
    let suffix_hex: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
    let tmp_path = PathBuf::from(format!("{}.tmp.{suffix_hex}", path.display()));

    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp_path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_roundtrip_and_aad_tamper_detection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.json");
        let vault = Vault::open(path.clone()).unwrap();
        vault.set_secret("openai_key", "sk-secret-123").unwrap();
        assert_eq!(
            vault.get_secret("openai_key").unwrap().as_deref(),
            Some("sk-secret-123")
        );
        assert_eq!(vault.list_keys().unwrap(), vec!["openai_key"]);

        // Re-open with same key
        let vault2 = Vault::open(path.clone()).unwrap();
        assert_eq!(
            vault2.get_secret("openai_key").unwrap().as_deref(),
            Some("sk-secret-123")
        );

        // Tampering by renaming key inside JSON must fail AES-GCM AAD check
        let raw = fs::read_to_string(&path).unwrap().replace("openai_key", "other_key");
        fs::write(&path, raw).unwrap();
        assert!(vault2.get_secret("other_key").is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key_meta = fs::metadata(format!("{}.key", path.display())).unwrap();
            assert_eq!(key_meta.permissions().mode() & 0o777, 0o600);
        }
    }
}
