// SPDX-License-Identifier: Apache-2.0
//! Durable child invocation contract. No credential values are serialized.
use crate::{
    agent_registry::AgentProfile,
    host_invocation_request::{HostInvocationRequest, HostModelRequest},
    store::NolaStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use nola_host_protocol::delegation::{CredentialReference, CredentialSource};

#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct DelegatedInvocation(
    pub nola_host_protocol::delegation::DelegatedInvocationSnapshot,
);

pub(crate) fn validate_task_workspace_execution(config:&HostInvocationRequest, metadata:&str)->Result<(),String> {
    let value:Value=serde_json::from_str(metadata).map_err(|e|e.to_string())?;
    let snapshot:nola_host_protocol::delegation::DelegatedInvocationSnapshot=
        serde_json::from_value(value["delegated_invocation"].clone()).map_err(|_|"Original task execution snapshot is unavailable")?;
    if config.tool_settings.bounded_file_writes!=snapshot.bounded_file_writes
        || config.tool_settings.task_worktree!=snapshot.worktree
        || config.tool_settings.task_sandbox_requirement!=snapshot.sandbox_requirement
        || config.tool_settings.process_sandbox.as_ref().map(serde_json::to_value).transpose().map_err(|error|error.to_string())?!=snapshot.process_sandbox {
        return Err("Worker workspace differs from the Core-owned task snapshot".into());
    }
    if snapshot.worktree.is_some() && config.tool_settings.allowed_dirs!=vec![config.tool_settings.project_root.clone()] {
        return Err("Bound child workspace scope must not retain the parent's directories".into());
    }
    config.host_execution_settings().validate_workspace_authority()
}

pub(crate) trait CredentialReferenceExt {
    fn siwc_context(
        &self,
        store: &NolaStore,
    ) -> Result<Option<crate::model_siwc::SiwcExecutionContext>, String>;
    fn resolve(&self, parent: &HostInvocationRequest, store: &NolaStore) -> Result<String, String>;
}

pub(crate) fn route_identity(endpoint: &str, provider: Option<&str>) -> Result<String, String> {
    let protocol = crate::llm::resolved_model_protocol(provider, endpoint);
    nola_host_protocol::delegation::route_identity(endpoint, Some(protocol.as_str()))
}

impl DelegatedInvocation {
    #[cfg(test)]
    pub(crate) fn capture(
        parent: &HostInvocationRequest,
        owner: &str,
        profile: &AgentProfile,
        store: &NolaStore,
    ) -> Result<Self, String> {
        let mut frozen = parent_snapshot(parent, store)?;
        frozen
            .profiles
            .insert(profile.name.clone(), profile.clone());
        let mut core_store =
            nola_store::NolaStore::open(store.path()).map_err(|e| e.to_string())?;
        Ok(Self(
            nola_application::delegation_preparation::capture_delegated_invocation(
                &mut core_store,
                &frozen,
                owner,
                &profile.name,
            )?,
        ))
    }

