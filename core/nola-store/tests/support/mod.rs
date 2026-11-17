// SPDX-License-Identifier: Apache-2.0
//! Shared fixture re-exports for `nola-store` integration suites.
//! Note: individual files in `support/` remain intact at their paths for cross-crate `#[path]` callers.

#[allow(unused_imports)]
pub use nola_store::NolaStore;

#[allow(dead_code)]
pub mod conversation_deletion_fixture;
#[allow(dead_code)]
pub mod outbox;
#[allow(dead_code)]
pub mod remote_submission;
#[allow(dead_code)]
pub mod remote_submission_from;
#[allow(dead_code)]
pub mod terminal_sources;
