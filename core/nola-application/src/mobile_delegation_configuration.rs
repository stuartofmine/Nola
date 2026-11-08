// SPDX-License-Identifier: Apache-2.0
//! Embedded child configuration derives from the committed parent, never a
//! client-provided replacement policy or a second profile resolver.
use nola_agent_engine::agent::{host::*, AgentCoreConfig, SessionId};
use nola_host_protocol::delegation::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobileDelegationSettings {
    pub project_root: String,
    /// Host can rebuild an exact independent HTTP client from existing credentials.
    #[serde(default)]
    pub independent_model_profiles: bool,
    pub selected_profile: Option<nola_host_protocol::HostModelProfileReference>,
    pub agents: BTreeMap<String, nola_domain::execution_policy::ConversationAgentPolicy>,
}

/// Called only by a Host with an installed independent child executor. The
/// complete Host route is already frozen; credential material never enters Core.
pub fn capture_parent(
    mut config: AgentCoreConfig,
    settings: MobileDelegationSettings,
) -> Result<AgentCoreConfig, String> {
    if config.turn_identity.delegation.is_some() {
        return Err("Mobile delegation configuration was already captured".into());
    }
    if !config.turn_identity.model_capability.native_tool_calls {
        return Ok(config);
    }
    let binding = config
        .turn_identity
        .host_invocation
        .as_ref()
        .ok_or("Delegation requires a bound Host invocation")?;
    binding.validate().map_err(str::to_owned)?;
    let nola_host_protocol::HostModelRouteBinding::Http(route) = &binding.primary else {
        return Ok(config); // This executor capability is unavailable for native model routes.
    };
    if route.endpoint_query_sha256.is_some() || !binding.fallbacks.is_empty() {
        return Ok(config); // Do not advertise a child transport the Host cannot reproduce.
    }
    if config.model.as_deref() != Some(route.model.as_str()) {
        return Err("Parent model identity differs from its execution binding".into());
    }
    let generation = &binding.generation;
    let runtime = DelegatedRuntimeConfiguration {
        selected_profile: settings.selected_profile,
        provider: Some(route.protocol.as_str().into()),
        temperature: generation.temperature.as_f64(),
        top_p: generation.top_p.as_f64(),
        max_output_tokens: Some(generation.max_output_tokens),
        reasoning_selection: generation.reasoning_selection.clone(),
        model_capabilities: nola_domain::model_runtime::RuntimeModelCapabilities {
            reasoning_summary_supported: generation.reasoning_summary_supported,
            ..Default::default()
        },
        reasoning_tier: generation.reasoning_tier,
        sampling_override: generation.sampling_override,
        generation_options: generation.generation_options.to_options(),
        agents: settings.agents.clone(),
        core_permission_rules: config.permission_rules.rules.clone(),
        openai_prompt_cache: Some(generation.prompt_cache),
        openai_prompt_cache_retention: generation.prompt_cache_retention.clone(),
        ..Default::default()
    };
    let catalog = config
        .host_catalog
        .as_ref()
        .ok_or("Delegation requires an admitted tool catalog")?;
    let read_only_tools = catalog
        .capabilities
        .iter()
        .filter(|(_, value)| {
            value.input_normalization.permission_contract.risk
                == nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead
        })
        .map(|(key, _)| key.clone())
        .collect();
    let reference = CredentialReference::inherited(route_identity(
        &route.endpoint,
        Some(route.protocol.as_str()),
    )?);
    let mut profiles = nola_domain::delegation::effective_agent_profiles(&settings.agents);
    profiles.retain(|_, profile| profile.executor.is_none()); // Local mobile Host cannot launch desktop harnesses.
    if !settings.independent_model_profiles { profiles.retain(|_, profile| profile.model.is_none()); }
    let parent = DelegationParentSnapshot {
        process_sandbox: None,
        bounded_file_tools: Default::default(),
            project_repository_identity: None,
        task_worktree: None, task_sandbox_requirement: None,
        authorized_workspace_roots: Vec::new(),
        requested_native_sandbox: None,
        model: DelegatedModel {
            endpoint: route.endpoint.clone(),
            model: route.model.clone(),
            swe_mode: false,
        },
        runtime,
        credential: Some(reference),
        fallback_credentials: vec![],
        profiles,
        project_root: settings.project_root,
        read_only_tools,
        profile_credentials: BTreeMap::new(),
        execution_backend: "nola".into(),
        external_execution_model: None,
        external_execution_model_provider: None,
        max_iterations: config.max_model_iterations,
    };
    let supported_profiles: Vec<_> = parent.profiles.values()
        .map(|profile| profile.name.clone()).collect();
    config.tools.retain(|value| {
        value.pointer("/function/name").and_then(|v| v.as_str()) != Some("spawn_agent")
    });
    config.host_catalog.as_mut().unwrap().capabilities.remove("spawn_agent");
    if supported_profiles.is_empty() {
        bind_policy(&mut config)?;
        return Ok(config);
    }
    config.turn_identity.delegation = Some(ValidatedDelegationParent::new(parent)?);
    let schema = json!({"type":"function","function":{"name":"spawn_agent","description":"Delegate one bounded task to an independent child using this Host and the frozen parent model. Independent profile models require the same authorized connection and frozen generation policy. Other connections and nested delegation are unavailable. Supply a self-contained brief and authorized source references, not the complete parent transcript. The parent supervises and accepts the child result.","parameters":{"type":"object","properties":{"agent_type":{"type":"string","enum":supported_profiles},"prompt":{"type":"string","description":nola_domain::delegation::WORKSPACE_GUIDANCE},"task_id":{"type":"string","description":"Derive new follow-up work from a terminal Task owned by this parent/project. Creates a NEW task_id with resumed_from_task_id lineage and the existing child conversation; the original Task and sealed Run remain terminal. For a running Task use send_agent_message."},"description":{"type":"string"},"max_turns":{"type":"integer","minimum":1,"maximum":20,"description":"Explicit task LLM-turn budget. When omitted use the frozen profile default, then parent iteration budget; fixed during recovery."},"permission_mode":{"type":"string","enum":["ask","read_only"]}},"required":["agent_type","prompt"],"additionalProperties":false}}});
    config.tools.push(schema.clone());
    let catalog = config.host_catalog.as_mut().unwrap();
    catalog.capabilities.insert(
        "spawn_agent".into(),
        HostCapabilityDescriptor {
            id: "spawn_agent".into(),
            title: "Delegate task".into(),
            route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Visible,
            permission_domain: None,
            input_schema: schema["function"]["parameters"].clone(),
            input_normalization: HostInputNormalization {
                permission_contract:
                    nola_domain::permission_resource_plan::PermissionOperationContract {
                        risk:
                            nola_domain::permission_resource_plan::PermissionOperationRisk::NewFile,
                        ..Default::default()
                    },
                ..Default::default()
            },
            output_policy: HostOutputPolicy {
                preview_char_limit: 4000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: false,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        },
    );
    let guidance = json!({"type":"function","function":{"name":"send_agent_message",
        "description":"Send a bounded follow-up to this parent's running Task at its next model boundary. Supply the exact task_id; no new Task or authorization is created. Accepted means queued, not consumed. Stopped/terminal tasks cannot be revived; use spawn_agent task_id only to derive explicitly requested new follow-up work.",
        "parameters":{"type":"object","properties":{"task_id":{"type":"string"},"message":{"type":"string"}},"required":["task_id","message"],"additionalProperties":false}}});
    config.tools.retain(|t|t.pointer("/function/name").and_then(|v|v.as_str()) != Some("send_agent_message"));
    config.tools.push(guidance.clone());
    let mut descriptor = config.host_catalog.as_ref().unwrap().capabilities["spawn_agent"].clone();
    descriptor.id = "send_agent_message".into(); descriptor.title = "Guide task".into();
    descriptor.input_schema = guidance["function"]["parameters"].clone();
    config.host_catalog.as_mut().unwrap().capabilities.insert("send_agent_message".into(), descriptor);
    for (name, description) in [
        ("read_task", "Read this parent's delegated Task state and immutable result preview. A queued/running state is not a completed result."),
        ("wait_task", "Wait once for this parent's delegated Task result, up to timeout_sec (default 30, maximum 300). Prefer this when the child result blocks progress. A timeout or cancelled wait does not stop the child."),
    ] {
        let mut properties = json!({"task_id":{"type":"string"}});
        if name == "wait_task" { properties["timeout_sec"] = json!({"type":"integer","minimum":1,"maximum":300}); }
        let schema = json!({"type":"function","function":{"name":name,"description":description,
            "parameters":{"type":"object","properties":properties,"required":["task_id"],"additionalProperties":false}}});
        config.tools.retain(|t| t.pointer("/function/name").and_then(|v|v.as_str()) != Some(name));
        config.tools.push(schema.clone());
        let mut descriptor = config.host_catalog.as_ref().unwrap().capabilities["spawn_agent"].clone();
        descriptor.id = name.into(); descriptor.title = name.into();
        descriptor.input_schema = schema["function"]["parameters"].clone();
        descriptor.input_normalization.permission_contract.risk = nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead;
        config.host_catalog.as_mut().unwrap().capabilities.insert(name.into(), descriptor);
    }
    bind_policy(&mut config)?;
    Ok(config)
}

/// Narrow independent-model admission: the same credential owner and transport
/// facts, not merely the same hostname. No secret is read or transferred here.
pub(super) fn validate_profile_model<R: nola_store_port::model::ModelRepositoryProvider>(
    store: &mut R, parent: &DelegationParentSnapshot, child: &DelegatedInvocationSnapshot,
) -> Result<(), String> {
    use nola_store_port::model::ModelRepository;
    if child.execution_backend != "nola" { return Err("This local mobile Host cannot execute desktop harnesses; use a connected desktop Host".into()); }
    if child.model.endpoint != parent.model.endpoint { return Err("Embedded child requires a separately authorized connection".into()); }
    let a=&parent.runtime; let b=&child.runtime;
    if a.provider != b.provider || a.temperature != b.temperature || a.top_p != b.top_p
        || a.max_output_tokens != b.max_output_tokens || a.reasoning_selection != b.reasoning_selection
        || a.reasoning_tier != b.reasoning_tier || a.sampling_override != b.sampling_override
        || a.generation_options != b.generation_options
        || a.model_capabilities.reasoning_summary_supported != b.model_capabilities.reasoning_summary_supported {
        return Err("Embedded child generation policy differs from the frozen parent".into());
    }
    let Some(intent) = &child.profile.model else {
        return if child.model.model == parent.model.model && child.runtime.selected_profile == parent.runtime.selected_profile { Ok(()) } else { Err("Inherited child model profile changed".into()) };
    };
    if !parent.profiles.get(&child.profile.name).is_some_and(|p| p.model.is_some()) {
        return Err("This Host has no independent profile executor".into());
    }
    let original = parent.runtime.selected_profile.as_ref().ok_or("Independent child requires a canonical parent profile")?;
    let models = store.model_repository();
    let source = models.model_profile(&original.profile_id).map_err(|e|e.to_string())?.ok_or("Parent model profile unavailable")?;
    let target = models.model_profile(&intent.profile_id).map_err(|e|e.to_string())?.ok_or("Child model profile unavailable")?;
    if source.revision != original.admitted_revision || target.revision != intent.expected_revision {
        return Err("Independent child model profile revision changed".into());
    }
    let parse = |s: &str| serde_json::from_str::<serde_json::Value>(s).map_err(|e|e.to_string());
    let source_facts = |raw: &str| -> Result<serde_json::Value, String> {
        let mut value = parse(raw)?;
        if let Some(fields) = value.as_object_mut() {
            // Catalog identity/order are presentation, not connection authority.
            fields.remove("catalogKey"); fields.remove("modelOrder");
        }
        Ok(value)
    };
    if source_facts(&source.source_json)? != source_facts(&target.source_json)?
        || parse(&source.parameters_json)? != parse(&target.parameters_json)? {
        return Err("Embedded child requires the same authorized connection, capabilities and profile generation settings".into());
    }
    Ok(())
}

fn bind_policy(config: &mut AgentCoreConfig) -> Result<(), String> {
    let mut policy = json!({"host_catalog":config.host_catalog,"tools":config.tools,"permission_rules":config.permission_rules,"context_window":config.context_window,
        "model_capability":config.turn_identity.model_capability});
    if let Some(dependencies) = &config.turn_identity.host_dependencies {
        policy["host_dependencies"] = dependencies.clone();
    }
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&policy).map_err(|e| e.to_string())?)
    );
    config
        .turn_identity
        .host_invocation
        .as_mut()
        .ok_or("Missing execution binding")?
        .execution_policy_sha256 = digest;
    Ok(())
}