    pub(crate) fn restore(
        &self,
        parent: &HostInvocationRequest,
        owner: &str,
        store: &NolaStore,
    ) -> Result<(AgentProfile, HostInvocationRequest), String> {
        if self.0.owner_session_id != owner {
            return Err("Delegated invocation owner mismatch".into());
        }
        if self.0.fallback_credentials.len() != self.0.runtime.llm_fallbacks.len() {
            return Err("Delegated fallback reference mismatch".into());
        }
        if let Some(credential) = &self.0.credential {
            if route_identity(&self.0.model.endpoint, self.0.runtime.provider.as_deref())?
                != credential.route_identity
            {
                return Err("Delegated route identity mismatch".into());
            }
        }
        let mut child = parent.clone();
        child.tool_settings.process_sandbox=self.0.process_sandbox.clone().map(serde_json::from_value).transpose()
            .map_err(|_|"Original process sandbox policy cannot be restored")?;
        child.tool_settings.bounded_file_writes=self.0.bounded_file_writes;
        child.tool_settings.task_worktree=self.0.worktree.clone();
        child.tool_settings.task_sandbox_requirement=self.0.sandbox_requirement;
        if let Some(binding)=&self.0.worktree {
            let roots=parent.tool_settings.allowed_dirs.iter()
                .map(|root|root.canonicalize().map(|path|path.to_string_lossy().into_owned()).map_err(|e|e.to_string()))
                .collect::<Result<Vec<_>,_>>()?;
            if parent.tool_settings.task_worktree.as_ref()==Some(binding) {binding.validate_execution_root(&roots)?;}
            else {binding.validate_authorized_roots(&roots)?;}
            child.tool_settings.project_root=std::path::PathBuf::from(&binding.selection.root);
            child.tool_settings.allowed_dirs=vec![child.tool_settings.project_root.clone()];
        }
        child.session.user_delegation = None;
        child.execution.execution_backend = self.0.execution_backend.clone();
        child.execution.external_execution_model = self.0.external_execution_model.clone();
        child.execution.external_execution_model_provider = self.0.external_execution_model_provider.clone();
        child.execution.external_execution_session_id = None; // child never resumes the parent's harness thread
        child.host_execution_settings().validate_workspace_authority()?;

        child.model_settings = HostModelRequest {
            endpoint: self.0.model.endpoint.clone(),
            model: self.0.model.model.clone(),
            swe_mode: self.0.model.swe_mode,
            api_key: String::new(),
        };
        child.runtime = serde_json::from_value(
            serde_json::to_value(&self.0.runtime).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("Invalid shared delegated runtime: {e}"))?;
        child.runtime.siwc = self
            .0
            .credential
            .as_ref()
            .map(|value| value.siwc_context(store))
            .transpose()?
            .flatten();
        // Newly applied restrictions still narrow a frozen child. New grants
        // are not imported into a task admitted under an earlier policy.
        // Do not append a frozen deny again after its later allow exception.
        // Append-only policy updates contribute only their new restrictions;
        // replaced/reordered policies keep the conservative restriction merge.
        let current_permissions = parent.runtime.permissions
            .strip_prefix(child.runtime.permissions.as_slice())
            .unwrap_or(parent.runtime.permissions.as_slice());
        child.runtime.permissions.extend(current_permissions.iter()
            .filter(|rule| rule.action != crate::runtime_config::PermissionAction::Allow).cloned());
        let current_core_rules = parent.runtime.core_permission_rules
            .strip_prefix(child.runtime.core_permission_rules.as_slice())
            .unwrap_or(parent.runtime.core_permission_rules.as_slice());
        child.runtime.core_permission_rules.extend(current_core_rules.iter()
            .filter(|rule| rule.decision != nola_domain::PermissionPolicyDecision::Allow).cloned());
        child.model_settings.api_key = self
            .0
            .credential
            .as_ref()
            .map(|credential| credential.resolve(parent, store))
            .transpose()?
            .unwrap_or_default();
        for (route, credential) in child
            .runtime
            .llm_fallbacks
            .iter_mut()
            .zip(&self.0.fallback_credentials)
        {
            if route_identity(&route.endpoint, route.provider.as_deref())?
                != credential.route_identity
            {
                return Err("Delegated fallback identity mismatch".into());
            }
            route.api_key = Some(credential.resolve(parent, store)?);
        }
        Ok((self.0.profile.clone(), child))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_workspace_scope_is_private_independent_and_idempotent() {
        let _environment = crate::store::test_env_lock();
        let project = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(crate::platform_paths::git_program())
            .args(["init", "--quiet"]).current_dir(project.path()).status().unwrap();
        assert!(status.success());
        assert!(std::process::Command::new(crate::platform_paths::git_program())
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@localhost", "commit", "--allow-empty", "-qm", "baseline"])
            .current_dir(project.path()).status().unwrap().success());
        let root = project.path().canonicalize().unwrap();
        let mut parent = HostInvocationRequest::for_test().unwrap();
        parent.tool_settings.project_root = root.clone();
        parent.tool_settings.allowed_dirs = vec![root.clone()];
        parent.tool_settings.tools_enabled = true;
        prepare_managed_workspace_scope(&mut parent).unwrap();
        assert_eq!(parent.tool_settings.allowed_dirs.len(), 2);
        let scratch = parent.tool_settings.allowed_dirs[1].clone();
        assert!(!scratch.starts_with(&root));
        assert_ne!(scratch, std::path::Path::new("/"));
        assert!(scratch.is_dir());
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&scratch).unwrap().mode() & 0o077, 0);
        }
        prepare_managed_workspace_scope(&mut parent).unwrap();
        assert_eq!(parent.tool_settings.allowed_dirs.len(), 2);
        std::fs::remove_dir(&scratch).unwrap(); // This test's empty Host container only.
    }

    #[test]
    fn child_roundtrip_freezes_model_parameters_and_profile_without_serializing_keys() {
        let _environment = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("state.sqlite")).unwrap();
        let mut parent = HostInvocationRequest::for_test().unwrap();
        parent.model_settings.endpoint = "https://fixture.invalid/v1".into();
        parent.model_settings.api_key = "private-child-credential".into();
        parent.model_settings.model = "original".into();
        parent.runtime.temperature = Some(0.25);
        parent.runtime.permissions = serde_json::from_value(json!([
            {"permission":"bash","pattern":"*","action":"Deny"},
            {"permission":"bash","pattern":"python3 -m unittest discover -s tests-local -v","action":"Allow"}
        ])).unwrap();
        let registry = crate::agent_registry::AgentRegistry::from_profiles(&Default::default());
        let profile = registry.get("explore").unwrap();
        let snapshot = DelegatedInvocation::capture(&parent, "owner", profile, &store).unwrap();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("private-child-credential"));
        let snapshot: DelegatedInvocation = serde_json::from_str(&encoded).unwrap();
        parent.model_settings.model = "new-default".into();
        parent.runtime.temperature = Some(1.5);
        parent.runtime.agents.clear();
        let (restored_profile, child) = snapshot.restore(&parent, "owner", &store).unwrap();
        assert_eq!(child.model_settings.model, "original");
        assert_eq!(child.runtime.temperature, Some(0.25));
        assert_eq!(child.runtime.permissions, parent.runtime.permissions,
            "unchanged parent policy must not append its old deny after a precise frozen allow");
        let mut restricted = parent.clone();
        restricted.runtime.permissions.push(serde_json::from_value(json!({
            "permission":"bash","pattern":"python3 -m unittest discover -s tests-local -v","action":"Deny"
        })).unwrap());
        let (_, narrowed) = snapshot.restore(&restricted, "owner", &store).unwrap();
        assert_eq!(narrowed.runtime.permissions.last(), restricted.runtime.permissions.last());
        let mut granted = parent.clone();
        granted.runtime.permissions.push(serde_json::from_value(json!({"permission":"bash","pattern":"new command","action":"Allow"})).unwrap());
        let (_, frozen) = snapshot.restore(&granted, "owner", &store).unwrap();
        assert!(!frozen.runtime.permissions.iter().any(|rule| rule.pattern == "new command"));
        assert_eq!(frozen.runtime.permissions, child.runtime.permissions);
        assert_eq!(restored_profile.prompt, profile.prompt);
        assert_eq!(child.model_settings.api_key, "private-child-credential");
        assert!(snapshot.restore(&parent, "other-owner", &store).is_err());
        parent.model_settings.endpoint = "https://different.invalid/v1".into();
        assert!(snapshot.restore(&parent, "owner", &store).is_err());
    }

    #[test]
    fn external_profile_freezes_its_harness_without_inheriting_parent_api_credentials() {
        let _environment = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("state.sqlite")).unwrap();
        let mut parent = HostInvocationRequest::for_test().unwrap();
        parent.model_settings.api_key = "never-export-parent-key".into();
        let profiles = crate::agent_registry::AgentRegistry::from_profiles(&Default::default());
        let mut profile = profiles.get("codex").unwrap().clone();
        profile.executor.as_mut().unwrap().model = Some("harness-model".into());
        let snapshot = DelegatedInvocation::capture(&parent, "owner", &profile, &store).unwrap();
        assert_eq!(snapshot.0.execution_backend, "codex");
        assert!(snapshot.0.credential.is_none());
        assert!(snapshot.0.model.endpoint.is_empty());
        parent.execution.execution_backend = "claude_code".into();
        parent.execution.external_execution_session_id = Some("parent-thread".into());
        let (_, child) = snapshot.restore(&parent, "owner", &store).unwrap();
        assert_eq!(child.execution.execution_backend, "codex");
        assert_eq!(child.execution.external_execution_model.as_deref(), Some("harness-model"));
        assert!(child.execution.external_execution_session_id.is_none());
        assert!(child.model_settings.api_key.is_empty());
        assert!(!serde_json::to_string(&snapshot).unwrap().contains("never-export-parent-key"));
    }

    #[test]
    fn query_credentials_are_not_a_durable_route() {
        assert!(
            route_identity("https://fixture.invalid/v1?api_key=secret", Some("openai")).is_err()
        );
        assert!(route_identity("https://user:secret@fixture.invalid/v1", None).is_err());
    }

    #[test]
    fn explicit_profile_resolves_full_route_and_revision_not_just_model_name() {
        let _environment = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("state.sqlite")).unwrap();
        let selected = nola_application::ModelProfile {
            id: "child-profile".into(), display_name: "Child".into(), model_name: "child-model".into(),
            provider: "openai-compatible".into(), runtime: "remote".into(),
            source_json: json!({"baseURL":"https://child.invalid/v1", "wireProtocol":"responses",
                "endpointID":"child-endpoint", "endpointEnabled":true, "modelEnabled":true,
                "capabilities":["chat"], "executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":true},
                "reasoning":{"control":"effort","efforts":["low","high"]}}).to_string(),
            parameters_json: json!({"temperature":0.4,"topP":0.8,"maxOutputTokens":8192,"maxContextLength":64000}).to_string(),
            selected:false, enabled:true, revision:1, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
        };
        store.upsert_model_profile(selected).unwrap();
        let selected = store.model_profile("child-profile").unwrap().unwrap();
        let mut parent = HostInvocationRequest::for_test().unwrap();
        parent.model_settings.endpoint = "https://parent.invalid/v1".into();
        parent.model_settings.api_key = "parent-secret".into();
        let registry = crate::agent_registry::AgentRegistry::from_profiles(&Default::default());
        let mut agent = registry.get("explore").unwrap().clone();
        agent.model = Some(
            serde_json::from_value(
                json!({"profile_id":selected.id,"expected_revision":selected.revision,
            "tier":"concise","overrides":{"temperature":0.2}}),
            )
            .unwrap(),
        );
        let snapshot = DelegatedInvocation::capture(&parent, "owner", &agent, &store).unwrap();
        let (_, child) = snapshot.restore(&parent, "owner", &store).unwrap();
        assert_eq!(child.model_settings.model, "child-model");
        assert_eq!(child.model_settings.endpoint, "https://child.invalid/v1");
        assert!(
            child.model_settings.api_key.is_empty(),
            "never reuse parent key at another endpoint"
        );
        assert_eq!(child.runtime.temperature, Some(0.2));
        assert_eq!(child.runtime.max_output_tokens, Some(8192));
        assert_eq!(child.runtime.compaction.context_window_tokens, Some(64000));
        assert_eq!(
            crate::llm::resolved_model_protocol(
                child.runtime.provider.as_deref(),
                &child.model_settings.endpoint
            ),
            nola_domain::generation::ModelProtocol::Responses
        );
        agent.model.as_mut().unwrap().expected_revision += 1;
        assert!(DelegatedInvocation::capture(&parent, "owner", &agent, &store).is_err());
    }
}

