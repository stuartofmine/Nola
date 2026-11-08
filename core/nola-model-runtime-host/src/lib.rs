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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

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
    client: Client,
    models: Arc<Mutex<HashMap<String, ModelState>>>,
    cancels: Arc<Mutex<HashMap<String, bool>>>,
}

impl ModelManager {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            models: Arc::new(Mutex::new(HashMap::new())),
            cancels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn download_file(&self, url: &str, dest: &PathBuf) -> Result<(), String> {
        self.cancels
            .lock()
            .map_err(|_| "model cancellation state is unavailable".to_string())?
            .remove(url);
        self.set_state(url, ModelState::Downloading(0.0));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let partial = PathBuf::from(format!("{}.partial", dest.display()));
        let result = self.download_to_partial(url, &partial);
        if let Err(error) = result {
            let _ = std::fs::remove_file(&partial);
            self.set_state(url, ModelState::Failed(error.clone()));
            return Err(error);
        }
        std::fs::rename(&partial, dest).map_err(|error| {
            let message = format!("finalize model download failed: {error}");
            self.set_state(url, ModelState::Failed(message.clone()));
            message
        })?;
        self.set_state(url, ModelState::Downloaded);
        Ok(())
    }

    fn download_to_partial(&self, url: &str, partial: &PathBuf) -> Result<(), String> {
        let mut resp = self.client.get(url).send().map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("model download returned HTTP {}", resp.status()));
        }
        let mut f = File::create(partial).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 8192];
        let total = resp.content_length().map(|v| v as i64).unwrap_or(-1);
        let mut written: i64 = 0;
        loop {
            if let Ok(canc) = self.cancels.lock() {
                if canc.get(url) == Some(&true) {
                    return Err("cancelled".to_string());
                }
            }
            let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
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

    pub fn safe_model_path_for(&self, model_id: &str) -> PathBuf {
        // Use HOME/.cache/nola as fallback if no cache_dir
        let mut base = std::env::var("HOME")
            .map(|h| PathBuf::from(h))
            .unwrap_or(std::env::temp_dir());
        base.push(".cache");
        base.push("nola");
        let mut path = base;
        path.push("models");
        path.push(model_id);
        path
    }

    pub fn is_model_downloaded(&self, model_id: &str) -> bool {
        let p = self.safe_model_path_for(model_id);
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
}
