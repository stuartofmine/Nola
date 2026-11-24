// SPDX-License-Identifier: Apache-2.0
//! Single Source of Truth domain types for Nola v2.
//!
//! Architectural invariants enforced at compile time:
//! 1. `ToolModelPayload` makes silent text-vs-data drops unrepresentable.
//! 2. `VersionTransition` makes missing `before_version`/`after_version` on file
//!    mutations unrepresentable.
//! 3. `StructuredModelRequest` separates immutable system prefix, prefix-stable
//!    tools, history (strictly User/Assistant/Tool), and ephemeral tail reminders,
//!    making mid-conversation `role: "system"` KV-cache wipes unrepresentable.
//! 4. One unified UTF-8/CJK `TokenEstimator` and `ContextBudget` across the workspace.

pub mod budget;
pub mod model;
pub mod modules;
pub mod policy;
pub mod tool;

pub use budget::*;
pub use model::*;
pub use modules::*;
pub use policy::*;
pub use tool::*;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable SHA-256 hex helper shared across crates.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// Core session metadata persisted in `nola-store`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub workspace_root: String,
    pub model_connection: Option<String>,
    pub model_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Canonical append-only event record stored per session/scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub scope: String,
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at_ms: i64,
}

/// Current epoch milliseconds.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Generate a new UUIDv4 string.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
