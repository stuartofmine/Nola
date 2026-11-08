// SPDX-License-Identifier: Apache-2.0
// Explicit MCP connection admission on the existing Core tool/resource lane.
impl CliAgentHostDriver {
    fn restore_lazy_mcp_routes(&mut self, frozen:&HostCapabilityCatalog) -> Result<(),String> {
        for (id,descriptor) in &frozen.capabilities {
            if self.catalog.capabilities.contains_key(id) { continue; }
            if let Some(route) = &descriptor.input_normalization.mcp_route {
                let resource = crate::mcp_connection_authority::permission_resource(&route.connection_ref)?;
                let canonical_id = nola_core::code_intelligence::stable_capability_id(&route.server_id,&route.tool_name)
                    .map(str::to_owned).unwrap_or_else(||mcp_model_tool_id(&route.server_id,&route.tool_name));
                if resource.host_id != self.catalog.host_id.0 || Some(resource.session_id.as_str()) != self.boundary_session_id.as_deref()
                    || route.config_digest != resource.config_digest || canonical_id != *id
                    || descriptor.input_normalization.origin.kind != nola_core::agent::host::HostCapabilityOriginKind::Mcp
                    || descriptor.input_normalization.origin.provider_id.as_deref() != Some(route.server_id.as_str())
                    || descriptor.permission_domain.is_none() {
                    return Err("MCP recovery descriptor does not match its current owner/configuration".into());
                }
                let server = self.mcp_server_registry.iter().find(|value|value["connection_ref"] == route.connection_ref)
                    .ok_or("MCP recovery target is not configured")?;
                let transport = match resource.transport.as_str() {"stdio"=>McpTransportKind::Stdio,
                    "streamable_http"=>McpTransportKind::StreamableHttp,_=>return Err("Unsupported MCP recovery transport".into())};
                self.mcp_tool_routes.insert(id.clone(),McpToolRoute {transport,connection_ref:route.connection_ref.clone(),
                    server_id:route.server_id.clone(),server_name:server["display_name"].as_str().unwrap_or(&route.server_id).into(),tool_name:route.tool_name.clone()});
                self.catalog.capabilities.insert(id.clone(),descriptor.clone());
            } else {
                let definition = match id.as_str() {
                    MCP_RESOURCE_READ_CAPABILITY_ID => Some(("Read MCP resource",mcp_resource_read_model_definition())),
                    MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID => Some(("Read MCP resource template",mcp_resource_template_read_model_definition())),
                    MCP_PROMPT_GET_CAPABILITY_ID => Some(("Get MCP prompt",mcp_prompt_get_model_definition())), _ => None,
                };
                if let Some((title,definition)) = definition {
                    let expected = mcp_catalog_access_descriptor(id,title,definition["function"]["parameters"].clone());
                    if descriptor != &expected { return Err("MCP recovery helper schema differs from its current contract".into()); }
                    self.catalog.capabilities.insert(id.clone(),descriptor.clone());
                }
            }
        }
        Ok(())
    }
    fn install_owned_mcp_registry(&mut self, configured: Vec<Value>) -> Result<(),String> {
        let session = self.boundary_session_id.as_deref().ok_or("MCP requires its canonical session owner")?;
        match self.tool_context.permission_scope.owner_session_id() {
            Some(owner) if owner != session => return Err("MCP ToolContext belongs to another session".into()),
            None => self.tool_context.permission_scope.bind_owner_session(session),
            _ => {},
        }
        let configured = crate::mcp_connection_authority::bind_configurations(&configured,&self.catalog.host_id.0,session)?;
        crate::mcp_host::register_configured_servers(&configured).map_err(|error|error.message)?;
        let described = crate::mcp_host::describe_connected_cache(&configured).map_err(|error|error.message)?;
        let mut catalog = self.capability_catalog.mcp.clone();
        for (index,value) in described.iter().enumerate() {
            // A disabled configured server cannot retain an older Available
            // descriptor merely because metadata decoding skips disabled rows.
            let id = crate::mcp_host::configured_server_id(value,index).map_err(|error|error.message)?;
            catalog.servers.remove(&id);
            if value["enabled"] == false { continue; }
            let server: McpServerDescriptor = serde_json::from_value(value.clone()).map_err(|error|error.to_string())?;
            catalog.servers.insert(server.server_id.clone(),server);
        }
        self.mcp_server_registry = described;
        self.install_mcp_capability_catalog(catalog);
        let references = self.mcp_server_registry.iter().filter(|value|value["enabled"] != false)
            .map(|value|value["connection_ref"].clone()).collect::<Vec<_>>();
        if references.is_empty() { self.catalog.capabilities.remove(crate::mcp_connection_authority::CONNECT_CAPABILITY); return Ok(()); }
        let mut descriptor = nola_mcp_host::catalog::connect_descriptor(self.mcp_server_registry.clone());
        descriptor.permission_domain = Some("mcp_connection".into());
        descriptor.input_normalization.permission_contract = crate::tools::builtin_permission_contract(
            crate::mcp_connection_authority::CONNECT_CAPABILITY,false);
        descriptor.input_normalization.effect = nola_core::agent::host::HostCapabilityEffectDeclaration::fixed(
            nola_core::agent::host::HostCapabilityEffect::Execute);
        descriptor.parallelism = HostCapabilityParallelism::Serial;
        descriptor.replay_safety = HostReplaySafety::RequiresRecoveryDecision;
        self.catalog.capabilities.insert(descriptor.id.clone(),descriptor);
        Ok(())
    }

