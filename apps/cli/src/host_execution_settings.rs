// SPDX-License-Identifier: Apache-2.0
//! Physical Host effect inputs, independent of CLI startup and presentation.

#[derive(Clone)]
pub(crate) struct HostExecutionSettings {
    pub memory_workspace_id: Option<String>,
    pub bounded_file_writes: bool,
    pub task_worktree: Option<nola_domain::task_workspace::TaskWorktreeBinding>,
    pub task_sandbox_requirement: Option<nola_domain::task_workspace::TaskSandboxRequirement>,
    pub external: crate::external_execution_settings::ExternalExecutionSettings,
    pub reasoning_tier: nola_domain::reasoning_tiers::ReasoningTier,
    pub native_model_route: bool,
    pub admitted_memory_policy: Option<nola_core::memory::MemoryPolicy>,
}

impl HostExecutionSettings {
    pub(crate) fn validate_workspace_authority(&self) -> Result<(),String> {
        if self.bounded_file_writes && (self.task_worktree.is_none() || self.external.execution_backend!="nola") {
            return Err("Bounded file execution requires its frozen native worktree".into());
        }
        if self.task_sandbox_requirement.is_some()
            && (self.external.execution_backend!="codex"
                || crate::external_executor::external_permission_policy(&self.external).1!="workspace-write") {
            return Err("Required Codex workspace-write policy is unavailable; unrestricted fallback is forbidden".into());
        }
        if let Some(binding)=&self.task_worktree {
            if self.external.project_root!=std::path::Path::new(&binding.selection.root) {
                return Err("Execution directory differs from the frozen task worktree".into());
            }
            crate::tools::verify_task_worktree(binding)?;
        }
        Ok(())
    }
    /// Recheck live authority at each side-effect boundary. Missing admission
    /// fails closed; a cancellation does not require this grant.
    pub(crate) fn validate_memory_authority(&self) -> Result<(), String> {
        let frozen = self.admitted_memory_policy.as_ref()
            .ok_or("Host memory policy has not been admitted")?;
        crate::host_memory_admission::validate_memory_policy_ceiling(
            frozen, &crate::host_memory_admission::resolve_host_memory_policy(None)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_fail_closed_without_admitted_memory_without_cli_configuration() {
        let settings = HostExecutionSettings {
            memory_workspace_id: None,
            bounded_file_writes: false, task_worktree: None, task_sandbox_requirement: None,
            external: crate::external_execution_settings::ExternalExecutionSettings {
                project_root: std::path::PathBuf::from("/fixture"),
                execution_backend: "nola".into(),
                external_execution_session_id: None,
                external_execution_model: None,
                external_execution_model_provider: None,
                host_context_components: Vec::new(),
                host_attachments: Vec::new(),
                budget: Default::default(),
                permission_profile: crate::permissions::PermissionProfile::Cautious,
                host_input_origin: nola_core::agent::AgentInputOrigin::SystemRecovery,
            },
            reasoning_tier: Default::default(),
            native_model_route: false,
            admitted_memory_policy: None,
        };
        assert_eq!(settings.validate_memory_authority().unwrap_err(), "Host memory policy has not been admitted");
    }
}

pub(crate) fn execution_workspace_identity(root: &std::path::Path,
    worktree: Option<&nola_domain::task_workspace::TaskWorktreeBinding>,
    sandbox: Option<nola_domain::task_workspace::TaskSandboxRequirement>,
    process_sandbox: Option<&crate::process_sandbox::FrozenProcessSandbox>,
) -> Result<nola_core::agent::AgentWorkspaceIdentity, String> {
    use sha2::{Digest, Sha256};
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let material = serde_json::to_vec(&(root, worktree, sandbox,process_sandbox)).map_err(|error| error.to_string())?;
    let digest = format!("{:x}", Sha256::digest(material));
    Ok(nola_core::agent::AgentWorkspaceIdentity { workspace_id: digest.clone(), root_digest: digest, summary: None })
}