/// Hosts obtain this exact Core-built configuration before preparing an
/// independent child transport/tool executor. No parent Turn grants transfer.
pub(crate) fn child_config<R: crate::repositories::AgentRuntimeCommandRepositories>(
    store: &R,
    task_id: &str,
) -> Result<AgentCoreConfig, String> {
    let task = store
        .runtime_task(task_id)
        .map_err(|e| e.to_string())?
        .ok_or("Task missing")?;
    let metadata: serde_json::Value =
        serde_json::from_str(&task.metadata_json).map_err(|e| e.to_string())?;
    let snapshot: DelegatedInvocationSnapshot = serde_json::from_value(
        metadata
            .get("delegated_invocation")
            .ok_or("Frozen delegated invocation missing")?
            .clone(),
    )
    .map_err(|e| e.to_string())?;
    if snapshot.worktree.is_some() || snapshot.sandbox_requirement.is_some() || snapshot.process_sandbox.is_some() {
        return Err("This local mobile Host cannot execute a desktop worktree or required native sandbox; use its original desktop Host".into());
    }
    let origin = metadata
        .get("delegation_origin")
        .ok_or("Exact delegation origin missing")?;
    let parent_run = origin
        .get("parent_run_id")
        .and_then(|v| v.as_str())
        .ok_or("Parent Run missing")?;
    let frozen = store
        .load_run_invocation_config(parent_run)
        .map_err(|e| e.to_string())?
        .ok_or("Frozen parent missing")?;
    if format!(
        "{:x}",
        Sha256::digest(frozen.invocation_config_json.as_bytes())
    ) != frozen.invocation_config_digest
    {
        return Err("Frozen parent digest mismatch".into());
    }
    let mut config: AgentCoreConfig =
        serde_json::from_str(&frozen.invocation_config_json).map_err(|e| e.to_string())?;
    let parent = config
        .turn_identity
        .delegation
        .as_ref()
        .ok_or("Parent delegation missing")?
        .snapshot()?;
    if snapshot.model.endpoint != parent.model.endpoint {
        return Err("Embedded child requires a separately authorized connection".into());
    }
    // Preparation has verified equal connection/capability and generation facts.
    // Only the model identity changes; credential/query digests and policy do not.
    if snapshot.profile.model.is_some() {
        let binding = config.turn_identity.host_invocation.as_mut().ok_or("Missing child model binding")?;
        let nola_host_protocol::HostModelRouteBinding::Http(route) = &mut binding.primary else {
            return Err("Independent embedded model requires HTTP".into());
        };
        route.model = snapshot.model.model.clone();
        binding.output_admission.routes[0].route = binding.primary.clone();
        binding.validate().map_err(str::to_owned)?;
        config.model = Some(snapshot.model.model.clone());
    } else if snapshot.model.model != parent.model.model {
        return Err("Inherited child model identity changed".into());
    }
    let child = task
        .child_session_id
        .as_deref()
        .ok_or("Child identity missing")?;
    config.session_id = Some(SessionId::from_string(child.into()));
    config.initial_items.retain(|item| {
        matches!(
            item,
            nola_agent_engine::agent::AgentItem::Message {
                role: nola_agent_engine::agent::AgentRole::System,
                ..
            }
        )
    });
    config
        .initial_items
        .push(nola_agent_engine::agent::AgentItem::Message {
            id: nola_agent_engine::agent::ItemId::from_string(format!("{task_id}:profile")),
            role: nola_agent_engine::agent::AgentRole::System,
            content: format!("{}\n\n{}", snapshot.profile.prompt, nola_domain::delegation::WORKSPACE_GUIDANCE),
        });
    config.max_model_iterations = metadata.get("max_turns").and_then(|value| value.as_u64())
        .map(|value| value as usize).or(snapshot.profile.max_turns).unwrap_or(parent.max_iterations);
    config.turn_identity.delegation = None;
    config.turn_identity.remote_tool_plan = None;
    config.permission_rules.rules.retain(|rule| {
        !matches!(
            rule.source,
            nola_domain::PermissionRuleSource::Conversation
                | nola_domain::PermissionRuleSource::Turn
        )
    });
    for rule in &snapshot.profile.permission {
        use nola_domain::execution_policy::ConversationPermissionAction;
        // A child profile may further restrict inherited policy; it cannot mint
        // a new Allow grant absent from the parent's frozen authority.
        if rule.action == ConversationPermissionAction::Allow {
            continue;
        }
        config
            .permission_rules
            .rules
            .push(nola_domain::PermissionRule {
                capability_pattern: rule.permission.clone(),
                resource_pattern:
                    nola_agent_engine::agent::permissions::permission_resource_pattern_for_rule(
                        &rule.permission,
                        &rule.pattern,
                    ),
                decision: if rule.action == ConversationPermissionAction::Deny {
                    nola_domain::PermissionPolicyDecision::Deny
                } else {
                    nola_domain::PermissionPolicyDecision::Ask
                },
                source: nola_domain::PermissionRuleSource::InvocationPolicy,
            });
    }
    let read_only = metadata
        .get("requested_read_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let allowed = |name: &str| {
        !matches!(
            name,
            "spawn_agent"
                | "list_tasks"
                | "read_task"
                | "wait_task"
                | "stop_task"
                | "send_agent_message"
                | "request_user_input"
        ) && (snapshot.profile.tools.is_empty()
            || snapshot.profile.tools.iter().any(|tool| tool == name))
            && (!read_only || parent.read_only_tools.contains(name))
    };
    config.tools.retain(|value| {
        value
            .pointer("/function/name")
            .and_then(|v| v.as_str())
            .is_some_and(allowed)
    });
    if let Some(catalog) = config.host_catalog.as_mut() {
        catalog.capabilities.retain(|name, value| {
            matches!(
                value.exposure,
                HostCapabilityExposure::Internal | HostCapabilityExposure::Hidden
            ) || allowed(name)
        });
    }
    bind_policy(&mut config)?;
    Ok(config)
}

