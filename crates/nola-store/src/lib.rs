// SPDX-License-Identifier: Apache-2.0
//! Lean SQLite WAL + FTS5 persistence, AES-256-GCM secret vault, and
//! single-writer service lock/discovery for Nola v2.

pub mod db;
pub mod service_lock;
pub mod vault;

pub use db::{
    EventPage, Store, StoreError, DEFAULT_MAX_EVENTS_PER_SCOPE, NOLA_SQLITE_APPLICATION_ID,
    NOLA_SQLITE_USER_VERSION,
};
pub use service_lock::{
    database_identity_hash, default_socket_path, discovery_path, read_discovery, ServiceDiscovery,
    ServiceLock, PROTOCOL_VERSION,
};
pub use vault::Vault;