impl CredentialReferenceExt for CredentialReference {
    fn siwc_context(
        &self,
        store: &NolaStore,
    ) -> Result<Option<crate::model_siwc::SiwcExecutionContext>, String> {
        let CredentialSource::Siwc { account } = &self.source else {
            return Ok(None);
        };
        let host = crate::model_siwc::SiwcHost::open(store.path())?;
        let current = host.execution_binding(
            &account.account_id,
            account.account_revision,
            &account.credential_ref,
        )?;
        if &current != account {
            return Err("Frozen SIWC account identity changed".into());
        }
        Ok(Some(crate::model_siwc::SiwcExecutionContext {
            database: store.path().to_path_buf(),
            account: account.clone(),
        }))
    }

    fn resolve(&self, parent: &HostInvocationRequest, store: &NolaStore) -> Result<String, String> {
        if matches!(self.source, CredentialSource::Siwc { .. }) {
            self.siwc_context(store)?;
            return Ok(String::new());
        }
        if let CredentialSource::Account { account_id } = &self.source {
            let (protocol, endpoint) = self
                .route_identity
                .split_once('\n')
                .ok_or("Invalid frozen model account route")?;
            return crate::model_connection_host::credential_for_profile(
                store,
                &json!({"accountID":account_id,"baseURL":endpoint}),
                protocol,
            )
            .map(|secret| secret.unwrap_or_default());
        }
        if let CredentialSource::Endpoint { endpoint_id } = &self.source {
            let record = crate::host_credentials::HostCredentialVault::open(store.path())?
                .record(endpoint_id)?;
            return match record {
                Some(record) if record.identity == self.route_identity => {
                    Ok(record.secret.unwrap_or_default())
                }
                Some(_) => Err("Delegated endpoint credential identity changed".into()),
                // Credential-free LAN endpoints are valid. Absence never
                // borrows a secret from a different configured endpoint.
                None => Ok(String::new()),
            };
        }
        if route_identity(
            &parent.model_settings.endpoint,
            parent.runtime.provider.as_deref(),
        )? == self.route_identity
        {
            return Ok(parent.model_settings.api_key.clone());
        }
        for route in &parent.runtime.llm_fallbacks {
            if route_identity(&route.endpoint, route.provider.as_deref())? == self.route_identity {
                return Ok(route
                    .api_key
                    .clone()
                    .unwrap_or_else(|| parent.model_settings.api_key.clone()));
            }
        }
        Err(
            "Original delegated credential route is unavailable; no model fallback was performed"
                .into(),
        )
    }
}

