// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Tool Host facade composed from responsibility-focused implementation units.

mod computer;
mod code_evidence;
mod execution_policy;
mod permission_resolution;
mod git_read_plan;
mod workspace_review;
pub(crate) use workspace_review::read_host_workspace_review;
mod shell_read_plan;
mod process_execution;
mod language_service_wire;
mod language_service_protocol;
mod language_service_document;
mod language_service_tools;
#[cfg(unix)]
mod language_service_transport;
#[cfg(unix)]
mod language_service_session;
pub(crate) use process_execution::{enter_local_execution_identity, current_local_execution_identity};
pub(crate) use permission_resolution::{builtin_permission_contract, resolve_permission_resources,
    current_browser_permission_resource_expectation,
    current_mcp_permission_resource_expectation,
    enter_permission_resource_expectations, validate_permission_resource_execution, permission_resource_execution_failure};
pub use execution_policy::ToolExecutionPolicy;
mod output_store;
pub(crate) use output_store::ToolOutputStore;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod platform_catalog_contract_tests;
pub(crate) mod web_search_core;
mod web_search_engine;

include!("parts/contracts_execution.rs");
include!("parts/resource_evidence.rs");
include!("parts/file_read_evidence.rs");
include!("parts/file_mutations.rs");
include!("parts/approval.rs");
include!("parts/catalog.rs");
include!("parts/context.rs");
include!("parts/binary_documents.rs");
include!("parts/environment.rs");
include!("parts/repository_analysis.rs");
include!("parts/editing.rs");
include!("parts/checkpoints_review.rs");
include!("parts/search_planning.rs");
include!("parts/background.rs");
include!("parts/git_io.rs");
include!("parts/process_preservation.rs");
include!("parts/git_commit.rs");
include!("parts/git_worktree.rs");
include!("parts/git_source_snapshot.rs");
include!("parts/browser.rs");
include!("parts/permissions_output.rs");
