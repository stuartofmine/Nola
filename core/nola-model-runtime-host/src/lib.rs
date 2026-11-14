// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements model manager logic in the Rust core module.
// - Interfaces:
//   - ModelFileInfo
//   - ModelState
//   - ModelManager
// - APIs:
//   - none
//
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

const CONNECT_TIMEOUT_SECS: u64 = 15;
const REQUEST_TIMEOUT_SECS: u64 = 1_800;

#[derive(Clone, Debug)]
pub struct ModelFileInfo {
    pub name: String,
    pub size: i64,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub enum ModelState {
    NotDownloaded,
    Downloading(f64),
    Downloaded,
    Failed(String),
}

#[derive(Clone)]
pub struct ModelManager {
    client: Arc<OnceLock<Client>>,
    models: Arc<Mutex<HashMap<String, ModelState>>>,
    cancels: Arc<Mutex<HashMap<String, bool>>>,
}

impl ModelManager {
    pub fn new() -> Self {
        Self {
            client: Arc::new(OnceLock::new()),
            models: Arc::new(Mutex::new(HashMap::new())),
            cancels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn http_client(&self) -> &Client {
        self.client.get_or_init(|| {
            Client::builder()
                .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .or_else(|_| {
                    Client::builder()
                        .no_proxy()
                        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                        .build()
                })
                .unwrap_or_else(|_| Client::new())
        })
    }

    pub fn download_file(&self, url: &str, dest: &PathBuf) -> Result<(), String> {
        self.download_file_verified(url, dest, None)
    }

    pub fn download_file_verified(
        &self,
        url: &str,
        dest: &PathBuf,
        expected_sha256: Option<&str>,
    ) -> Result<(), String> {
        self.stage_and_commit_download(url, dest, expected_sha256, |partial| {
            self.download_to_partial(url, partial)
        })
    }

    fn stage_and_commit_download<F>(
        &self,
        url: &str,
        dest: &PathBuf,
        expected_sha256: Option<&str>,
        write_partial: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&PathBuf) -> Result<(), String>,
    {
        self.cancels
            .lock()
            .map_err(|_| "model cancellation state is unavailable".to_string())?
            .remove(url);
        self.set_state(url, ModelState::Downloading(0.0));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                let message = error.to_string();
                self.set_state(url, ModelState::Failed(message.clone()));
                message
            })?;
        }
        let partial = PathBuf::from(format!("{}.partial", dest.display()));
        if let Err(error) = write_partial(&partial) {
            let _ = std::fs::remove_file(&partial);
            self.set_state(url, ModelState::Failed(error.clone()));
            return Err(error);
        }
        if let Some(expected_raw) = expected_sha256 {
            let expected = expected_raw.trim().to_ascii_lowercase();
            if expected.len() != 64 || !expected.chars().all(|ch| ch.is_ascii_hexdigit()) {
                let _ = std::fs::remove_file(&partial);
                let error = "invalid expected model sha256 digest".to_string();
                self.set_state(url, ModelState::Failed(error.clone()));
                return Err(error);
            }
            let actual = match Self::sha256_of_file(&partial) {
                Ok(digest) => digest,
                Err(error) => {
                    let _ = std::fs::remove_file(&partial);
                    self.set_state(url, ModelState::Failed(error.clone()));
                    return Err(error);
                }
            };
            if actual != expected {
                let _ = std::fs::remove_file(&partial);
                let error = format!(
                    "model sha256 mismatch: expected {expected}, got {actual}"
                );
                self.set_state(url, ModelState::Failed(error.clone()));
                return Err(error);
            }
        }
        std::fs::rename(&partial, dest).map_err(|error| {
            let _ = std::fs::remove_file(&partial);
            let message = format!("finalize model download failed: {error}");
            self.set_state(url, ModelState::Failed(message.clone()));
            message
        })?;
        self.set_state(url, ModelState::Downloaded);
        Ok(())
    }