/// Translate composed Host facts once, before Core admission. This adapter
/// removes secrets and live handles; profile selection/capture lives in Core.
/// Host-owned scratch scope, frozen before the Run is admitted. It authorizes
/// only this project's independent workspace container, never its parent or `/`.
pub(crate) fn prepare_managed_workspace_scope(parent: &mut HostInvocationRequest) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::path::Path;
    if !parent.tool_settings.tools_enabled || parent.tool_settings.task_worktree.is_some()
        || parent.session.host_task_execution.is_some()
        || crate::tools::observe_parent_repository(&parent.tool_settings.project_root)?.is_none() {
        return Ok(());
    }
    let project = parent.tool_settings.project_root.canonicalize().map_err(|e| e.to_string())?;
    let database = crate::platform_paths::application_database_path()?;
    let identity = serde_json::to_vec(&(&project, &database)).map_err(|e| e.to_string())?;
    let digest = format!("{:x}", Sha256::digest(identity));
    let directory = std::env::temp_dir().join(format!("nola-workspaces-{}", &digest[..32]));
    if directory.starts_with(&project) {
        return Err("Managed workspaces require an independent Host scratch directory outside the source project".into());
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)] {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&directory) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(format!("Prepare managed workspace scope: {e}")),
    }
    let metadata = std::fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Managed workspace scope is not an owned directory; no existing entry was replaced".into());
    }
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err("Managed workspace scope must be private to this Host user; no permissions were changed".into());
        }
    }
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if directory.starts_with(&project) || directory == Path::new("/") {
        return Err("Managed workspace scope is not independent of the source project".into());
    }
    if !parent.tool_settings.allowed_dirs.contains(&directory) {
        parent.tool_settings.allowed_dirs.push(directory);
    }
    Ok(())
}

