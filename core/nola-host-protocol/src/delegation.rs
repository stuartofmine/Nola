// SPDX-License-Identifier: Apache-2.0
//! Portable non-secret delegation snapshots. Platform vaults resolve references
//! only at execution; the snapshot itself never authorizes a tool invocation.
use nola_domain::{
    delegation::AgentProfile,
    execution_policy::{
        ConversationAgentPolicy, ConversationCompactionPolicy, ConversationExternalExecutionBudget,
        ConversationPermissionDeclaration,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedModel {
    pub endpoint: String,
    pub model: String,
    pub swe_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedFallback {
    pub provider: Option<String>,
    pub endpoint: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedRuntimeConfiguration {
    pub selected_profile: Option<crate::HostModelProfileReference>,
    pub code_mode_limits: nola_domain::code_mode::CodeCellLimits,
    pub language_services: Vec<nola_domain::language_service::LanguageServiceConfiguration>,
    pub memory_enabled: Option<bool>,
    pub provider: Option<String>,
    pub llm_fallbacks: Vec<DelegatedFallback>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u64>,
    pub chat_output_token_field: nola_domain::model_capabilities::ChatOutputTokenField,
    pub reasoning_selection: Option<nola_domain::generation::ReasoningSelection>,
    pub reasoning_tier: nola_domain::reasoning_tiers::ReasoningTier,
    pub sampling_override: bool,
    pub generation_options: nola_domain::generation::GenerationOptions,
    pub language: Option<String>,
    pub openai_prompt_cache: Option<bool>,
    pub openai_prompt_cache_key: Option<String>,
    pub openai_prompt_cache_key_prefix: Option<String>,
    pub openai_prompt_cache_retention: Option<String>,
    pub openai_prompt_cache_history_breakpoint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_prompt_cache_history_breakpoint_endpoints: Option<Vec<String>>,
    pub model_capabilities: nola_domain::model_runtime::RuntimeModelCapabilities,
    pub instructions: Vec<String>,
    pub agents: BTreeMap<String, ConversationAgentPolicy>,
    pub permissions: Vec<ConversationPermissionDeclaration>,
    pub core_permission_rules: Vec<nola_domain::PermissionRule>,
    pub compaction: ConversationCompactionPolicy,
    pub external_execution: ConversationExternalExecutionBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialReference {
    pub route_identity: String,
    pub source: CredentialSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialSource {
    Siwc {
        account: crate::HostSiwcAccountBinding,
    },
    Inherited,
    Endpoint {
        endpoint_id: String,
    },
    Account {
        account_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedInvocationSnapshot {
    /// Opaque non-secret trusted Host process policy; never a tool access grant.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub process_sandbox: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if="is_false")]
    pub bounded_file_writes: bool,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub project_repository_identity: Option<nola_domain::task_workspace::TaskRepositoryIdentity>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub worktree: Option<nola_domain::task_workspace::TaskWorktreeBinding>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub sandbox_requirement: Option<nola_domain::task_workspace::TaskSandboxRequirement>,
    pub owner_session_id: String,
    pub execution_backend: String,
    pub external_execution_model: Option<String>,
    pub external_execution_model_provider: Option<String>,
    pub profile: AgentProfile,
    pub model: DelegatedModel,
    pub runtime: DelegatedRuntimeConfiguration,
    pub credential: Option<CredentialReference>,
    pub fallback_credentials: Vec<CredentialReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationParentSnapshot {
    /// Opaque non-secret trusted Host process policy; never a tool access grant.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub process_sandbox: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if="std::collections::BTreeSet::is_empty")]
    pub bounded_file_tools: std::collections::BTreeSet<String>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub project_repository_identity: Option<nola_domain::task_workspace::TaskRepositoryIdentity>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub task_worktree: Option<nola_domain::task_workspace::TaskWorktreeBinding>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub task_sandbox_requirement: Option<nola_domain::task_workspace::TaskSandboxRequirement>,
    #[serde(default, skip_serializing_if="Vec::is_empty")]
    pub authorized_workspace_roots: Vec<String>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub requested_native_sandbox: Option<String>,
    pub model: DelegatedModel,
    pub runtime: DelegatedRuntimeConfiguration,
    pub credential: Option<CredentialReference>,
    pub fallback_credentials: Vec<CredentialReference>,
    pub profiles: BTreeMap<String, AgentProfile>,
    pub project_root: String,
    /// Concrete Host catalog facts, not inferred from tool names or user text.
    pub read_only_tools: std::collections::BTreeSet<String>,
    /// Host-owned subscription references captured without secrets; Core checks profile/account revisions.
    pub profile_credentials: BTreeMap<String, CredentialReference>,
    pub execution_backend: String,
    pub external_execution_model: Option<String>,
    pub external_execution_model_provider: Option<String>,
    pub max_iterations: usize,
}

pub fn route_identity(endpoint: &str, provider: Option<&str>) -> Result<String, String> {
    let mut url = url::Url::parse(endpoint).map_err(|_| "Invalid delegated endpoint")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Delegated endpoints must be HTTP routes without URL credentials, queries or fragments"
                .into(),
        );
    }
    let protocol =
        nola_domain::generation::ModelProtocol::parse(provider.unwrap_or("chat_completions"))?;
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(format!("{}\n{}", protocol.as_str(), url))
}

impl DelegationParentSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.process_sandbox.as_ref().is_some_and(|value|!value.is_object()) {return Err("Invalid Host process sandbox snapshot".into());}
        self.runtime.generation_options.validate()?;
        if let Some(repository)=&self.project_repository_identity {repository.validate()?;}
        if let Some(binding)=&self.task_worktree {
            binding.validate_execution_root(&self.authorized_workspace_roots)?;
            if self.project_root!=binding.selection.root { return Err("Inherited task worktree differs from parent execution root".into()); }
        }
        if self.project_root.trim().is_empty()
            || self
                .runtime
                .temperature
                .is_some_and(|n| !(0.0..=2.0).contains(&n))
            || self
                .runtime
                .top_p
                .is_some_and(|n| !(0.0..=1.0).contains(&n))
        {
            return Err("Invalid frozen delegated execution settings".into());
        }
        if self.execution_backend.trim().is_empty() || self.profiles.is_empty() {
            return Err("Delegation requires a frozen backend and profiles".into());
        }
        if self.runtime.llm_fallbacks.len() != self.fallback_credentials.len() {
            return Err("Delegated fallback reference mismatch".into());
        }
        if self.execution_backend == "nola" {
            let credential = self
                .credential
                .as_ref()
                .ok_or("Delegated model credential reference is absent")?;
            if credential.route_identity
                != route_identity(&self.model.endpoint, self.runtime.provider.as_deref())?
            {
                return Err("Delegated route identity mismatch".into());
            }
        } else if self.credential.is_some()
            || !self.model.endpoint.is_empty()
            || !self.runtime.llm_fallbacks.is_empty()
        {
            return Err("External delegation cannot inherit HTTP credentials".into());
        }
        for (route, credential) in self
            .runtime
            .llm_fallbacks
            .iter()
            .zip(&self.fallback_credentials)
        {
            if credential.route_identity
                != route_identity(&route.endpoint, route.provider.as_deref())?
            {
                return Err("Delegated fallback identity mismatch".into());
            }
        }
        for (name, profile) in &self.profiles {
            if name != &profile.name || name.trim().is_empty() {
                return Err("Delegation profile identity mismatch".into());
            }
            if let Some(executor) = &profile.executor {
                executor.validate()?;
                if profile.model.is_some() || !profile.tools.is_empty() {
                    return Err("External profiles cannot claim a Nola model or tool allowlist".into());
                }
            }
            if let Some(model) = &profile.model {
                model.validate()?;
            }
        }
        Ok(())
    }
}

/// Eq identity of a validated typed snapshot, using the same canonical JSON
/// representation pattern as ValidatedGenerationOptions. No untyped fields are
/// accepted; decoding always validates the complete current snapshot contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ValidatedDelegationParent(serde_json::Value);
impl ValidatedDelegationParent {
    pub fn new(snapshot: DelegationParentSnapshot) -> Result<Self, String> {
        snapshot.validate()?;
        Ok(Self(
            serde_json::to_value(snapshot).map_err(|e| e.to_string())?,
        ))
    }
    pub fn snapshot(&self) -> Result<DelegationParentSnapshot, String> {
        serde_json::from_value(self.0.clone()).map_err(|e| e.to_string())
    }
}
impl<'de> Deserialize<'de> for ValidatedDelegationParent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(DelegationParentSnapshot::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

impl CredentialReference {
    pub fn siwc(route_identity: String, account: crate::HostSiwcAccountBinding) -> Self {
        Self {
            route_identity,
            source: CredentialSource::Siwc { account },
        }
    }
    pub fn inherited(route_identity: String) -> Self {
        Self {
            route_identity,
            source: CredentialSource::Inherited,
        }
    }
    pub fn configured(route_identity: String, source: &serde_json::Value) -> Result<Self, String> {
        let source = if source["connectionID"].as_str().is_some() {
            CredentialSource::Account {
                account_id: source["accountID"].as_str().map(str::to_owned),
            }
        } else {
            CredentialSource::Endpoint {
                endpoint_id: source["endpointID"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or("Model endpoint credential reference missing")?
                    .into(),
            }
        };
        Ok(Self {
            route_identity,
            source,
        })
    }
}

fn is_false(value:&bool)->bool {!*value}
