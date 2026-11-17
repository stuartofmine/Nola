// SPDX-License-Identifier: Apache-2.0
//! Unified CLI, Agent Host, architecture boundary, and persistence contract suite.

use std::sync::{Mutex, MutexGuard, OnceLock};

#[path = "support/assistant_journal.rs"]
pub mod assistant_journal;

pub fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[path = "suites/dependency_boundary.rs"]
mod dependency_boundary;
#[path = "suites/executable_entry_identity.rs"]
mod executable_entry_identity;
#[cfg(unix)]
#[path = "suites/service_schema_preservation.rs"]
mod service_schema_preservation;
#[path = "suites/skill_runtime_architecture.rs"]
mod skill_runtime_architecture;
#[path = "suites/source_layout.rs"]
mod source_layout;
#[path = "suites/canonical_session_e2e.rs"]
mod canonical_session_e2e;
#[path = "suites/canonical_turn_recovery_e2e.rs"]
mod canonical_turn_recovery_e2e;
#[path = "suites/desktop_host_canonical_identity_e2e.rs"]
mod desktop_host_canonical_identity_e2e;
#[path = "suites/pending_effect_recovery.rs"]
mod pending_effect_recovery;
#[path = "suites/tool_output_roundtrip_e2e.rs"]
mod tool_output_roundtrip_e2e;
#[path = "suites/local_pipeline_baseline.rs"]
mod local_pipeline_baseline;
