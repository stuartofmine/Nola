// SPDX-License-Identifier: Apache-2.0
//! Request-scoped cancellation for read-only catalog discovery, not inference.
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Running,
    Cancelled,
    Completed,
}
struct Entry {
    connection: String,
    revision: i64,
    created: Instant,
    phase: Arc<Mutex<Phase>>,
    catalog: bool,
}
type Registry = BTreeMap<(PathBuf, String), Entry>;
static REQUESTS: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    REQUESTS.get_or_init(Mutex::default)
}
fn key(store: &crate::store::NolaStore, request: &str) -> Result<(PathBuf, String), String> {
    uuid::Uuid::parse_str(request).map_err(|_| "model_discovery_request_id_invalid")?;
    Ok((
        std::fs::canonicalize(store.path()).unwrap_or_else(|_| store.path().to_path_buf()),
        request.into(),
    ))
}
fn prune(entries: &mut Registry) {
    entries.retain(|_, entry| entry.created.elapsed() < Duration::from_secs(60)
        || entry.phase.lock().map(|phase| *phase == Phase::Running).unwrap_or(true));
}
pub(crate) struct Operation {
    phase: Arc<Mutex<Phase>>,
}
impl Operation {
    pub(crate) fn begin(
        store: &crate::store::NolaStore,
        request: &str,
        connection: &str,
        revision: i64,
    ) -> Result<Self, String> {
        Self::begin_owned(store, request, connection, revision, false)
    }
    /// Only one catalog refresh owns a connection at a time. Probes keep their
    /// separate request identity and must never share a paid inference request.
    pub(crate) fn begin_catalog(
        store: &crate::store::NolaStore,
        request: &str,
        connection: &str,
        revision: i64,
    ) -> Result<Self, String> {
        Self::begin_owned(store, request, connection, revision, true)
    }
    fn begin_owned(
        store: &crate::store::NolaStore,
        request: &str,
        connection: &str,
        revision: i64,
        catalog: bool,
    ) -> Result<Self, String> {
        let key = key(store, request)?;
        let mut entries = registry()
            .lock()
            .map_err(|_| "model_discovery_unavailable")?;
        prune(&mut entries);
        if let Some(entry) = entries.get(&key) {
            if entry.connection != connection || entry.revision != revision {
                return Err("model_discovery_owner_mismatch".into());
            }
            return Err(if *entry
                .phase
                .lock()
                .map_err(|_| "model_discovery_unavailable")?
                == Phase::Cancelled
            {
                "model_discovery_cancelled"
            } else {
                "model_discovery_request_reused"
            }
            .into());
        }
        if catalog && entries.iter().any(|((database, _), entry)| {
            database == &key.0 && entry.catalog && entry.connection == connection
                && entry.phase.lock().map(|phase| *phase == Phase::Running).unwrap_or(true)
        }) {
            return Err("model_discovery_busy".into());
        }
        if entries.len() >= 64 {
            return Err("model_discovery_busy".into());
        }
        let phase = Arc::new(Mutex::new(Phase::Running));
        entries.insert(
            key,
            Entry {
                connection: connection.into(),
                revision,
                created: Instant::now(),
                phase: phase.clone(),
                catalog,
            },
        );
        Ok(Self { phase })
    }
    pub(crate) fn commit<T>(
        &self,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut phase = self
            .phase
            .lock()
            .map_err(|_| "model_discovery_unavailable")?;
        if *phase != Phase::Running {
            return Err("model_discovery_cancelled".into());
        }
        // Cancellation cannot report success while this exact request commits.
        let result = action();
        *phase = Phase::Completed;
        result
    }
    pub(crate) fn fetch(
        &self,
        url: &str,
        protocol: &str,
        secret: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        self.send(url, protocol, secret, None)
    }
    pub(crate) fn probe(
        &self,
        url: &str,
        protocol: &str,
        secret: Option<&str>,
        body: &serde_json::Value,
    ) -> Result<Vec<u8>, String> {
        self.send(url, protocol, secret, Some(body))
    }
    fn send(
        &self,
        url: &str,
        protocol: &str,
        secret: Option<&str>,
        body: Option<&serde_json::Value>,
    ) -> Result<Vec<u8>, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "model_discovery_transport_unavailable")?;
        runtime.block_on(async {
            let cancelled = async {
                loop {
                    if self.phase.lock().map(|phase| *phase != Phase::Running).unwrap_or(true) { break; }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            };
            let fetch = async {
                let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(10)).build().map_err(|_| "model_discovery_transport_unavailable")?;
                let mut request = if let Some(body) = body { client.post(url).json(body) } else { client.get(url) };
                if protocol == "anthropic" {
                    request = request.header("anthropic-version", "2023-06-01");
                    if let Some(secret) = secret { request = request.header("x-api-key", secret); }
                } else if let Some(secret) = secret { request = request.bearer_auth(secret); }
                let mut response = request.send().await.map_err(|_| "model_discovery_transport_failed")?;
                if !response.status().is_success() {
                    return Err(match response.status().as_u16() {
                        401 => "model_account_rejected", 403 => "model_account_denied", 429 => "model_discovery_rate_limited",
                        404 | 405 => "model_catalog_unavailable_use_manual_model", 500..=599 => "model_provider_temporarily_unavailable",
                        _ => "model_discovery_http_failed",
                    }.into());
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| "model_discovery_read_failed")? {
                    if bytes.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 { return Err("model_catalog_too_large".into()); }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(bytes)
            };
            tokio::select! { biased; _ = cancelled => Err("model_discovery_cancelled".into()), result = fetch => result }
        })
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        if let Ok(mut phase) = self.phase.lock() {
            if *phase == Phase::Running {
                *phase = Phase::Completed;
            }
        }
    }
}
pub(crate) fn cancel(
    store: &crate::store::NolaStore,
    request: &str,
    connection: &str,
    revision: i64,
) -> Result<bool, String> {
    let key = key(store, request)?;
    let mut entries = registry()
        .lock()
        .map_err(|_| "model_discovery_unavailable")?;
    prune(&mut entries);
    if let Some(entry) = entries.get(&key) {
        if entry.connection != connection || entry.revision != revision {
            return Err("model_discovery_owner_mismatch".into());
        }
        let mut phase = entry
            .phase
            .lock()
            .map_err(|_| "model_discovery_unavailable")?;
        if *phase == Phase::Completed {
            return Ok(false);
        }
        *phase = Phase::Cancelled;
    } else {
        if entries.len() >= 64 {
            return Err("model_discovery_busy".into());
        }
        // Preserve cancellation that overtakes its HTTP start request.
        entries.insert(
            key,
            Entry {
                connection: connection.into(),
                revision,
                created: Instant::now(),
                phase: Arc::new(Mutex::new(Phase::Cancelled)),
                catalog: false,
            },
        );
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_admission_is_connection_scoped_and_release_is_request_scoped() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::NolaStore::open(&root.path().join("one.sqlite")).unwrap();
        let other = crate::store::NolaStore::open(&root.path().join("two.sqlite")).unwrap();
        let first_id = uuid::Uuid::new_v4().to_string();
        let first = Operation::begin_catalog(&store, &first_id, "one", 1).unwrap();
        let second_id = uuid::Uuid::new_v4().to_string();
        assert!(matches!(Operation::begin_catalog(&store, &second_id, "one", 2), Err(error) if error == "model_discovery_busy"));
        let independent = Operation::begin_catalog(&store, &uuid::Uuid::new_v4().to_string(), "two", 1).unwrap();
        let separate_database = Operation::begin_catalog(&other, &uuid::Uuid::new_v4().to_string(), "one", 1).unwrap();
        let probe = Operation::begin(&store, &uuid::Uuid::new_v4().to_string(), "one", 1).unwrap();
        assert!(cancel(&store, &first_id, "one", 1).unwrap());
        assert!(first.commit::<()>(|| panic!("cancelled owner must not commit")).is_err());
        let second = Operation::begin_catalog(&store, &second_id, "one", 2).unwrap();
        // Repeated cancellation of the old owner must not touch the new one.
        cancel(&store, &first_id, "one", 1).unwrap();
        assert_eq!(second.commit(|| Ok(7)).unwrap(), 7);
        drop((independent, separate_database, probe));
    }

    #[test]
    fn failed_catalog_owner_releases_admission_and_early_cancel_checks_identity() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::NolaStore::open(&root.path().join("fixture.sqlite")).unwrap();
        let request = uuid::Uuid::new_v4().to_string();
        let failed = Operation::begin_catalog(&store, &request, "one", 1).unwrap();
        drop(failed);
        let next = Operation::begin_catalog(&store, &uuid::Uuid::new_v4().to_string(), "one", 1).unwrap();
        drop(next);
        let early = uuid::Uuid::new_v4().to_string();
        cancel(&store, &early, "one", 1).unwrap();
        assert!(matches!(Operation::begin_catalog(&store, &early, "two", 1), Err(error) if error == "model_discovery_owner_mismatch"));
        assert!(matches!(Operation::begin_catalog(&store, &early, "one", 1), Err(error) if error == "model_discovery_cancelled"));
    }

    #[test]
    fn cancellation_is_identity_bound_and_cannot_undo_a_committed_catalog() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::NolaStore::open(&root.path().join("fixture.sqlite")).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let operation = Operation::begin(&store, &id, "connection", 7).unwrap();
        assert!(cancel(&store, &id, "other", 7).is_err());
        assert!(cancel(&store, &id, "connection", 8).is_err());
        assert_eq!(operation.commit(|| Ok(12)).unwrap(), 12);
        assert!(!cancel(&store, &id, "connection", 7).unwrap());
        let early = uuid::Uuid::new_v4().to_string();
        assert!(cancel(&store, &early, "connection", 7).unwrap());
        assert!(Operation::begin(&store, &early, "connection", 7).is_err());
        let cancelled = uuid::Uuid::new_v4().to_string();
        let operation = Operation::begin(&store, &cancelled, "connection", 7).unwrap();
        assert!(cancel(&store, &cancelled, "connection", 7).unwrap());
        assert_eq!(
            operation
                .commit::<()>(|| panic!("cancelled request must never commit"))
                .unwrap_err(),
            "model_discovery_cancelled"
        );
    }
}
