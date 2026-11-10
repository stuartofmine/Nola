// SPDX-License-Identifier: Apache-2.0
//! Execution-only inputs. No CLI parsing, terminal access, or model credentials.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostAttachment {
    pub path: PathBuf,
    pub name: String,
    pub mime_type: String,
    pub byte_count: u64,
}

/// Owned per-invocation snapshot; executors borrow it and never reload user
/// configuration. Composition resolves permission defaults before this boundary.
#[derive(Clone)]
pub(crate) struct ExternalExecutionSettings {
    pub project_root: PathBuf,
    pub execution_backend: String,
    pub external_execution_session_id: Option<String>,
    pub external_execution_model: Option<String>,
    pub external_execution_model_provider: Option<String>,
    pub host_context_components: Vec<nola_core::context_engine::ContextComponent>,
    pub host_attachments: Vec<HostAttachment>,
    pub budget: crate::runtime_config::RuntimeExternalExecution,
    pub permission_profile: crate::permissions::PermissionProfile,
    pub host_input_origin: nola_core::agent::AgentInputOrigin,
}