    fn download_to_partial(&self, url: &str, partial: &PathBuf) -> Result<(), String> {
        let mut resp = self
            .http_client()
            .get(url)
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("model download returned HTTP {}", resp.status()));
        }
        let total = resp.content_length().map(|v| v as i64).unwrap_or(-1);
        self.copy_reader_to_partial(url, &mut resp, total, partial)
    }

    fn copy_reader_to_partial<R: Read>(
        &self,
        url: &str,
        reader: &mut R,
        total: i64,
        partial: &PathBuf,
    ) -> Result<(), String> {
        let mut f = File::create(partial).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 8192];
        let mut written: i64 = 0;
        loop {
            if let Ok(canc) = self.cancels.lock() {
                if canc.get(url) == Some(&true) {
                    return Err("cancelled".to_string());
                }
            }
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            written += n as i64;
            if total > 0 {
                let progress = written as f64 / total as f64;
                self.set_state(url, ModelState::Downloading(progress));
            }
        }
        f.flush().map_err(|error| error.to_string())?;
        f.sync_all().map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn download_file_background(&self, url: &str, dest: &PathBuf) -> Result<(), String> {
        let manager = self.clone();
        let url = url.to_string();
        let destination = dest.clone();
        thread::spawn(move || {
            let _ = manager.download_file(&url, &destination);
        });
        Ok(())
    }

    fn set_state(&self, url: &str, state: ModelState) {
        if let Ok(mut models) = self.models.lock() {
            models.insert(url.to_string(), state);
        }
    }

    pub fn get_progress(&self, url: &str) -> Option<f64> {
        if let Ok(m) = self.models.lock() {
            if let Some(st) = m.get(url) {
                match st {
                    ModelState::Downloading(p) => Some(*p),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn cancel_download(&self, url: &str) -> bool {
        if let Ok(mut c) = self.cancels.lock() {
            c.insert(url.to_string(), true);
            true
        } else {
            false
        }
    }

    fn models_root() -> PathBuf {
        let mut base = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        base.push(".cache");
        base.push("nola");
        base.push("models");
        base
    }

    pub fn checked_model_path_for(&self, model_id: &str) -> Result<PathBuf, String> {
        if model_id.is_empty() || model_id.trim().is_empty() || model_id.trim() != model_id {
            return Err("model_id must not be empty or whitespace-padded".to_string());
        }
        if model_id.contains('\0')
            || model_id.contains('/')
            || model_id.contains('\\')
            || model_id == "."
            || model_id == ".."
        {
            return Err("model_id contains unsafe path characters".to_string());
        }
        let mut components = Path::new(model_id).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => Ok(Self::models_root().join(model_id)),
            _ => Err("model_id must be a single normal path segment".to_string()),
        }
    }

    pub fn safe_model_path_for(&self, model_id: &str) -> PathBuf {
        if let Ok(path) = self.checked_model_path_for(model_id) {
            return path;
        }
        let digest = hex::encode(Sha256::digest(model_id.as_bytes()));
        Self::models_root().join(format!("_rejected_{}", &digest[..16]))
    }

    pub fn is_model_downloaded(&self, model_id: &str) -> bool {
        let Ok(p) = self.checked_model_path_for(model_id) else {
            return false;
        };
        if !p.exists() {
            return false;
        }
        // quick check - has files? at least one file exists
        if let Ok(entries) = std::fs::read_dir(&p) {
            return entries.count() > 0;
        }
        false
    }

    pub fn sha256_of_file(path: &PathBuf) -> Result<String, String> {
        let mut f = File::open(path).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex::encode(hasher.finalize()))
    }
}

impl Default for ModelManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;

    #[test]
    fn file_digest_is_stable_and_model_path_has_one_nola_root() {
        let directory = tempdir().unwrap();
        let file = directory.path().join("model.bin");
        std::fs::write(&file, b"hello").unwrap();
        assert_eq!(
            ModelManager::sha256_of_file(&file).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let path = ModelManager::new().safe_model_path_for("fixture");
        assert!(path.ends_with(".cache/nola/models/fixture"));
        assert!(!path.to_string_lossy().contains("nola/nola/models"));
    }

    #[test]
    fn model_path_rejects_traversal_and_never_escapes_models_root() {
        let manager = ModelManager::new();
        let root = ModelManager::models_root();
        for unsafe_id in [
            "",
            "   ",
            ".",
            "..",
            "../secret",
            "../../etc/passwd",
            "nested/model",
            "nested\\model",
            "bad\0id",
            "/absolute/path",
            " leading",
            "trailing ",
        ] {
            assert!(
                manager.checked_model_path_for(unsafe_id).is_err(),
                "expected {unsafe_id:?} to be rejected"
            );
            let sanitized = manager.safe_model_path_for(unsafe_id);
            assert_eq!(
                sanitized.parent(),
                Some(root.as_path()),
                "safe_model_path_for({unsafe_id:?}) escaped models root: {sanitized:?}"
            );
            assert!(!manager.is_model_downloaded(unsafe_id));
        }
    }

    #[test]
    fn verified_download_cleans_partial_and_records_failed_state_on_digest_mismatch() {
        let directory = tempdir().unwrap();
        let dest = directory.path().join("weights.gguf");
        std::fs::write(&dest, b"existing-good-weights").unwrap();
        let partial = PathBuf::from(format!("{}.partial", dest.display()));

        let manager = ModelManager::new();
        let url = "https://models.example.invalid/weights.gguf";
        let wrong_sha = "0".repeat(64);
        let err = manager
            .stage_and_commit_download(url, &dest, Some(&wrong_sha), |partial_path| {
                let mut reader = Cursor::new(b"tampered-bytes".as_slice());
                manager.copy_reader_to_partial(url, &mut reader, 14, partial_path)
            })
            .expect_err("digest mismatch must fail before rename");
        assert!(err.contains("model sha256 mismatch"), "{err}");
        assert!(!partial.exists(), ".partial file must be cleaned up");
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"existing-good-weights",
            "existing destination must remain untouched on mismatch"
        );
        let recorded = manager.models.lock().unwrap().get(url).cloned();
        assert!(
            matches!(recorded, Some(ModelState::Failed(ref reason)) if reason.contains("sha256 mismatch")),
            "expected ModelState::Failed, got {recorded:?}"
        );

        // Matching digest promotes `.partial` atomically into `dest`.
        let hello_sha = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        manager
            .stage_and_commit_download(url, &dest, Some(hello_sha), |partial_path| {
                let mut reader = Cursor::new(b"hello".as_slice());
                manager.copy_reader_to_partial(url, &mut reader, 5, partial_path)
            })
            .expect("matching sha256 must succeed");
        assert!(!partial.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        assert!(matches!(
            manager.models.lock().unwrap().get(url),
            Some(ModelState::Downloaded)
        ));
    }
}
