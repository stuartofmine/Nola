// SPDX-License-Identifier: Apache-2.0
// The immutable non-secret binding is part of AgentCoreConfig and therefore
// enters the same canonical Run transaction/digest as the checkpoint.

// Identity only: it grants no access and never changes a running executor cwd.
fn host_execution_workspace_identity(config: &HostInvocationRequest) -> Result<nola_core::agent::AgentWorkspaceIdentity, String> {
    execution_workspace_identity(&config.tool_settings.project_root,
        config.tool_settings.task_worktree.as_ref(), config.tool_settings.task_sandbox_requirement,config.tool_settings.process_sandbox.as_ref())
}

use crate::host_execution_settings::execution_workspace_identity;

fn freeze_host_retry_admission(config: &mut HostInvocationRequest) {
    if config.runtime.admitted_max_retries.is_none() {
        config.runtime.admitted_max_retries = Some(crate::llm::configured_llm_max_retries() as u32);
    }
}

fn freeze_host_output_admission(config: &mut HostInvocationRequest) -> Result<(), String> {
    crate::delegated_invocation::prepare_managed_workspace_scope(config)?;
    if config.runtime.output_admission.is_some() {
        freeze_host_memory_admission(config)?;
        freeze_host_retry_admission(config);
        crate::host_output_admission::admit(&config.model_invocation_settings()?, false)?;
        return Ok(());
    }
    if config.session.save_session {
        if let Some(session_id) = config.session.session_id.as_deref() {
            let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
            if application.active_run(session_id)?.is_some() {
                let stored = application.load_checkpoint(session_id)?
                    .ok_or("active Run is missing its output admission checkpoint")?;
                let checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_str(&stored.checkpoint_json)
                    .map_err(|error| format!("active Run has no compatible output admission: {error}"))?;
                restore_host_memory_admission(config, &checkpoint.state.configuration.memory_policy)?;
                let binding = checkpoint.state.configuration.turn_identity.host_invocation
                    .ok_or("active Run predates frozen output admission; explicit cancellation is required")?;
                binding.validate().map_err(str::to_string)?;
                config.runtime.admitted_max_retries = Some(binding.generation.max_retries);
                crate::host_output_admission::validate_snapshot(&config.model_invocation_settings()?, &binding.output_admission)?;
                config.runtime.output_admission = Some(binding.output_admission);
                return Ok(());
            }
        }
    }
    freeze_host_memory_admission(config)?;
    config.runtime.output_admission = Some(crate::host_output_admission::admit(&config.model_invocation_settings()?, true)?);
    freeze_host_retry_admission(config);
    Ok(())
}

#[cfg(test)]
pub(crate) fn fixture_host_invocation_binding(config: &mut HostInvocationRequest) -> Result<nola_host_protocol::HostInvocationBinding, String> {
    freeze_host_memory_admission(config)?;
    freeze_host_retry_admission(config);
    host_invocation_binding(config)
}

