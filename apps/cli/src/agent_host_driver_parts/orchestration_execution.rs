// SPDX-License-Identifier: Apache-2.0
// Mechanical extraction: runtime implementation in the original module scope.

#[cfg(all(test, unix))]
mod plugin_driver_tests {
    use super::*;

    #[test]
    fn owned_plugin_catalog_is_discoverable_and_revoked_without_erasing_manual_servers() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("server.py"), r#"#!/usr/bin/env python3
import json, sys
for line in sys.stdin:
    r = json.loads(line)
    if 'id' not in r: continue
    m = r.get('method')
    if m == 'initialize': result = {'protocolVersion':'2025-06-18','capabilities':{}}
    elif m == 'tools/list': result = {'tools':[{'name':'echo','inputSchema':{'type':'object'}}]}
    else: result = {'content':[]}
    print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':result}), flush=True)
"#).unwrap();
        std::fs::set_permissions(source.join("server.py"), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(source.join("nola-plugin.json"), json!({
            "schema_version":1,"id":"driver-fixture","version":"1","display_name":"Fixture","description":"",
            "mcp_servers":[{"id":"echo","transport":{"type":"stdio",
                "executable":{"type":"package","path":"./server.py"}}}]
        }).to_string()).unwrap();
        let package = nola_plugin_host::inspect(&source).unwrap();
        let store = root.path().join("packages");
        package.publish(&store).unwrap();
        let record = nola_domain::plugin::PluginInstallation {
            manifest: package.manifest().clone(), package_digest: package.digest().into(),
            enabled: true, revision: 2, installed_at_ms: 0, updated_at_ms: 1, removed_at_ms: None,
        };
        let prepared = nola_plugin_host::prepare_runtime(&store, &record,
            |_| panic!("no secrets"), |_| panic!("packaged executable")).unwrap();
        let current = std::sync::Arc::new(std::sync::Mutex::new(Some(record)));
        let reader = current.clone();
        let session = crate::plugin_runtime::PluginMcpSession::new(
            vec![crate::plugin_runtime::PluginMcpRuntime::register(prepared).unwrap()],
            move |_| Ok(reader.lock().unwrap().clone()));
        let context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(vec![])),
            allowed_dirs: vec![],
            policy: crate::tools::ToolExecutionPolicy::new(vec![], false, "deny".into(), vec![]),
            permission_scope: Default::default(), cancellation: Default::default(), plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: crate::tools::BrowserRuntime::new(root.path().join("browser")),
        };
        let manual: McpServerDescriptor = serde_json::from_value(json!({
            "server_id":"manual", "display_name":"Manual", "transport":"stdio",
            "connection_ref":"manual-fixture", "availability":"available"
        })).unwrap();
        let mut driver = CliAgentHostDriver::new("plugin-driver-test",
            effect_delivery_start_barrier_tests::test_llm("http://127.0.0.1:1".into()), context)
            .with_boundary_session_id("plugin-driver-session")
            .with_mcp_capability_catalog(McpCapabilityCatalog {
                revision: 1, servers: BTreeMap::from([("manual".into(), manual)]),
            }).with_plugin_mcp_session(session.into()).unwrap();
        let existing_mcp = driver.capability_catalog.mcp.clone();
        driver = driver.with_code_mode_limits(nola_domain::code_mode::CodeCellLimits::default()).unwrap();
        assert_eq!(driver.capability_catalog.mcp, existing_mcp,
            "Code Mode configuration must preserve previously registered MCP providers");
        let code_descriptors = ["code_mode_exec", "code_mode_wait"].map(|id|
            driver.catalog.capabilities.get(id).unwrap().clone());
        assert!(driver.capability_catalog.builtin_tools.contains("code_mode_exec"));
        assert!(driver.capability_catalog.builtin_tools.contains("code_mode_wait"));
        let id = mcp_model_tool_id("plugin:driver-fixture:mcp:echo", "echo");
        assert!(driver.catalog.resolve_available(&id).is_none(),"configuration alone cannot expose an executable remote tool");
        assert!(driver.catalog.resolve_available(crate::mcp_connection_authority::CONNECT_CAPABILITY).is_some());
        let reference = driver.mcp_server_registry[0]["connection_ref"].as_str().unwrap();
        let expected = crate::mcp_connection_authority::permission_resource(reference).unwrap();
        crate::mcp_host::connect_approved(reference,&expected).unwrap();
        driver.refresh_mcp_capability_catalog().unwrap();
        assert!(driver.catalog.resolve_available(&id).is_some());
        for descriptor in &code_descriptors {
            assert_eq!(driver.catalog.capabilities.get(&descriptor.id), Some(descriptor),
                "MCP refresh must preserve frozen Code Mode schemas and budgets");
        }
        let issued_revision = driver.catalog.revision;
        let issued_epoch = driver.catalog.availability_epoch;
        assert!(driver.accepts_catalog_revision(issued_revision, issued_epoch, false));
        assert!(!driver.accepts_catalog_revision(issued_revision, issued_epoch.wrapping_add(1), false));
        assert!(!driver.accepts_catalog_revision(issued_revision, issued_epoch.wrapping_add(1), true));
        driver.refresh_mcp_capability_catalog().unwrap();
        assert!(driver.accepts_catalog_revision(issued_revision, issued_epoch, true));
        assert!(!driver.accepts_catalog_revision(issued_revision, issued_epoch, false));
        assert!(!driver.accepts_catalog_revision(u64::MAX, issued_epoch, true));
        let frozen = driver.catalog.clone();
        driver.retained_model_catalogs.clear();
        driver.restore_catalog_fence(&frozen, [(issued_revision, issued_epoch)]).unwrap();
        assert!(driver.accepts_catalog_revision(issued_revision, issued_epoch, true));
        assert!(!driver.accepts_catalog_revision(issued_revision, issued_epoch, false));
        let mut changed = frozen.clone();
        changed.capabilities.remove(&id);
        assert!(driver.restore_catalog_fence(&changed, []).is_err(),
            "cold restoration must reject executable capability drift");
        assert_eq!(driver.catalog, frozen);
        assert_eq!(driver.mcp_server_registry.len(),1,"plugin configuration stays in the Host-owned runtime registry");
        assert!(driver.mcp_server_registry[0].get("_nola_plugin_registration_ref").is_some());
        *current.lock().unwrap() = None;
        let previously_committed = driver.catalog.clone();
        let active_host_revision = driver.catalog.revision;
        driver.reconcile_plugin_mcp().unwrap();
        assert_eq!(driver.catalog.revision, active_host_revision,
            "local revocation must not invalidate unrelated in-flight Core requests");
        assert!(driver.catalog.resolve_available(&id).is_none());
        assert!(!driver.mcp_tool_routes.contains_key(&id));
        driver.adopt_committed_catalog(previously_committed).unwrap();
        assert!(driver.catalog.resolve_available(&id).is_none(), "committed snapshots cannot resurrect revoked plugin tools");
        driver.refresh_mcp_capability_catalog().unwrap();
        assert!(driver.capability_catalog.mcp.servers.contains_key("manual"));
        assert_eq!(driver.capability_catalog.mcp.servers.len(), 1);
    }
}

