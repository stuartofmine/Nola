// SPDX-License-Identifier: Apache-2.0
//! Unified integration test suite for `nola-application`.

#[allow(unused_imports)]
use nola_store::NolaStore;

#[path = "support/mod.rs"]
pub mod support;

#[allow(dead_code)]
#[path = "support/committed_usage.rs"]
pub mod committed_usage;

#[allow(dead_code)]
#[path = "support/committed_usage_stream.rs"]
pub mod committed_usage_stream;

#[allow(dead_code)]
#[path = "../../nola-store/tests/support/remote_submission.rs"]
pub mod remote_submission;

#[allow(dead_code)]
#[path = "../../nola-store/tests/support/remote_submission_from.rs"]
pub mod remote_submission_from;

#[allow(dead_code)]
#[path = "../../nola-store/tests/support/terminal_sources.rs"]
pub mod terminal_sources;

#[allow(dead_code)]
pub fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

mod suites;