    fn execute_mcp_connection(&mut self, request:&HostRequestEnvelope, call_id:&str, input:&Value,
        sequence:&mut u64, emit:&mut dyn FnMut(HostResponseEnvelope)->Result<(),String>) -> Result<(),String> {
        let reference = input["connection_ref"].as_str().ok_or("MCP connection reference is missing")?;
        let expected = crate::tools::current_mcp_permission_resource_expectation()
            .ok_or("MCP connection lacks its validated Core resource facts")?;
        if expected.host_id != request.host_id.0 || expected.session_id != request.session_id
            || expected.connection_ref != reference {
            return Err("MCP connection permission belongs to another Host/session".into());
        }
        let connected = crate::mcp_host::connect_approved(reference,&expected);
        let mut connection_effects = Vec::new();
        let mut connection_checks = Vec::new();
        let (output,outcome,activation,state) = match connected {
            Ok(established) => {
                let generation = established.resource.generation.clone().ok_or("Connected MCP resource has no generation")?;
                let uri = format!("mcp-connection://{}",reference.trim_start_matches("mcp:"));
                if expected.generation.is_none() {
                    connection_effects.push(nola_host_protocol::tool_result::ResourceEffect {
                        resource_uri:uri.clone(),operation:"connect".into(),before_version:None,after_version:Some(generation.clone()),
                    });
                }
                connection_checks.push(nola_host_protocol::tool_result::PostconditionCheck {
                    kind:nola_host_protocol::tool_result::ExecutionCheckKind::Postcondition,
                    resource_uri:uri,version:generation,predicate:"Host retained an initialized MCP client and its discovered descriptor for this exact owner/configuration".into(),passed:true,
                });
                let server = established.server;
                let id = server.server_id.clone();
                let previous = self.catalog.capabilities.clone();
                let previous_routes = self.mcp_tool_routes.clone();
                let mut catalog = self.capability_catalog.mcp.clone();
                catalog.servers.insert(id.clone(),server);
                self.install_mcp_capability_catalog(catalog);
                let changed = previous.iter().filter(|(id,old)|self.catalog.capabilities.get(*id) != Some(*old))
                    .map(|(id,old)|(id.clone(),old.clone())).collect::<Vec<_>>();
                let refresh_required = !changed.is_empty();
                // Existing schemas change only at the formal catalog refresh
                // boundary, never as an incidental result of reconnecting.
                for (id,old) in changed {
                    if let Some(route) = previous_routes.get(&id) { self.mcp_tool_routes.insert(id.clone(),route.clone()); }
                    self.catalog.capabilities.insert(id,old);
                }
                let descriptors = self.catalog.capabilities.values().filter(|capability|
                    !previous.contains_key(&capability.id) && (capability.input_normalization.origin.provider_id.as_deref() == Some(&id)
                    || [MCP_RESOURCE_READ_CAPABILITY_ID,MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID,MCP_PROMPT_GET_CAPABILITY_ID].contains(&capability.id.as_str())))
                    .cloned().collect::<Vec<_>>();
                let ids = descriptors.iter().map(|descriptor|descriptor.id.clone()).collect::<Vec<_>>();
                let activation = nola_core::agent::host::HostCapabilityActivation {source:"mcp_connection".into(),
                    capability_ids:ids.clone(),capability_descriptors:descriptors.into_iter()
                        .map(serde_json::to_value).collect::<Result<Vec<_>,_>>().map_err(|error|error.to_string())?};
                (json!({"server_id":id,"connected":true,"capability_ids":ids,"catalog_refresh_required":refresh_required,
                    "authority":"Connection only; every tool call requires its independent Core permission evaluation."}),
                    HostToolOutcome::Succeeded,Some(activation),nola_host_protocol::tool_result::ExecutionState::Completed)
            }
            Err(error) => {
                let state = match error.code {
                    "mcp_registry_unavailable" | "mcp_connection_required" | "mcp_connection_unavailable"
                    | "mcp_connection_changed" | "mcp_connection_retired" | "mcp_descriptor_missing"
                    | "mcp_process_start_failed" => nola_host_protocol::tool_result::ExecutionState::NotStarted,
                    _ => nola_host_protocol::tool_result::ExecutionState::Unknown,
                };
                (json!({"error":error.code,"message":error.message,"safe_to_retry":false}),HostToolOutcome::Failed,None,state)
            },
        };
        let mut output = external_tool_result(output,state);
        output["execution"]["effects"] = serde_json::to_value(connection_effects).map_err(|error|error.to_string())?;
        output["execution"]["checks"] = serde_json::to_value(connection_checks).map_err(|error|error.to_string())?;
        Self::emit(request,sequence,HostResponse::ToolCompleted {call_id:call_id.into(),output,outcome,activation,
            full_output_ref:None,artifacts:Vec::new()},emit)
    }
}