pub(crate) fn parent_snapshot(
    parent: &HostInvocationRequest,
    store: &NolaStore,
) -> Result<nola_host_protocol::delegation::DelegationParentSnapshot, String> {
    use nola_host_protocol::delegation::*;
    let mut raw = serde_json::to_value(&parent.runtime).map_err(|e| e.to_string())?;
    raw["provider"] = json!(crate::llm::resolved_model_protocol(
        parent.runtime.provider.as_deref(),
        &parent.model_settings.endpoint
    )
    .as_str());
    let mut fallback_credentials = Vec::new();
    for route in raw["llm_fallbacks"]
        .as_array_mut()
        .ok_or("Host fallback routes missing")?
    {
        let endpoint = route["endpoint"]
            .as_str()
            .ok_or("Host fallback endpoint missing")?
            .to_owned();
        let protocol = crate::llm::resolved_model_protocol(route["provider"].as_str(), &endpoint);
        let route_id = route_identity(&endpoint, Some(protocol.as_str()))?;
        fallback_credentials.push(CredentialReference::inherited(route_id));
        route
            .as_object_mut()
            .ok_or("Host fallback route invalid")?
            .remove("api_key");
        route["provider"] = json!(protocol.as_str());
    }
    let mut runtime: DelegatedRuntimeConfiguration =
        serde_json::from_value(raw).map_err(|e| format!("Invalid portable Host runtime: {e}"))?;
    let mut model = DelegatedModel {
        endpoint: parent.model_settings.endpoint.clone(),
        model: parent.model_settings.model.clone(),
        swe_mode: parent.model_settings.swe_mode,
    };
    let credential = if parent.execution.execution_backend == "nola" {
        let identity = route_identity(&model.endpoint, runtime.provider.as_deref())?;
        Some(match &parent.runtime.siwc {
            Some(context) => CredentialReference::siwc(identity, context.account.clone()),
            None => CredentialReference::inherited(identity),
        })
    } else {
        model.endpoint.clear();
        runtime.llm_fallbacks.clear();
        fallback_credentials.clear();
        None
    };
    let profiles = nola_domain::delegation::effective_agent_profiles(&parent.runtime.agents);
    let mut profile_credentials = std::collections::BTreeMap::new();
    for profile in profiles.values() {
        let Some(intent) = &profile.model else {
            continue;
        };
        let Some(selected) = store.model_profile(&intent.profile_id)? else {
            continue;
        };
        let source: Value = serde_json::from_str(&selected.source_json)
            .map_err(|_| "Invalid delegated model source")?;
        if source["wireProtocol"] != "siwc_responses" {
            continue;
        }
        let account_id = source["accountID"]
            .as_str()
            .ok_or("Subscription account missing")?;
        let account: Option<nola_domain::model_connection::ModelAccountSnapshot> = store
            .application()?
            .execute(nola_application::ApplicationCommand::Model {
                command: nola_application::ModelCommand::GetAccount {
                    id: account_id.into(),
                },
            })?;
        let account = account.ok_or("Subscription account unavailable")?;
        let binding = crate::model_siwc::SiwcHost::open(store.path())?.execution_binding(
            account_id,
            account.revision,
            account
                .account
                .credential_ref
                .as_deref()
                .ok_or("Subscription credential reference missing")?,
        )?;
        let endpoint = source["baseURL"]
            .as_str()
            .ok_or("Subscription endpoint missing")?;
        profile_credentials.insert(
            intent.profile_id.clone(),
            CredentialReference::siwc(
                nola_host_protocol::delegation::route_identity(endpoint, Some("siwc_responses"))?,
                binding,
            ),
        );
    }
    let read_only_tools = profiles
        .values()
        .flat_map(|profile| profile.tools.iter())
        .filter(|name| crate::tools::tool_names_are_read_only(std::slice::from_ref(*name)))
        .cloned()
        .collect();
    let result = DelegationParentSnapshot {
        process_sandbox: parent.tool_settings.process_sandbox.as_ref().map(serde_json::to_value).transpose().map_err(|error|error.to_string())?,
        bounded_file_tools: profiles.values().flat_map(|profile|profile.tools.iter())
            .filter(|name|crate::tools::tool_has_bounded_file_writes(name)).cloned().collect(),
        project_repository_identity: crate::tools::observe_parent_repository(&parent.tool_settings.project_root)?,
        task_worktree: parent.tool_settings.task_worktree.clone(),
        task_sandbox_requirement: parent.tool_settings.task_sandbox_requirement,
        authorized_workspace_roots: parent.tool_settings.allowed_dirs.iter()
            .map(|root| root.canonicalize().unwrap_or_else(|_|root.clone()).to_string_lossy().into_owned()).collect(),
        requested_native_sandbox: Some(crate::external_executor::external_permission_policy(&parent.external_execution_settings()).1.into()),
        project_root: parent.tool_settings.project_root.canonicalize()
            .unwrap_or_else(|_|parent.tool_settings.project_root.clone()).to_string_lossy().into_owned(),
        read_only_tools,
        model,
        runtime,
        credential,
        fallback_credentials,
        profiles,
        profile_credentials,
        execution_backend: parent.execution.execution_backend.clone(),
        external_execution_model: parent.execution.external_execution_model.clone(),
        external_execution_model_provider: parent
            .execution
            .external_execution_model_provider
            .clone(),
        max_iterations: parent.execution.max_iterations,
    };
    result.validate()?;
    Ok(result)
}