pub(crate) fn validated_child_config<R: crate::repositories::AgentRuntimeCommandRepositories>(
    store: &R,
    task_id: &str,
    candidate: AgentCoreConfig,
) -> Result<AgentCoreConfig, String> {
    let expected = child_config(store, task_id)?;
    if serde_json::to_value(&expected).map_err(|e| e.to_string())?
        != serde_json::to_value(&candidate).map_err(|e| e.to_string())?
    {
        return Err("Child candidate differs from the Core-frozen execution policy".into());
    }
    Ok(expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> MobileDelegationSettings {
        MobileDelegationSettings {
            project_root: "device".into(),
            independent_model_profiles: false,
            selected_profile: None,
            agents: BTreeMap::new(),
        }
    }
    #[test]
    fn model_without_tool_execution_does_not_advertise_delegation() {
        let config = AgentCoreConfig::default();
        let captured = capture_parent(config, settings()).unwrap();
        assert!(captured.turn_identity.delegation.is_none());
        assert!(captured.tools.is_empty());
    }
    #[test]
    fn tool_executor_requires_actual_host_binding_before_capture() {
        let mut config = AgentCoreConfig::default();
        config.turn_identity.model_capability.native_tool_calls = true;
        assert!(capture_parent(config, settings())
            .unwrap_err()
            .contains("bound Host invocation"));
    }
    #[test]
    fn policy_identity_changes_with_concrete_child_tool_surface() {
        // Actual bound route fixture also verifies the hash builder leaves
        // model/output admission untouched while changing only tool policy.
        let mut config = AgentCoreConfig::default();
        let binding = crate::run_recovery_port_tests::recovery_host_binding("fixture");
        config.turn_identity.host_invocation = Some(binding.clone());
        bind_policy(&mut config).unwrap();
        let before = config
            .turn_identity
            .host_invocation
            .as_ref()
            .unwrap()
            .clone();
        config.tools.push(json!({"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}}));
        bind_policy(&mut config).unwrap();
        let after = config.turn_identity.host_invocation.unwrap();
        assert_ne!(
            before.execution_policy_sha256,
            after.execution_policy_sha256
        );
        assert_eq!(after.primary, binding.primary);
        assert_eq!(after.generation, binding.generation);
        assert_eq!(after.output_admission, binding.output_admission);
    }
    #[test]
    fn embedded_catalog_only_offers_executable_profiles_and_explicit_budget() {
        let mut config = crate::user_input_wait_tests::config();
        config.model = Some("fixture".into());
        config.turn_identity.model_capability.native_tool_calls = true;
        config.turn_identity.host_invocation = Some(crate::run_recovery_port_tests::recovery_host_binding("fixture"));
        let mut settings = settings();
        settings.agents.insert("explore".into(), nola_domain::execution_policy::ConversationAgentPolicy {
            model: Some(nola_domain::run_generation_intent::RunGenerationIntent {
                profile_id: "independent".into(), expected_revision: 1,
                tier: nola_domain::reasoning_tiers::ReasoningTier::Balanced, overrides: Default::default(),
            }), ..Default::default()
        });
        let captured = capture_parent(config, settings).unwrap();
        let spawn = captured.tools.iter().find(|tool| tool["function"]["name"] == "spawn_agent").unwrap();
        let properties=&spawn["function"]["parameters"]["properties"];
        let offered=properties["agent_type"]["enum"].as_array().unwrap();
        assert!(!offered.contains(&json!("explore")));
        assert!(offered.contains(&json!("general")));
        assert_eq!(properties["max_turns"]["maximum"],20);
        assert!(properties["max_turns"].get("default").is_none());
        assert_eq!(properties["prompt"]["description"], nola_domain::delegation::WORKSPACE_GUIDANCE);
    }

    #[test]
    fn independent_model_requires_same_connection_and_frozen_generation() {
        use nola_store_port::model::StoredModelProfile;
        let mut store = nola_store::NolaStore::open(":memory:").unwrap();
        let source=json!({"baseURL":"https://fixture.invalid/v1", "wireProtocol":"chat_completions",
            "connectionID":"same", "endpointID":"same", "accountID":"owner", "sourceKind":"api", "capabilities":["chat"],
            "reasoning":null, "executionCapabilities":{"toolCalls":true,"imageInput":false,"samplingParameters":true,"reasoning":false}});
        let parameters=json!({"temperature":0.7,"topP":0.9,"maxOutputTokens":8192,"maxContextLength":64000}).to_string();
        for (id, model, account) in [("parent","fixture","owner"),("child","worker","owner"),("foreign","worker","another-owner")] {
            let mut facts=source.clone(); facts["accountID"]=json!(account); facts["catalogKey"]=json!(id); facts["modelOrder"]=json!(id.len());
            store.upsert_model_profile(&StoredModelProfile { id:id.into(),display_name:id.into(),provider:"openai-compatible".into(),
                model_name:model.into(),runtime:"remote".into(),source_json:facts.to_string(),parameters_json:parameters.clone(),
                selected:false,enabled:true,revision:1,created_at_ms:1,updated_at_ms:1,deleted_at_ms:None }).unwrap();
        }
        let mut base=crate::user_input_wait_tests::config(); base.model=Some("fixture".into());
        base.turn_identity.host_invocation=Some(crate::run_recovery_port_tests::recovery_host_binding("fixture"));
        base.turn_identity.model_capability.native_tool_calls=true;
        let mut settings=settings(); settings.independent_model_profiles=true;
        settings.selected_profile=Some(nola_host_protocol::HostModelProfileReference {profile_id:"parent".into(),admitted_revision:1});
        settings.agents.insert("explore".into(), nola_domain::execution_policy::ConversationAgentPolicy {
            model:Some(nola_domain::run_generation_intent::RunGenerationIntent {profile_id:"child".into(),expected_revision:1,
                tier:nola_domain::reasoning_tiers::ReasoningTier::Balanced,overrides:Default::default()}),..Default::default() });
        let frozen=capture_parent(base,settings).unwrap();
        let mut parent=frozen.turn_identity.delegation.unwrap().snapshot().unwrap();
        // Match the parent Host's prepared generation facts, not new defaults.
        parent.runtime.temperature=Some(0.7);parent.runtime.top_p=Some(0.9);parent.runtime.max_output_tokens=Some(8192);
        parent.runtime.sampling_override=true;
        let child=crate::delegation_preparation::capture_delegated_invocation(&mut store,&parent,"owner","explore").unwrap();
        validate_profile_model(&mut store,&parent,&child).unwrap();
        assert_eq!(child.model.model,"worker");assert_eq!(child.model.endpoint,parent.model.endpoint);
        let mut changed=child.clone();changed.runtime.max_output_tokens=Some(16384);
        assert!(validate_profile_model(&mut store,&parent,&changed).is_err());
        parent.profiles.get_mut("explore").unwrap().model.as_mut().unwrap().profile_id="foreign".into();
        let foreign=crate::delegation_preparation::capture_delegated_invocation(&mut store,&parent,"owner","explore").unwrap();
        assert!(validate_profile_model(&mut store,&parent,&foreign).is_err());
        assert!(store.list_all_runtime_tasks().unwrap().is_empty(),"Model preflight creates no work or grants");
    }

}