fn prepare_host_invocation_verification(config: &HostInvocationRequest) -> Result<HostInvocationVerificationContext, String> {
    use nola_host_protocol::{HostGenerationBinding, HostInvocationBinding, HostModelRouteBinding, HOST_INVOCATION_BINDING_VERSION};
    use sha2::{Digest, Sha256};
    verify_loaded_automation_artifacts(config)?;
    let model_settings = config.model_invocation_settings()?;
    fn sha(value: &[u8]) -> String { format!("{:x}", Sha256::digest(value)) }
    fn route(provider: Option<&str>, endpoint: &str, model: &str, credential: &str, field: nola_application::ChatOutputTokenField) -> Result<HostModelRouteBinding, String> {
        crate::model_route::route_binding(provider, endpoint, model, credential, field)
    }
    let memory_policy = config.runtime.admitted_memory_policy.as_ref()
        .ok_or("Host memory policy has not been admitted")?;
    validate_memory_policy_ceiling(memory_policy, &resolve_host_memory_policy(None)?)?;
    let device_advertisements = current_device_invocation_advertisements()?;
    let mut execution_policy = serde_json::json!({
        "memory_policy": memory_policy,
        // One-time automation input is committed through Core admission:
        // spec in the Run configuration, loaded bytes in context components.
        // A cold executor must not need the original lease or temporary files
        // to reproduce its physical binding. Actual resource policy below is
        // still frozen and checked on every recovery.
        "device_advertisements": device_advertisements,
        "project_root": config.tool_settings.project_root,
        "allowed_dirs": config.tool_settings.allowed_dirs,
        "bash_prefixes": config.tool_settings.bash_prefixes,
        "network_policy": config.tool_settings.network_policy,
        "auto_approve": config.tool_settings.auto_approve,
        "permission_profile": config.tool_settings.permission_profile.map(
            crate::permissions::PermissionProfile::canonical_name
        ),
        "tools_enabled": config.tool_settings.tools_enabled,
        "force_tool_set": config.tool_settings.force_tool_set,
        "tool_allowlist": config.tool_settings.tool_allowlist,
        "skill_dirs": config.tool_settings.skill_dirs,
        "execution_backend": config.execution.execution_backend,
        "external_execution_model": config.execution.external_execution_model,
        "external_execution_model_provider": config.execution.external_execution_model_provider,
        "permission_scope_id": config.session.permission_scope_id,
        "permissions": config.runtime.permissions,
        "core_permission_rules": config.runtime.core_permission_rules,
        "mcp_capability_catalog": config.tool_settings.mcp_capability_catalog,
        "mcp_server_registry": config.tool_settings.mcp_server_registry,
        "host_attachments": config.input.host_attachments,
        "expected_artifact_refs": config.input.expected_artifact_refs,
    });
    if let Some(spec)=&config.tool_settings.process_sandbox {
        spec.validate_host_storage(&config.tool_settings.allowed_dirs)?;
        execution_policy["process_sandbox"]=serde_json::to_value(spec).map_err(|error|error.to_string())?;
    }
    if config.tool_settings.bounded_file_writes {execution_policy["bounded_file_writes"]=serde_json::json!(true);}
    if let Some(binding)=&config.tool_settings.task_worktree {
        execution_policy["task_worktree"]=serde_json::to_value(binding).map_err(|e|e.to_string())?;
    }
    if let Some(requirement)=&config.tool_settings.task_sandbox_requirement {
        execution_policy["task_sandbox_requirement"]=serde_json::to_value(requirement).map_err(|e|e.to_string())?;
    }
    let binding = HostInvocationBinding {
        mac_launch: config.runtime.mac_launch.clone(),
        version: HOST_INVOCATION_BINDING_VERSION,
        primary: crate::host_output_admission::primary_route_binding(&model_settings)?,
        fallbacks: model_settings.options().llm_fallbacks.iter().map(|entry| route(entry.provider.as_deref(), &entry.endpoint, entry.model.as_deref().unwrap_or(&config.model_settings.model), entry.api_key.as_deref().unwrap_or(&config.model_settings.api_key), config.runtime.chat_output_token_field)).collect::<Result<_, _>>()?,
        generation: HostGenerationBinding {
            max_retries: config.runtime.admitted_max_retries.ok_or("Host retry policy has not been admitted")?,
            // Capability support is not an explicit request preference.
            parallel_tool_calls: None,
            reasoning_selection: config.runtime.reasoning_selection.clone(),
            reasoning_tier: config.runtime.reasoning_tier,
            sampling_override: config.runtime.sampling_override,
            generation_options: config.runtime.generation_options.clone().try_into()?,
            temperature: serde_json::Number::from_f64(config.runtime.temperature.unwrap_or(0.7).clamp(0.0, 2.0)).ok_or("invalid Host temperature")?,
            top_p: serde_json::Number::from_f64(config.runtime.top_p.unwrap_or(0.9).clamp(0.0, 1.0)).ok_or("invalid Host top_p")?,
            max_output_tokens: crate::host_output_admission::admitted_output_tokens(&model_settings)?,
            thinking_enabled: crate::runtime_config::reasoning_enabled(config.runtime.reasoning_selection.as_ref(),
                config.runtime.model_capabilities.reasoning, config.runtime.provider.as_deref(), &config.model_settings.model),
            reasoning_summary_supported: config.runtime.model_capabilities.reasoning_summary_supported,
            prompt_cache: config.runtime.openai_prompt_cache.unwrap_or(true),
            prompt_cache_key_sha256: config.runtime.openai_prompt_cache_key.as_ref().map(|v| sha(v.as_bytes())),
            prompt_cache_key_prefix_sha256: config.runtime.openai_prompt_cache_key_prefix.as_ref().map(|v| sha(v.as_bytes())),
            prompt_cache_retention: config.runtime.openai_prompt_cache_retention.clone(),
        },
        output_admission: crate::host_output_admission::admit(&model_settings, false)?,
        execution_policy_sha256: sha(&serde_json::to_vec(&execution_policy).map_err(|error| error.to_string())?),
    };
    binding.validate().map_err(str::to_string)?;
    Ok(HostInvocationVerificationContext {
        binding,
        memory_policy: memory_policy.clone(),
        device_advertisements_sha256: sha(&serde_json::to_vec(&device_advertisements).map_err(|error| error.to_string())?),
        loaded_artifacts: config.session.automation_artifacts.iter().map(|artifact| {
            let id = automation_artifact_component_id(artifact);
            let component = config.input.host_context_components.iter().find(|component| component.id == id)
                .ok_or("automation artifact lost its loaded component")?;
            Ok(LoadedArtifactVerification {
                bytes: component.content.as_str().ok_or("automation artifact body changed type")?.as_bytes().to_vec(),
                expected_sha256: artifact.sha256.clone(),
            })
        }).collect::<Result<_, String>>()?,
    })
}