/// Public delegation snapshots cannot carry private endpoint query material.
/// Inspect the same validated binding used by the primary Host invocation;
/// never strip the query and silently delegate to a different route.
pub(crate) fn supports_portable_delegation(parent: &HostInvocationRequest) -> Result<bool, String> {
    if parent.runtime.native_model_route.is_some() { return Ok(false); }
    if parent.execution.execution_backend != "nola" { return Ok(true); }
    let settings = parent.model_invocation_settings().map_err(|error| error.to_string())?;
    if matches!(settings.primary(), nola_host_protocol::HostModelRouteBinding::Http(route) if route.endpoint_query_sha256.is_some()) {
        return Ok(false);
    }
    for fallback in &parent.runtime.llm_fallbacks {
        let route = crate::model_route::route_binding(fallback.provider.as_deref(), &fallback.endpoint,
            fallback.model.as_deref().unwrap_or(&parent.model_settings.model),
            fallback.api_key.as_deref().unwrap_or(&parent.model_settings.api_key), parent.runtime.chat_output_token_field)?;
        route.validate().map_err(str::to_string)?;
        if matches!(route, nola_host_protocol::HostModelRouteBinding::Http(route) if route.endpoint_query_sha256.is_some()) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn frozen_parent(
    parent: &HostInvocationRequest,
) -> Result<Option<nola_host_protocol::delegation::ValidatedDelegationParent>, String> {
    if !supports_portable_delegation(parent)? { return Ok(None); }
    let store = NolaStore::open_default()?;
    nola_host_protocol::delegation::ValidatedDelegationParent::new(parent_snapshot(parent, &store)?)
        .map(Some)
}
