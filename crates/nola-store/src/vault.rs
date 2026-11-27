// SPDX-License-Identifier: Apache-2.0
//! CSPRNG + AES-256-GCM secret vault with atomic 0600 file writes, AAD key-binding,
//! in-process + cross-process `flock` serialization, TOCTOU-free master key creation,
//! and master-key zeroization on Drop.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    lock_path: PathBuf,
    master_key: [u8; 32],
    mu: Mutex<()>,
}

impl Drop for Vault {
    fn drop(&mut self) {
        for b in &mut self.master_key {
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

impl Vault {
    /// Open or initialize a vault at `vault_path` (key stored in `<vault_path>.key` with mode 0600).
    pub fn open(vault_path: PathBuf) -> Result<Self, StoreError> {
        if let Some(parent) = vault_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let lock_path = PathBuf::from(format!("{}.lock", vault_path.display()));
        let _flock = acquire_blocking_flock(&lock_path)?;

        let key_path = PathBuf::from(format!("{}.key", vault_path.display()));
        let master_key = match fs::read(&key_path) {
            Ok(bytes) => {
                if bytes.len() != 32 {
                    return Err(StoreError::Crypto("invalid master key length".into()));
                }
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                key
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut key = [0u8; 32];
                getrandom::getrandom(&mut key)
                    .map_err(|e| StoreError::Crypto(format!("csprng failed: {e}")))?;
                let mut opts = OpenOptions::new();
                opts.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    opts.mode(0o600);
                }
                match opts.open(&key_path) {
                    Ok(mut f) => {
                        f.write_all(&key)?;
                        f.sync_all()?;
                        drop(f);
                        protect_private_path(&key_path)?;
                        key
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                        let bytes = fs::read(&key_path)?;
                        if bytes.len() != 32 {
                            return Err(StoreError::Crypto("invalid master key length".into()));
                        }
                        let mut existing = [0u8; 32];
                        existing.copy_from_slice(&bytes);
                        existing
                    }
                    Err(err) => return Err(StoreError::Io(err)),
                }
            }
            Err(e) => return Err(StoreError::Io(e)),
        };

        Ok(Self {
            vault_path,
            lock_path,
            master_key,
            mu: Mutex::new(()),
        })
    }

    pub fn set_secret(&self, name: &str, plaintext: &str) -> Result<(), StoreError> {
        let _inproc = self
            .mu
            .lock()
            .map_err(|_| StoreError::Other("vault mutex poisoned".into()))?;
        let _flock = acquire_blocking_flock(&self.lock_path)?;

        let mut file = self.load_file()?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.master_key));
        let mut nonce_bytes = [0u8; 12];
        getrandom::getrandom(&mut nonce_bytes)
            .map_err(|e| StoreError::Crypto(format!("nonce csprng failed: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
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
        let _inproc = self
            .mu
            .lock()
            .map_err(|_| StoreError::Other("vault mutex poisoned".into()))?;
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
        let _inproc = self
            .mu
            .lock()
            .map_err(|_| StoreError::Other("vault mutex poisoned".into()))?;
        let _flock = acquire_blocking_flock(&self.lock_path)?;

        let mut file = self.load_file()?;
        let removed = file.entries.remove(name).is_some();
        if removed {
            let serialized = serde_json::to_vec_pretty(&file)?;
            atomic_write_restricted(&self.vault_path, &serialized)?;
        }
        Ok(removed)
    }

    pub fn list_keys(&self) -> Result<Vec<String>, StoreError> {
        let _inproc = self
            .mu
            .lock()
            .map_err(|_| StoreError::Other("vault mutex poisoned".into()))?;
        let file = self.load_file()?;
        Ok(file.entries.keys().cloned().collect())
    }

    fn load_file(&self) -> Result<VaultFile, StoreError> {
        match fs::read(&self.vault_path) {
            Ok(raw) => Ok(serde_json::from_slice(&raw)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(VaultFile {
                version: 2,
                entries: BTreeMap::new(),
            }),
            Err(e) => Err(StoreError::Io(e)),
        }
    }
}

struct FileLockGuard {
    _file: File,
}

#[cfg(windows)]
impl Drop for FileLockGuard {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
        use windows_sys::Win32::System::IO::OVERLAPPED;

        unsafe {
            let mut overlapped: OVERLAPPED = std::mem::zeroed();
            let _ = UnlockFileEx(
                self._file.as_raw_handle() as HANDLE,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            );
        }
    }
}

fn acquire_blocking_flock(lock_path: &Path) -> Result<FileLockGuard, StoreError> {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        loop {
            let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
            if rc == 0 {
                break;
            }
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                return Err(StoreError::Io(err));
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK};
        use windows_sys::Win32::System::IO::OVERLAPPED;

        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            LockFileEx(
                f.as_raw_handle() as HANDLE,
                LOCKFILE_EXCLUSIVE_LOCK,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            )
        };
        if ok == 0 {
            return Err(StoreError::Io(std::io::Error::last_os_error()));
        }
    }
    Ok(FileLockGuard { _file: f })
}

/// Tighten filesystem permissions on `path` so only the current OS user can access it.
///
/// - On Unix: sets mode `0o600`.
/// - On Windows: replaces the DACL with a protected DACL granting `FILE_ALL_ACCESS`
///   exclusively to the current process token user SID, stripping inherited ACEs.
pub fn protect_private_path(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE};
        use windows_sys::Win32::Security::Authorization::{
            SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE,
            SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
        };
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
            PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
        };
        use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut token: HANDLE = 0;
        let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
        if opened == 0 || token == 0 {
            return Err(StoreError::Io(std::io::Error::last_os_error()));
        }

        let res: Result<(), StoreError> = (|| {
            let mut needed: u32 = 0;
            unsafe {
                let _ = GetTokenInformation(
                    token,
                    TokenUser,
                    std::ptr::null_mut(),
                    0,
                    &mut needed,
                );
            }
            if needed == 0 {
                return Err(StoreError::Io(std::io::Error::last_os_error()));
            }
            let word_size = std::mem::size_of::<usize>();
            let words = (needed as usize).div_ceil(word_size);
            let mut buf = vec![0usize; words];
            let buf_bytes = (buf.len() * word_size) as u32;
            let got = unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    buf.as_mut_ptr() as *mut c_void,
                    buf_bytes,
                    &mut needed,
                )
            };
            if got == 0 {
                return Err(StoreError::Io(std::io::Error::last_os_error()));
            }
            let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
            let user_sid = token_user.User.Sid;
            if user_sid.is_null() {
                return Err(StoreError::Other(
                    "failed to resolve current Windows user SID for private ACL".into(),
                ));
            }

            let ea = EXPLICIT_ACCESS_W {
                grfAccessPermissions: FILE_ALL_ACCESS,
                grfAccessMode: SET_ACCESS,
                grfInheritance: NO_INHERITANCE,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: std::ptr::null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: user_sid as *mut u16,
                },
            };

            let mut new_dacl: *mut ACL = std::ptr::null_mut();
            let acl_rc = unsafe { SetEntriesInAclW(1, &ea, std::ptr::null(), &mut new_dacl) };
            if acl_rc != ERROR_SUCCESS || new_dacl.is_null() {
                return Err(StoreError::Io(std::io::Error::from_raw_os_error(
                    acl_rc as i32,
                )));
            }

            let wide_path: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let sec_rc = unsafe {
                SetNamedSecurityInfoW(
                    wide_path.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    new_dacl,
                    std::ptr::null(),
                )
            };
            unsafe {
                let _ = LocalFree(new_dacl as _);
            }
            if sec_rc != ERROR_SUCCESS {
                return Err(StoreError::Io(std::io::Error::from_raw_os_error(
                    sec_rc as i32,
                )));
            }
            Ok(())
        })();

        unsafe {
            let _ = CloseHandle(token);
        }
        res
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

/// Write `bytes` to a unique temp file with owner-only `0600` (or Windows current-user DACL)
/// permissions BEFORE atomic rename, cleaning up the temp file on any error.
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
    let write_res = f.write_all(bytes).and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(StoreError::Io(e));
    }
    if let Err(e) = protect_private_path(&tmp_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(StoreError::Io(e));
    }
    #[cfg(windows)]
    {
        let _ = protect_private_path(path);
    }
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

        let vault2 = Vault::open(path.clone()).unwrap();
        assert_eq!(
            vault2.get_secret("openai_key").unwrap().as_deref(),
            Some("sk-secret-123")
        );

        let raw = fs::read_to_string(&path)
            .unwrap()
            .replace("openai_key", "other_key");
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