impl CliAgentHostDriver {
    fn model_target_key(provider: Option<&str>, endpoint: &str, model: &str) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            provider.unwrap_or_default().trim().to_ascii_lowercase(),
            endpoint.trim().trim_end_matches('/').to_ascii_lowercase(),
            model.trim()
        )
    }

    fn model_target_keys(llm: &LlmConfig) -> Vec<String> {
        std::iter::once(Self::model_target_key(
            llm.provider.as_deref(),
            &llm.endpoint,
            &llm.model,
        ))
        .chain(llm.fallbacks.iter().map(|target| {
            Self::model_target_key(
                target.provider.as_deref(),
                &target.endpoint,
                target.model.as_deref().unwrap_or(&llm.model),
            )
        }))
        .collect()
    }

    fn omit_unavailable_model_targets(&self, llm: &mut LlmConfig) {
        if self.current_turn_unavailable_model_targets.is_empty() {
            return;
        }
        let mut targets = std::mem::take(&mut llm.fallbacks);
        let primary_key =
            Self::model_target_key(llm.provider.as_deref(), &llm.endpoint, &llm.model);
        if self
            .current_turn_unavailable_model_targets
            .contains(&primary_key)
        {
            if let Some(index) = targets.iter().position(|target| {
                !self
                    .current_turn_unavailable_model_targets
                    .contains(&Self::model_target_key(
                        target.provider.as_deref(),
                        &target.endpoint,
                        target.model.as_deref().unwrap_or(&llm.model),
                    ))
            }) {
                let promoted = targets.remove(index);
                llm.native_model_route = None;
                llm.provider = promoted.provider;
                llm.endpoint = promoted.endpoint;
                if let Some(api_key) = promoted.api_key {
                    llm.api_key = api_key;
                }
                if let Some(model) = promoted.model {
                    llm.model = model;
                }
            }
        }
        llm.fallbacks = targets
            .into_iter()
            .filter(|target| {
                !self
                    .current_turn_unavailable_model_targets
                    .contains(&Self::model_target_key(
                        target.provider.as_deref(),
                        &target.endpoint,
                        target.model.as_deref().unwrap_or(&llm.model),
                    ))
            })
            .collect();
    }

    fn materialize_capability_activation(
        &self,
        capability_id: &str,
        output: &Value,
        mut activation: Option<nola_core::agent::host::HostCapabilityActivation>,
    ) -> Option<nola_core::agent::host::HostCapabilityActivation> {
        if activation.is_none() && matches!(capability_id, "Skill" | "skill_invoke") {
            let capability_ids = serde_json::from_value::<
                nola_core::agent::capability::MaterializedSkillInvocation,
            >(output.clone())
            .ok()
            .map(|invocation| {
                invocation
                    .required_tools
                    .iter()
                    .map(|reference| reference.model_capability_id())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
            activation = (!capability_ids.is_empty()).then_some(
                nola_core::agent::host::HostCapabilityActivation {
                    source: "skill".to_string(),
                    capability_ids,
                    capability_descriptors: Vec::new(),
                },
            );
        }
        activation.map(|mut activation| {
            activation.capability_descriptors = activation
                .capability_ids
                .iter()
                .filter_map(|capability_id| self.catalog.capabilities.get(capability_id))
                .filter_map(|descriptor| serde_json::to_value(descriptor).ok())
                .collect();
            activation
        })
    }

    /// Requests cooperative cancellation for external operations that still
    /// own a Host-effect delivery when Application rejects lease renewal.
    pub fn effect_delivery_abort_handle(&self) -> std::sync::Arc<dyn Fn(&str) + Send + Sync> {
        let registry = std::sync::Arc::clone(&self.active_effect_cancellations);
        std::sync::Arc::new(move |effect_id| {
            if let Ok(cancellations) = registry.lock() {
                if let Some(cancellation) = cancellations.get(effect_id) {
                    cancellation.request();
                }
            }
        })
    }

    fn insert_delegated_agent_session(
        &mut self,
        effect_id: String,
        session: DelegatedAgentWorkerSession,
    ) {
        if let Ok(mut cancellations) = self.active_effect_cancellations.lock() {
            cancellations.insert(effect_id.clone(), session.cancellation.context());
        }
        self.delegated_agent_sessions.insert(effect_id, session);
    }

    pub fn new(host_id: impl Into<String>, llm: LlmConfig, tool_context: ToolContext) -> Self {
        let catalog = build_cli_host_catalog_with_skills(host_id, &tool_context.skills);
        let capability_catalog = build_cli_capability_catalog(&catalog);
        let base_allowed_dirs = tool_context.allowed_dirs.clone();
        Self {
            ui_publisher: None,
            catalog,
            capability_catalog,
            boundary_session_id: None,
            bound_dispatch_run_id: None,
            boundary_submission_state: Default::default(),
            mcp_tool_routes: BTreeMap::new(),
            mcp_server_registry: Vec::new(),
            plugin_mcp_session: None,
            retained_model_catalogs: BTreeMap::new(),
            revoked_plugin_capabilities: HashSet::new(),
            code_intelligence_provider: None,
            code_memory_workspace: None,
            current_turn_code_memory_assessment: None,
            code_memory_initialization: Default::default(),
            llm,
            compaction_request_witness: None,
            tool_context,
            permission_handler: Box::new(|_, _, _, _, _| Ok(HostPermissionResolution::deny())),
            context_handler: Box::new(|_, source_id, _| {
                Err(format!("context source is unavailable: {source_id}"))
            }),
            runtime_tool_replay_handler: Box::new(|_, _, _, _, _| None),
            runtime_tool_handler: Box::new(|_, _, _, _, _| None),
            runtime_tool_ids: HashSet::new(),
            configured_tool_allowlist: None,
            mcp_extension_policy: None,
            turn_policy: HostTurnPolicy::default(),
            current_turn_project_overview_file_count: None,
            current_turn_project_overview_attempted: false,
            current_turn_skill_decided: false,
            current_turn_deterministic_tool_failures: BTreeMap::new(),
            current_turn_available_tool_ids: HashSet::new(),
            current_turn_instruction_contexts: HashSet::new(),
            current_turn_unavailable_model_targets: HashSet::new(),
            lifecycle: LifecycleHookExecutor::default(),
            active_effect_cancellations: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            execution_settings: None,
            delegated_agent_sessions: BTreeMap::new(),
            code_cells: BTreeMap::new(),
            code_mode_limits: Default::default(),
            started_effect_deliveries: HashSet::new(),
            cancellation_stop_validator: crate::core_agent_runtime::validate_local_cancellation_stop,
            quiescent_effects: BTreeMap::new(),
            base_allowed_dirs,
        }
    }

    pub(crate) fn with_execution_settings(mut self, config: HostExecutionSettings) -> Self {
        self.set_execution_settings(config);
        self
    }

    /// Binds durable boundary submissions to the canonical Core session.
    /// Permission scopes are authorization state and must not be used as the
    /// routing identity for queued user or Agent input.
    pub fn with_boundary_session_id(mut self, session_id: impl Into<String>) -> Self {
        let session_id = session_id.into();
        self.tool_context.memory_binding.conversation_id = Some(session_id.clone());
        self.boundary_session_id = Some(session_id);
        self
    }

    pub(crate) fn set_execution_settings(&mut self, config: HostExecutionSettings) {
        self.tool_context.memory_binding.workspace_id = config.memory_workspace_id.clone();
        apply_initial_tool_tier(&mut self.catalog, config.reasoning_tier);
        let previously_native = self.execution_settings.as_ref()
            .is_some_and(|config| config.native_model_route);
        if let Some(capability) = self.catalog.capabilities.get_mut("spawn_agent") {
            if config.native_model_route {
                capability.availability = CapabilityAvailability::RequiresConnection;
            } else if previously_native && capability.availability == CapabilityAvailability::RequiresConnection {
                capability.availability = CapabilityAvailability::Available;
            }
        }
        if config.native_model_route {
            self.capability_catalog.builtin_tools.remove("spawn_agent");
        } else if previously_native {
            self.capability_catalog.builtin_tools.insert("spawn_agent".into());
        }
        self.execution_settings = Some(config);
    }

    pub fn catalog(&self) -> &HostCapabilityCatalog {
        &self.catalog
    }

    /// Adopts durable Host ownership while retaining the process-local
    /// discovery registry. Checkpoints contain the bootstrap and actually
    /// activated subset; replacing the runtime registry with that projection
    /// would make later typed tool searches forget all optional tools.
    pub fn adopt_committed_catalog(
        &mut self,
        catalog: HostCapabilityCatalog,
    ) -> Result<(), String> {
        self.catalog = self.prepare_committed_catalog(catalog)?;
        Ok(())
    }

    /// Preview the complete native registry without changing live ownership.
    /// The runtime validates its candidate checkpoint before committing this.
    pub fn prepare_committed_catalog(
        &self,
        catalog: HostCapabilityCatalog,
    ) -> Result<HostCapabilityCatalog, String> {
        if catalog.host_id != self.catalog.host_id {
            return Err("committed Host catalog belongs to a different Host".to_string());
        }
        let mut candidate = self.catalog.clone();
        candidate.revision = catalog.revision;
        candidate.availability_epoch = catalog.availability_epoch;
        candidate.lease = catalog.lease;
        candidate.capabilities.extend(catalog.capabilities);
        candidate.capabilities.retain(|id, _| !self.revoked_plugin_capabilities.contains(id));
        if self.execution_settings.as_ref().is_some_and(|config| config.native_model_route) {
            if let Some(capability) = candidate.capabilities.get_mut("spawn_agent") {
                capability.availability = CapabilityAvailability::RequiresConnection;
            }
        }
        Ok(candidate)
    }

    pub fn with_capability_catalog(mut self, catalog: CapabilityCatalog) -> Self {
        self.capability_catalog = catalog;
        self
    }

    /// Installs Nola's native code-intelligence surface directly in the Host
    /// catalog. It deliberately creates no MCP server, connection reference,
    /// Supervisor lease, or subprocess lifecycle.
    pub fn with_code_intelligence_provider(
        mut self,
        provider: crate::code_intelligence_provider::SharedCodeIntelligenceProvider,
    ) -> Self {
        for spec in crate::code_intelligence_provider::tool_specs() {
            let exposure = initial_tool_exposure_for_tier(&spec.capability_id,
                self.execution_settings.as_ref()
                    .map(|config| config.reasoning_tier).unwrap_or_default());
            let operation =
                nola_core::code_intelligence::operation_for_capability_id(&spec.capability_id)
                    .expect("native code-intelligence specs use Core-owned capability IDs");
            let effect = code_intelligence_host_effect(operation);
            let mutates_user_state =
                matches!(effect, nola_core::agent::host::HostCapabilityEffect::Mutate);
            let mut input_schema = spec.input_schema;
            if let (Some(schema), Some(description)) = (input_schema.as_object_mut(), spec.description) {
                schema.insert("description".into(), Value::String(description));
            }
            self.catalog.capabilities.insert(
                spec.capability_id.clone(),
                HostCapabilityDescriptor {
                    id: spec.capability_id.clone(),
                    title: spec.title,
                    route: HostCapabilityRoute::Tool,
                    availability: CapabilityAvailability::Available,
                    exposure,
                    permission_domain: mutates_user_state
                        .then(|| "code_intelligence_mutation".to_string()),
                    input_schema,
                    input_normalization: HostInputNormalization {
                        origin: nola_core::agent::host::HostCapabilityOrigin {
                            kind: nola_core::agent::host::HostCapabilityOriginKind::Builtin,
                            provider_id: Some(
                                nola_core::code_intelligence::CODEBASE_MEMORY_PROVIDER_ID
                                    .to_string(),
                            ),
                            provider_title: Some("Nola Code Intelligence".to_string()),
                            namespace: Some("code_intelligence".to_string()),
                        },
                        effect: nola_core::agent::host::HostCapabilityEffectDeclaration::fixed(
                            effect,
                        ),
                        permission_resource:
                            nola_core::agent::host::HostPermissionResourceDeclaration::FullInput,
                        permission_contract: crate::tools::builtin_permission_contract(
                            &spec.capability_id, !mutates_user_state,
                        ),
                        ..Default::default()
                    },
                    output_policy: HostOutputPolicy {
                        preview_char_limit: 8_000,
                        persist_full_output: true,
                        allow_artifacts: true,
                    },
                    supports_streaming: false,
                    supports_argument_streaming: false,
                    parallelism: if spec.read_only {
                        HostCapabilityParallelism::ConcurrentSafe
                    } else {
                        HostCapabilityParallelism::Serial
                    },
                    cancellation: if spec.read_only && provider.process_bound()
                        && self.lifecycle.lifecycle_hook_rules.is_empty() {
                        HostCancellationMode::ProcessBound
                    } else { HostCancellationMode::BestEffort },
                    replay_safety: if !mutates_user_state {
                        HostReplaySafety::Idempotent
                    } else {
                        HostReplaySafety::RequiresRecoveryDecision
                    },
                },
            );
        }
        self.code_intelligence_provider = Some(provider);
        self
    }

    pub fn with_code_memory_workspace(mut self, root: PathBuf) -> Self {
        self.code_memory_workspace = Some(root);
        self
    }

    pub fn with_lifecycle_hook_registry<F>(
        mut self,
        registry: nola_core::lifecycle_hooks::LifecycleHookRegistrySnapshot,
        capabilities: Vec<nola_core::lifecycle_hooks::LifecycleHookRequiredCapability>,
        handler: F,
    ) -> Self
    where
        F: FnMut(
                &nola_core::lifecycle_hooks::LifecycleHookInvocation,
                &nola_core::lifecycle_hooks::LifecycleHookEvent,
            ) -> Result<HostLifecycleHookResult, String>
            + Send
            + 'static,
    {
        self.lifecycle.lifecycle_hook_registry_revision_sha256 = Some(registry.revision_sha256.clone());
        if !registry.rules.is_empty() {
            for capability in self.catalog.capabilities.values_mut() {
                if capability.cancellation == HostCancellationMode::ProcessBound {
                    capability.cancellation = HostCancellationMode::BestEffort;
                }
            }
        }
        self.lifecycle.lifecycle_hook_rules = registry.rules;
        self.lifecycle.lifecycle_hook_capabilities = capabilities;
        self.lifecycle.lifecycle_hook_handler = Some(std::sync::Arc::new(std::sync::Mutex::new(Box::new(
            handler,
        ))));
        self
    }

    pub fn with_mcp_server_registry(mut self, registry: Vec<Value>) -> Result<Self,String> {
        self.install_owned_mcp_registry(registry)?;
        Ok(self)
    }

    pub fn with_plugin_mcp_session(
        mut self,
        session: crate::plugin_runtime::SharedPluginSession,
    ) -> Result<Self, String> {
        let mut configured = self.mcp_server_registry.clone();
        configured.extend(session.configurations()?);
        self.install_owned_mcp_registry(configured)?;
        session.bind_connections(&self.mcp_server_registry)?;
        self.plugin_mcp_session = Some(session);
        Ok(self)
    }

    fn reconcile_plugin_mcp(&mut self) -> Result<(), String> {
        let Some(session) = self.plugin_mcp_session.as_mut() else { return Ok(()); };
        let result = session.reconcile();
        for (id, route) in &self.mcp_tool_routes {
            if session.contains_connection(&route.connection_ref)
                && !session.connection_is_active(&route.connection_ref) {
                self.revoked_plugin_capabilities.insert(id.clone());
            }
        }
        let mut catalog = self.capability_catalog.mcp.clone();
        catalog.servers.retain(|_, server| !session.contains_connection(&server.connection_ref)
            || session.connection_is_active(&server.connection_ref));
        self.mcp_server_registry.retain(|value|value["connection_ref"].as_str().is_none_or(|reference|
            !session.contains_connection(reference) || session.connection_is_active(reference)));
        if catalog.servers.len() != self.capability_catalog.mcp.servers.len() {
            catalog.revision = self.capability_catalog.revision.max(catalog.revision).saturating_add(1);
            let active_host_revision = self.catalog.revision;
            self.install_mcp_capability_catalog(catalog);
            // Local revocation can deny stale calls immediately, but cannot
            // unilaterally change the fence on every other pending request.
            // Only a correlated catalog refresh advances Core's Host revision.
            self.catalog.revision = active_host_revision;
        }
        result
    }

    pub fn with_mcp_capability_catalog(mut self, catalog: McpCapabilityCatalog) -> Self {
        self.install_mcp_capability_catalog(catalog);
        self
    }

    fn install_mcp_capability_catalog(&mut self, catalog: McpCapabilityCatalog) {
        for capability_id in self.mcp_tool_routes.keys() {
            self.catalog.capabilities.remove(capability_id);
        }
        self.mcp_tool_routes.clear();
        for capability_id in [
            MCP_RESOURCE_READ_CAPABILITY_ID,
            MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID,
            MCP_PROMPT_GET_CAPABILITY_ID,
        ] {
            self.catalog.capabilities.remove(capability_id);
        }

        for server in catalog
            .servers
            .values()
            .filter(|server| server.availability == McpServerAvailability::Available)
        {
            if server.transport != McpTransportKind::InProcess {
                let owned = crate::mcp_connection_authority::permission_resource(&server.connection_ref).ok();
                if !owned.is_some_and(|resource|resource.host_id == self.catalog.host_id.0
                    && Some(resource.session_id.as_str()) == self.boundary_session_id.as_deref()
                    && resource.generation.is_some()) { continue; }
            }
            for tool in server.tools.values() {
                let capability_id = nola_core::code_intelligence::stable_capability_id(
                    &server.server_id,
                    &tool.name,
                )
                .map(str::to_string)
                .unwrap_or_else(|| mcp_model_tool_id(&server.server_id, &tool.name));
                let route = McpToolRoute {
                    transport: server.transport,
                    connection_ref: server.connection_ref.clone(),
                    server_id: server.server_id.clone(),
                    server_name: server.display_name.clone(),
                    tool_name: tool.name.clone(),
                };
                if self.catalog.capabilities.contains_key(&capability_id) {
                    continue;
                }
                self.catalog.capabilities.insert(
                    capability_id.clone(),
                    nola_mcp_host::catalog::tool_descriptor(server, tool),
                );
                self.mcp_tool_routes.insert(capability_id, route);
            }
        }
        if catalog.servers.values().any(|server| {
            server.availability == McpServerAvailability::Available && !server.resources.is_empty()
        }) {
            let definition = mcp_resource_read_model_definition();
            self.catalog.capabilities.insert(
                MCP_RESOURCE_READ_CAPABILITY_ID.to_string(),
                mcp_catalog_access_descriptor(
                    MCP_RESOURCE_READ_CAPABILITY_ID,
                    "Read MCP resource",
                    definition["function"]["parameters"].clone(),
                ),
            );
        }
        if catalog.servers.values().any(|server| {
            server.availability == McpServerAvailability::Available
                && !server.resource_templates.is_empty()
        }) {
            let definition = mcp_resource_template_read_model_definition();
            self.catalog.capabilities.insert(
                MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID.to_string(),
                mcp_catalog_access_descriptor(
                    MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID,
                    "Read MCP resource template",
                    definition["function"]["parameters"].clone(),
                ),
            );
        }
        if catalog.servers.values().any(|server| {
            server.availability == McpServerAvailability::Available && !server.prompts.is_empty()
        }) {
            let definition = mcp_prompt_get_model_definition();
            self.catalog.capabilities.insert(
                MCP_PROMPT_GET_CAPABILITY_ID.to_string(),
                mcp_catalog_access_descriptor(
                    MCP_PROMPT_GET_CAPABILITY_ID,
                    "Get MCP prompt",
                    definition["function"]["parameters"].clone(),
                ),
            );
        }
        self.catalog.revision = self.catalog.revision.max(catalog.revision);
        self.capability_catalog.revision = self.capability_catalog.revision.max(catalog.revision);
        self.capability_catalog.mcp = catalog;
    }

    fn refresh_mcp_capability_catalog(&mut self) -> Result<CapabilityCatalog, String> {
        self.reconcile_plugin_mcp()?;
        if self.mcp_server_registry.is_empty() && self.plugin_mcp_session.is_none() {
            return Ok(self.capability_catalog.clone());
        }
        let discovered = crate::mcp_host::discover_connected_servers(&self.mcp_server_registry)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        let next_revision = self
            .capability_catalog
            .revision
            .max(self.capability_catalog.mcp.revision)
            .saturating_add(1);
        let mut mcp = self.capability_catalog.mcp.clone();
        mcp.revision = next_revision;
        for (id,server) in configured_mcp_capability_catalog(&discovered,next_revision)?.servers {
            mcp.servers.insert(id,server);
        }
        self.mcp_server_registry = discovered;
        if let Some(model) = self.catalog.capabilities.get("model.stream") {
            self.retained_model_catalogs.insert(self.catalog.revision,
                (self.catalog.availability_epoch, model.clone()));
        }
        self.install_mcp_capability_catalog(mcp);
        self.capability_catalog.revision = next_revision;
        Ok(self.capability_catalog.clone())
    }

    fn accepts_catalog_revision(&self, revision: u64, epoch: u64, is_model: bool) -> bool {
        (revision == self.catalog.revision && epoch == self.catalog.availability_epoch)
            || (is_model && self.retained_model_catalogs.get(&revision)
            .is_some_and(|(original_epoch, model)| *original_epoch == epoch
                && epoch == self.catalog.availability_epoch
                && self.catalog.capabilities.get("model.stream") == Some(model)))
    }

    pub(crate) fn restore_catalog_fence(
        &mut self, frozen: &HostCapabilityCatalog,
        pending_models: impl IntoIterator<Item = (u64, u64)>,
    ) -> Result<(), String> {
        self.restore_lazy_mcp_routes(frozen)?;
        if frozen.host_id != self.catalog.host_id || frozen.capabilities != self.catalog.capabilities {
            return Err("Cannot restore a Run with changed executable Host capabilities".into());
        }
        let mut retained = BTreeMap::new();
        if let Some(model) = frozen.capabilities.get("model.stream") {
            for (revision, epoch) in pending_models {
                if revision > frozen.revision || epoch != frozen.availability_epoch {
                    return Err("Pending model request has an incompatible catalog fence".into());
                }
                retained.insert(revision, (epoch, model.clone()));
            }
        }
        self.catalog = frozen.clone();
        self.capability_catalog.revision = frozen.revision;
        self.capability_catalog.mcp.revision = frozen.revision;
        self.retained_model_catalogs = retained;
        Ok(())
    }

    pub(crate) fn foreground_cancellation_requested(&self) -> bool {
        self.tool_context.cancellation.is_requested()
    }

    /// Called with the canonical Run immediately before committed effects are
    /// dispatched. Re-entering the same Run preserves its observations; a
    /// successor gets a fresh read ledger without clearing session approvals.
    pub(crate) fn bind_file_read_run(&mut self, run_id: &str) {
        self.bound_dispatch_run_id = Some(run_id.to_owned());
        self.tool_context.memory_binding.turn_id = Some(run_id.to_owned());
        self.tool_context.permission_scope.bind_file_read_run(run_id);
    }


    /// An independently admitted Run shares the foreground cancellation scope,
    /// not its predecessor's transient grants, failures or provider exclusions.
    pub(crate) fn begin_successor_turn(&mut self) {
        self.begin_turn(self.tool_context.cancellation.clone(), HostTurnPolicy::default());
    }

    pub(crate) fn begin_resumed_turn(&mut self, run_id: &str,
        cancellation: RuntimeExecutionControl, turn_policy: HostTurnPolicy) {
        // Only the same retained Host and exact Run may keep physical stop
        // observations. Cold/new Hosts and successor Runs have no such proof.
        let stopped = if self.bound_dispatch_run_id.as_deref() == Some(run_id) {
            std::mem::take(&mut self.quiescent_effects)
        } else { Default::default() };
        self.begin_turn(cancellation, turn_policy);
        self.quiescent_effects = stopped;
        for witness in self.quiescent_effects.values_mut() { witness.requires_current_target_validation = true; }
        self.bind_file_read_run(run_id);
    }

    pub fn begin_turn(&mut self, cancellation: RuntimeExecutionControl, turn_policy: HostTurnPolicy) {
        self.bound_dispatch_run_id = None;
        self.quiescent_effects.clear();
        self.retained_model_catalogs.clear();
        self.boundary_submission_state = Default::default();
        crate::tools::reset_current_host_permission_session_block();
        self.llm.cancellation = cancellation.context();
        self.tool_context.cancellation = cancellation;
        self.tool_context.allowed_dirs = self.base_allowed_dirs.clone();
        self.current_turn_project_overview_file_count = None;
        self.current_turn_project_overview_attempted = false;
        self.current_turn_skill_decided = false;
        self.lifecycle.current_turn_lifecycle_event_ids.clear();
        self.current_turn_deterministic_tool_failures.clear();
        self.current_turn_available_tool_ids.clear();
        self.current_turn_instruction_contexts.clear();
        self.current_turn_code_memory_assessment = None;
        self.code_memory_initialization = Default::default();
        self.current_turn_unavailable_model_targets.clear();
        if let Ok(mut cancellations) = self.active_effect_cancellations.lock() {
            cancellations.clear();
        }
        self.turn_policy = turn_policy;
    }

    fn refresh_current_turn_available_tool_ids(&mut self) {
        self.current_turn_available_tool_ids = self
            .catalog
            .capabilities
            .iter()
            .filter_map(|(id, capability)| {
                (capability.route == HostCapabilityRoute::Tool
                    && capability.availability == CapabilityAvailability::Available
                    && self.tool_authorized_by_host_policy(id)
                    && matches!(
                        capability.exposure,
                        HostCapabilityExposure::Visible | HostCapabilityExposure::Deferred
                    ))
                .then(|| id.clone())
            })
            .collect();
    }

    /// Returns schemas from the same frozen Host catalog that authorizes
    /// execution. `tool_search` must not depend on the legacy static ToolSpec
    /// table or on a provider-specific registry: native providers, device
    /// bridges, and MCP servers all participate through this one contract.
    fn current_turn_discovery_tool_schemas(&self) -> Vec<Value> {
        self.catalog
            .capabilities
            .values()
            .filter(|capability| {
                capability.route == HostCapabilityRoute::Tool
                    && capability.availability == CapabilityAvailability::Available
                    && capability.exposure.is_model_discoverable()
                    && self
                        .current_turn_available_tool_ids
                        .contains(&capability.id)
            })
            .map(discovery_tool_schema)
            .collect()
    }

    fn model_target_labels(llm: &LlmConfig) -> Vec<String> {
        let label = |provider: Option<&str>, model: &str| {
            let provider = provider
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("openai-compatible");
            format!("{provider}/{model}")
        };
        std::iter::once(label(llm.provider.as_deref(), &llm.model))
            .chain(llm.fallbacks.iter().map(|target| {
                label(
                    target.provider.as_deref().or(llm.provider.as_deref()),
                    target.model.as_deref().unwrap_or(&llm.model),
                )
            }))
            .collect()
    }

    fn model_retry_route(
        llm: &LlmConfig,
        attempt: usize,
        target_exhausted: bool,
        failure_message: &str,
    ) -> String {
        if !target_exhausted {
            return format!("{failure_message} · #{}", attempt.saturating_add(1));
        }
        let failed_index = attempt.saturating_sub(2);
        let labels = Self::model_target_labels(llm);
        match (
            labels.get(failed_index),
            labels.get(failed_index.saturating_add(1)),
        ) {
            (Some(failed), Some(next)) => {
                format!("{failed} → {next} · {failure_message}")
            }
            _ => failure_message.to_owned(),
        }
    }

    pub fn with_permission_handler(
        mut self,
        handler: impl FnMut(
                &HostRequestEnvelope,
                &str,
                &str,
                &str,
                &Value,
            ) -> Result<HostPermissionResolution, PermissionHandlerError>
            + Send
            + 'static,
    ) -> Self {
        self.permission_handler = Box::new(handler);
        self
    }

    pub fn with_runtime_tool_handler(
        mut self,
        handler: impl FnMut(
                &HostRequestEnvelope,
                &str,
                &str,
                &serde_json::Map<String, Value>,
                &ToolContext,
            ) -> Option<RuntimeToolResult>
            + Send
            + 'static,
    ) -> Self {
        self.runtime_tool_handler = Box::new(handler);
        self
    }

    pub fn with_runtime_tool_replay_handler(
        mut self,
        handler: impl FnMut(
                &HostRequestEnvelope,
                &str,
                &str,
                &serde_json::Map<String, Value>,
                &ToolContext,
            ) -> Option<RuntimeTaskUpdate>
            + Send
            + 'static,
    ) -> Self {
        self.runtime_tool_replay_handler = Box::new(handler);
        self
    }

    pub fn with_runtime_tool_ids(mut self, ids: impl IntoIterator<Item = String>) -> Self {
        self.runtime_tool_ids = ids.into_iter().collect();
        self
    }

    pub fn with_tool_allowlist(mut self, ids: Option<impl IntoIterator<Item = String>>) -> Self {
        self.configure_tool_allowlist(ids, true);
        self
    }

    /// Applies an execution authorization filter without changing the Host's
    /// Visible/Deferred presentation contract. Desktop policy receipts use
    /// this path: being allowed to execute a capability must not force its
    /// entire schema into every model request.
    pub fn with_authorized_tool_allowlist(
        mut self,
        ids: Option<impl IntoIterator<Item = String>>,
    ) -> Self {
        self.configure_tool_allowlist(ids, false);
        self
    }

    pub fn with_mcp_extension_policy(mut self, policy: Option<nola_core::tool_policy::ToolPolicyInput>) -> Self {
        self.mcp_extension_policy = policy;
        self.refresh_current_turn_available_tool_ids();
        self
    }

    fn tool_authorized_by_host_policy(&self, id: &str) -> bool {
        self.configured_tool_allowlist.as_ref().is_none_or(|allowed| allowed.contains(&id.to_ascii_lowercase()))
            || self.catalog.capabilities.get(id).is_some_and(|descriptor| self.mcp_extension_policy.as_ref()
                .is_some_and(|policy| nola_core::tool_policy::mcp_host_descriptor_allowed(policy, descriptor)))
    }

    fn configure_tool_allowlist(
        &mut self,
        ids: Option<impl IntoIterator<Item = String>>,
        promote_directly: bool,
    ) {
        self.configured_tool_allowlist = ids.map(|items| {
            items
                .into_iter()
                .map(|item| item.trim().to_ascii_lowercase())
                .collect()
        });
        // An explicit allowlist is already a user-selected, bounded model
        // surface. Promote those entries directly so constrained hosts do not
        // need an unavailable discovery tool to reach the only tools they were
        // configured to use.
        if promote_directly {
            if let Some(allowed) = &self.configured_tool_allowlist {
                for capability in self.catalog.capabilities.values_mut() {
                    if allowed.contains(&capability.id.to_ascii_lowercase()) {
                        capability.exposure = HostCapabilityExposure::Visible;
                    }
                }
            }
        }
    }

    pub fn with_context_handler(
        mut self,
        handler: impl FnMut(&HostRequestEnvelope, &str, &Value) -> Result<Vec<Value>, String>
            + Send
            + 'static,
    ) -> Self {
        self.context_handler = Box::new(handler);
        self
    }

    fn emit(
        request: &HostRequestEnvelope,
        sequence: &mut u64,
        payload: HostResponse,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        emit(HostResponseEnvelope {
            delegated_invocation_id: request.delegated_invocation_id.clone(),
            attempt_id: request.attempt_id.clone(),
            request_id: request.request_id.clone(),
            effect_id: request.effect_id.clone(),
            host_id: request.host_id.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            delivery: request.delivery.clone(),
            sequence: *sequence,
            payload,
        })?;
        *sequence = sequence.saturating_add(1);
        Ok(())
    }

    fn response_sequence(request: &HostRequestEnvelope) -> u64 {
        request
            .resume_after_sequence
            .map(|sequence| sequence.saturating_add(1))
            .unwrap_or(0)
    }

    /// Commits the Application-owned execution-start barrier before any Host
    /// adapter may invoke a provider, process, tool, or permission transport.
    fn start_effect_delivery(
        &mut self,
        request: &HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        let Some(delivery) = request.delivery.as_ref() else {
            return Ok(());
        };
        self.validate_effect_delivery_request(request)?;
        let mut sequence = Self::response_sequence(request);
        Self::emit(request, &mut sequence, HostResponse::ExecutionStarted, emit)?;
        self.started_effect_deliveries
            .insert(delivery.attempt_id.clone());
        Ok(())
    }

    fn validate_effect_delivery_request(
        &self,
        request: &HostRequestEnvelope,
    ) -> Result<(), String> {
        let Some(delivery) = request.delivery.as_ref() else {
            return Ok(());
        };
        if delivery.attempt_id.trim().is_empty()
            || delivery.worker_generation.trim().is_empty()
            || delivery.lease_owner.trim().is_empty()
            || delivery.lease_token.trim().is_empty()
        {
            return Err("Host effect delivery has an invalid fencing identity".into());
        }
        if self
            .started_effect_deliveries
            .contains(&delivery.attempt_id)
        {
            return Err("Host effect delivery has already started in this worker".into());
        }
        Ok(())
    }

    fn allows_absent_tool_stop_receipt(&self, request: &HostRequestEnvelope) -> bool {
        let HostRequest::ExecuteTool { capability_id, input, .. } = &request.payload else { return false; };
        self.catalog.resolve_available(capability_id).is_some_and(|capability|
            capability.input_normalization.origin.kind != nola_core::agent::host::HostCapabilityOriginKind::Mcp
                && capability.effect_for(input) == nola_core::agent::host::HostCapabilityEffect::Investigate)
    }

    /// Called only for a request still present in this dispatch batch. It is
    /// not a cold-recovery probe or an inference from an absent worker.
    fn record_queued_not_started(&mut self, request: &HostRequestEnvelope) -> Result<bool, String> {
        let identity = (request.host_id.0.clone(), request.session_id.clone(),
            request.turn_id.clone(), request.effect_id.0.clone());
        self.quiescent_effects.remove(&identity);
        if !self.tool_context.cancellation.is_requested()
            || self.boundary_session_id.as_deref() != Some(request.session_id.as_str())
            || self.bound_dispatch_run_id.as_deref() != request.turn_id.as_deref()
            || self.bound_dispatch_run_id.is_none()
            || request.host_id != self.catalog.host_id
            || !self.accepts_catalog_revision(request.catalog_revision, request.availability_epoch,
                matches!(&request.payload, HostRequest::ModelStream { .. }))
            || request.request_id.trim().is_empty() || request.effect_id.0.trim().is_empty() {
            return Err("queued NotStarted proof requires the current exact Host/session/Run/catalog".into());
        }
        let delivery = request.delivery.as_ref()
            .ok_or("queued NotStarted proof requires a committed delivery lease")?;
        if self.started_effect_deliveries.contains(&delivery.attempt_id)
            || delivery.lease_expires_at_ms <= nola_runtime_driver::monotonic_epoch_millis()
            || request.resume_after_sequence.is_some() || request.attempt_started
            || self.delegated_agent_sessions.contains_key(&request.effect_id.0)
            || self.active_effect_cancellations.lock()
                .map_err(|_| "effect cancellation registry poisoned")?
                .contains_key(&request.effect_id.0) {
            // No NotStarted proof. Continue to later controls, whose exact
            // owned worker may still be stopped/joined by the Cancel adapter.
            return Ok(false);
        }
        self.validate_effect_delivery_request(request)?;
        self.quiescent_effects.insert(identity, ObservedHostStop::from_request(request, None));
        Ok(true)
    }

    fn emit_failure(
        request: &HostRequestEnvelope,
        sequence: &mut u64,
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        Self::emit(
            request,
            sequence,
            HostResponse::Failed {
                error: HostError {
                    code: code.into(),
                    message: message.into(),
                    retryable,
                    http_status: None,
                },
            },
            emit,
        )?;
        Ok(())
    }

    fn execute_model(
        &mut self,
        request: &HostRequestEnvelope,
        model_id: &str,
        text_part_id: &str,
        reasoning_part_id: &str,
        messages: &[Value],
        tools: &[Value],
        invocation_plan: &ModelInvocationPlan,
        model_prefix_digest: &str,
        stable_fragment_digests: std::collections::BTreeMap<String, String>,
        previous_prompt_cache_identity: Option<nola_core::prompt_cache::PromptCacheIdentity>,
        response_mode: ModelResponseMode,
        defer_text_presentation: bool,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        let execution_started_at = std::time::Instant::now();
        let _measurement_scope = nola_measurement::enter(nola_measurement::with_attempt(nola_measurement::host_context(
            &request.session_id, &request.effect_id.0, &request.request_id),request.attempt_id.as_deref()));
        let mut sampling_request = request.clone();
        let mut sequence = Self::response_sequence(request);
        if !invocation_plan.native_tool_calls && !tools.is_empty() {
            Self::emit_failure(
                &sampling_request,
                &mut sequence,
                "invalid_model_tool_projection",
                "Core supplied callable schemas for a model step with tool calls disabled",
                false,
                emit,
            )?;
            return Ok(());
        }
        let cold_resampling = request.attempt_started;
        let mut sampling_count = 0usize;
        let original_text_part_id = text_part_id.to_string();
        let original_reasoning_part_id = reasoning_part_id.to_string();
        let mut text_part_id = original_text_part_id.clone();
        let mut reasoning_part_id = original_reasoning_part_id.clone();
        self.run_lifecycle_hooks(
            &sampling_request,
            &mut sequence,
            nola_core::lifecycle_hooks::LifecycleHookEventName::SessionStart,
            None,
            None,
            None,
            true,
            emit,
        )?;
        self.run_lifecycle_hooks(
            &sampling_request,
            &mut sequence,
            nola_core::lifecycle_hooks::LifecycleHookEventName::UserPromptSubmit,
            None,
            None,
            None,
            true,
            emit,
        )?;
        // Tool discovery searches the complete frozen Host catalog, while the
        // model receives only Core's exact Step projection below. Keeping
        // these surfaces separate lets deferred capabilities be discovered
        // without allowing the Host to mutate Core's model-visible schema.
        self.refresh_current_turn_available_tool_ids();
        let model_cancellation = self.llm.cancellation.child();
        if let Ok(mut cancellations) = self.active_effect_cancellations.lock() {
            cancellations.insert(request.effect_id.0.clone(), model_cancellation.clone());
        }
        let _cancellation_guard = ActiveEffectCancellationGuard {
            effect_id: request.effect_id.0.clone(),
            registry: std::sync::Arc::clone(&self.active_effect_cancellations),
        };
        let mut llm = self.llm.clone();
        llm.cancellation = model_cancellation;
        if !model_id.trim().is_empty() {
            llm.model = model_id.trim().to_string();
        }
        self.omit_unavailable_model_targets(&mut llm);
        let declared_llm = llm.clone();
        let mut pinned_router_model = restored_router_model(
            &declared_llm,
            invocation_plan.provider_continuation.as_ref(),
        );
        if let Some(model) = pinned_router_model.as_ref() {
            llm.model = model.clone();
            llm.fallbacks.clear();
        }
        llm.openai_prompt_cache &= invocation_plan.prompt_cache;
        llm.thinking_enabled &= invocation_plan.reasoning;
        if !invocation_plan.reasoning {
            llm.reasoning_selection = None;
        }
        // Capability/step constraints may narrow an explicit preference, but
        // must not invent a request parameter when admission left it absent.
        llm.parallel_tool_calls = constrained_parallel_tool_preference(
            llm.parallel_tool_calls,
            invocation_plan.parallel_tool_calls,
        );
        // Core's invocation plan owns the independent-review policy for all
        // Hosts, including disabling hidden reasoning for protocol JSON.
        if invocation_plan.effective_max_output_tokens > 0 {
            llm.max_output_tokens = invocation_plan.effective_max_output_tokens as u64;
        }
        if invocation_plan.prompt_cache {
            crate::llm::apply_main_turn_prompt_cache_identity(&mut llm, model_prefix_digest);
        }
        let mut base_messages = provider_messages_for_wire_protocol(messages, &declared_llm);
        inject_canonical_user_media(messages, &mut base_messages, invocation_plan.vision_input, invocation_plan.native_pdf_input);
        let media_preparation = inject_tool_artifact_media(&mut base_messages, invocation_plan.vision_input, invocation_plan.native_pdf_input);
        if !media_preparation.is_empty() {
            Self::emit(&sampling_request, &mut sequence, HostResponse::ToolProgress {
                title:"tool_media.prepared".into(), detail:None,
                metadata:json!({"request_id":sampling_request.request_id,
                    "effect_id":sampling_request.effect_id.0, "artifacts":media_preparation,
                    "model_receipt_proven":false}),
            }, emit)?;
        }
        apply_provider_continuation_to_messages(
            &mut base_messages,
            invocation_plan.provider_continuation.as_ref(),
            &declared_llm,
        );
        self.current_turn_skill_decided |= has_skill_decision_since_latest_user(&base_messages);
        let overview_file_count = self
            .current_turn_project_overview_file_count
            .or_else(|| project_overview_file_count_since_latest_user(&base_messages));
        let skill_decision_available = skill_decision_tools_available(tools);
        if !skill_decision_available {
            self.current_turn_skill_decided = true;
        }
        let turn_tools = tools_for_model_turn_with_skill_state(
            tools,
            &self.turn_policy,
            &base_messages,
            self.current_turn_skill_decided,
        );
        let mut turn_request_messages = base_messages.clone();
        // Execution preflight owns the assessment. Ordinary chat must not scan
        // repositories merely because code tools happen to be available.
        if response_mode != ModelResponseMode::StructuredAudit && !turn_tools.is_empty()
            && self.code_intelligence_provider.is_some()
            && self.configured_tool_allowlist.as_ref()
                .is_none_or(|allowed| allowed.contains("codebase_index"))
        {
            if let Some(assessment) = &self.current_turn_code_memory_assessment {
                turn_request_messages.push(host_runtime_fact("code-memory-initialization", assessment.clone()));
            }
        }
        if response_mode != ModelResponseMode::StructuredAudit {
            move_skill_catalog_to_turn_reminder(
                &mut turn_request_messages,
                skill_decision_available && !self.current_turn_skill_decided,
            );
        }
        if let Some(file_count) = overview_file_count
            .filter(|count| *count > 10 && response_mode != ModelResponseMode::StructuredAudit)
        {
            // The model may select project_overview without a separate scope
            // classifier. Use the observed structured result as soft
            // progressive-disclosure evidence instead of inferring routing
            // from user wording or silently losing the checkpoint.
            turn_request_messages.push(host_runtime_fact("project-overview", json!({"sampled_file_count":file_count})));
        }
        let mut request_messages = turn_request_messages.clone();
        let mut combined_content = String::new();
        let mut combined_reasoning = String::new();
        let mut final_tool_calls;
        let mut final_response_id = None;
        let mut final_response_items;
        let mut final_provider_namespace;
        let mut final_provider_model;
        // Core decides when a response must remain provisional. Merely exposing
        // a tool catalog must not turn every ordinary answer into a buffered
        // response: most requests expose tools, including requests that produce
        // only final prose. The presentation gate already withholds a short
        // ambiguous prefix and suppresses native-tool preambles when a tool call
        // arrives, while accepted answer text continues in semantic batches.
        let mut text_presentation =
            ModelTextPresentationGate::for_request(defer_text_presentation, !turn_tools.is_empty());
        let mut reasoning_presentation = SemanticPresentationBuffer::default();
        // A Core final-synthesis effect has an intentionally empty projection.
        // Tell compatible providers explicitly that native tool calls are not
        // valid; leaving `auto` here lets some providers replay remembered
        // tool calls from the retained transcript despite receiving no schema.
        let model_tool_choice = provider_tool_choice(!turn_tools.is_empty(), response_mode);
        let mut final_usage = TokenUsage::default();
        let mut physical_cache_identity;
        let mut output_stream_budget = ModelOutputStreamBudget::new(llm.max_output_tokens);
        let mut recovery_budget = ModelRecoveryBudget::new();
        let mut output_limited = false;
        let mut physical_round = 0usize;
        crate::llm::log_local_debug_payload(
            "harness_step_timing",
            &json!({
                "type": "harness_step_timing",
                "stage": "host_model_preparation",
                "elapsed_ms": execution_started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
                "physical_round": 0,
                "tool_count": turn_tools.len(),
                "message_count": request_messages.len(),
            }),
        );
        loop {
            physical_round += 1;
            let mut buffered_continuation = String::new();
            let mut buffered_reasoning = String::new();
            let model_started_at = std::time::Instant::now();
            let mut first_text_logged = false;
            let mut first_visible_text_logged = false;
            let mut admission_error = None;
            let mut staged_tool_call_ids = HashSet::new();
            let model_target_keys = Self::model_target_keys(&llm);
            let mut newly_unavailable_model_targets = Vec::new();
            // Supervisor facts are generated after semantic/tool decisions and
            // appended only to this physical request. Never mutate canonical
            // history or the stable policy prefix with a ticking countdown.
            let budget_messages = crate::execution_context::external_execution_budget_message()
                .map(|budget| {
                    let mut messages = request_messages.clone();
                    messages.push(budget);
                    messages
                });
            let physical_messages = budget_messages.as_deref().unwrap_or(&request_messages);
            let result = stream_chat_checked_with_presentation_ticks(
                &llm,
                physical_messages,
                &turn_tools,
                model_tool_choice,
                |notification| {
                    let accepted = (|| -> Result<(), String> {
                        let event = match notification {
                            ModelPresentationNotification::Provider(event) => event,
                            ModelPresentationNotification::Idle => {
                                if invocation_plan.stream_text {
                                    let visible_delta = text_presentation
                                        .poll_at(std::time::Instant::now(), &buffered_reasoning)
                                        .concat();
                                    if !visible_delta.is_empty() {
                                        first_visible_text_logged = true;
                                        Self::emit(
                                            &sampling_request,
                                            &mut sequence,
                                            HostResponse::ModelDelta {
                                                channel: ModelDeltaChannel::Text,
                                                part_id: text_part_id.to_string(),
                                                delta: visible_delta,
                                            },
                                            emit,
                                        )?;
                                    }
                                }
                                if invocation_plan.stream_reasoning {
                                    let visible_delta = reasoning_presentation
                                        .flush_due(
                                            std::time::Instant::now(),
                                            ModelTextPresentationGate::MAXIMUM_PREFIX_WAIT,
                                        )
                                        .concat();
                                    if !visible_delta.is_empty() {
                                        Self::emit(
                                            &sampling_request,
                                            &mut sequence,
                                            HostResponse::ModelDelta {
                                                channel: ModelDeltaChannel::Reasoning,
                                                part_id: reasoning_part_id.to_string(),
                                                delta: visible_delta,
                                            },
                                            emit,
                                        )?;
                                    }
                                }
                                return Ok(());
                            }
                        };
                        match event {
                            LlmEvent::SamplingStarted { provider, model } => {
                                // Replaced attempts discard their content; the
                                // in-memory byte backstop must discard its count
                                // too. Retry limits are owned separately.
                                output_stream_budget.start_attempt();
                                let replacing = sampling_count > 0 || cold_resampling;
                                if replacing && invocation_plan.stream_reasoning {
                                    let visible_delta = reasoning_presentation.finish().concat();
                                    if !visible_delta.is_empty() {
                                        Self::emit(
                                            &sampling_request,
                                            &mut sequence,
                                            HostResponse::ModelDelta {
                                                channel: ModelDeltaChannel::Reasoning,
                                                part_id: reasoning_part_id.to_string(),
                                                delta: visible_delta,
                                            },
                                            emit,
                                        )?;
                                    }
                                }
                                let attempt_id = if !replacing {
                                    sampling_request.attempt_id.clone().ok_or_else(|| {
                                        "prepared model Attempt is missing".to_string()
                                    })?
                                } else {
                                    uuid::Uuid::new_v4().to_string()
                                };
                                sampling_request.attempt_id = Some(attempt_id.clone());
                                nola_measurement::set_attempt(&attempt_id);
                                let reason = if sampling_count == 0 {
                                    if cold_resampling {
                                        "cold_redispatch"
                                    } else {
                                        "initial"
                                    }
                                } else if physical_round > 1 {
                                    "protocol_correction"
                                } else {
                                    "retry_or_fallback"
                                };
                                Self::emit(
                                    &sampling_request,
                                    &mut sequence,
                                    HostResponse::ModelAttemptStarted {
                                        provider,
                                        model,
                                        reason: reason.into(),
                                    },
                                    emit,
                                )?;
                                if replacing {
                                    text_part_id = format!("{attempt_id}:text");
                                    reasoning_part_id = format!("{attempt_id}:reasoning");
                                }
                                sampling_request.attempt_started = true;
                                sampling_count += 1;
                                staged_tool_call_ids.clear();
                                buffered_continuation.clear();
                                buffered_reasoning.clear();
                                combined_content.clear();
                                combined_reasoning.clear();
                                text_presentation = ModelTextPresentationGate::for_request(
                                    defer_text_presentation,
                                    !turn_tools.is_empty(),
                                );
                                reasoning_presentation = SemanticPresentationBuffer::default();
                            }
                            LlmEvent::TextDelta(delta) => {
                                if !output_stream_budget.admit(&delta) {
                                    return Err(MODEL_OUTPUT_BYTE_LIMIT_REACHED.to_string());
                                }
                                if !delta.is_empty() && !first_text_logged {
                                    first_text_logged = true;
                                    if std::env::var_os("NOLA_CLI_DEBUG").is_some() {
                                        eprintln!(
                                            "nola_model_first_text_timing elapsed_ms={}",
                                            model_started_at.elapsed().as_millis()
                                        );
                                    }
                                }
                                buffered_continuation.push_str(&delta);
                                if invocation_plan.stream_text {
                                    let visible_deltas = text_presentation
                                        .push_text_against_reasoning(delta, &buffered_reasoning);
                                    if !visible_deltas.is_empty() {
                                        let visible_delta = visible_deltas.concat();
                                        if !first_visible_text_logged {
                                            first_visible_text_logged = true;
                                            if std::env::var_os("NOLA_CLI_DEBUG").is_some() {
                                                eprintln!(
                                            "nola_model_first_visible_text_timing elapsed_ms={}",
                                            model_started_at.elapsed().as_millis()
                                        );
                                            }
                                        }
                                        Self::emit(
                                            &sampling_request,
                                            &mut sequence,
                                            HostResponse::ModelDelta {
                                                channel: ModelDeltaChannel::Text,
                                                part_id: text_part_id.to_string(),
                                                delta: visible_delta,
                                            },
                                            emit,
                                        )?;
                                    }
                                }
                            }
                            LlmEvent::ReasoningDelta(delta) => {
                                if !output_stream_budget.admit(&delta) {
                                    return Err(MODEL_OUTPUT_BYTE_LIMIT_REACHED.to_string());
                                }
                                buffered_reasoning.push_str(&delta);
                                if invocation_plan.stream_reasoning {
                                    let visible_delta = reasoning_presentation.push_reasoning(delta).concat();
                                    if !visible_delta.is_empty() {
                                        Self::emit(
                                            &sampling_request,
                                            &mut sequence,
                                            HostResponse::ModelDelta {
                                                channel: ModelDeltaChannel::Reasoning,
                                                part_id: reasoning_part_id.to_string(),
                                                delta: visible_delta,
                                            },
                                            emit,
                                        )?;
                                    }
                                }
                            }
                            LlmEvent::ToolCallDelta {
                                call_id,
                                name: _,
                                arguments_delta,
                            } => {
                                staged_tool_call_ids.insert(call_id);
                                validate_model_tool_batch_size(staged_tool_call_ids.len())?;
                                if !output_stream_budget.admit(&arguments_delta) {
                                    return Err(MODEL_OUTPUT_BYTE_LIMIT_REACHED.to_string());
                                }
                                text_presentation.observe_tool_call();
                                // Partial argument fragments are transport state, not
                                // durable process events. Core receives the complete,
                                // validated calls in ModelCompleted; withholding these
                                // fragments makes a truncated stream safely retryable
                                // and prevents argument-token UI churn.
                            }
                            LlmEvent::StageTiming { stage, elapsed_ms } => {
                                crate::llm::log_local_debug_payload(
                                    "harness_step_timing",
                                    &json!({
                                        "type": "harness_step_timing",
                                        "stage": stage,
                                        "elapsed_ms": elapsed_ms,
                                        "physical_round": physical_round,
                                        "tool_count": turn_tools.len(),
                                        "message_count": request_messages.len(),
                                    }),
                                );
                            }
                            LlmEvent::Retrying {
                                attempt,
                                delay_ms,
                                failure,
                                target_exhausted,
                            } => {
                                let route = Self::model_retry_route(
                                    &llm,
                                    attempt,
                                    target_exhausted,
                                    &failure.message,
                                );
                                if target_exhausted && failure.exhausts_target {
                                    let failed_index = attempt.saturating_sub(2);
                                    if let Some(key) = model_target_keys.get(failed_index) {
                                        newly_unavailable_model_targets.push(key.clone());
                                    }
                                }
                                Self::emit(
                                    &sampling_request,
                                    &mut sequence,
                                    HostResponse::ToolProgress {
                                        title: "model_retry".to_string(),
                                        detail: Some(route),
                                        metadata: json!({
                                            "attempt": attempt,
                                            "delay_ms": delay_ms,
                                            "target_exhausted": target_exhausted,
                                            "failure_code": failure.code,
                                            "failure_message": failure.message,
                                            "retryable": failure.retryable,
                                            "provider_status": failure.status,
                                            "retry_after_ms": failure.retry_after_ms,
                                            "provider": failure.provider,
                                            "model": failure.model,
                                        }),
                                    },
                                    emit,
                                )?
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = &accepted {
                        admission_error = Some(error.clone());
                    }
                    accepted
                },
            );
            self.current_turn_unavailable_model_targets
                .extend(newly_unavailable_model_targets);
            if let Some(error) = admission_error
                .as_deref()
                .filter(|error| model_stream_admission_requires_abort(error))
            {
                return Err(error.to_string());
            }
            // Capacity rejection is an atomic planning retry, never a partial
            // completion or a transport fallback. No tool in this response ran.
            // Validate the terminal snapshot too: some providers emit no deltas.
            let oversized_batch = match &result {
                Ok(choice) => validate_model_tool_batch_size(choice.tool_calls.len()).is_err(),
                Err(error) => error.diagnostic == MODEL_TOOL_BATCH_LIMIT_EXCEEDED,
            };
            if oversized_batch {
                llm.parallel_tool_calls = Some(false);
                text_presentation.discard_for_tool_round();
                combined_content.clear();
                combined_reasoning.clear();
                if recovery_budget.take_protocol_correction() {
                    Self::emit(
                        &sampling_request,
                        &mut sequence,
                        HostResponse::ToolProgress {
                            title: "model_protocol_recovery".to_string(),
                            detail: None,
                            metadata: json!({
                                "failure": MODEL_TOOL_BATCH_LIMIT_EXCEEDED,
                                "recovery": "bounded_tool_batch_replan",
                            }),
                        },
                        emit,
                    )?;
                    request_messages = turn_request_messages.clone();
                    request_messages.push(host_runtime_fact("rejected-tool-batch", json!({
                        "failure":MODEL_TOOL_BATCH_LIMIT_EXCEEDED, "executed_calls":0,
                        "next_response_max_native_calls":1, "guidance":model_tool_batch_guidance(),
                    })));
                    text_presentation = ModelTextPresentationGate::for_request(
                        defer_text_presentation,
                        !turn_tools.is_empty(),
                    );
                    continue;
                }
                Self::emit_failure(
                    &sampling_request,
                    &mut sequence,
                    MODEL_TOOL_BATCH_LIMIT_EXCEEDED,
                    model_tool_batch_guidance(),
                    false,
                    emit,
                )?;
                return Ok(());
            }
            let mut choice = match result {
                Ok(choice) => choice,
                Err(error) if error.diagnostic == MODEL_OUTPUT_BYTE_LIMIT_REACHED => {
                    StreamedChoice {
                        content: buffered_continuation.clone(),
                        reasoning: buffered_reasoning.clone(),
                        finish_reason: "max_output_bytes".to_string(),
                        ..StreamedChoice::default()
                    }
                }
                Err(error) => {
                    crate::llm::log_local_debug_payload(
                        "model_provider_failure",
                        &json!({
                            "type": "model_provider_failure",
                            "code": error.code,
                            "status": error.status,
                            "retryable": error.retryable,
                            "provider": error.provider,
                            "model": error.model,
                        }),
                    );
                    Self::emit(
                        &sampling_request,
                        &mut sequence,
                        HostResponse::Failed {
                            error: HostError {
                                code: error.code.clone(),
                                message: error.display_message(),
                                retryable: error.retryable,
                                http_status: error.status,
                            },
                        },
                        emit,
                    )?;
                    return Ok(());
                }
            };
            if let Some(expected) = pinned_router_model.as_deref() {
                if choice.response_model.as_deref() != Some(expected) {
                    Self::emit_failure(
                        &sampling_request,
                        &mut sequence,
                        "model_route_changed",
                        "The provider changed the resolved model during this turn.",
                        false,
                        emit,
                    )?;
                    return Ok(());
                }
            } else if let Some(actual) = choice
                .response_model
                .as_deref()
                .and_then(|model| openrouter_resolved_model(&declared_llm, model))
            {
                llm.model = actual.clone();
                llm.fallbacks.clear();
                pinned_router_model = Some(actual);
            }
            // Persist the accepted streaming response alongside (not inside)
            // the physical request, for cache-safe compaction after restart.
            if !buffered_continuation.is_empty() {
                choice.content = std::mem::take(&mut buffered_continuation);
            }
            if !buffered_reasoning.is_empty() {
                choice.reasoning = std::mem::take(&mut buffered_reasoning);
            }
            if let Some(mut snapshot) = choice.request_snapshot.take() {
                std::sync::Arc::make_mut(&mut snapshot).record_completed_response(&choice);
                choice.request_snapshot = Some(snapshot);
            }
            // Commit this physical attempt before provenance/content publication
            // can reject the response. Logical-turn totals are separate.
            let mut physical_usage = Self::core_token_usage(&choice.usage);
            physical_usage.cache_usage = Some(choice.usage.cache_usage(
                choice.provider_namespace.as_deref() == Some("anthropic.messages"), false));
            Self::emit(&sampling_request, &mut sequence, HostResponse::ModelUsageReported {
                usage: serde_json::to_value(physical_usage).map_err(|error| error.to_string())?,
            }, emit)?;
            final_usage.cache_usage = Some(choice.usage.cache_usage(
                choice.provider_namespace.as_deref() == Some("anthropic.messages"), false));
            physical_cache_identity = choice.request_snapshot.as_ref().map(|snapshot| {
                let mut identity = snapshot.prompt_cache_identity();
                identity.stable_fragment_sha256 = stable_fragment_digests.clone();
                identity.cache_control = Self::prompt_cache_control_identity(snapshot.config());
                identity
            });
            self.compaction_request_witness = choice.request_snapshot.take()
                .filter(|snapshot| choice.response_model.as_deref().and_then(|model| openrouter_resolved_model(snapshot.config(), model)).is_none())
                .map(|snapshot| CompactionRequestWitness {
                request: snapshot,
                session_id: request.session_id.to_string(),
                configuration_identity: crate::llm::request_configuration_identity(&self.llm),
                context_window: invocation_plan.effective_context_window_tokens,
                usage: choice.usage.clone(),
            });
            let provenance = model_response_provenance(
                &llm,
                &choice,
                sampling_count,
                output_stream_budget.utf8_bytes(),
            );
            Self::emit(
                &sampling_request,
                &mut sequence,
                HostResponse::ToolProgress {
                    title: "model_response_provenance".into(),
                    detail: Some(provenance.to_string()),
                    metadata: provenance,
                },
                emit,
            )?;
            // Streaming deltas are the authoritative content record. Some
            // compatible providers return only the last chunk (or no text) in
            // the terminal choice; trusting that terminal snapshot makes the
            // persisted message disagree with what clients already rendered.
            if !buffered_continuation.is_empty() {
                choice.content = buffered_continuation;
            }
            if !buffered_reasoning.is_empty() {
                choice.reasoning = buffered_reasoning;
            }
            if choice.response_id.is_some() {
                final_response_id = choice.response_id.clone();
            }
            final_response_items = choice.response_items.clone();
            final_provider_namespace = choice.provider_namespace.clone();
            final_provider_model = if pinned_router_model.is_some() {
                Some(llm.model.clone())
            } else {
                choice.provider_model.clone()
            };
            Self::accumulate_token_usage(&mut final_usage, &choice.usage);

            if tool_batch_requires_activation_replan(&choice.tool_calls) {
                const GUIDANCE: &str = "A Skill or SOP activation must be the only call in its batch. The entire previous batch was rejected before execution; none of its calls ran. Emit one activation call, consume its instructions, then plan subsequent work in a new model turn.";
                text_presentation.discard_for_tool_round();
                if recovery_budget.take_protocol_correction() {
                    Self::emit(
                        &sampling_request,
                        &mut sequence,
                        HostResponse::ToolProgress {
                            title: "model_protocol_recovery".into(),
                            detail: Some(GUIDANCE.into()),
                            metadata: json!({
                                "failure": "activation_batch_requires_replan",
                                "recovery": "atomic_batch_replan",
                                "rejected_call_ids": choice.tool_calls.iter().map(|call| &call.id).collect::<Vec<_>>(),
                                "tool_execution_skipped": true,
                            }),
                        },
                        emit,
                    )?;
                    request_messages = turn_request_messages.clone();
                    request_messages.push(host_runtime_fact("rejected-activation-batch", json!({
                        "failure":"activation_batch_requires_replan", "executed_calls":0,
                        "activation_batch_limit":1,
                    })));
                    text_presentation = ModelTextPresentationGate::for_request(
                        defer_text_presentation,
                        !turn_tools.is_empty(),
                    );
                    continue;
                }
                Self::emit_failure(
                    &sampling_request,
                    &mut sequence,
                    "activation_batch_requires_replan",
                    GUIDANCE,
                    false,
                    emit,
                )?;
                return Ok(());
            }
            combined_content.push_str(&choice.content);
            combined_reasoning.push_str(&choice.reasoning);
            let finish_reason = choice.finish_reason.clone();
            final_tool_calls = choice.tool_calls;
            // `parallel_tool_calls` is a request preference frozen by Core,
            // not a validity constraint on provider output. OpenAI-compatible
            // providers can return more than one native function call even
            // when the preference is absent or false. Core already owns the
            // batch scheduler and applies each capability's parallelism and
            // permission contract, so forward the complete batch instead of
            // failing the Turn after the model has produced valid calls.

            // Protocol validity comes from the provider parser, never prose,
            // XML examples, code fences or language-specific execution words.
            let invalid_tool_transcript = model_response_requires_protocol_recovery(&finish_reason, !final_tool_calls.is_empty());
            if invalid_tool_transcript {
                text_presentation.discard_for_tool_round();
                combined_content.clear();
                combined_reasoning.clear();
                if recovery_budget.take_protocol_correction() {
                    let (recovery, detail, correction) =
                        model_protocol_correction(!turn_tools.is_empty(), response_mode);
                    Self::emit(
                        &sampling_request,
                        &mut sequence,
                        HostResponse::ToolProgress {
                            title: "model_protocol_recovery".to_string(),
                            detail: Some(detail.to_string()),
                            metadata: json!({
                                "failure": "invalid_model_protocol",
                                "recovery": recovery,
                            }),
                        },
                        emit,
                    )?;
                    request_messages = turn_request_messages.clone();
                    request_messages.push(host_runtime_fact("model-protocol-failure", json!({
                        "failure":"protocol_error", "executed_calls":0, "contract":correction,
                    })));
                    text_presentation = ModelTextPresentationGate::for_request(
                        defer_text_presentation,
                        !turn_tools.is_empty(),
                    );
                    continue;
                }
                Self::emit_failure(
                    &sampling_request,
                    &mut sequence,
                    "invalid_model_protocol",
                    "The provider repeatedly returned an invalid model protocol response",
                    false,
                    emit,
                )?;
                return Ok(());
            }

            if final_tool_calls.is_empty() {
                let remaining_text = text_presentation.finish_answer().concat();
                if !remaining_text.is_empty() {
                    Self::emit(
                        &sampling_request,
                        &mut sequence,
                        HostResponse::ModelDelta {
                            channel: ModelDeltaChannel::Text,
                            part_id: text_part_id.to_string(),
                            delta: remaining_text,
                        },
                        emit,
                    )?;
                }
                if !defer_text_presentation && !choice.content.is_empty() {
                    // Commit the already projected preview without replaying
                    // its bytes. Until this terminal decision, the same stream
                    // may still become a tool round and must remain retractable.
                    Self::emit(
                        &sampling_request,
                        &mut sequence,
                        HostResponse::ModelCommittedDelta {
                            part_id: text_part_id.to_string(),
                            delta: String::new(),
                        },
                        emit,
                    )?;
                }
            } else {
                text_presentation.discard_for_tool_round();
            }

            if final_tool_calls.is_empty() {
                let Some(incomplete) = incomplete_model_output(&finish_reason) else {
                    break;
                };
                // Both an explicit provider output limit and a transport that
                // ended after yielding valid model fragments are partial
                // completions. Commit those fragments to Core as one limited
                // completion so Core can request a bounded semantic
                // continuation and append it to the same canonical message.
                // Returning Failure here stranded the retained UI fragment and
                // bypassed Core's duplicate/no-progress protection entirely.
                match incomplete {
                    IncompleteModelOutput::OutputLimit
                    | IncompleteModelOutput::TransportInterrupted => {
                        output_limited = true;
                        break;
                    }
                }
            }
            break;
        }

        if invocation_plan.stream_reasoning {
            let visible_delta = reasoning_presentation.finish().concat();
            if !visible_delta.is_empty() {
                Self::emit(
                    &sampling_request,
                    &mut sequence,
                    HostResponse::ModelDelta {
                        channel: ModelDeltaChannel::Reasoning,
                        part_id: reasoning_part_id.to_string(),
                        delta: visible_delta,
                    },
                    emit,
                )?;
            }
        }

        // No physical witness means unknown identity, not a fabricated claim
        // that the catalog or nominal Core prefix was exactly sent.
        let current_prompt_cache_identity = physical_cache_identity.clone().unwrap_or_default();
        let prompt_cache_diagnostic = nola_core::prompt_cache::diagnose_prompt_cache(
            &nola_core::prompt_cache::PromptCacheDiagnosticInput {
                schema_version: nola_core::prompt_cache::PROMPT_CACHE_SCHEMA_VERSION,
                enabled: invocation_plan.prompt_cache && llm.openai_prompt_cache,
                current: current_prompt_cache_identity.clone(),
                previous: previous_prompt_cache_identity,
                usage: nola_core::prompt_cache::PromptCacheProviderUsage {
                    input_tokens: final_usage.input_tokens.map(|value| value as u64),
                    cache_read_input_tokens: final_usage.cached_input_tokens.map(|value| value as u64),
                    cache_write_input_tokens: final_usage.cache_write_input_tokens.map(|value| value as u64),
                },
            },
        );
        crate::llm::log_local_debug_payload("model_cache_comparison", &serde_json::to_value(&prompt_cache_diagnostic).unwrap_or(Value::Null));
        final_usage.prompt_cache_identity = physical_cache_identity;
        final_usage.prompt_cache_diagnostic = Some(prompt_cache_diagnostic);
        if let Some(diagnostic) = final_usage.prompt_cache_diagnostic.as_mut() {
            diagnostic.cache_usage = final_usage.cache_usage;
        }

        let has_tool_calls = !final_tool_calls.is_empty();
        if !has_tool_calls {
            self.run_lifecycle_hooks(
                &sampling_request,
                &mut sequence,
                nola_core::lifecycle_hooks::LifecycleHookEventName::TurnEnd,
                None,
                None,
                None,
                true,
                emit,
            )?;
        }

        let mut items = Vec::new();
        // Only the provider reasoning channel may populate reasoning_content.
        // Tool-call preambles are ordinary assistant content, not reasoning
        // and never a substitute for missing provider continuation data.
        let completed_reasoning = combined_reasoning;
        // A provider continuation is scoped to the model configured on the
        // active Turn. A successful cross-model fallback completes that Turn,
        // but its opaque continuation cannot be attached to the original
        // model without violating Core's continuation identity contract.
        let continuation_model = final_provider_model.as_deref().unwrap_or(&llm.model);
        let provider_continuation = if response_mode == ModelResponseMode::StructuredAudit {
            // The reviewer is an independent model invocation. Its protocol
            // state must not become the user's conversation continuation.
            None
        } else if final_provider_namespace.as_deref() == Some("openai.responses")
            && (continuation_model == declared_llm.model
                || pinned_router_model.as_deref() == Some(continuation_model))
            && !final_response_items.is_empty()
        {
            let mut state = json!({
                "response_items": final_response_items,
                "route_identity": crate::llm::responses_route_identity(&declared_llm),
            });
            if let Some(resolved_model) = pinned_router_model.as_ref() {
                state["resolved_model_id"] = json!(resolved_model);
            }
            Some(ModelProviderContinuationState {
                provider_namespace: "openai.responses".into(),
                model_id: declared_llm.model.clone(),
                response_id: final_response_id.clone(),
                state,
                integrity_kind: "encrypted".into(),
                compaction_policy: ProviderContinuationCompactionPolicy::ClearOnCompaction,
            })
        } else if continuation_model == llm.model
            && final_provider_namespace.as_deref() == Some(OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE)
            && (final_response_id.is_some()
                || !completed_reasoning.is_empty()
                || pinned_router_model.is_some())
        {
            let mut state = serde_json::Map::new();
            if !completed_reasoning.is_empty() {
                state.insert(
                    "reasoning_content".to_string(),
                    Value::String(completed_reasoning.clone()),
                );
            }
            if let Some(response_id) = final_response_id.as_ref() {
                state.insert(
                    "previous_response_id".to_string(),
                    Value::String(response_id.clone()),
                );
            }
            if let Some(resolved_model) = pinned_router_model.as_ref() {
                state.insert(
                    "resolved_model_id".to_string(),
                    Value::String(resolved_model.clone()),
                );
                state.insert(
                    "route_identity".to_string(),
                    Value::String(crate::llm::model_route_identity(&declared_llm)),
                );
            }
            Some(ModelProviderContinuationState {
                provider_namespace: OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE.to_string(),
                model_id: if pinned_router_model.is_some() {
                    declared_llm.model.clone()
                } else {
                    continuation_model.to_string()
                },
                response_id: final_response_id.clone(),
                state: Value::Object(state),
                integrity_kind: if completed_reasoning.is_empty() {
                    "opaque_reference".to_string()
                } else {
                    "plaintext".to_string()
                },
                compaction_policy: if pinned_router_model.is_some()
                    && completed_reasoning.is_empty()
                {
                    ProviderContinuationCompactionPolicy::Preserve
                } else {
                    ProviderContinuationCompactionPolicy::ClearOnCompaction
                },
            })
        } else {
            None
        };
        // Persist opaque Responses output even when the provider emits no
        // displayable reasoning text. Later tool continuations need every
        // same-turn output item, not just the most recent model response.
        if !completed_reasoning.is_empty()
            || provider_continuation
                .as_ref()
                .is_some_and(|state| state.provider_namespace == "openai.responses")
        {
            items.push(
                serde_json::to_value(AgentItem::Reasoning {
                    id: ItemId::from_string(reasoning_part_id.to_string()),
                    summary: String::new(),
                    content: completed_reasoning,
                    provider_continuation: provider_continuation.clone(),
                })
                .unwrap_or(Value::Null),
            );
        }
        if !has_tool_calls && !combined_content.is_empty() {
            items.push(
                serde_json::to_value(AgentItem::Message {
                    id: ItemId::from_string(text_part_id.to_string()),
                    role: AgentRole::Assistant,
                    content: combined_content,
                })
                .unwrap_or(Value::Null),
            );
        }
        let tool_calls = final_tool_calls
            .into_iter()
            .map(|call| {
                serde_json::to_value(ModelToolCall {
                    call_id: nola_core::agent::protocol::CallId::from_string(
                        core_scoped_model_call_id(request.effect_id.0.as_str(), call.id.as_str()),
                    ),
                    name: call.name,
                    arguments: crate::llm::normalized_tool_arguments_value(&call.arguments),
                    permission_scope: None,
                    permission_rationale: None,
                })
                .unwrap_or(Value::Null)
            })
            .collect();
        if let Some(continuation) = provider_continuation {
            Self::emit(
                &sampling_request,
                &mut sequence,
                HostResponse::ModelContinuation { continuation },
                emit,
            )?;
        }
        Self::emit(
            &sampling_request,
            &mut sequence,
            if output_limited {
                HostResponse::ModelOutputLimited {
                    items,
                    tool_calls,
                    usage: serde_json::to_value(final_usage).unwrap_or_else(|_| json!({})),
                }
            } else {
                HostResponse::ModelCompleted {
                    items,
                    tool_calls,
                    usage: serde_json::to_value(final_usage).unwrap_or_else(|_| json!({})),
                }
            },
            emit,
        )?;
        // This file is an optional cache witness, not the authoritative turn
        // checkpoint. Publish the accepted result first. Only after sync+rename
        // succeeds can a restart reuse it; failure leaves the in-memory witness
        // usable and must not turn a successful model result into a failed turn.
        if let Some(witness) = &self.compaction_request_witness {
            if witness.persist(self.tool_context.output_store.root()).is_err() {
                eprintln!("nola_compaction_snapshot_persist_failed: request remains available only in memory");
            }
        } else if CompactionRequestWitness::invalidate(
            self.tool_context.output_store.root(), request.session_id.as_str(),
            &crate::llm::request_configuration_identity(&self.llm),
        ).is_err() {
            eprintln!("nola_compaction_snapshot_invalidation_failed: latest response has no physical witness");
        }
        Ok(())
    }

    fn core_token_usage(usage: &crate::llm::LlmUsage) -> TokenUsage {
        TokenUsage {
            cache_usage: None,
            input_tokens: usage.input_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            cache_write_input_tokens: usage.cache_write_input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            total_tokens: usage.total_tokens,
            request_count: 1,
            known_token_lower_bound: 0,
            prompt_cache_identity: None,
            prompt_cache_diagnostic: None,
        }
    }

    fn accumulate_token_usage(total: &mut TokenUsage, usage: &crate::llm::LlmUsage) {
        total.accumulate(&Self::core_token_usage(usage));
    }

    fn prompt_cache_control_identity(
        llm: &LlmConfig,
    ) -> Option<nola_core::prompt_cache::PromptCacheControlIdentity> {
        if let Some(strategy) = llm
            .openai_prompt_cache_retention
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(nola_core::prompt_cache::PromptCacheControlIdentity {
                strategy: strategy.to_string(),
                ttl_seconds: Self::prompt_cache_ttl_seconds(strategy),
            });
        }
        (crate::llm::prompt_cache_provider_id(llm) == "anthropic").then(|| {
            nola_core::prompt_cache::PromptCacheControlIdentity {
                strategy: "ephemeral".to_string(),
                ttl_seconds: None,
            }
        })
    }

    fn run_lifecycle_hooks(
        &mut self, request: &HostRequestEnvelope, sequence: &mut u64,
        event_name: nola_core::lifecycle_hooks::LifecycleHookEventName,
        event_id: Option<&str>, tool_name: Option<&str>, tool_call_id: Option<&str>,
        once_per_turn: bool,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        self.lifecycle.run_lifecycle_hooks(request, sequence, event_name, event_id,
            tool_name, tool_call_id, once_per_turn, false, emit)
    }
    fn run_tool_completion_hook(
        &mut self,
        request: &HostRequestEnvelope,
        sequence: &mut u64,
        capability_id: &str,
        call_id: &str,
        is_error: bool,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        self.run_lifecycle_hooks(
            request,
            sequence,
            if is_error {
                nola_core::lifecycle_hooks::LifecycleHookEventName::ToolError
            } else {
                nola_core::lifecycle_hooks::LifecycleHookEventName::PostToolUse
            },
            None,
            Some(capability_id),
            Some(call_id),
            false,
            emit,
        )?;
        Ok(())
    }

    fn prompt_cache_ttl_seconds(value: &str) -> Option<u64> {
        let (digits, multiplier) = match value.as_bytes().last().copied()? {
            b's' => (&value[..value.len().saturating_sub(1)], 1),
            b'm' => (&value[..value.len().saturating_sub(1)], 60),
            b'h' => (&value[..value.len().saturating_sub(1)], 3_600),
            b'd' => (&value[..value.len().saturating_sub(1)], 86_400),
            _ => (value, 1),
        };
        digits.trim().parse::<u64>().ok()?.checked_mul(multiplier)
    }

    fn execute_tool(
        &mut self,
        request: &HostRequestEnvelope,
        capability_id: &str,
        call_id: &str,
        input: &Value,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        self.execute_tool_with_sop(request, capability_id, call_id, input, None, emit)?;
        Ok(())
    }

    fn execute_tool_with_sop(
        &mut self,
        request: &HostRequestEnvelope,
        capability_id: &str,
        call_id: &str,
        input: &Value,
        sop_invocation: Option<nola_core::agent::ToolInvocationContext>,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        Self::measure_host_execution(request, nola_measurement::Stage::HostTool, emit, |emit| {
            self.execute_tool_with_sop_measured(request, capability_id, call_id, input, sop_invocation, emit)
        })
    }

    fn measure_host_execution(
        request: &HostRequestEnvelope,
        stage: nola_measurement::Stage,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
        operation: impl FnOnce(&mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>) -> Result<(), String>,
    ) -> Result<(), String> {
        use nola_measurement::Outcome;
        let _scope = nola_measurement::enter(nola_measurement::with_attempt(nola_measurement::host_context(
            &request.session_id, &request.effect_id.0, &request.request_id),request.attempt_id.as_deref()));
        let span = nola_measurement::Span::start(stage);
        let _child = nola_measurement::enter(span.child_context());
        let mut outcome = Outcome::Incomplete;
        let result = operation(&mut |response| {
            let terminal = match &response.payload {
                HostResponse::ToolCompleted { outcome, .. } => Some(match outcome {
                    nola_domain::host_effect::HostToolOutcome::Succeeded => Outcome::Succeeded,
                    nola_domain::host_effect::HostToolOutcome::Deferred => Outcome::Incomplete,
                    nola_domain::host_effect::HostToolOutcome::Failed => Outcome::Failed,
                    nola_domain::host_effect::HostToolOutcome::Cancelled => Outcome::Cancelled,
                }),
                HostResponse::CompactionCompleted { .. } => Some(Outcome::Succeeded),
                HostResponse::Failed { error } => Some(if error.code == "compaction_cancelled" {
                    Outcome::Cancelled
                } else { Outcome::Failed }),
                _ => None,
            };
            let accepted = emit(response);
            if accepted.is_err() { outcome = Outcome::Rejected; }
            else if let Some(terminal) = terminal { outcome = terminal; }
            accepted
        });
        if result.is_err() && outcome == Outcome::Incomplete { outcome = Outcome::Failed; }
        span.finish(outcome);
        result
    }

    fn execute_tool_with_sop_measured(
        &mut self,
        request: &HostRequestEnvelope,
        capability_id: &str,
        call_id: &str,
        input: &Value,
        sop_invocation: Option<nola_core::agent::ToolInvocationContext>,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        let _sop_invocation_scope = crate::tools::enter_sop_invocation_scope(sop_invocation);
        let permission_profile = match &request.payload {
            HostRequest::ExecuteTool { permission_profile, .. } => *permission_profile,
            _ => return Err("tool dispatch requires an admitted ExecuteTool effect".into()),
        };
        let _admitted_policy = crate::tools::enter_execution_permission_profile(Some(permission_profile));
        let mut sequence = Self::response_sequence(request);
        let plugin_call = self.plugin_mcp_session.as_ref().is_some_and(|session| {
            let reference = self.mcp_tool_routes.get(capability_id)
                .map(|route| route.connection_ref.as_str())
                .or_else(|| {
                    [MCP_RESOURCE_READ_CAPABILITY_ID, MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID,
                        MCP_PROMPT_GET_CAPABILITY_ID].contains(&capability_id)
                        .then(|| input.get("server_id").and_then(Value::as_str)
                            .and_then(|id| self.capability_catalog.mcp.servers.get(id))
                            .map(|server| server.connection_ref.as_str())).flatten()
                });
            reference.is_some_and(|reference| session.contains_connection(reference))
        });
        if plugin_call {
            if let Err(error) = self.reconcile_plugin_mcp() {
                Self::emit_failure(request, &mut sequence, "plugin_authority_unavailable",
                    error, true, emit)?;
                return Ok(());
            }
        }
        let Some(capability) = self.catalog.resolve_available(capability_id) else {
            Self::emit_failure(
                request,
                &mut sequence,
                "capability_unavailable",
                format!("Host capability is unavailable: {capability_id}"),
                false,
                emit,
            )?;
            return Ok(());
        };
        let effect_declaration = capability.input_normalization.effect.clone();
        let input_normalization = capability.input_normalization.clone();
        if let Err(error) = capability.validate_input(input) {
            Self::emit_failure(request,&mut sequence,"invalid_tool_input",error,false,emit)?;
            return Ok(());
        }
        let checked = {
            let grant = match &request.payload {HostRequest::ExecuteTool {grant,..} => grant.as_ref(),_ => None};
            crate::tools::validate_permission_resource_execution(&self.tool_context,
                &input_normalization.permission_contract,&request.host_id.0,input,grant)
        };
        let browser_expectation = match checked {
            Ok(expected) => expected,
            Err(error) => {
                let outcome = crate::tools::permission_resource_execution_failure(error);
                Self::emit(request,&mut sequence,HostResponse::ToolCompleted {
                    call_id:call_id.into(),output:serde_json::to_value(tool_result_contract(&outcome,None)).map_err(|error|error.to_string())?,
                    outcome:HostToolOutcome::Failed,activation:None,full_output_ref:None,artifacts:Vec::new(),
                },emit)?;
                return Ok(());
            }
        };
        let _permission_resource_guard = crate::tools::enter_permission_resource_expectations(browser_expectation);
        if capability_id == nola_domain::generative_ui_tool::UI_TOOL_ID {
            let publish = self.ui_publisher.as_ref().ok_or("UI publisher is unavailable")?;
            let delivery = request.delivery.clone().ok_or("UI publication requires a durable delivery lease")?;
            let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?.as_millis();
            let result = publish(nola_application::generative_ui::ExecuteUiTool {
                effect_id: request.effect_id.0.clone(), request_id: request.request_id.clone(), delivery,
                now_ms: i64::try_from(now_ms).map_err(|_| "UI publication clock overflow")?,
            });
            let (output, outcome) = match result {
                Ok(output) => (output, HostToolOutcome::Succeeded),
                // No terminal Host response: retain the pending effect and exact
                // command receipt for existing reconciliation, never invite a
                // second model publication after a possibly committed mutation.
                Err(error) if error.requires_reconciliation() => return Err(error.to_string()),
                Err(error) => (serde_json::json!({"error":"ui_publication_failed",
                    "message":error.to_string(),"command_error":error}), HostToolOutcome::Failed),
            };
            Self::emit(request, &mut sequence, HostResponse::ToolCompleted {
                call_id: call_id.into(), output, outcome, activation: None, full_output_ref: None, artifacts: Vec::new(),
            }, emit)?;
            return Ok(());
        }

        let Some(mut args) = input.as_object().cloned() else {
            Self::emit_failure(
                request,
                &mut sequence,
                "invalid_tool_input",
                "tool input must be an object",
                false,
                emit,
            )?;
            return Ok(());
        };
        normalize_host_tool_args(&input_normalization, &mut args);
        // Register the exact effect before hooks and Code Memory preflight.
        // Cold/parallel cancellation must also reach initialization and lock waits.
        let invocation_cancellation = self.tool_context.cancellation.child();
        self.active_effect_cancellations.lock().map_err(|_| "tool cancellation registry poisoned")?
            .insert(request.effect_id.0.clone(), invocation_cancellation.context());
        let _invocation_cancellation_guard = ActiveEffectCancellationGuard {
            effect_id: request.effect_id.0.clone(), registry: std::sync::Arc::clone(&self.active_effect_cancellations),
        };

        if capability_id == crate::mcp_connection_authority::CONNECT_CAPABILITY {
            return self.execute_mcp_connection(request,call_id,&Value::Object(args),&mut sequence,emit);
        }
        if [MCP_RESOURCE_READ_CAPABILITY_ID,MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID,MCP_PROMPT_GET_CAPABILITY_ID].contains(&capability_id) {
            let owned = args.get("server_id").and_then(Value::as_str)
                .and_then(|id|self.capability_catalog.mcp.servers.get(id))
                .and_then(|server|crate::mcp_connection_authority::permission_resource(&server.connection_ref).ok());
            if !owned.is_some_and(|resource|resource.host_id == request.host_id.0 && resource.session_id == request.session_id
                && resource.generation.is_some()) {
                Self::emit(request,&mut sequence,HostResponse::ToolCompleted {call_id:call_id.into(),
                    output:external_tool_result(json!({"error":"mcp_connection_required","message":"Connect this owner target before reading MCP resources"}),
                        nola_host_protocol::tool_result::ExecutionState::NotStarted),outcome:HostToolOutcome::Failed,
                    activation:None,full_output_ref:None,artifacts:Vec::new()},emit)?;
                return Ok(());
            }
        }
        // ExecuteTool arrives only after Core authorization. The typed dispatch
        // authority is passed out-of-band, never injected into model arguments.
        let discovery_catalog = if capability_id == "tool_search" {
            let mut available = self
                .current_turn_available_tool_ids
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            available.sort_unstable();
            let dynamic_tool_schemas = self.current_turn_discovery_tool_schemas();
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let max_results = args.get("max_results").and_then(Value::as_u64).unwrap_or(5) as usize;
            let ranked = self
                .catalog
                .search_available_tools(query, max_results)
                .into_iter()
                .map(|entry| entry.capability_id)
                .collect();
            Some(crate::tools::ToolDiscoveryCatalog {
                available: available.into_iter().collect(), ranked, schemas: dynamic_tool_schemas,
            })
        } else { None };
        let failure_key = deterministic_tool_failure_key(capability_id, &args);
        if self
            .current_turn_deterministic_tool_failures
            .get(&failure_key)
            .copied()
            .unwrap_or_default()
            >= 3
        {
            Self::emit_failure(
                request,
                &mut sequence,
                "repeated_deterministic_tool_failure",
                format!(
                    "The model repeated the same invalid {capability_id} call after three deterministic failures"
                ),
                false,
                emit,
            )?;
            return Ok(());
        }
        self.run_lifecycle_hooks(
            request,
            &mut sequence,
            nola_core::lifecycle_hooks::LifecycleHookEventName::PreToolUse,
            None,
            Some(capability_id),
            Some(call_id),
            false,
            emit,
        )?;
        if self.code_memory_workspace.is_some()
            && self.configured_tool_allowlist.as_ref()
                .is_none_or(|allowed| allowed.contains("codebase_index"))
        {
            if self.code_intelligence_provider.is_none() {
                self.code_memory_initialization = Default::default();
            }
            let root = self.code_memory_workspace.as_ref()
                .or_else(|| self.tool_context.allowed_dirs.first());
            if let Some(mut preflight) = root.and_then(|root|
                self.code_memory_initialization.preflight(
                    capability_id, &Value::Object(args.clone()), root,
                    &|| invocation_cancellation.is_requested(),
                    self.code_intelligence_provider.as_deref(),
                ))
            {
                if self.code_intelligence_provider.is_none() {
                    preflight["error"] = json!("code_memory_unavailable");
                    preflight["message"] = json!("Code memory initialization is required for this scope, but the native provider is unavailable. Report this initialization failure; the repository operation has not run.");
                    preflight.as_object_mut().unwrap().remove("recovery");
                }
                preflight["_nola_display"] = json!(match preflight["error"].as_str() {
                    Some("code_memory_initialization_required") => "Code index needs initialization or refresh; this search has not run",
                    Some("code_memory_unavailable") => "Code index unavailable; this search has not run",
                    _ => "Code index readiness check failed; this search has not run",
                });
                preflight["guidance"] = preflight["message"].clone();
                preflight["message"] = preflight["_nola_display"].clone();
                self.current_turn_code_memory_assessment = Some(preflight.clone());
                self.run_tool_completion_hook(request, &mut sequence, capability_id, call_id, true, emit)?;
                Self::emit(request, &mut sequence, HostResponse::ToolCompleted {
                    call_id: call_id.into(),
                    output: external_tool_result(preflight,
                        nola_host_protocol::tool_result::ExecutionState::NotStarted),
                    outcome: HostToolOutcome::Failed,
                    activation: None,
                    full_output_ref: None,
                    artifacts: Vec::new(),
                }, emit)?;
                return Ok(());
            }
        }
        if let Some((context_key, context_args, outcome, targets)) =
            repository_instruction_preflight(capability_id, &args, &self.tool_context)
        {
            let has_instructions = !outcome.display.starts_with("Instruction context (0 file");
            if outcome.error
                || (has_instructions && self.current_turn_instruction_contexts.insert(context_key))
            {
                self.run_tool_completion_hook(
                    request,
                    &mut sequence,
                    capability_id,
                    call_id,
                    true,
                    emit,
                )?;
                Self::emit(
                    request,
                    &mut sequence,
                    HostResponse::ToolCompleted {
                        call_id: call_id.to_string(),
                        output: external_tool_result(
                            json!({
                                "error": if outcome.error {
                                    "repository_instruction_preflight_failed"
                                } else {
                                    "repository_instructions_loaded"
                                },
                                "message": if outcome.error {
                                    "The file change was not executed because applicable repository instructions could not be resolved."
                                } else {
                                    "The file change was not executed. Review these applicable repository instructions, then retry the concrete operation if it still complies."
                                },
                                "target_paths": targets,
                                "instruction_context": outcome.text,
                                "instruction_request": context_args,
                            }),
                            nola_host_protocol::tool_result::ExecutionState::NotStarted,
                        ),
                        outcome: HostToolOutcome::Failed,
                        activation: None,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    },
                    emit,
                )?;
                return Ok(());
            }
        }
        let progress_title = self
            .mcp_tool_routes
            .get(capability_id)
            .map(|route| format!("{} / {}", route.server_name, route.tool_name))
            .unwrap_or_else(|| capability_id.to_string());
        Self::emit(
            request,
            &mut sequence,
            HostResponse::ToolCallProgress {
                call_id: call_id.to_string(),
                channel: nola_core::agent::ToolProgressChannel::Status,
                title: Some(progress_title),
                detail: Some("running".to_string()),
                delta: None,
                metadata: json!({"call_id": call_id}),
            },
            emit,
        )?;
        let resolved_effect = effect_declaration.resolve(&Value::Object(args.clone()));
        if let (Some(provider), Some(operation)) = (
            self.code_intelligence_provider.clone(),
            nola_core::code_intelligence::operation_for_capability_id(capability_id),
        ) {
            let transaction = match prepare_effect_work_step_transaction(
                &ToolRequest {
                    id: call_id.to_string(),
                    name: capability_id.to_string(),
                    args: args.clone(),
                },
                &self.tool_context,
                resolved_effect,
                crate::tools::tool_transaction_boundary(capability_id),
            ) {
                Ok(transaction) => transaction,
                Err(outcome) => {
                    self.emit_direct_transaction_outcome(
                        request,
                        &mut sequence,
                        capability_id,
                        call_id,
                        &args,
                        outcome,
                        emit,
                    )?;
                    return Ok(());
                }
            };
            let cancellation = invocation_cancellation.clone();
            if let Ok(mut cancellations) = self.active_effect_cancellations.lock() {
                cancellations.insert(request.effect_id.0.clone(), cancellation.context());
            }
            let _cancellation_guard = ActiveEffectCancellationGuard {
                effect_id: request.effect_id.0.clone(),
                registry: std::sync::Arc::clone(&self.active_effect_cancellations),
            };
            // Resource admission above validated this exact index call, including
            // sensitive descendants. The launch directory is not an authorization boundary.
            // Limit this provider invocation to the admitted root; do not persist a grant.
            let mut index_roots = self.tool_context.allowed_dirs.clone();
            if operation == nola_core::code_intelligence::CodeIntelligenceOperation::Index {
                if let Some(path) = args.get("repo_path").and_then(Value::as_str) {
                    let path = std::path::PathBuf::from(path);
                    let path = if path.is_absolute() { path } else {
                        self.tool_context.allowed_dirs.first().cloned().unwrap_or_default().join(path)
                    };
                    if let Ok(root) = path.canonicalize() { index_roots = vec![root]; }
                }
            }
            let mut execution = provider.execute(
                crate::code_intelligence_provider::CodeIntelligenceCall {
                    operation,
                    arguments: Value::Object(args.clone()),
                    workspace_roots: index_roots,
                },
                &|| cancellation.is_requested(),
            );
            self.code_memory_initialization.observe(operation, &execution);
            // A successful index makes its read-only navigation schemas usable
            // on the next model step. This is typed Host discovery, not a tool
            // call, permission grant, or a routing decision based on user text.
            let activation = if operation == nola_core::code_intelligence::CodeIntelligenceOperation::Index
                && !execution.is_error
            {
                use nola_core::code_intelligence::CodeIntelligenceOperation as Op;
                let capabilities: Vec<String> = [Op::Architecture, Op::Search, Op::ReadSymbol, Op::Trace, Op::TextSearch]
                    .into_iter().map(|op| op.capability_id().to_owned())
                    .filter(|id| self.catalog.capabilities.contains_key(id)
                        && self.configured_tool_allowlist.as_ref().is_none_or(|allowed| allowed.contains(id)))
                    .collect();
                if let Some(result) = execution.value.as_object_mut() {
                    result.insert("navigation".into(), json!({
                        "project": result.get("project").cloned(),
                        "available_capabilities": capabilities,
                        "guidance": "Use the returned project with codebase_search to locate symbol names; codebase_architecture can orient unfamiliar structure. Read an exact qualified_name with codebase_read_symbol and follow callers/callees with codebase_trace. Use codebase_text_search for literals and comments. Once a relevant definition or caller is found, inspect that source rather than guessing more names across the repository. These schemas are exposed on the next model step when listed here; they do not grant resource access."
                    }));
                }
                (!capabilities.is_empty()).then_some(nola_core::agent::host::HostCapabilityActivation {
                    source: "code_memory".into(), capability_ids: capabilities,
                    capability_descriptors: Vec::new(),
                })
            } else { None };
            if operation == nola_core::code_intelligence::CodeIntelligenceOperation::DeleteIndex
                && !execution.is_error
            {
                self.current_turn_code_memory_assessment = None;
            }
            if operation == nola_core::code_intelligence::CodeIntelligenceOperation::Index {
                self.current_turn_code_memory_assessment = Some(json!({
                    "state": if execution.is_error { "initialization_failed" } else { "ready" },
                    "index": execution.value,
                    "guidance": if execution.is_error {
                        "Initialization failed. Correct the cause before retrying; repository analysis remains blocked."
                    } else {
                        "Initialization completed for the reported root. Use indexed architecture and symbols to locate code, and verify current source before editing."
                    }
                }));
            }
            if let Some(workspace) = self.tool_context.allowed_dirs.first() {
                crate::codebase_memory::attach_result_provenance(
                    capability_id,
                    workspace,
                    &mut execution.value,
                );
            }
            let display = if execution.is_error {
                format!("{capability_id}: {}", execution.value.get("message")
                    .or_else(|| execution.value.get("error")).and_then(Value::as_str)
                    .unwrap_or("Operation failed"))
            } else if operation == nola_core::code_intelligence::CodeIntelligenceOperation::Index {
                format!("Code index ready: {} updated, {} unchanged, {} removed",
                    execution.value["indexed_files"], execution.value["unchanged_files"],
                    execution.value["removed_files"])
            } else { format!("Nola Code Intelligence / {capability_id}") };
            let (output, full_output_ref) = bound_external_tool_output_in_store(
                execution.value,
                &self.tool_context.output_store,
            );
            let outcome = finish_tool_work_step_transaction(
                transaction,
                ToolOutcome {
                    full_output_ref: None,
                    data: None,
                    execution: None,
                    text: output.to_string(),
                    display,
                    error: execution.is_error,
                    artifacts: Vec::new(),
                    activation,
                },
            );
            self.emit_direct_transaction_outcome_with_ref(
                request,
                &mut sequence,
                capability_id,
                call_id,
                &args,
                outcome,
                full_output_ref,
                emit,
            )?;
            return Ok(());
        }
        if let Some(route) = self.mcp_tool_routes.get(capability_id).cloned() {
            let owned = crate::mcp_connection_authority::permission_resource(&route.connection_ref).ok();
            let current_schema = self.capability_catalog.mcp.servers.get(&route.server_id)
                .and_then(|server|server.tools.get(&route.tool_name)).map(mcp_tool_model_input_schema);
            if !owned.is_some_and(|resource|resource.host_id == request.host_id.0 && resource.session_id == request.session_id
                && resource.generation.is_some()) || current_schema.as_ref() != self.catalog.capabilities.get(capability_id).map(|capability|&capability.input_schema) {
                Self::emit(request,&mut sequence,HostResponse::ToolCompleted {call_id:call_id.into(),
                    output:external_tool_result(json!({"error":"mcp_connection_or_catalog_required","message":"Connect the current owner and refresh changed schemas before calling this tool"}),
                        nola_host_protocol::tool_result::ExecutionState::NotStarted),outcome:HostToolOutcome::Failed,
                    activation:None,full_output_ref:None,artifacts:Vec::new()},emit)?;
                return Ok(());
            }
            let transaction = match prepare_effect_work_step_transaction(
                &ToolRequest {
                    id: call_id.to_string(),
                    name: capability_id.to_string(),
                    args: args.clone(),
                },
                &self.tool_context,
                resolved_effect,
                crate::tools::tool_transaction_boundary(capability_id),
            ) {
                Ok(transaction) => transaction,
                Err(outcome) => {
                    self.emit_direct_transaction_outcome(
                        request,
                        &mut sequence,
                        capability_id,
                        call_id,
                        &args,
                        outcome,
                        emit,
                    )?;
                    return Ok(());
                }
            };
            let Some(supervisor_config) = self.execution_settings.as_ref() else {
                let outcome = finish_tool_work_step_transaction(
                    transaction,
                    ToolOutcome {
                        full_output_ref: None,
                        data: None,
                        execution: None,
                        text: json!({
                            "error": "mcp_supervisor_unavailable",
                            "message": "MCP calls require the configured Host Supervisor"
                        })
                        .to_string(),
                        display: format!("{} / {}", route.server_name, route.tool_name),
                        error: true,
                        artifacts: Vec::new(),
                        activation: None,
                    },
                );
                self.emit_direct_transaction_outcome(
                    request,
                    &mut sequence,
                    capability_id,
                    call_id,
                    &args,
                    outcome,
                    emit,
                )?;
                return Ok(());
            };
            let mut lease = match crate::task_runtime::McpExecutionTaskLease::acquire(
                &supervisor_config.external.project_root,
                Some(request.session_id.clone()),
                &request.effect_id.0,
                &route.server_id,
                &route.tool_name,
                resolved_effect == nola_core::agent::host::HostCapabilityEffect::Investigate,
                &Value::Object(args.clone()),
                &self.tool_context.cancellation,
            ) {
                Ok(lease) => lease,
                Err(error) => {
                    let outcome = finish_tool_work_step_transaction(
                        transaction,
                        ToolOutcome {
                            full_output_ref: None,
                            data: None,
                            execution: None,
                            text: json!({
                                "error": "mcp_supervisor_failed",
                                "message": error
                            })
                            .to_string(),
                            display: format!("{} / {}", route.server_name, route.tool_name),
                            error: true,
                            artifacts: Vec::new(),
                            activation: None,
                        },
                    );
                    self.emit_direct_transaction_outcome(
                        request,
                        &mut sequence,
                        capability_id,
                        call_id,
                        &args,
                        outcome,
                        emit,
                    )?;
                    return Ok(());
                }
            };
            let execution = crate::mcp_host::call_tool_with_cancellation_and_progress(
                route.transport,
                &route.connection_ref,
                &route.tool_name,
                Value::Object(args.clone()),
                &self.tool_context.cancellation,
                &mut |progress| {
                    let _ = lease.record_progress(progress);
                },
            );
            let lease_commit = match &execution {
                Ok(execution) if execution.is_error => {
                    lease.finish_failed(&execution.output.to_string())
                }
                Ok(_) => lease.finish_completed(),
                Err(error) if error.code == "mcp_cancelled_before_send" => {
                    lease.finish_failed(&error.message)
                }
                Err(error) => lease.finish_reconciliation(&error.message),
            };
            let execution_state = match &execution {
                Ok(result) if !result.is_error => nola_host_protocol::tool_result::ExecutionState::Completed,
                Err(error) if matches!(error.code,"mcp_cancelled_before_send" | "mcp_connection_required"
                    | "mcp_connection_unavailable" | "mcp_connection_retired" | "mcp_transport_unsupported") =>
                    nola_host_protocol::tool_result::ExecutionState::NotStarted,
                _ => nola_host_protocol::tool_result::ExecutionState::Unknown,
            };
            let (output, is_error, full_output_ref, artifacts) = match execution {
                Ok(execution) => {
                    let artifacts = execution.artifacts;
                    let (output, full_output_ref) = bound_external_tool_output_in_store(
                        execution.output,
                        &self.tool_context.output_store,
                    );
                    (output, execution.is_error, full_output_ref, artifacts)
                }
                Err(error) => (
                    json!({
                        "error": error.code,
                        "message": error.message,
                        "server_id": route.server_id,
                        "tool_name": route.tool_name,
                    }),
                    true,
                    None,
                    Vec::new(),
                ),
            };
            let (output, is_error) = match lease_commit {
                Ok(()) => (output, is_error),
                Err(error) => (
                    json!({
                        "error": "mcp_supervisor_commit_failed",
                        "message": error,
                        "transport_output": output
                    }),
                    true,
                ),
            };
            let outcome = finish_tool_work_step_transaction(
                transaction,
                ToolOutcome {
                    full_output_ref: None,
                    data: None,
                    execution: Some(nola_host_protocol::tool_result::ExecutionReceipt {
                        state:execution_state,..Default::default()
                    }),
                    text: output.to_string(),
                    display: format!("{} / {}", route.server_name, route.tool_name),
                    error: is_error,
                    artifacts,
                    activation: None,
                },
            );
            self.emit_direct_transaction_outcome_with_ref(
                request,
                &mut sequence,
                capability_id,
                call_id,
                &args,
                outcome,
                full_output_ref,
                emit,
            )?;
            return Ok(());
        }
        if capability_id == MCP_RESOURCE_READ_CAPABILITY_ID {
            let execution =
                serde_json::from_value::<McpResourceReadRequest>(Value::Object(args.clone()))
                    .map_err(|error| {
                        mcp_invalid_input("mcp_resource_request_invalid", error.to_string())
                    })
                    .and_then(|request| {
                        resolve_mcp_resource_route(&self.capability_catalog.mcp, &request)
                    })
                    .and_then(|route| {
                        crate::mcp_host::read_resource(
                            route.transport,
                            &route.connection_ref,
                            &route.uri,
                        )
                        .map_err(|error| {
                            json!({
                                "error": error.code,
                                "message": error.message,
                                "server_id": route.server_id,
                                "resource_id": route.resource_id,
                            })
                        })
                    });
            self.run_tool_completion_hook(
                request,
                &mut sequence,
                capability_id,
                call_id,
                execution.is_err(),
                emit,
            )?;
            emit_mcp_catalog_completion(
                request,
                &mut sequence,
                call_id,
                execution,
                &self.tool_context.output_store,
                emit,
            )?;
            return Ok(());
        }
        if capability_id == MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID {
            let execution = serde_json::from_value::<McpResourceTemplateReadRequest>(
                Value::Object(args.clone()),
            )
            .map_err(|error| {
                mcp_invalid_input("mcp_resource_template_request_invalid", error.to_string())
            })
            .and_then(|request| {
                resolve_mcp_resource_template_route(&self.capability_catalog.mcp, request)
            })
            .and_then(|route| {
                crate::mcp_host::read_resource(route.transport, &route.connection_ref, &route.uri)
                    .map_err(|error| {
                        json!({
                            "error": error.code,
                            "message": error.message,
                            "server_id": route.server_id,
                            "template_id": route.template_id,
                            "uri": route.uri,
                        })
                    })
            });
            self.run_tool_completion_hook(
                request,
                &mut sequence,
                capability_id,
                call_id,
                execution.is_err(),
                emit,
            )?;
            emit_mcp_catalog_completion(
                request,
                &mut sequence,
                call_id,
                execution,
                &self.tool_context.output_store,
                emit,
            )?;
            return Ok(());
        }
        if capability_id == MCP_PROMPT_GET_CAPABILITY_ID {
            let execution =
                serde_json::from_value::<McpPromptGetRequest>(Value::Object(args.clone()))
                    .map_err(|error| {
                        mcp_invalid_input("mcp_prompt_request_invalid", error.to_string())
                    })
                    .and_then(|request| {
                        resolve_mcp_prompt_route(&self.capability_catalog.mcp, request)
                    })
                    .and_then(|route| {
                        crate::mcp_host::get_prompt(
                            route.transport,
                            &route.connection_ref,
                            &route.prompt_name,
                            route.arguments,
                        )
                        .map_err(|error| {
                            json!({
                                "error": error.code,
                                "message": error.message,
                                "server_id": route.server_id,
                                "prompt_name": route.prompt_name,
                            })
                        })
                    });
            self.run_tool_completion_hook(
                request,
                &mut sequence,
                capability_id,
                call_id,
                execution.is_err(),
                emit,
            )?;
            emit_mcp_catalog_completion(
                request,
                &mut sequence,
                call_id,
                execution,
                &self.tool_context.output_store,
                emit,
            )?;
            return Ok(());
        }
        if self.runtime_tool_ids.contains(capability_id) {
            // Durable runtime capabilities outlive the generic work-step
            // transaction that launched them. Reconcile a redelivered effect
            // against the durable task before transaction recovery can replay
            // the original ToolCompleted outcome. Core already completed that
            // tool call when it accepted TaskPromoted, so the only valid replay
            // response is the task boundary update.
            if let Some(update) = (self.runtime_tool_replay_handler)(
                request,
                capability_id,
                call_id,
                &args,
                &self.tool_context,
            ) {
                Self::emit(
                    request,
                    &mut sequence,
                    HostResponse::TaskUpdated {
                        task_id: update.task_id,
                        state: update.state,
                        progress: update.progress,
                        current_step: update.current_step,
                        result_ref: update.result_ref,
                        cancel_requested: update.cancel_requested,
                        ownership_token: update.ownership_token,
                        completion_watermark: update.completion_watermark,
                    },
                    emit,
                )?;
                return Ok(());
            }
            let runtime_transaction = match prepare_effect_work_step_transaction(
                &ToolRequest {
                    id: call_id.to_string(),
                    name: capability_id.to_string(),
                    args: args.clone(),
                },
                &self.tool_context,
                resolved_effect,
                crate::tools::tool_transaction_boundary(capability_id),
            ) {
                Ok(transaction) => transaction,
                Err(outcome) => {
                    self.emit_direct_transaction_outcome(
                        request,
                        &mut sequence,
                        capability_id,
                        call_id,
                        &args,
                        outcome,
                        emit,
                    )?;
                    return Ok(());
                }
            };
            let mut runtime_context = self.tool_context.clone();
            runtime_context.cancellation = invocation_cancellation.clone();
            if let Ok(mut cancellations) = self.active_effect_cancellations.lock() {
                cancellations.insert(
                    request.effect_id.0.clone(),
                    runtime_context.cancellation.context(),
                );
            }
            let _cancellation_guard = ActiveEffectCancellationGuard {
                effect_id: request.effect_id.0.clone(),
                registry: std::sync::Arc::clone(&self.active_effect_cancellations),
            };
            let Some(mut runtime_result) = (self.runtime_tool_handler)(
                request,
                capability_id,
                call_id,
                &args,
                &runtime_context,
            ) else {
                let outcome = finish_tool_work_step_transaction(
                    runtime_transaction,
                    ToolOutcome {
                        full_output_ref: None,
                        data: None,
                        execution: None,
                        text: "The registered runtime capability had no accepting Host handler."
                            .to_string(),
                        display: "Capability route was not accepted".to_string(),
                        error: true,
                        artifacts: Vec::new(),
                        activation: None,
                    },
                );
                self.emit_direct_transaction_outcome(
                    request,
                    &mut sequence,
                    capability_id,
                    call_id,
                    &args,
                    outcome,
                    emit,
                )?;
                return Ok(());
            };
            runtime_result.outcome =
                finish_tool_work_step_transaction(runtime_transaction, runtime_result.outcome);
            let RuntimeToolResult {
                outcome,
                task_promotion,
                task_update,
            } = runtime_result;
            if let Some(update) = task_update {
                Self::emit(
                    request,
                    &mut sequence,
                    HostResponse::TaskUpdated {
                        task_id: update.task_id,
                        state: update.state,
                        progress: update.progress,
                        current_step: update.current_step,
                        result_ref: update.result_ref,
                        cancel_requested: update.cancel_requested,
                        ownership_token: update.ownership_token,
                        completion_watermark: update.completion_watermark,
                    },
                    emit,
                )?;
                return Ok(());
            }
            let full_output_ref = full_output_ref_from_outcome(&outcome);
            self.record_tool_outcome(&failure_key, &outcome);
            if !outcome.error
                && matches!(
                    capability_id,
                    "Skill" | "skill_invoke" | "skill_skip" | "SOP" | "sop_invoke"
                )
            {
                self.current_turn_skill_decided = true;
            }
            self.run_tool_completion_hook(
                request,
                &mut sequence,
                capability_id,
                call_id,
                outcome.error,
                emit,
            )?;
            if let Some(promotion) = task_promotion.filter(|_| !outcome.error) {
                Self::emit(
                    request,
                    &mut sequence,
                    HostResponse::TaskPromoted {
                        task_id: promotion.task_id,
                        input_watermark: promotion.input_watermark,
                        kind: promotion.kind,
                        state: promotion.state,
                        progress: promotion.progress,
                        current_step: promotion.current_step,
                        lineage: promotion.lineage,
                    },
                    emit,
                )?;
                return Ok(());
            }
            let (artifacts, media_failures) = host_artifacts_for_tool(capability_id, &args, &outcome, &self.tool_context);
            let mut result = tool_result_contract(&outcome, None);
            annotate_media_publication_failures(&mut result, media_failures);
            let output = result.clone().into_core_output();
            let activation = self.materialize_capability_activation(
                capability_id,
                &output,
                outcome.activation.clone(),
            );
            Self::emit(
                request,
                &mut sequence,
                HostResponse::ToolCompleted {
                    call_id: call_id.to_string(),
                    output: serde_json::to_value(result).expect("tool result serializes"),
                    outcome: if outcome.error {
                        HostToolOutcome::Failed
                    } else {
                        successful_tool_completion(capability_id, &outcome)
                    },
                    activation,
                    full_output_ref,
                    artifacts,
                },
                emit,
            )?;
            return Ok(());
        }
        let mut context = self.tool_context.clone();
        context.memory_binding.conversation_id = Some(request.session_id.clone());
        context.memory_binding.turn_id = request.turn_id.clone();
        context.memory_binding.validate()?;
        // Core has already admitted this exact operation and its resource facts.
        // Preserve that decision through the one-shot marker, without enabling
        // automatic approval for subsequent operations. The cwd is not a sandbox.
        context.policy = context.policy.without_auto_approval();
        context.cancellation = invocation_cancellation.clone();
        if let Ok(mut cancellations) = self.active_effect_cancellations.lock() {
            cancellations.insert(request.effect_id.0.clone(), context.cancellation.context());
        }
        crate::llm::log_local_debug_payload(
            "host_tool_dispatch",
            &json!({
                "type": "host_tool_dispatch",
                "phase": "execute_started",
                "effect_id": request.effect_id.0,
                "request_id": request.request_id,
                "call_id": call_id,
                "capability_id": capability_id,
                "response_sequence": sequence,
            }),
        );
        // Capture the exact resource identity before executing this approved
        // invocation. A different root in the returned data cannot widen it.
        let approved_workspace_root = (capability_id == "workspace_access")
            .then(|| args.get("root").and_then(Value::as_str))
            .flatten()
            .filter(|root| std::path::Path::new(root).is_absolute())
            .and_then(|root| std::path::Path::new(root).canonicalize().ok())
            .filter(|root| root.is_dir());
        let mut outcome = crate::tools::execute_authorized_tool_with_catalog(
            &ToolRequest {
                id: call_id.to_string(),
                name: capability_id.to_string(),
                args: args.clone(),
            },
            &context,
            crate::tools::ExecutionAuthority::CoreApproved(permission_profile),
            discovery_catalog.as_ref(),
        );
        crate::llm::log_local_debug_payload(
            "host_tool_dispatch",
            &json!({
                "type": "host_tool_dispatch",
                "phase": "execute_finished",
                "effect_id": request.effect_id.0,
                "request_id": request.request_id,
                "call_id": call_id,
                "capability_id": capability_id,
                "response_sequence": sequence,
                "outcome_error": outcome.error,
            }),
        );
        if capability_id == "instruction_context" && !outcome.error {
            self.current_turn_instruction_contexts
                .insert(instruction_context_key(&args, &outcome.text));
        }
        // The dedicated approved capability owns directory expansion. Generic
        // operation approvals and arbitrary tool output cannot widen roots.
        if capability_id == "workspace_access" && !outcome.error {
            let directory = approved_workspace_root.as_deref()
                .ok_or_else(|| "Workspace access invocation has no valid approved directory".to_string())
                .and_then(|root| self.apply_workspace_access_result(root, &outcome));
            match directory {
                Ok(()) => {}
                Err(message) => {
                    let data = json!({
                        "error": "workspace_access_result_invalid",
                        "message": message,
                        "tool_execution_skipped": true,
                    });
                    outcome.text = data.to_string();
                    outcome.data = Some(data);
                    outcome.display = "Workspace access result rejected".to_string();
                    outcome.error = true;
                    outcome.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
                        state: nola_host_protocol::tool_result::ExecutionState::NotStarted,
                        ..Default::default()
                    });
                }
            }
        }
        if let Ok(mut cancellations) = self.active_effect_cancellations.lock() {
            cancellations.remove(&request.effect_id.0);
        }
        self.record_tool_outcome(&failure_key, &outcome);
        if !outcome.error
            && matches!(
                capability_id,
                "Skill" | "skill_invoke" | "skill_skip" | "SOP" | "sop_invoke"
            )
        {
            self.current_turn_skill_decided = true;
            if matches!(capability_id, "Skill" | "skill_invoke") {
                if args.get("skill").and_then(Value::as_str).is_some() {
                    // Only the successful materialization supplies its actual
                    // resource directory. An explicit-root Runtime must never
                    // expand permissions through a same-named managed Skill.
                    let directory = skill_resource_directory_from_outcome(&outcome);
                    if let Some(directory) = directory {
                        if !self.tool_context.allowed_dirs.contains(&directory) {
                            self.tool_context.allowed_dirs.push(directory);
                        }
                    }
                }
            }
        }
        if capability_id == "project_overview" {
            self.current_turn_project_overview_attempted = true;
            if !outcome.error {
                self.current_turn_project_overview_file_count =
                    project_overview_file_count_from_text(&outcome.text);
            }
        }
        let full_output_ref = full_output_ref_from_outcome(&outcome);
        let memory_usage = memory_usage_metadata(capability_id, &outcome);
        let (artifacts, media_failures) = host_artifacts_for_tool(capability_id, &args, &outcome, &self.tool_context);
        let mut result = tool_result_contract(&outcome, Some(memory_usage));
        annotate_media_publication_failures(&mut result, media_failures);
        let output = result.clone().into_core_output();
        let activation = self.materialize_capability_activation(
            capability_id,
            &output,
            outcome.activation.clone(),
        );
        self.run_tool_completion_hook(
            request,
            &mut sequence,
            capability_id,
            call_id,
            outcome.error,
            emit,
        )?;
        crate::llm::log_local_debug_payload(
            "host_tool_dispatch",
            &json!({
                "type": "host_tool_dispatch",
                "phase": "terminal_response_emitting",
                "effect_id": request.effect_id.0,
                "request_id": request.request_id,
                "call_id": call_id,
                "capability_id": capability_id,
                "response_sequence": sequence,
            }),
        );
        Self::emit(
            request,
            &mut sequence,
            HostResponse::ToolCompleted {
                call_id: call_id.to_string(),
                output: serde_json::to_value(result).expect("tool result serializes"),
                outcome: if context.cancellation.is_requested() {
                    HostToolOutcome::Cancelled
                } else if outcome.error {
                    HostToolOutcome::Failed
                } else {
                    successful_tool_completion(capability_id, &outcome)
                },
                activation,
                full_output_ref,
                artifacts,
            },
            emit,
        )?;
        Ok(())
    }

    fn apply_workspace_access_result(
        &mut self,
        approved_root: &std::path::Path,
        outcome: &ToolOutcome,
    ) -> Result<(), String> {
        let root = workspace_access_directory_from_outcome(approved_root, outcome)?;
        for roots in [
            &mut self.tool_context.allowed_dirs,
            &mut self.base_allowed_dirs,
        ] {
            if !roots.contains(&root) {
                roots.push(root.clone());
            }
        }
        Ok(())
    }

    fn record_tool_outcome(&mut self, failure_key: &str, outcome: &ToolOutcome) {
        if outcome.error && is_deterministic_tool_failure(&outcome.text) {
            let failures = self
                .current_turn_deterministic_tool_failures
                .entry(failure_key.to_string())
                .or_default();
            *failures = failures.saturating_add(1);
        } else {
            self.current_turn_deterministic_tool_failures
                .remove(failure_key);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_direct_transaction_outcome(
        &mut self,
        request: &HostRequestEnvelope,
        sequence: &mut u64,
        capability_id: &str,
        call_id: &str,
        args: &serde_json::Map<String, Value>,
        outcome: ToolOutcome,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        self.emit_direct_transaction_outcome_with_ref(
            request,
            sequence,
            capability_id,
            call_id,
            args,
            outcome,
            None,
            emit,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_direct_transaction_outcome_with_ref(
        &mut self,
        request: &HostRequestEnvelope,
        sequence: &mut u64,
        capability_id: &str,
        call_id: &str,
        args: &serde_json::Map<String, Value>,
        outcome: ToolOutcome,
        external_full_output_ref: Option<String>,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        self.record_tool_outcome(
            &deterministic_tool_failure_key(capability_id, args),
            &outcome,
        );
        self.run_tool_completion_hook(
            request,
            sequence,
            capability_id,
            call_id,
            outcome.error,
            emit,
        )?;
        let full_output_ref =
            external_full_output_ref.or_else(|| full_output_ref_from_outcome(&outcome));
        let (artifacts, media_failures) = host_artifacts_for_tool(capability_id, args, &outcome, &self.tool_context);
        let mut result = tool_result_contract(&outcome, None);
        annotate_media_publication_failures(&mut result, media_failures);
        result.full_output_ref = full_output_ref.clone();
        let output = result.clone().into_core_output();
        let activation = self.materialize_capability_activation(
            capability_id,
            &output,
            outcome.activation.clone(),
        );
        Self::emit(
            request,
            sequence,
            HostResponse::ToolCompleted {
                call_id: call_id.to_string(),
                output: serde_json::to_value(result).expect("tool result serializes"),
                outcome: if outcome.error {
                    HostToolOutcome::Failed
                } else {
                    successful_tool_completion(capability_id, &outcome)
                },
                activation,
                full_output_ref,
                artifacts,
            },
            emit,
        )?;
        Ok(())
    }

    fn execute_compaction(
        &mut self,
        request: &HostRequestEnvelope,
        instructions: &str,
        messages: &[Value],
        target_tokens: u64,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        Self::measure_host_execution(request, nola_measurement::Stage::HostCompaction, emit, |emit| {
            self.execute_compaction_measured(request, instructions, messages, target_tokens, emit)
        })
    }

    fn execute_compaction_measured(
        &mut self,
        request: &HostRequestEnvelope,
        instructions: &str,
        messages: &[Value],
        target_tokens: u64,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut sampling_request = request.clone();
        let mut sampling_count = 0usize;
        let mut sequence = Self::response_sequence(request);
        let restored = match self.compaction_request_witness.take() {
            Some(witness) => Ok(witness),
            None => CompactionRequestWitness::restore(self.tool_context.output_store.root(), request.session_id.as_str(), &self.llm),
        };
        let snapshot_failure = restored.as_ref().err().map(|error| error.code());
        let witness = restored.ok();
        let summary_window = match &request.payload {
            HostRequest::Compact { context_window_tokens, .. } => *context_window_tokens as usize,
            _ => return Err("compaction requires a Core-admitted window budget".into()),
        };
        let summary_window = witness.as_ref().map_or(summary_window,
            |value| summary_window.min(value.context_window));
        let configuration_identity = crate::llm::request_configuration_identity(&self.llm);
        // Cold-cache summaries still use the last admitted physical model, not
        // a cheaper/default model accidentally selected by the Host bootstrap.
        let mut summary_config = witness
            .as_ref()
            .filter(|witness| {
                witness.session_id == request.session_id
                    && witness.configuration_identity == configuration_identity
            })
            .map_or(&self.llm, |witness| witness.request.config())
            .clone();
        summary_config.cancellation = self.llm.cancellation.clone();
        let fork = witness.ok_or(snapshot_failure.unwrap_or("no_request_snapshot")).and_then(|witness| {
            if witness.session_id != request.session_id.to_string() {
                return Err("session_changed");
            }
            if witness.configuration_identity != configuration_identity {
                return Err("configuration_changed");
            }
            let mut archive = provider_messages_for_wire_protocol(messages, witness.request.config());
            // Locate ranges using the same validated native continuation items
            // as ordinary requests (not a text-only reconstruction of Responses).
            apply_provider_continuation_to_messages(&mut archive, None, witness.request.config());
            witness.request.fork(
                &archive,
                instructions,
                target_tokens,
                summary_window,
                &witness.usage,
                self.llm.cancellation.clone(),
            )
        });
        let mut fallback_reason = fork.as_ref().err().copied();
        let mut fork = fork.ok();
        // Keep the model's configured output allowance. Check actual rolling
        // checkpoint input size; never impose a separate summary byte cap.
        let checkpoint_bytes = 0usize;
        let mut llm = match summary_config.for_compaction() {
            Ok(configured) => configured,
            Err(error) => {
                Self::emit_failure(
                    &sampling_request,
                    &mut sequence,
                    "compaction_budget_invalid",
                    error,
                    false,
                    emit,
                )?;
                return Ok(());
            }
        };
        llm.temperature = 0.2;
        llm.thinking_enabled = false;
        llm.reasoning_selection = None;
        llm.sampling_override = false;
        llm.generation_options = Default::default();
        llm.openai_prompt_cache = false;
        llm.openai_prompt_cache_key = None;
        llm.openai_prompt_cache_key_prefix = None;
        llm.openai_prompt_cache_retention = None;
        // Never silently migrate a maintenance request to another model.
        llm.fallbacks.clear();
        let mut summary_plan = None;
        let mut recovery_attempted = false;
        let mut continuation = None;
        let mut partial_summary = String::new();
        let mut continuation_count = 0usize;
        let mut summary_rewrite_count = 0usize;
        let mut cold_reasoning_resolved = false;
        self.omit_unavailable_model_targets(&mut llm);
        loop {
            if self.llm.cancellation.is_requested() {
                Self::emit_failure(&sampling_request, &mut sequence, "cancelled",
                    "Compaction cancelled; original context retained", false, emit)?;
                break;
            }
            let llm_messages = if fork.is_none() && continuation.is_none() {
                if !cold_reasoning_resolved {
                    crate::llm::configure_cold_compaction_reasoning(&mut llm);
                    cold_reasoning_resolved = true;
                }
                if summary_plan.is_none() {
                    match CompactionSummaryPlan::with_checkpoint_budget(messages, summary_window, llm.max_output_tokens as usize, checkpoint_bytes, instructions) {
                        Ok(plan) => summary_plan = Some(plan),
                        Err(error) => {
                            Self::emit_failure(&sampling_request, &mut sequence, "compaction_budget_invalid", error, true, emit)?;
                            break;
                        }
                    }
                }
                let plan = summary_plan.as_mut().expect("standalone plan prepared");
                loop {
                    match plan.messages(instructions, llm.max_output_tokens) {
                        Ok(messages) => break messages,
                        Err(_) if plan.split_rejected() => continue,
                        Err(error) => {
                            Self::emit_failure(&sampling_request, &mut sequence, "compaction_budget_invalid", error, true, emit)?;
                            return Ok(());
                        }
                    }
                }
            } else { Vec::new() };
            let started = std::time::Instant::now();
            let is_fork = fork.is_some();
            let accepted_summary = |choice: &crate::llm::StreamedChoice| {
                crate::llm::valid_compaction_summary(choice)
            };
            let physical_request = continuation.as_ref().or(fork.as_ref());
            let response_output_limit = physical_request.map_or(llm.max_output_tokens, |value| value.config().max_output_tokens);
            let strategy = if continuation.is_some() {
                "summary_continuation"
            } else if summary_rewrite_count > 0 && is_fork {
                "summary_budget_rewrite"
            } else if is_fork {
                "cached_request_fork"
            } else {
                "incremental_summary"
            };
            let mut admission_error = None;
            let model_target_keys = Self::model_target_keys(&llm);
            let mut newly_unavailable_model_targets = Vec::new();
            let mut on_event = |event| {
                if let LlmEvent::SamplingStarted { provider, model } = event {
                    let replacing = sampling_count > 0 || request.attempt_started;
                    if replacing {
                        sampling_request.attempt_id = Some(uuid::Uuid::new_v4().to_string());
                        if let Some(attempt_id) = &sampling_request.attempt_id {
                            nola_measurement::set_attempt(attempt_id);
                        }
                    } else if sampling_request.attempt_id.is_none() {
                        let error = "prepared compaction Attempt is missing".to_string();
                        admission_error = Some(error.clone());
                        return Err(error);
                    }
                    let accepted = Self::emit(
                        &sampling_request,
                        &mut sequence,
                        HostResponse::ModelAttemptStarted {
                            provider,
                            model,
                            reason: if replacing {
                                "compaction_resampling"
                            } else {
                                "initial"
                            }
                            .into(),
                        },
                        emit,
                    );
                    if let Err(error) = &accepted {
                        admission_error = Some(error.clone());
                    }
                    if accepted.is_ok() {
                        sampling_request.attempt_started = true;
                        sampling_count += 1;
                    }
                    return accepted;
                }
                if let LlmEvent::Retrying {
                    attempt,
                    delay_ms,
                    failure,
                    target_exhausted,
                } = event
                {
                    if target_exhausted && failure.exhausts_target {
                        let failed_index = attempt.saturating_sub(2);
                        if let Some(key) = model_target_keys.get(failed_index) {
                            newly_unavailable_model_targets.push(key.clone());
                        }
                    }
                    let accepted = Self::emit(
                        &sampling_request,
                        &mut sequence,
                        HostResponse::ToolProgress {
                            title: "compaction_retry".into(),
                            detail: Some(format!(
                                "attempt={attempt} delay_ms={delay_ms} reason={}",
                                failure.message
                            )),
                            metadata: json!({
                                "attempt": attempt,
                                "delay_ms": delay_ms,
                                "target_exhausted": target_exhausted,
                                "failure_code": failure.code,
                                "failure_message": failure.message,
                                "retryable": failure.retryable,
                                "provider_status": failure.status,
                                "retry_after_ms": failure.retry_after_ms,
                                "provider": failure.provider,
                                "model": failure.model,
                            }),
                        },
                        emit,
                    );
                    if let Err(error) = &accepted {
                        admission_error = Some(error.clone());
                    }
                    accepted
                } else {
                    Ok(())
                }
            };
            let mut result = if let Some(fork) = physical_request {
                fork.stream(&mut on_event)
            } else {
                stream_chat_checked(&llm, &llm_messages, &[], "none", &mut on_event)
            };
            self.current_turn_unavailable_model_targets
                .extend(newly_unavailable_model_targets);
            if let Some(error) = admission_error {
                return Err(error);
            }
            let usage = result
                .as_ref()
                .ok()
                .map(|choice| {
                    let mut usage = Self::core_token_usage(&choice.usage);
                    usage.cache_usage = Some(choice.usage.cache_usage(
                        choice.provider_namespace.as_deref() == Some("anthropic.messages"), true));
                    usage
                });
            if let Some(usage) = &usage {
                // Account the physical response before deciding whether its
                // summary is usable. Rejected summaries still cost tokens.
                Self::emit(&sampling_request, &mut sequence,
                    HostResponse::ModelUsageReported { usage: serde_json::to_value(usage)
                        .map_err(|error| error.to_string())? }, emit)?;
            }
            Self::emit(
                &sampling_request,
                &mut sequence,
                HostResponse::ToolProgress {
                    title: "compaction_request_metrics".into(),
                    detail: None,
                    metadata: json!({
                        "strategy": strategy,
                        "fallback_reason": if is_fork { None } else { fallback_reason.or(Some("fork_rejected")) },
                        "elapsed_ms": started.elapsed().as_millis() as u64,
                        "recovery_attempted": recovery_attempted,
                        "continuation_count": continuation_count,
                        "partial_summary_bytes": partial_summary.len(),
                        "completed_segments": summary_plan.as_ref().map(|plan| plan.completed_segments()),
                        "remaining_segments": summary_plan.as_ref().map(|plan| plan.remaining_segments()),
                        "segment_splits": summary_plan.as_ref().map(|plan| plan.segment_splits()),
                        "usage": usage,
                        "output_limit_tokens": response_output_limit,
                        "accepted_summary": result.as_ref().is_ok_and(&accepted_summary),
                        "summary_rewrite_count": summary_rewrite_count,
                        "failure_code": result.as_ref().err().map(|failure| failure.code.as_str()),
                        "request_digest": fork.as_ref().map(|value| &value.prefix_digest),
                        "prefix_tokens": fork.as_ref().map(|value| value.prefix_tokens),
                        "source_entry_count": fork.as_ref().map(|value| value.source_entry_count),
                    }),
                },
                emit,
            )?;
            if let Ok(choice) = &result {
                let response_bytes = choice
                    .content
                    .len()
                    .saturating_add(choice.reasoning.len())
                    .saturating_add(
                        choice
                            .tool_calls
                            .iter()
                            .map(|call| call.arguments.len())
                            .sum::<usize>(),
                    );
                let mut provenance = model_response_provenance(
                    physical_request.map_or(&llm, |fork| fork.config()),
                    choice,
                    sampling_count,
                    response_bytes,
                );
                provenance["cache_usage"] = serde_json::to_value(choice.usage.cache_usage(
                    choice.provider_namespace.as_deref() == Some("anthropic.messages"), true)).unwrap_or(Value::Null);
                Self::emit(
                    &sampling_request,
                    &mut sequence,
                    HostResponse::ToolProgress {
                        title: "model_response_provenance".into(),
                        detail: Some(provenance.to_string()),
                        metadata: provenance,
                    },
                    emit,
                )?;
            }
            // A fork is text-only maintenance even though its schemas are unchanged.
            // Never feed its tool calls into the Agent loop. Cancellation/admission
            // failure must not initiate another paid request.
            if self.llm.cancellation.is_requested() {
                Self::emit_failure(&sampling_request, &mut sequence, "cancelled",
                    "Compaction cancelled; original context retained", false, emit)?;
                break;
            }
            if let Ok(choice) = &result {
                if choice.tool_calls.is_empty()
                    && matches!(choice.finish_reason.as_str(), "length" | "max_tokens" | "max_output_tokens")
                {
                    // An output limit is not an input-size error. Continue the same
                    // summary without discarding text or re-summarizing the archive.
                    let next = if !matches!(compaction_maintenance_decision(choice, target_tokens, continuation_count, summary_rewrite_count, true, true), nola_domain::compaction_maintenance::CompactionMaintenanceDecision::Continue { .. }) {
                        Err("continuation_attempts_exhausted")
                    } else {
                        choice.request_snapshot.as_ref().ok_or("continuation_snapshot_missing")
                            .and_then(|snapshot| snapshot.continue_compaction(choice, summary_window))
                    };
                    match next {
                        Ok(next) => {
                            partial_summary.push_str(&choice.content);
                            continuation = Some(next);
                            continuation_count += 1;
                            continue;
                        }
                        Err(reason) => {
                            Self::emit_failure(&sampling_request, &mut sequence,
                                reason, "Compaction continuation could not proceed; original context retained", true, emit)?;
                            break;
                        }
                    }
                }
            }
            let was_continuation = continuation.is_some();
            if let Ok(choice) = &mut result {
                if accepted_summary(choice) && was_continuation {
                    partial_summary.push_str(&choice.content);
                    choice.content = std::mem::take(&mut partial_summary);
                    continuation = None;
                    continuation_count = 0;
                }
            }
            if !self.llm.cancellation.is_requested()
                && !was_continuation
                && !result
                    .as_ref()
                    .is_ok_and(&accepted_summary)
                && !result
                    .as_ref()
                    .is_err_and(|failure| failure.code == "cancelled")
                && (is_fork || result.as_ref().is_err_and(|failure| failure.code == "context_length_exceeded"))
            {
                recovery_attempted = true;
                fallback_reason = Some(if is_fork { "fork_rejected" } else { "summary_rejected" });
                if is_fork {
                    fork = None;
                    continue;
                }
                // Rejected output never enters the rolling checkpoint. Change
                // the input geometry instead of paying for the same archive again.
                if summary_plan.as_mut().is_some_and(CompactionSummaryPlan::split_rejected) {
                    continue;
                }
            }
            if let Ok(choice) = &result {
                if !is_fork && accepted_summary(choice) {
                    let plan = summary_plan.as_mut().expect("standalone plan");
                    if !plan.accept(choice.content.clone()) {
                        if !was_continuation {
                            let request_bytes = serde_json::to_vec(&llm_messages).map_or(0, |body| body.len());
                            plan.pack_after_usage(request_bytes, choice.usage.input_tokens.unwrap_or(0), instructions);
                        }
                        continue;
                    }
                }
            }
            // Validate before claiming completion. Core still owns final plan
            // admission, but a known over-budget answer must not strand its
            // effect as an uncommittable CompactionCompleted response.
            if let Ok(choice) = &result {
                if accepted_summary(choice)
                    && !matches!(compaction_maintenance_decision(choice, target_tokens, continuation_count, summary_rewrite_count, false, false), nola_domain::compaction_maintenance::CompactionMaintenanceDecision::Accept)
                {
                    if matches!(compaction_maintenance_decision(choice, target_tokens, continuation_count, summary_rewrite_count, false, true), nola_domain::compaction_maintenance::CompactionMaintenanceDecision::Rewrite { .. }) {
                        if let Some(next) = choice.request_snapshot.as_ref().and_then(|snapshot|
                            snapshot.rewrite_compaction_summary(choice, summary_window, target_tokens).ok()) {
                            summary_rewrite_count += 1;
                            recovery_attempted = true;
                            fork = Some(next);
                            continuation = None;
                            partial_summary.clear();
                            continue;
                        }
                    }
                    Self::emit_failure(&sampling_request, &mut sequence,
                        "compaction_summary_budget_exceeded",
                        "Compaction summary still exceeds its admitted budget; original context retained",
                        false, emit)?;
                    break;
                }
            }
            match result {
                Ok(choice) if accepted_summary(&choice) => Self::emit(
                    &sampling_request,
                    &mut sequence,
                    HostResponse::CompactionCompleted {
                        summary: choice.content,
                        usage: serde_json::to_value({
                            let mut usage = Self::core_token_usage(&choice.usage);
                            usage.cache_usage = Some(choice.usage.cache_usage(
                                choice.provider_namespace.as_deref() == Some("anthropic.messages"), true));
                            usage
                        })
                            .unwrap_or_else(|_| json!({})),
                    },
                    emit,
                )?,
                Ok(choice) => Self::emit_failure(
                    &sampling_request,
                    &mut sequence,
                    "invalid_compaction_summary",
                    if !choice.tool_calls.is_empty() {
                        "Compaction requested tools instead of a checkpoint; original context was retained".to_string()
                    } else if choice.content.trim().is_empty() {
                        "Compaction returned no checkpoint text; original context was retained".to_string()
                    } else if matches!(choice.finish_reason.as_str(), "length" | "max_tokens" | "max_output_tokens") {
                        format!("Compaction did not finish within the admitted {}-token output limit after bounded segment recovery; original context was retained", llm.max_output_tokens)
                    } else {
                        "Compaction response did not complete after a bounded rewrite; original context was retained".to_string()
                    },
                    true,
                    emit,
                )?,
                Err(error) => Self::emit_failure(
                    &sampling_request,
                    &mut sequence,
                    &error.code,
                    error.display_message(),
                    error.retryable,
                    emit,
                )?,
            };
            break;
        }
        Ok(())
    }
}