#[cfg(test)]
mod mcp_lazy_recovery_tests {
    use super::*;
    use nola_core::agent::host::{HostEffectId,HostId};

    #[test]
    fn cold_mcp_route_restoration_is_owner_bound_and_does_not_connect() {
        let root = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let configured = vec![json!({"server_id":"cold-mcp","endpoint":format!("http://{}/mcp",listener.local_addr().unwrap())})];
        let context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills:crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(vec![])),allowed_dirs:vec![],
            policy:crate::tools::ToolExecutionPolicy::new(vec![],false,"ask".into(),vec![]),
            permission_scope:Default::default(),cancellation:Default::default(),plan_state:Default::default(),
            output_store:crate::tools::ToolOutputStore::for_test(),browser:crate::tools::BrowserRuntime::new(root.path().join("browser")),
        };
        let mut driver = CliAgentHostDriver::new("cold-host",
            effect_delivery_start_barrier_tests::test_llm("http://127.0.0.1:1".into()),context)
            .with_boundary_session_id(uuid::Uuid::new_v4().to_string())
            .with_mcp_server_registry(configured).unwrap();
        use nola_core::tool_policy::{ToolPolicyInput, ToolRoute, ToolRisk, CapabilityAuthorization, RuntimeCapabilityManifest};
        let mut policy = ToolPolicyInput { now_ms: 1_000,
            runtime: RuntimeCapabilityManifest { manifest_id: "mac-mcp-test".into(), manifest_version: 1,
                issued_at_ms: 1, expires_at_ms: 60_000, routes: vec![ToolRoute::AgentHost],
                capabilities: vec!["network".into()],
                authorizations: [("network".into(), CapabilityAuthorization::Granted)].into_iter().collect(),
                ..Default::default() }, ..Default::default() };
        policy.user_policy.max_risk = ToolRisk::High;
        driver.configure_tool_allowlist(Some(["read_file".into()]), false);
        driver.refresh_current_turn_available_tool_ids();
        assert!(!driver.current_turn_available_tool_ids.contains("mcp_connect"));
        driver = driver.with_mcp_extension_policy(Some(policy.clone()));
        assert!(driver.current_turn_available_tool_ids.contains("mcp_connect"));
        assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
        policy.user_policy.deny_capabilities.push("network".into());
        driver = driver.with_mcp_extension_policy(Some(policy));
        assert!(!driver.current_turn_available_tool_ids.contains("mcp_connect"));
        driver.mcp_extension_policy = None;
        driver.configure_tool_allowlist(None::<Vec<String>>, false);
        let reference = driver.mcp_server_registry[0]["connection_ref"].as_str().unwrap().to_owned();
        let resource = crate::mcp_connection_authority::permission_resource(&reference).unwrap();
        let id = mcp_model_tool_id("cold-mcp","echo");
        let mut descriptor = mcp_catalog_access_descriptor(&id,"Cold MCP echo",json!({"type":"object"}));
        descriptor.permission_domain = Some("mcp".into());
        descriptor.input_normalization.origin.provider_id = Some("cold-mcp".into());
        descriptor.input_normalization.mcp_route = Some(nola_core::agent::host::HostMcpToolRoute {
            server_id:"cold-mcp".into(),tool_name:"echo".into(),connection_ref:reference.clone(),config_digest:resource.config_digest,
        });
        let mut frozen = driver.catalog.clone();
        frozen.capabilities.insert(id.clone(),descriptor);
        driver.restore_catalog_fence(&frozen,std::iter::empty()).unwrap();
        assert!(driver.mcp_tool_routes.contains_key(&id));
        assert!(crate::mcp_connection_authority::permission_resource(&reference).unwrap().generation.is_none());
        assert_eq!(crate::mcp_host::call_tool(McpTransportKind::StreamableHttp,&reference,"echo",json!({})).unwrap_err().code,"mcp_connection_required");
        let request = HostRequestEnvelope {
            attempt_started:false,delegated_invocation_id:None,attempt_id:None,
            request_id:"cold-call-request".into(),effect_id:HostEffectId("cold-call-effect".into()),
            host_id:HostId("cold-host".into()),lease_id:None,session_id:driver.boundary_session_id.clone().unwrap(),
            turn_id:Some("cold-turn".into()),catalog_revision:driver.catalog.revision,
            availability_epoch:driver.catalog.availability_epoch,created_at_ms:0,deadline_ms:None,batch:None,
            resume_after_sequence:None,delivery:None,payload:HostRequest::ExecuteTool {
                permission_profile:nola_domain::PermissionProfile::AllowAll,capability_id:id.clone(),call_id:"cold-call".into(),
                input:json!({}),grant:None,permission_instructions:Vec::new(),
            },
        };
        let mut responses = Vec::new();
        assert!(!driver.allows_absent_tool_stop_receipt(&request),
            "remote readOnlyHint is not a Host proof that remote execution stopped");
        let mut absent_receipt = external_tool_result(json!({"remote":"returned"}),nola_host_protocol::tool_result::ExecutionState::Unknown);
        absent_receipt["execution"] = Value::Null;
        let missing = HostResponse::ToolCompleted {call_id:"cold-call".into(),output:absent_receipt,
            outcome:HostToolOutcome::Failed,activation:None,full_output_ref:None,artifacts:Vec::new()};
        assert!(!terminal_response_proves_local_stop(&missing,driver.allows_absent_tool_stop_receipt(&request),None));
        let uncertain = HostResponse::ToolCompleted {call_id:"cold-call".into(),
            output:external_tool_result(json!({"error":"mcp_request_timed_out"}),nola_host_protocol::tool_result::ExecutionState::Unknown),
            outcome:HostToolOutcome::Failed,activation:None,full_output_ref:None,artifacts:Vec::new()};
        assert!(!terminal_response_proves_local_stop(&uncertain,true,None));
        driver.execute_tool(&request,&id,"cold-call",&json!({}),&mut |response| {responses.push(response);Ok(())}).unwrap();
        let output = responses.iter().find_map(|response|match &response.payload {
            HostResponse::ToolCompleted {output,..}=>Some(output),_=>None,
        }).expect("cold call must produce an explicit result");
        assert_eq!(output["execution"]["state"],"not_started");
        assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
        let connect = &driver.catalog.capabilities[crate::mcp_connection_authority::CONNECT_CAPABILITY];
        assert_eq!(connect.permission_domain.as_deref(),Some("mcp_connection"));
        assert!(connect.permission_subject(&json!({"connection_ref":reference})).is_some());
        assert!(connect.input_normalization.permission_contract.resource_plan.is_some());
        let mut foreign = frozen.clone();
        foreign.capabilities.get_mut(&id).unwrap().input_normalization.mcp_route.as_mut().unwrap().config_digest="foreign".into();
        // Rebuilding from a fresh disconnected catalog cannot adopt foreign identity.
        driver.catalog.capabilities.remove(&id); driver.mcp_tool_routes.remove(&id);
        assert!(driver.restore_catalog_fence(&foreign,std::iter::empty()).is_err());
    }
}
