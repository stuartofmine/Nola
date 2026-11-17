// SPDX-License-Identifier: Apache-2.0
//! Consolidated integration test binary for `nola-service`.
#![cfg(unix)]

pub mod support;

#[path = "suites/goal_idle_cold_restart_e2e.rs"]
pub mod goal_idle_cold_restart_e2e;

#[path = "suites/runtime_task_worker_recovery_e2e.rs"]
pub mod runtime_task_worker_recovery_e2e;

#[path = "suites/cold_restart_persistence_e2e.rs"]
pub mod cold_restart_persistence_e2e;

#[path = "suites/archive_restore_process_e2e.rs"]
pub mod archive_restore_process_e2e;

#[path = "suites/client_process_e2e.rs"]
pub mod client_process_e2e;

#[path = "suites/database_service_ownership_e2e.rs"]
pub mod database_service_ownership_e2e;

#[path = "suites/projection_query_process_e2e.rs"]
pub mod projection_query_process_e2e;

#[path = "suites/replica_sync_process_e2e.rs"]
pub mod replica_sync_process_e2e;

#[path = "suites/run_recovery_process_e2e.rs"]
pub mod run_recovery_process_e2e;

#[path = "suites/service_process_e2e.rs"]
pub mod service_process_e2e;

#[path = "suites/task_guidance_receipt_restart_e2e.rs"]
pub mod task_guidance_receipt_restart_e2e;

#[path = "suites/transcript_export_process_e2e.rs"]
pub mod transcript_export_process_e2e;

#[path = "suites/transcript_recall_process_e2e.rs"]
pub mod transcript_recall_process_e2e;

#[path = "suites/transport_idempotency_and_cas_e2e.rs"]
pub mod transport_idempotency_and_cas_e2e;

#[path = "suites/uploaded_transcript_publication_process_e2e.rs"]
pub mod uploaded_transcript_publication_process_e2e;
