// SPDX-License-Identifier: Apache-2.0
//! Process-wide memory + disk TTL cache for model acquisition metadata and plans.
//! All disk errors are non-fatal so read-only or restricted environments keep working.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) const MODELFIT_TTL: Duration = Duration::from_secs(6 * 3600);
pub(super) const HUB_METADATA_TTL: Duration = Duration::from_secs(6 * 3600);
pub(super) const RECOMMEND_TTL: Duration = Duration::from_secs(30 * 60);
pub(super) const PREVIEW_TTL: Duration = Duration::from_secs(30 * 60);
pub(super) const HARDWARE_TTL: Duration = Duration::from_secs(5 * 60);

const MAX_MEMORY_ENTRIES: usize = 512;

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    expires_at: u64,
    value: T,
}

#[derive(Clone)]
struct MemoryEntry {
    expires_at: u64,
    payload: Vec<u8>,
}

pub(super) struct CacheStore {
    memory: Mutex<HashMap<String, MemoryEntry>>,
    dir: Option<PathBuf>,
}

impl CacheStore {
    pub(super) fn new(dir: Option<PathBuf>) -> Self {
        Self {
            memory: Mutex::new(HashMap::new()),
            dir,
        }
    }

    pub(super) fn get<T: DeserializeOwned + Serialize>(&self, namespace: &str, key: &str) -> Option<T> {
        let now = now_secs();
        let composite = composite_key(namespace, key);
        if let Ok(mut guard) = self.memory.lock() {
            if let Some(entry) = guard.get(&composite) {
                if entry.expires_at > now {
                    if let Ok(val) = serde_json::from_slice::<T>(&entry.payload) {
                        return Some(val);
                    }
                } else {
                    guard.remove(&composite);
                }
            }
        }
        let path = self.disk_path(namespace, key)?;
        let bytes = fs::read(&path).ok()?;
        let envelope: Envelope<T> = serde_json::from_slice(&bytes).ok()?;
        if envelope.expires_at <= now {
            let _ = fs::remove_file(&path);
            return None;
        }
        if let Ok(payload) = serde_json::to_vec(&envelope.value) {
            self.insert_memory(composite, envelope.expires_at, payload);
        }
        Some(envelope.value)
    }

    pub(super) fn set<T: Serialize>(&self, namespace: &str, key: &str, value: &T, ttl: Duration) {
        let expires_at = now_secs().saturating_add(ttl.as_secs().max(1));
        let Ok(payload) = serde_json::to_vec(value) else {
            return;
        };
        let composite = composite_key(namespace, key);
        self.insert_memory(composite, expires_at, payload);
        let Some(path) = self.disk_path(namespace, key) else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let envelope = Envelope { expires_at, value };
        let Ok(encoded) = serde_json::to_vec(&envelope) else {
            return;
        };
        let tmp = parent.join(format!(
            ".{}.{}.tmp",
            path.file_name().and_then(|s| s.to_str()).unwrap_or("cache"),
            std::process::id()
        ));
        if fs::write(&tmp, &encoded).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }

    fn insert_memory(&self, composite: String, expires_at: u64, payload: Vec<u8>) {
        if let Ok(mut guard) = self.memory.lock() {
            if guard.len() >= MAX_MEMORY_ENTRIES {
                let now = now_secs();
                guard.retain(|_, v| v.expires_at > now);
                if guard.len() >= MAX_MEMORY_ENTRIES {
                    guard.clear();
                }
            }
            guard.insert(
                composite,
                MemoryEntry {
                    expires_at,
                    payload,
                },
            );
        }
    }

    fn disk_path(&self, namespace: &str, key: &str) -> Option<PathBuf> {
        let base = self.dir.as_ref()?;
        let safe_ns: String = namespace
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if safe_ns.is_empty() {
            return None;
        }
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        Some(base.join(format!("{safe_ns}_{digest}.json")))
    }
}

fn composite_key(namespace: &str, key: &str) -> String {
    format!("{namespace}:{key}")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn default_cache_dir() -> Option<PathBuf> {
    if let Some(nola_home) = std::env::var_os("NOLA_HOME") {
        let p = PathBuf::from(nola_home);
        if !p.as_os_str().is_empty() {
            return Some(p.join("cache").join("model_acquisition"));
        }
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let p = PathBuf::from(home);
        if !p.as_os_str().is_empty() {
            return Some(p.join(".nola").join("cache").join("model_acquisition"));
        }
    }
    None
}

fn global() -> &'static CacheStore {
    static STORE: OnceLock<CacheStore> = OnceLock::new();
    STORE.get_or_init(|| CacheStore::new(default_cache_dir()))
}

pub(super) fn get<T: DeserializeOwned + Serialize>(namespace: &str, key: &str) -> Option<T> {
    global().get(namespace, key)
}

pub(super) fn set<T: Serialize>(namespace: &str, key: &str, value: &T, ttl: Duration) {
    global().set(namespace, key, value, ttl)
}

#[cfg(test)]
pub(super) fn store_in(dir: &Path) -> CacheStore {
    CacheStore::new(Some(dir.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn memory_and_disk_ttl_cache_hits_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        store.set("recommend", "modelscope:llama:16g", &json!({"ok": true}), Duration::from_secs(60));
        // Memory hit
        let hit: Value = store.get("recommend", "modelscope:llama:16g").unwrap();
        assert_eq!(hit["ok"], json!(true));

        // Fresh store pointing at the same dir should hit disk cache
        let cold_store = store_in(dir.path());
        let disk_hit: Value = cold_store.get("recommend", "modelscope:llama:16g").unwrap();
        assert_eq!(disk_hit["ok"], json!(true));

        // Expired entry on disk is rejected
        let expired_path = cold_store.disk_path("recommend", "expired-key").unwrap();
        let expired_envelope = json!({"expires_at": 1, "value": {"stale": true}});
        fs::write(&expired_path, serde_json::to_vec(&expired_envelope).unwrap()).unwrap();
        assert!(cold_store.get::<Value>("recommend", "expired-key").is_none());
    }

    #[test]
    fn unwritable_disk_cache_still_serves_memory_without_failing() {
        let dir = tempfile::tempdir().unwrap();
        let file_not_dir = dir.path().join("blocker");
        fs::write(&file_not_dir, b"not a dir").unwrap();
        let store = store_in(&file_not_dir.join("nested"));
        store.set("hub", "repo/a", &json!({"v": 1}), Duration::from_secs(60));
        let hit: Value = store.get("hub", "repo/a").unwrap();
        assert_eq!(hit["v"], json!(1));
    }
}