fn validate_host_invocation_binding(identity: &AgentTurnIdentity, current: &HostInvocationRequest) -> Result<(), String> {
    let saved = identity.host_invocation.as_ref().ok_or_else(||
        "Host invocation unavailable: active Run predates the frozen Host binding; explicit cancellation is required".to_string())?;
    saved.require_exact(&host_invocation_binding(current)?).map_err(str::to_string)
}

impl CoreAgentHost {
    fn validate_invocation_binding(&self, config: &HostInvocationRequest) -> Result<(), String> {
        validate_host_invocation_binding(&self.runtime.config.turn_identity, config)
    }
}


// Host-owned immutable verification evidence. This contains neither CLI
// configuration nor live authorization. Every use rechecks external authority.
#[derive(Clone)]
struct HostInvocationVerificationContext {
    binding: nola_host_protocol::HostInvocationBinding,
    memory_policy: nola_core::memory::MemoryPolicy,
    device_advertisements_sha256: String,
    loaded_artifacts: Vec<LoadedArtifactVerification>,
}

#[derive(Clone)]
struct LoadedArtifactVerification {
    bytes: Vec<u8>,
    expected_sha256: String,
}

impl LoadedArtifactVerification {
    fn verify(&self) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        if format!("{:x}", Sha256::digest(&self.bytes)) != self.expected_sha256 {
            return Err("automation loaded context differs from its artifact witness".into());
        }
        Ok(())
    }
}

impl HostInvocationVerificationContext {
    fn verify(&self) -> Result<nola_host_protocol::HostInvocationBinding, String> {
        use sha2::{Digest, Sha256};
        validate_memory_policy_ceiling(&self.memory_policy, &resolve_host_memory_policy(None)?)?;
        for artifact in &self.loaded_artifacts {
            artifact.verify()?;
        }
        let device_projection = current_device_invocation_advertisements()?;
        let actual = format!("{:x}", Sha256::digest(
            serde_json::to_vec(&device_projection).map_err(|error| error.to_string())?
        ));
        if actual != self.device_advertisements_sha256 {
            return Err("Host invocation unavailable: Device invocation surface changed".into());
        }
        self.binding.validate().map_err(str::to_string)?;
        Ok(self.binding.clone())
    }
}

