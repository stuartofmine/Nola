// SPDX-License-Identifier: Apache-2.0
//! Prepare child execution from the parent's immutable admission, never mutable
//! platform defaults. TaskPromoted remains the sole source-admission transition.
use nola_host_protocol::delegation::*;
use nola_store_port::model::{ModelRepository, ModelRepositoryProvider};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub fn capture_delegated_invocation<R: ModelRepositoryProvider>(
    store: &mut R,
    parent: &DelegationParentSnapshot,
    owner: &str,
    profile_name: &str,
) -> Result<DelegatedInvocationSnapshot, String> {
    parent.validate()?;
    if owner.trim().is_empty() {
        return Err("Delegation requires a parent conversation".into());
    }
    let mut profile = parent
        .profiles
        .get(profile_name)
        .cloned()
        .ok_or("Unknown frozen agent profile")?;
    let mut model = parent.model.clone();
    let mut runtime = parent.runtime.clone();
    let mut credential = parent.credential.clone();
    let mut fallback_credentials = parent.fallback_credentials.clone();
    let execution_backend = profile.executor.as_ref().map(|e| e.backend.id().to_owned())
        .unwrap_or_else(|| parent.execution_backend.clone());
    let external_execution_model = profile.executor.as_ref().map(|e| e.model.clone())
        .unwrap_or_else(|| parent.external_execution_model.clone());
    let external_execution_model_provider = profile.executor.as_ref().map(|e| e.model_provider.clone())
        .unwrap_or_else(|| parent.external_execution_model_provider.clone());
    if execution_backend != "nola" {
        if parent.process_sandbox.is_some() {return Err("Required native process sandbox cannot cover the external executor".into());}
        if !profile.tools.is_empty() {
            return Err("External harness cannot enforce a Nola-specific tool allowlist; use an external profile".into());
        }
        // Harness login belongs to the Host. Never pass the parent's API route/key.
        model.endpoint.clear();
        model.model.clear();
        runtime.selected_profile = None;
        runtime.llm_fallbacks.clear();
        credential = None;
        fallback_credentials.clear();
    }
    if let Some(intent) = &profile.model {
        if execution_backend != "nola" {
            return Err("A remote profile cannot override an external executor route".into());
        }
        intent.validate()?;
        let models = store.model_repository();
        let selected: crate::ModelProfile = models
            .model_profile(&intent.profile_id)
            .map_err(|e| e.to_string())?
            .ok_or("Delegated model profile is unavailable")?
            .into();
        if selected.revision != intent.expected_revision {
            return Err("Delegated model profile revision changed".into());
        }
        let source: Value = serde_json::from_str(&selected.source_json)
            .map_err(|_| "Invalid delegated model source")?;
        let protocol = nola_domain::generation::ModelProtocol::parse(
            source["wireProtocol"]
                .as_str()
                .ok_or("Missing delegated model protocol")?,
        )?;
        // The shared resolver owns tier-to-provider mapping. Passing both its
        // resolved selection and the original tier is an ambiguous request.
        let mut request = intent.overrides.to_request()?;
        if intent.overrides.reasoning_selection.is_none() {
            request["reasoning_tier"] = json!(intent.tier);
        }
        let selected_model = crate::model_profile_resolution::resolve_remote_profile(
            &selected,
            &request,
            runtime.reasoning_tier,
        )?;
        model.endpoint = selected_model.endpoint;
        model.model = selected_model.model;
        let selected_runtime = selected_model.runtime;
        runtime.selected_profile = Some(selected_model.selected_profile);
        runtime.provider = Some(protocol.as_str().into());
        runtime.temperature = selected_runtime.temperature;
        runtime.top_p = selected_runtime.top_p;
        runtime.max_output_tokens = selected_runtime.max_output_tokens;
        runtime.reasoning_selection = selected_runtime.reasoning_selection;
        runtime.reasoning_tier = selected_runtime.reasoning_tier;
        runtime.sampling_override = selected_runtime.sampling_override;
        runtime.generation_options = selected_runtime.generation_options;
        runtime.model_capabilities = selected_runtime.model_capabilities;
        runtime.chat_output_token_field = selected_runtime.chat_output_token_field;
        runtime.compaction.context_window_tokens = selected_runtime.context_window_tokens;
        runtime.llm_fallbacks.clear();
        fallback_credentials.clear();
        let identity = route_identity(&model.endpoint, runtime.provider.as_deref())?;
        let source = if protocol == nola_domain::generation::ModelProtocol::SiwcResponses {
            let reference = parent
                .profile_credentials
                .get(&intent.profile_id)
                .ok_or("Subscription profile has no frozen Host reference")?;
            let CredentialSource::Siwc { account } = &reference.source else {
                return Err("Subscription profile reference has another adapter".into());
            };
            let current = models
                .model_account(&account.account_id)
                .map_err(|e| e.to_string())?
                .ok_or("Subscription account unavailable")?;
            if reference.route_identity != identity
                || source["accountID"].as_str() != Some(account.account_id.as_str())
                || current.revision != account.account_revision
                || current.account.credential_ref.as_deref()
                    != Some(account.credential_ref.as_str())
            {
                return Err("Frozen subscription account reference changed".into());
            }
            reference.source.clone()
        } else if source["connectionID"].as_str().is_some() {
            CredentialSource::Account {
                account_id: source["accountID"].as_str().map(str::to_owned),
            }
        } else {
            CredentialSource::Endpoint {
                endpoint_id: source["endpointID"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or("Model endpoint reference missing")?
                    .into(),
            }
        };
        credential = Some(CredentialReference {
            route_identity: identity,
            source,
        });
    }
    runtime.agents.clear();
    profile.max_turns = Some(profile.max_turns.unwrap_or(parent.max_iterations));
    Ok(DelegatedInvocationSnapshot {
        process_sandbox: parent.process_sandbox.clone(),
        bounded_file_writes: false,
        project_repository_identity: parent.project_repository_identity.clone(),
        worktree: parent.task_worktree.clone(),
        sandbox_requirement: parent.task_sandbox_requirement,
        owner_session_id: owner.into(),
        execution_backend,
        external_execution_model,
        external_execution_model_provider,
        profile,
        model,
        runtime,
        credential,
        fallback_credentials,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareDelegatedTask {
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub observed_worktree: Option<nola_domain::task_workspace::TaskWorktreeBinding>,
    pub request: nola_agent_engine::agent::host::HostRequestEnvelope,
    pub input: crate::CreateRuntimeTask,
    pub now_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedDelegatedTask {
    pub task: crate::RuntimeTask,
    pub snapshot: DelegatedInvocationSnapshot,
    pub replayed: bool,
    pub source_admitted: bool,
}

pub(crate) fn bind_task_workspace(
    args: &serde_json::Map<String, Value>,
    parent: &DelegationParentSnapshot,
    observed: Option<&nola_domain::task_workspace::TaskWorktreeBinding>,
    mut snapshot: DelegatedInvocationSnapshot,
    existing: bool,
) -> Result<DelegatedInvocationSnapshot, String> {
    use nola_domain::task_workspace::{TaskWorktreeSelection, TaskSandboxRequirement};
    let selected = args.get("worktree").map(|value| serde_json::from_value::<TaskWorktreeSelection>(value.clone()))
        .transpose().map_err(|_|"Invalid task worktree selection")?;
    let sandbox = args.get("sandbox_requirement").map(|value| serde_json::from_value::<TaskSandboxRequirement>(value.clone()))
        .transpose().map_err(|_|"Unsupported task sandbox requirement")?;
    if existing {
        if selected.as_ref().is_some_and(|value| snapshot.worktree.as_ref().map(|b|&b.selection)!=Some(value))
            || sandbox.is_some_and(|value|snapshot.sandbox_requirement!=Some(value))
            || snapshot.worktree.as_ref()!=observed {
            return Err("Task worktree or sandbox differs from its original frozen invocation".into());
        }
    } else {
        let expected = selected.as_ref().or(parent.task_worktree.as_ref().map(|binding|&binding.selection));
        if expected!=observed.map(|binding|&binding.selection) {
            return Err("Task worktree requires the exact Host-observed selection".into());
        }
        snapshot.worktree = observed.cloned();
        snapshot.sandbox_requirement = sandbox.or(parent.task_sandbox_requirement);
    }
    if let Some(binding) = &snapshot.worktree {
        let source_root=parent.task_worktree.as_ref().and_then(|bound|bound.selection.source_snapshot.as_ref())
            .map(|source|source.source_root.as_str()).unwrap_or(parent.project_root.as_str());
        if binding.selection.source_snapshot.as_ref().is_some_and(|source|source.source_root!=source_root) {
            return Err("Source snapshot differs from the frozen parent checkout".into());
        }
        let repository=parent.task_worktree.as_ref().map(|bound|bound.repository_identity())
            .or_else(||parent.project_repository_identity.clone())
            .ok_or("Parent project Git repository was not observed; select a Git project before binding a worktree")?;
        if binding.repository_identity()!=repository {
            return Err("Selected worktree belongs to a different repository than the frozen parent project".into());
        }
        if parent.task_worktree.as_ref()==Some(binding) { binding.validate_execution_root(&parent.authorized_workspace_roots)?; }
        else { binding.validate_authorized_roots(&parent.authorized_workspace_roots)?; }
    }
    if snapshot.sandbox_requirement.is_some()
        && (snapshot.execution_backend!="codex" || parent.requested_native_sandbox.as_deref()!=Some("workspace-write")) {
        return Err("Required Codex workspace-write sandbox is unavailable; task was not admitted".into());
    }
    Ok(snapshot)
}

pub(crate) fn bounded_file_profile(parent:&DelegationParentSnapshot,snapshot:&DelegatedInvocationSnapshot)->bool {
    snapshot.worktree.is_some() && snapshot.execution_backend=="nola" && !snapshot.profile.tools.is_empty()
        && snapshot.profile.tools.iter().all(|name|parent.read_only_tools.contains(name)||parent.bounded_file_tools.contains(name))
}

pub(crate) fn prepare<R: crate::repositories::AgentRuntimeCommandRepositories>(
    store: &mut R,
    input: PrepareDelegatedTask,
) -> Result<PreparedDelegatedTask, String> {
    use nola_agent_engine::agent::{host::HostRequest, AgentCoreConfig};
    use sha2::{Digest, Sha256};
    let PrepareDelegatedTask {
        request,
        mut input,
        now_ms,
        observed_worktree,
    } = input;
    let HostRequest::ExecuteTool {
        capability_id,
        call_id,
        input: arguments,
        ..
    } = &request.payload
    else {
        return Err("Delegation requires a committed ExecuteTool request".into());
    };
    if capability_id != "spawn_agent" {
        return Err("Delegation preparation requires spawn_agent".into());
    }
    let (run_id, conversation_id, _) =
        crate::started_host_effect::validate(store, &request, "execute_tool", now_ms)?;
    if store.read_room_context_owner(&conversation_id).map_err(|e|e.to_string())?.is_some() {return Err("experts_cannot_spawn_agents".into());}
    let frozen = store
        .load_run_invocation_config(&run_id)
        .map_err(|e| e.to_string())?
        .ok_or("Parent invocation configuration missing")?;
    if format!(
        "{:x}",
        Sha256::digest(frozen.invocation_config_json.as_bytes())
    ) != frozen.invocation_config_digest
    {
        return Err("Parent invocation integrity failure".into());
    }
    let configuration: AgentCoreConfig = serde_json::from_str(&frozen.invocation_config_json)
        .map_err(|_| "Invalid frozen parent invocation")?;
    let parent = configuration
        .turn_identity
        .delegation
        .ok_or("Parent Host did not admit delegation")?
        .snapshot()?;
    let args = arguments
        .as_object()
        .ok_or("Delegation arguments must be an object")?;
    let agent = args
        .get("agent_type")
        .and_then(Value::as_str)
        .unwrap_or("explore");
    let prompt = args
        .get("prompt")
        .or_else(|| args.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("spawn_agent requires a non-empty prompt")?;
    let readonly = match args.get("permission_mode") {
        None => false,
        Some(Value::String(mode)) if mode == "ask" => false,
        Some(Value::String(mode)) if mode == "read_only" => true,
        _ => return Err("Invalid delegation permission mode".into()),
    };
    if input.kind != "agent"
        || input.owner_session_id.as_deref() != Some(conversation_id.as_str())
        || input.parent_tool_call_id.as_deref() != Some(call_id)
        || input.agent_type.as_deref() != Some(agent)
        || input.prompt.as_deref() != Some(prompt)
        || input.project_root != parent.project_root
    {
        return Err("Prepared task differs from its exact parent tool invocation".into());
    }
    let origin = json!({"parent_conversation_id":conversation_id,"parent_run_id":run_id,
        "parent_effect_id":request.effect_id.0,"parent_call_id":call_id,
        "arguments_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(arguments).map_err(|e|e.to_string())?))});
    // Replay returns the original complete snapshot, including after profile
    // changes. It does not capture new credentials/settings for an old call.
    if let Some(task) = store
        .runtime_task_for_delegation(&conversation_id, &run_id, &request.effect_id.0)
        .map_err(|e| e.to_string())?
    {
        if task.kind != "agent"
            || task.owner_session_id.as_deref() != Some(conversation_id.as_str())
            || task.prompt.as_deref() != Some(prompt)
            || task.agent_type.as_deref() != Some(agent)
        {
            return Err("Existing delegation has another immutable owner or input".into());
        }
        let metadata: Value = serde_json::from_str(&task.metadata_json)
            .map_err(|_| "Invalid task execution metadata")?;
        if metadata["delegation_origin"] != origin {
            return Err("Prepared delegation origin or arguments changed".into());
        }
        let snapshot: DelegatedInvocationSnapshot =
            serde_json::from_value(metadata["delegated_invocation"].clone())
                .map_err(|_| "Existing delegation snapshot invalid")?;
        if snapshot.owner_session_id != conversation_id || snapshot.profile.name != agent {
            return Err("Existing delegation snapshot owner differs".into());
        }
        let authority = store
            .runtime_agent_task_authority(&task.id)
            .map_err(|e| e.to_string())?
            .ok_or("Prepared delegation authority missing")?;
        authority.validate()?;
        let source_admitted = authority.phase
            == nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Admitted;
        if let Some(admission) = authority.admission {
            if admission.parent_run_id != run_id
                || admission.parent_effect_id != request.effect_id.0
                || admission.parent_tool_call_id != *call_id
            {
                return Err("Delegated source admission changed".into());
            }
        }
        let snapshot = bind_task_workspace(args,&parent,observed_worktree.as_ref(),snapshot,true)?;
        return Ok(PreparedDelegatedTask {
            task: crate::runtime_task::runtime_task(task),
            snapshot,
            replayed: true,
            source_admitted,
        });
    }
    // Existing child continuation keeps its original execution contract. The
    // source task must be terminal, same parent/project/profile, with binding.
    let mut parent_task_id = None;
    let continued = args.get("task_id").and_then(Value::as_str).is_some();
    let snapshot = if let Some(previous_id) = args.get("task_id").and_then(Value::as_str) {
        let previous = store
            .runtime_task(previous_id)
            .map_err(|e| e.to_string())?
            .ok_or("Previous task unavailable")?;
        if !matches!(previous.status.as_str(), "sealed" | "failed" | "cancelled")
            || previous.owner_session_id.as_deref() != Some(conversation_id.as_str())
            || previous.project_root != parent.project_root
            || previous.agent_type.as_deref() != Some(agent)
            || input.child_session_id != previous.child_session_id
        {
            return Err("Delegation continuation differs from its original task".into());
        }
        let binding = store
            .runtime_task_run_binding(previous_id)
            .map_err(|e| e.to_string())?
            .ok_or("Previous task has no immutable child Run binding")?;
        if binding.owner_session_id != conversation_id
            || Some(binding.child_session_id.as_str()) != previous.child_session_id.as_deref()
        {
            return Err("Previous task child binding differs".into());
        }
        parent_task_id = previous.parent_task_id.clone();
        let metadata: Value = serde_json::from_str(&previous.metadata_json)
            .map_err(|_| "Invalid original task execution metadata")?;
        let snapshot: DelegatedInvocationSnapshot =
            serde_json::from_value(metadata["delegated_invocation"].clone())
                .map_err(|_| "Original delegated execution snapshot missing")?;
        if snapshot.owner_session_id != conversation_id {
            return Err("Original delegated execution owner differs".into());
        }
        snapshot
    } else {
        capture_delegated_invocation(store, &parent, &conversation_id, agent)?
    };
    let mut snapshot = bind_task_workspace(args,&parent,observed_worktree.as_ref(),snapshot,continued)?;
    if snapshot.execution_backend != "nola" && readonly {
        return Err("External harnesses cannot promise the Nola read-only tool catalog; delegate with ask or use a Nola read-only profile".into());
    }
    let can_mutate = !readonly
        && (snapshot.profile.tools.is_empty()
            || snapshot
                .profile
                .tools
                .iter()
                .any(|name| !parent.read_only_tools.contains(name)));
    if !continued {
        snapshot.bounded_file_writes = can_mutate && bounded_file_profile(&parent,&snapshot);
    }
    if parent.task_worktree.is_some() && can_mutate {
        return Err("A bound worktree task cannot admit another write/command child under a different project key. Keep the work in this task or use a genuinely read-only child; no silent permission downgrade.".into());
    }
    let max_turns = args
        .get("max_turns")
        .and_then(Value::as_u64)
        .map(|n| n.clamp(1, 20) as usize)
        .or(snapshot.profile.max_turns);
    input.parent_task_id = parent_task_id;
    input.created_at_ms = now_ms;
    input.input_watermark = Some(now_ms);
    let priority = match args
        .get("priority")
        .and_then(Value::as_str)
        .unwrap_or("normal")
    {
        "low" => crate::RuntimeTaskPriority::Low,
        "normal" => crate::RuntimeTaskPriority::Normal,
        "high" => crate::RuntimeTaskPriority::High,
        _ => return Err("Invalid task priority".into()),
    };
    let budget = crate::RuntimeTaskResourceBudget {
        max_wall_time_ms: Some(
            args.get("max_wall_time_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(7200)
                .clamp(1, 86400)
                .saturating_mul(1000),
        ),
        max_output_bytes: Some(
            args.get("max_output_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(2 * 1024 * 1024)
                .clamp(1024, 16 * 1024 * 1024),
        ),
        max_cpu_time_ms: args
            .get("max_cpu_time_seconds")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 86400).saturating_mul(1000)),
        max_memory_bytes: args
            .get("max_memory_bytes")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(16 * 1024 * 1024, 64 * 1024 * 1024 * 1024)),
        max_processes: args
            .get("max_processes")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 256) as u32),
    };
    input.scheduling = crate::RuntimeTaskScheduling {
        priority,
        queued_at_ms: input.created_at_ms,
        resource_budget: budget,
        output_bytes: 0,
        resource_usage: Default::default(),
        budget_exhaustion: None,
    };
    let metadata = input
        .metadata
        .as_object_mut()
        .ok_or("Task metadata must be an object")?;
    if metadata.keys().any(|key| {
        !matches!(
            key.as_str(),
            "max_turns"
                | "model_notified"
                | "can_mutate"
                | "requested_read_only"
                | "execution_backend"
                | "execution_model"
                | "execution_model_provider"
                | "resumed_from_task_id"
                | "_nolaExecution"
        )
    }) {
        return Err(
            "Caller cannot supply delegated execution authority or unknown metadata".into(),
        );
    }
    metadata.insert("model_notified".into(), json!(false));
    metadata.insert("delegation_origin".into(), origin);
    metadata.insert(
        "resumed_from_task_id".into(),
        json!(args.get("task_id").and_then(Value::as_str)),
    );
    metadata.insert("execution_backend".into(), json!(snapshot.execution_backend));
    metadata.insert(
        "execution_model".into(),
        json!(snapshot.external_execution_model),
    );
    metadata.insert(
        "execution_model_provider".into(),
        json!(snapshot.external_execution_model_provider),
    );
    metadata.insert(
        "delegated_invocation".into(),
        serde_json::to_value(&snapshot).map_err(|e| e.to_string())?,
    );
    metadata.insert("requested_read_only".into(), json!(readonly));
    metadata.insert("can_mutate".into(), json!(can_mutate));
    metadata.insert("max_turns".into(), json!(max_turns));
    let task = input.into_task()?;
    store
        .create_runtime_task_with_event(&crate::runtime_task::stored_task(task.clone()))
        .map_err(|e| e.to_string())?;
    Ok(PreparedDelegatedTask {
        task,
        snapshot,
        replayed: false,
        source_admitted: false,
    })
}
