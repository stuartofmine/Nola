// SPDX-License-Identifier: Apache-2.0
//! Composed Host execution input. No CLI parsing, UI, or configuration reload.
//! This input is not authorization: Application admission and live authority
//! checks still precede effects. Product adapters construct it only after overrides.
//! Serialization exists only for the private child-worker launch channel. This
//! is not a public settings, logging, or durable authority DTO. In-process
//! authority is never deserialized, and there is no conversion back to Config.
use std::path::PathBuf;
use std::collections::BTreeMap;
use serde::{Serialize, Deserialize};
use serde_json::Value;
use nola_core::agent::{AgentInputOrigin, capability::McpCapabilityCatalog};
use nola_core::context_engine::ContextComponent;
use crate::runtime_config::{RuntimeLlmProvider, RuntimeModelCapabilities, RuntimeAgent, PermissionRule, RuntimeCompaction, RuntimeExternalExecution};
use nola_core::agent::permissions::PermissionRule as CorePermissionRule;
use crate::external_execution_settings::HostAttachment;

crate::model_request_overrides::impl_model_request_overrides!(HostRuntimeOptions);


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopResumeIdentity {
    pub session_id: String,
    pub expected_run_id: String,
    pub assistant_message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopCancelIdentity {
    pub session_id: String,
    pub expected_run_id: String,
    pub assistant_message_id: String,
    pub cancellation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationArtifact {
    pub kind: AutomationArtifactKind,
    pub id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub source: Option<AutomationArtifactSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationArtifactSource {
    pub authorization: String,
    pub observed_at: String,
    pub maximum_age_minutes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationArtifactKind { PersonalData, WebSource, Skill }

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostInvocationRequest {
    pub model_settings: HostModelRequest,
    pub tool_settings: HostToolRequest,
    pub session: HostSessionRequest,
    pub input: HostInputRequest,
    pub execution: HostExecutionRequest,
    pub runtime: HostRuntimeOptions,
}

impl std::fmt::Debug for HostInvocationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostInvocationRequest { composed_execution_input: [redacted] }")
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostModelRequest {
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub swe_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostToolRequest {
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub process_sandbox: Option<crate::process_sandbox::FrozenProcessSandbox>,
    #[serde(default)]
    pub bounded_file_writes: bool,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub task_worktree: Option<nola_domain::task_workspace::TaskWorktreeBinding>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub task_sandbox_requirement: Option<nola_domain::task_workspace::TaskSandboxRequirement>,
    pub project_root: PathBuf,
    pub allowed_dirs: Vec<PathBuf>,
    pub skill_dirs: Option<Vec<PathBuf>>,
    pub bash_prefixes: Vec<String>,
    pub auto_approve: bool,
    pub permission_profile: Option<crate::permissions::PermissionProfile>,
    pub tools_enabled: bool,
    pub force_tool_set: String,
    pub tool_allowlist: Option<Vec<String>>,
    pub tool_allowlist_is_authorization_only: bool,
    pub remote_tool_plan: Option<nola_domain::host_tool_plan::HostToolPlanSnapshot>,
    pub network_policy: String,
    pub mcp_capability_catalog: Option<McpCapabilityCatalog>,
    pub mcp_server_registry: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSessionRequest {
    /// References a durable prepared target; it is never model/policy authority.
    #[serde(skip)]
    pub edited_resend_operation_id: Option<String>,
    /// Explicit command submission, never inferred from prompt text.
    #[serde(skip)]
    pub user_delegation: Option<nola_core::agent::AgentSubmission>,
    /// Admission-only user selection; Core resolves exact experts in the Main Run.
    #[serde(skip)]
    pub main_expert_selection: Option<nola_domain::expert_task_target::MainExpertSelectionContextV1>,
    pub supports_reviewed_text_parts: bool,
    pub session_id: Option<String>,
    pub permission_scope_id: Option<String>,
    pub save_session: bool,
    pub host_submission_id: Option<String>,
    pub host_user_message_id: Option<String>,
    pub host_assistant_message_id: Option<String>,
    /// The caller's observed configuration/transcript boundary. Recovery and
    /// internal continuation never manufacture this from current Store state.
    #[serde(default)]
    pub expected_configuration_revision: Option<i64>,
    #[serde(default)]
    pub expected_parent_message_id: Option<String>,
    pub resume_snapshot: Option<nola_application::ResumeSnapshot>,
    #[serde(skip)]
    pub desktop_resume: Option<DesktopResumeIdentity>,
    #[serde(skip)]
    pub desktop_cancel: Option<DesktopCancelIdentity>,
    #[serde(skip)]
    pub desktop_goal_control: Option<nola_application::goal_control_intent::PrepareGoalControlIntent>,
    #[serde(skip)]
    pub desktop_goal_wake: Option<String>,
    /// Exact coordinator-owned Goal for this new round, never inferred from a later active Goal.
    #[serde(skip)]
    pub goal_execution_id: Option<String>,
    /// Admission-only queue identity; never persisted in frozen configuration.
    #[serde(skip)]
    pub queued_user_command_id: Option<String>,
    #[serde(skip)]
    pub host_task_execution: Option<crate::runtime_task_port::RuntimeTaskExecutionAuthority>,
    #[serde(skip)]
    pub automation_execution_lease: Option<nola_domain::automation_execution::AutomationExecutionLease>,
    #[serde(skip)]
    pub automation_execution_spec: Option<nola_domain::automation_execution::AutomationExecutionSpec>,
    #[serde(skip)]
    pub automation_artifacts: Vec<AutomationArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostInputRequest {
    pub message: String,
    pub host_input_origin: AgentInputOrigin,
    pub host_turn_context: Option<String>,
    pub host_context_components: Vec<ContextComponent>,
    pub host_attachments: Vec<HostAttachment>,
    pub context_files: Vec<PathBuf>,
    pub expected_artifact_refs: Vec<String>,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostExecutionRequest {
    pub execution_backend: String,
    pub external_execution_session_id: Option<String>,
    pub external_execution_model: Option<String>,
    pub external_execution_model_provider: Option<String>,
    pub max_iterations: usize,
}

impl std::fmt::Debug for HostModelRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostModelRequest { route_and_credentials: [redacted] }")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRuntimeOptions {
    /// Trusted prepared-target model facts, never supplied by renderer wire.
    #[serde(skip)]
    pub edited_resend_model_capability: Option<nola_core::agent::ModelCapabilityProfile>,
    #[serde(skip)]
    pub edited_resend_prompt_definitions: Vec<nola_core::context_engine::ContextComponent>,
    /// Actual identity used only while the trusted native/server factory reads
    /// its prepared target. It is not portable delegation or worker state.
    /// Never constructed from Config or renderer settings; Core rechecks the
    /// retained source and current local Store scope before returning bodies.
    #[serde(skip)]
    pub expert_template_authority: Option<nola_application::expert_template_capture::HostExpertTemplateAuthority>,
    pub selected_profile: Option<nola_host_protocol::HostModelProfileReference>,
    #[serde(skip)]
    pub siwc: Option<crate::model_siwc::SiwcExecutionContext>,
    pub code_mode_limits: nola_domain::code_mode::CodeCellLimits,
    pub language_services: Vec<nola_domain::language_service::LanguageServiceConfiguration>,
    #[serde(skip)]
    pub native_model_route: Option<nola_host_protocol::HostNativeModelRouteBinding>,
    #[serde(skip)]
    pub mac_launch: Option<nola_host_protocol::HostMacLaunchDescriptor>,
    pub memory_enabled: Option<bool>,
    #[serde(skip)]
    pub admitted_memory_policy: Option<nola_core::memory::MemoryPolicy>,
    pub provider: Option<String>,
    pub llm_fallbacks: Vec<RuntimeLlmProvider>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u64>,
    #[serde(skip)]
    pub output_admission: Option<nola_host_protocol::HostOutputAdmission>,
    #[serde(skip)]
    pub admitted_max_retries: Option<u32>,
    pub chat_output_token_field: nola_application::ChatOutputTokenField,
    pub reasoning_selection: Option<nola_core::agent::reasoning::ReasoningSelection>,
    pub reasoning_tier: nola_domain::reasoning_tiers::ReasoningTier,
    pub sampling_override: bool,
    pub generation_options: nola_core::agent::reasoning::GenerationOptions,
    pub language: Option<String>,
    pub openai_prompt_cache: Option<bool>,
    pub openai_prompt_cache_key: Option<String>,
    pub openai_prompt_cache_key_prefix: Option<String>,
    pub openai_prompt_cache_retention: Option<String>,
    /// Explicit endpoint support for Responses history cache breakpoints; off by default.
    pub openai_prompt_cache_history_breakpoint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_prompt_cache_history_breakpoint_endpoints: Option<Vec<String>>,
    pub model_capabilities: RuntimeModelCapabilities,
    pub instructions: Vec<String>,
    pub agents: BTreeMap<String, RuntimeAgent>,
    pub permissions: Vec<PermissionRule>,
    pub core_permission_rules: Vec<CorePermissionRule>,
    pub compaction: RuntimeCompaction,
    pub external_execution: RuntimeExternalExecution,
}

impl HostInvocationRequest {
    pub(crate) fn host_execution_settings(&self) -> crate::host_execution_settings::HostExecutionSettings {
        crate::host_execution_settings::HostExecutionSettings {
            memory_workspace_id: crate::host_execution_settings::execution_workspace_identity(&self.tool_settings.project_root, self.tool_settings.task_worktree.as_ref(), self.tool_settings.task_sandbox_requirement, self.tool_settings.process_sandbox.as_ref()).ok().map(|identity| identity.workspace_id),
            bounded_file_writes: self.tool_settings.bounded_file_writes,
            task_worktree: self.tool_settings.task_worktree.clone(),
            task_sandbox_requirement: self.tool_settings.task_sandbox_requirement,
            external: self.external_execution_settings(),
            reasoning_tier: self.runtime.reasoning_tier,
            native_model_route: self.runtime.native_model_route.is_some(),
            admitted_memory_policy: self.runtime.admitted_memory_policy.clone(),
        }
    }

    pub(crate) fn external_execution_settings(&self) -> crate::external_execution_settings::ExternalExecutionSettings {
        crate::external_execution_settings::ExternalExecutionSettings {
            project_root: self.tool_settings.project_root.clone(),
            execution_backend: self.execution.execution_backend.clone(),
            external_execution_session_id: self.execution.external_execution_session_id.clone(),
            external_execution_model: self.execution.external_execution_model.clone(),
            external_execution_model_provider: self.execution.external_execution_model_provider.clone(),
            host_context_components: self.input.host_context_components.clone(),
            host_attachments: self.input.host_attachments.clone(),
            budget: self.runtime.external_execution.clone(),
            permission_profile: self.tool_settings.permission_profile.unwrap_or_default(),
            host_input_origin: self.input.host_input_origin,
        }
    }

    /// Called after product/session/request overrides have been composed.
    /// The immutable value cannot reload files, read stdin, or mutate Config.
    pub(crate) fn model_invocation_settings(&self) -> Result<crate::model_invocation_settings::ModelInvocationSettings, String> {
        use crate::model_invocation_settings::{ModelInvocationSettings, ModelInvocationOptions};
        let options = ModelInvocationOptions {
                provider: self.runtime.provider.clone(), siwc: self.runtime.siwc.clone(), native_model_route: self.runtime.native_model_route.clone(),
                llm_fallbacks: self.runtime.llm_fallbacks.clone(), max_output_tokens: self.runtime.max_output_tokens,
                chat_output_token_field: self.runtime.chat_output_token_field,
                reasoning_selection: self.runtime.reasoning_selection.clone(), output_admission: self.runtime.output_admission.clone(),
                compaction: self.runtime.compaction.clone(),
            };
        if self.execution.execution_backend != "nola" {
            return ModelInvocationSettings::external(self.execution.execution_backend.clone(),
                self.execution.external_execution_model.clone(), self.execution.external_execution_model_provider.clone(),
                options, self.model_settings.swe_mode).map_err(|error| error.to_string());
        }
        ModelInvocationSettings::new(self.model_settings.endpoint.clone(), self.model_settings.api_key.clone(), self.model_settings.model.clone(),
            options, self.model_settings.swe_mode).map_err(|error| error.to_string())
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Result<Self, String> {
        crate::config::Config::for_test().and_then(|config| config.host_invocation_request())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expert_factory_identity_is_not_a_portable_runtime_or_restorable_input_field() {
        let mut runtime = HostRuntimeOptions::default();
        runtime.expert_template_authority = Some(serde_json::from_value(serde_json::json!({
            "actor":{"kind":"local_user","id":"actual-native-user"},"request_host_id":null
        })).unwrap());
        let portable = serde_json::to_value(&runtime).unwrap();
        assert!(portable.get("expert_template_authority").is_none());
        let _: nola_host_protocol::delegation::DelegatedRuntimeConfiguration =
            serde_json::from_value(portable.clone()).unwrap();
        let mut forged = portable;
        forged["expert_template_authority"] = serde_json::to_value(runtime.expert_template_authority).unwrap();
        assert!(serde_json::from_value::<HostRuntimeOptions>(forged).is_err());
    }

    #[test]
    fn external_host_request_admits_executor_without_http_endpoint() {
        let _environment = crate::store::test_env_lock();
        let mut request = HostInvocationRequest::for_test().unwrap();
        request.execution.execution_backend = "codex".into();
        request.execution.external_execution_model = Some("gpt-6.1-sol".into());
        request.model_settings.endpoint.clear();
        request.model_settings.api_key.clear();
        request.runtime.output_admission = None;
        let settings = request.model_invocation_settings().unwrap();
        let snapshot = crate::host_output_admission::admit(&settings, true).unwrap();
        request.runtime.output_admission = Some(snapshot);
        assert!(crate::host_output_admission::admit(&request.model_invocation_settings().unwrap(), false).is_ok());
        request.execution.external_execution_model = Some("different-model".into());
        assert!(crate::host_output_admission::admit(&request.model_invocation_settings().unwrap(), false).is_err());
    }

    #[test]
    fn composition_owns_values_and_excludes_product_controls() {
        let _environment = crate::store::test_env_lock();
        let mut config = crate::config::Config::for_test().unwrap();
        config.continue_last = false;
        config.api_key = "private-model-credential".into();
        config.expected_configuration_revision = Some(7);
        config.expected_parent_message_id = Some("observed-parent".into());
        config.message = "original input".into();
        config.json_output = true;
        let request = config.host_invocation_request().unwrap();
        config.message = "later composer draft".into();
        config.api_key.clear();
        config.expected_configuration_revision = Some(8);
        config.expected_parent_message_id = Some("later-parent".into());
        assert_eq!(request.input.message, "original input");
        assert_eq!(request.model_settings.api_key, "private-model-credential");
        assert_eq!(request.session.expected_configuration_revision, Some(7));
        assert_eq!(request.session.expected_parent_message_id.as_deref(), Some("observed-parent"));
        assert!(!request.session.supports_reviewed_text_parts);
        let mut wire = serde_json::to_value(&request).unwrap();
        for key in ["command", "json_output", "continue_last", "resume_session", "web_host", "debug_mode"] {
            assert!(wire.get(key).is_none());
        }
        assert!(!format!("{request:?}").contains("private-model-credential"));
        // The old product-shaped envelope cannot accidentally enter a worker.
        wire.as_object_mut().unwrap().insert("command".into(), Value::String("Run".into()));
        assert!(serde_json::from_value::<HostInvocationRequest>(wire).is_err());
    }

    #[test]
    fn worker_serialization_cannot_recreate_process_authority() {
        let _environment = crate::store::test_env_lock();
        let mut request = HostInvocationRequest::for_test().unwrap();
        request.session.desktop_resume = Some(DesktopResumeIdentity {
            session_id: "conversation".into(), expected_run_id: "run".into(),
            assistant_message_id: "assistant".into(),
        });
        request.runtime.admitted_memory_policy = Some(nola_core::memory::MemoryPolicy::default());
        request.runtime.admitted_max_retries = Some(3);
        let recovered: HostInvocationRequest = serde_json::from_value(
            serde_json::to_value(&request).unwrap(),
        ).unwrap();
        assert!(recovered.session.desktop_resume.is_none());
        assert!(recovered.session.host_task_execution.is_none());
        assert!(recovered.session.automation_execution_lease.is_none());
        assert!(recovered.runtime.admitted_memory_policy.is_none());
        assert!(recovered.runtime.admitted_max_retries.is_none());
        assert!(recovered.runtime.output_admission.is_none());
    }
}