fn host_invocation_binding(config: &HostInvocationRequest) -> Result<nola_host_protocol::HostInvocationBinding, String> {
    prepare_host_invocation_verification(config)?.verify()
}

fn current_device_invocation_advertisements() -> Result<Vec<Value>, String> {
    let device_tools = current_device_bridge_runtime_tools();
    if !device_tools.is_empty() && !crate::tools::device_bridge_resolver_available() {
        return Err("Host invocation unavailable: frozen Device adapter has no active resolver".into());
    }
    device_tools.into_iter().map(|tool| {
        let receipt = &tool.result_egress_receipt;
        let expected = crate::tools::current_device_bridge_execution_identity()
            .ok_or("Host invocation unavailable: Device resolver has no authenticated identity")?;
        let tool_id = nola_core::device_bridge::device_tool_name(&tool.schema)?;
        nola_core::tool_policy::validate_result_egress_evidence(
            receipt, &tool.result_egress_grant, &expected, tool_id,
            nola_core::tool_policy::ToolRoute::Device, now_epoch_ms()? as u64,
        ).map_err(str::to_string)?;
        // The proof itself is rechecked at dispatch and result acceptance.
        // Freeze its intended owner/surface, not a renewable manifest/time.
        Ok(serde_json::json!({
            "schema": tool.schema, "capability": tool.capability,
            "tool_id": receipt.tool_id, "route": receipt.route,
            "host_id": receipt.host_id, "client_id": receipt.client_id,
            "session_id": receipt.session_id, "requires_result_egress": true,
        }))
    }).collect::<Result<Vec<_>, String>>()

}

#[cfg(test)]
mod host_invocation_verification_tests {
    use super::*;

    #[test]
    fn execution_workspace_identity_tracks_root_and_policy_without_exposing_paths() {
        let root=tempfile::tempdir().unwrap();
        let first=execution_workspace_identity(root.path(),None,None,None).unwrap();
        assert_eq!(first,execution_workspace_identity(&root.path().join("."),None,None,None).unwrap());
        let other=tempfile::tempdir().unwrap();
        assert_ne!(first.root_digest,execution_workspace_identity(other.path(),None,None,None).unwrap().root_digest);
        assert_ne!(first.root_digest,execution_workspace_identity(root.path(),None,
            Some(nola_domain::task_workspace::TaskSandboxRequirement::CodexWorkspaceWrite),None).unwrap().root_digest);
        assert!(!serde_json::to_string(&first).unwrap().contains(root.path().to_str().unwrap()));
        assert!(first.summary.is_none());
    }


    #[test]
    fn loaded_artifact_verification_uses_owned_bytes_and_rejects_changed_content() {
        use sha2::{Digest, Sha256};
        let mut source = b"loaded context".to_vec();
        let mut witness = LoadedArtifactVerification {
            bytes: source.clone(),
            expected_sha256: format!("{:x}", Sha256::digest(&source)),
        };
        source.clear();
        assert!(witness.verify().is_ok());
        witness.bytes.push(b'!');
        assert!(witness.verify().is_err());
    }

    #[test]
    fn verification_snapshot_does_not_reload_mutated_request_and_rejects_device_drift() {
        let _environment = crate::store::test_env_lock();
        let mut request = HostInvocationRequest::for_test().unwrap();
        freeze_host_memory_admission(&mut request).unwrap();
        freeze_host_retry_admission(&mut request);
        let mut verification = prepare_host_invocation_verification(&request).unwrap();
        let original = verification.verify().unwrap();
        request.model_settings.model = "later-unadmitted-model".into();
        assert_eq!(verification.verify().unwrap(), original);
        verification.device_advertisements_sha256 = "not-the-current-surface".into();
        assert!(verification.verify().unwrap_err().contains("Device invocation surface changed"));
    }
}
