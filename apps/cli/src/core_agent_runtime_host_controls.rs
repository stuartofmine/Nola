// SPDX-License-Identifier: Apache-2.0
// Mechanical extraction: runtime implementation in the original module scope.

impl CoreAgentHost {
    fn recover_projection_if_needed(&mut self) -> Result<(), String> {
        if !self.runtime.requires_committed_restore {
            return Ok(());
        }
        let store = self
            .session_store
            .as_mut()
            .ok_or("Committed runtime recovery requires its original Session Store")?;
        let state = store.load_host_execution_state()?;
        // Use the original port, including retained ephemeral sessions. Never
        // clear the fence by constructing a fresh empty runtime or current config.
        self.runtime
            .adopt_execution_state(state)
            .map_err(format_runtime_error)
    }

    fn commit_idle_submission_observing(
        &mut self,
        submission: AgentSubmission,
        emitter: &mut dyn crate::runtime_output::RuntimeOutputPort,
        observe: &mut dyn FnMut(&CoreEventEnvelope<AgentEvent>),
    ) -> Result<nola_core::agent::protocol::AgentControlSnapshot, String> {
        let core_config = self.runtime.core_config()?.clone();
        self.runtime.requires_committed_restore = true;
        let result = self
            .session_store
            .as_mut()
            .expect("Core Agent Host owns a Session Store")
            .reduce_idle_core_submission(core_config, submission);
        let accepted = self.runtime.receive_admission(result)?;
        {
            let mut presenter = CoreEventPresenter::new(emitter, self.tool_count)
                .with_assistant_journal_notifications(
                    self.session_store.as_mut().expect("Core Agent Host owns a Session Store"),
                    accepted.execution.control.session_id.as_str(),
                );
            for event in accepted.events {
                observe(&event);
                presenter.publish(CoreHostOutput::Event(event));
            }
            for trace in accepted.traces {
                presenter.publish(CoreHostOutput::Trace(trace));
            }
            presenter.publish_execution(&accepted.execution);
        }
        self.runtime
            .adopt_execution_state(accepted.execution)
            .map_err(format_runtime_error)?;
        Ok(self.runtime.control())
    }

    fn commit_core_submission(
        &mut self,
        label: &str,
        submission: AgentSubmission,
        emitter: &mut dyn crate::runtime_output::RuntimeOutputPort,
    ) -> Result<nola_core::agent::protocol::AgentControlSnapshot, String> {
        let renew_delivery_identity =
            matches!(&submission, AgentSubmission::ResumeHostEffect { .. });
        let core_config = self.runtime.core_config()?.clone();
        // Any later failure may occur after the Store committed. Never leave
        // the old execution projection dispatchable while adopting the reply.
        self.runtime.requires_committed_restore = true;
        let accepted = if let AgentSubmission::DeliverTaskGuidance {
            task_id,
            message_id,
            expected_run_id,
            ..
        } = &submission
        {
            let result = self
                .session_store
                .as_mut()
                .expect("Core Agent Host owns a Session Store")
                .apply_task_guidance(task_id, message_id, expected_run_id)?;
            match (result.accepted, result.recovered) {
                (Some(accepted), _) => (
                    None,
                    accepted.events,
                    accepted.traces,
                    accepted.control,
                ),
                (None, Some(recovered)) => (
                    Some(recovered.execution.clone()),
                    recovered.events,
                    recovered.traces,
                    recovered.execution.control,
                ),
                (None, None) => {
                    self.recover_projection_if_needed()?;
                    return Ok(self.runtime.control());
                }
            }
        } else if self
            .session_store
            .as_ref()
            .is_some_and(SessionStore::has_active_core_turn)
        {
            let accepted = self
                .session_store
                .as_mut()
                .expect("Core Agent Host owns a Session Store")
                .reduce_active_core_submission(
                    format!("{label}:{}", uuid::Uuid::new_v4()),
                    submission,
                )?;
            (
                None,
                accepted.events,
                accepted.traces,
                accepted.control,
            )
        } else {
            let result = self
                .session_store
                .as_mut()
                .expect("Core Agent Host owns a Session Store")
                .reduce_idle_core_submission(core_config, submission);
            let accepted = self.runtime.receive_admission(result)?;
            (
                Some(accepted.execution.clone()),
                accepted.events,
                accepted.traces,
                accepted.execution.control,
            )
        };
        {
            let mut presenter = CoreEventPresenter::new(emitter, self.tool_count)
                .with_assistant_journal_notifications(
                    self.session_store
                        .as_mut()
                        .expect("committed Session Store"),
                    accepted.3.session_id.as_str(),
                );
            for event in accepted.1 {
                presenter.publish(CoreHostOutput::Event(event));
            }
            for trace in accepted.2 {
                presenter.publish(CoreHostOutput::Trace(trace));
            }
            if let Some(execution) = &accepted.0 {
                presenter.publish_execution(execution);
            } else {
                presenter.publish_control(&accepted.3);
            }
        }
        match accepted.0 {
            Some(execution) => self.runtime.adopt_execution_state(execution),
            None => {
                let state = self.session_store.as_mut().expect("committed Session Store")
                    .load_host_execution_state()?;
                self.runtime.adopt_execution_state(state)
            }
        }.map_err(format_runtime_error)?;
        if renew_delivery_identity {
            self.runtime.effects.renew_delivery_identity(crate::process_observation::current_worker_identity()?)?;
        }
        Ok(self.runtime.control())
    }

    /// Drains effects that were durably created by a terminal submission.
    ///
    /// Interrupting an in-flight model or tool is a two-phase protocol: Core
    /// first commits Cancelling plus `host.cancel`, then Host confirms
    /// that cancellation. Returning after only the first phase leaves a
    /// terminal checkpoint with live effects, which prevents safe session
    /// reconfiguration and makes the next user Turn fail during restore.
    fn drain_committed_terminal_effects(
        &mut self,
        emitter: &mut dyn crate::runtime_output::RuntimeOutputPort,
    ) -> Result<nola_core::agent::protocol::AgentControlSnapshot, String> {
        let settled = self.runtime.control();
        if settled.turn_status.is_terminal() && settled.pending_host_effect_count == 0 {
            // Do not replace the exact executed Run with an empty active-run
            // lookup after its final transaction already sealed it.
            return Ok(settled);
        }
        let runtime = &mut self.runtime;
        let store = self
            .session_store
            .as_mut()
            .expect("Core Agent Host owns a Session Store");
        let mut presenter = CoreEventPresenter::new(emitter, self.tool_count)
            .with_assistant_journal_notifications(
                store,
                runtime.control_ref().session_id.as_str(),
            );
        let snapshot = runtime
            .execute_committed_application_port(store, &mut |output| presenter.publish(output))
            .map_err(format_runtime_error)?;
        if snapshot.turn_status == AgentTurnStatus::Cancelling {
            return Err("Cancellation remains pending: no exact stop confirmation was committed; durable owner reconciliation is required".into());
        }
        Ok(snapshot)
    }

    fn open(
        config: &HostInvocationRequest,
        skills: SkillRuntimeContext,
        tools: Vec<Value>,
        signature: u64,
        imported_initial_items: Vec<AgentItem>,
    ) -> Result<Self, String> {
        let mut admitted_config = config.clone();
        freeze_host_output_admission(&mut admitted_config)?;
        let config = &admitted_config;
        let plugins = if config.session.save_session && config.tool_settings.tools_enabled {
            crate::plugin_runtime::PluginMcpSession::persistent_default()?
                .map(crate::plugin_runtime::SharedPluginSession::from)
        } else { None };
        let plugin_revisions = plugins.as_ref().map(|plugins| plugins.revisions()).transpose()?.unwrap_or_default();
        let skills = match plugins.as_ref() {
            Some(plugins) => skills.with_plugins(plugins.clone()),
            None => skills,
        };
        let mut tools = tools;
        if plugins.is_some() {
            skills.index()?;
            for tool in &mut tools {
                if tool.pointer("/function/name").and_then(Value::as_str) == Some("Skill") {
                    tool["function"]["parameters"]["properties"]["skill"] = skills.invocation_schema();
                }
            }
        }
        let mut session_store = Some(if config.session.save_session {
            SessionStore::open(
                config.session.session_id.as_deref(),
                false,
                &config.tool_settings.project_root.to_string_lossy(),
            )?
        } else {
            SessionStore::open_ephemeral(
                config.session.session_id.as_deref(),
                &config.tool_settings.project_root.to_string_lossy(),
            )?
        });
        session_store
            .as_mut()
            .expect("canonical session store")
            .set_task_execution_authority(config.session.host_task_execution.clone());
        let session_id = session_store
            .as_ref()
            .map(|store| store.id.clone())
            .or_else(|| config.session.session_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // The canonical checkpoint owns the complete model transcript,
        // including typed ToolCall/ToolResult items. Always restore it when
        // present. Only an active Turn is eligible for pending-effect replay;
        // a terminal checkpoint starts the next Turn without replay.
        let should_restore_checkpoint = session_store
            .as_ref()
            .is_some_and(SessionStore::has_active_core_turn);
        let saved_checkpoint = if config.session.save_session && should_restore_checkpoint {
            let mut application =
                crate::application_port::CliApplicationPort::persistent_default()?;
            let active = application
                .active_run(&session_id)?
                .ok_or("Host invocation unavailable: active Run changed before restore")?;
            Some(load_exact_recovery_checkpoint(&mut application, &session_id, &active)?.0)
        } else if config.session.save_session {
            session_store
                .as_mut()
                .expect("Core Agent Host always owns a Session Store")
                .load_agent_checkpoint()?
                .map(CoreCheckpoint::<AgentThreadState>::decode_history_for_new_turn)
                .transpose()
                .map_err(|error| format!("Conversation history unavailable: {error}"))?
        } else {
            None
        };
        let invocation_binding = host_invocation_binding(config)?;
        if should_restore_checkpoint {
            let checkpoint = saved_checkpoint.as_ref().ok_or_else(|| {
                "Host invocation unavailable: active Run has no compatible checkpoint".to_string()
            })?;
            validate_host_invocation_binding(
                &checkpoint.state.configuration.turn_identity,
                config,
            )?;
            crate::plugin_runtime::validate_restored_revisions(
                &checkpoint.state.configuration.turn_identity.plugin_revisions, &plugin_revisions)?;
        }
        let memory_inbox_directory = session_store
            .as_ref()
            .map(|store| crate::platform_paths::memory_worker_inbox_directory(&store.id))
            .transpose()?;

        let cancellation = RuntimeExecutionControl::default();
        let model_settings = config.model_invocation_settings()?;
        let llm = LlmConfig {
            siwc: config.runtime.siwc.clone(), native_model_route: config.runtime.native_model_route.clone(),
            provider: config.runtime.provider.clone(),
            fallbacks: crate::host_output_admission::admitted_fallbacks(&model_settings)?,
            endpoint: config.model_settings.endpoint.clone(),
            api_key: config.model_settings.api_key.clone(),
            model: config.model_settings.model.clone(),
            temperature: config.runtime.temperature.unwrap_or(0.7).clamp(0.0, 2.0),
            top_p: config.runtime.top_p.unwrap_or(0.9).clamp(0.0, 1.0),
            max_output_tokens: crate::host_output_admission::admitted_output_tokens(&model_settings)?,
            compaction_output_budget: None,
            chat_output_token_field: config.runtime.chat_output_token_field,
            max_retries: invocation_binding.generation.max_retries as usize,
            thinking_enabled: crate::runtime_config::reasoning_enabled(config.runtime.reasoning_selection.as_ref(),
                config.runtime.model_capabilities.reasoning, config.runtime.provider.as_deref(), &config.model_settings.model),
            reasoning_summary_supported: config.runtime.model_capabilities.reasoning_summary_supported,
            reasoning_selection: config.runtime.reasoning_selection.clone(),
            sampling_override: config.runtime.sampling_override,
            generation_options: config.runtime.generation_options.clone(),
            cancellation: cancellation.context(),
            openai_prompt_cache: config.runtime.openai_prompt_cache.unwrap_or(true),
            openai_prompt_cache_key: config.runtime.openai_prompt_cache_key.clone(),
            openai_prompt_cache_key_prefix: config.runtime.openai_prompt_cache_key_prefix.clone(),
            openai_prompt_cache_retention: config.runtime.openai_prompt_cache_retention.clone(),
            openai_prompt_cache_history_breakpoint: config.runtime.openai_prompt_cache_history_breakpoint.unwrap_or(false),
            openai_prompt_cache_history_breakpoint_endpoints: config.runtime.openai_prompt_cache_history_breakpoint_endpoints.clone(),
            parallel_tool_calls: invocation_binding.generation.parallel_tool_calls,
        };
        let memory_llm = llm.clone();
        let permission_owner_session_id = config
            .session.permission_scope_id
            .clone()
            .unwrap_or_else(|| session_id.clone());
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(effective_permission_profile(config));
        // Concrete resources belong to the canonical conversation. The Host
        // approval/cache namespace below may be a separate `host:task:<id>`.
        permission_scope.bind_owner_session(session_id.clone());
        {
            let _guard = permission_scope.enter();
            if config.session.save_session {
                configure_session_permission_scope(Some(permission_owner_session_id));
            } else {
                crate::tools::configure_ephemeral_session_permission_scope(Some(
                    permission_owner_session_id,
                ));
            }
        }
        let plan_state = AgentPlanState::default();
        if config.session.save_session {
            plan_state.bind_session(&session_id)?;
        }
        nola_domain::language_service::validate_language_services(&config.runtime.language_services)?;
        let tool_context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding {
                profile_id: nola_domain::memory_owner::LOCAL_MEMORY_PROFILE_ID.into(),
                workspace_id: Some(host_execution_workspace_identity(config)?.workspace_id),
                conversation_id: Some(session_id.clone()),
                turn_id: None,
            },
            language_services: config.runtime.language_services.clone(),
            skills: skills.clone(),
            output_store: crate::tools::ToolOutputStore::for_session(
                &crate::platform_paths::application_database_path()?,
                &session_id,
            ),
            allowed_dirs: config.tool_settings.allowed_dirs.clone(),
            policy: crate::tools::ToolExecutionPolicy::new(
                config.tool_settings.bash_prefixes.clone(),
                config.tool_settings.auto_approve,
                config.tool_settings.network_policy.clone(),
                config.runtime.permissions.clone(),
            ).with_process_sandbox(config.tool_settings.process_sandbox.as_ref().map(|spec|
                config.tool_settings.task_worktree.as_ref().map(|binding|spec.for_worktree(binding)).unwrap_or_else(||Ok(spec.clone()))).transpose()?)
            .with_bound_file_workspace(config.tool_settings.bounded_file_writes.then(||config.tool_settings.project_root.clone())),
            permission_scope,
            cancellation,
            plan_state: plan_state.clone(),
            browser: BrowserRuntime::new(
                std::env::var_os("NOLA_PLAYWRIGHT_STATE_ROOT")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::temp_dir().join("nola-browser")),
            ),
        };
        let permission_session_id = session_id.clone();
        let runtime_tool_config = config.clone();
        let runtime_tool_owner_session_id = session_id.clone();
        let mut runtime_tool_ids = crate::task_runtime::runtime_tool_ids()
            .iter()
            .map(|id| (*id).to_string())
            .collect::<HashSet<_>>();
        // Tool visibility can expand after tool_search, but execution routing
        // must already know which calls belong to the signed Mac bridge. The
        // frozen model projection still controls exposure; this only prevents
        // a dynamically activated native tool from falling through to the
        // generic Agent Host executor.
        if crate::tools::mac_native_computer_bridge_is_available() {
            runtime_tool_ids.extend(
                crate::tools::mac_native_computer_bridge_tool_ids()
                    .iter()
                    .map(|id| (*id).to_string()),
            );
        }
        runtime_tool_ids.extend(
            current_device_bridge_runtime_tools()
                .into_iter()
                .filter_map(|runtime_tool| {
                    runtime_tool
                        .schema
                        .pointer("/function/name")
                        .or_else(|| runtime_tool.schema.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                }),
        );
        let permission_resolver = Arc::new(Mutex::new(None::<HostPermissionResolver>));
        let permission_resolver_for_driver = Arc::clone(&permission_resolver);
        let permission_skills = tool_context.skills.clone();
        let ui_database = crate::platform_paths::application_database_path()?;
        let ui_publisher = Arc::new(move |input: nola_application::generative_ui::ExecuteUiTool| {
            // Each Application command has its own receipt identity. The typed
            // failure retains this ID for reconciliation; effect/request/delivery
            // identity remains in ExecuteUiTool and must not be rewritten.
            let request_id = uuid::Uuid::new_v4().to_string();
            let mut application = crate::store::NolaStore::open(&ui_database).and_then(|store| store.application())
                .map_err(|message| nola_service_client::ApplicationServiceCommandError::Transport {
                    client_id: "ui-publisher".into(), request_id: request_id.clone(), message, outcome_unknown: false,
                })?;
            application.execute_typed_received::<serde_json::Value>(&request_id,
                nola_application::ApplicationCommand::ExecuteUiTool { input })
        });
        let expert_leaf=crate::task_runtime::is_expert_context(&session_id);
        let main_expert_dispatch_available=tools.iter().any(|schema|
            nola_domain::expert_task_target::schema_tool_name(schema)==Some("delegate_expert"));
        let driver = CliAgentHostDriver::new("nola-cli-host", llm, tool_context)
            .with_expert_leaf(expert_leaf)
            .with_main_expert_dispatch(main_expert_dispatch_available)
            .with_ui_publisher(ui_publisher)
            .with_code_memory_workspace(config.tool_settings.project_root.clone())
            .with_code_mode_limits(config.runtime.code_mode_limits.clone())?
            .with_boundary_session_id(session_id.clone())
            .with_execution_settings(config.host_execution_settings());
        let mut driver = if config.tool_settings.tool_allowlist_is_authorization_only {
            driver.with_authorized_tool_allowlist(config.tool_settings.tool_allowlist.clone())
                .with_mcp_extension_policy(crate::native_tool_policy::desktop_mcp_policy(&session_id)?)
        } else {
            driver.with_tool_allowlist(config.tool_settings.tool_allowlist.clone())
        };
        // The publisher is owned by Application, not the generic ToolSpec
        // registry. Registration alone does not put its schema in the frozen
        // model request. Include it only when this bound Host authorizes it.
        if config.tool_settings.tools_enabled {
            if let Some(schema) = driver.bound_ui_model_schema() {
                let id = nola_domain::generative_ui_tool::UI_TOOL_ID;
                tools.retain(|tool| tool.pointer("/function/name")
                    .or_else(|| tool.get("name")).and_then(Value::as_str) != Some(id));
                tools.push(schema);
            }
        }
        match crate::code_intelligence_provider::shared_for_application() {
            Ok(provider) => {
                driver = driver.with_code_intelligence_provider(provider);
            }
            Err(error) => crate::llm::log_local_debug_payload(
                "code_intelligence_startup",
                &json!({
                    "type": "code_intelligence_unavailable",
                    "error": error,
                    "fallback": "native_repository_tools"
                }),
            ),
        }
        if let Some(catalog) = config.tool_settings.mcp_capability_catalog.clone() {
            driver = driver.with_mcp_capability_catalog(catalog);
        }
        if !config.tool_settings.mcp_server_registry.is_empty() {
            driver = driver.with_mcp_server_registry(config.tool_settings.mcp_server_registry.clone())?;
        }
        // Durable conversations share installation intent, never live process
        // handles. Ephemeral/test conversations must not open the user's store.
        if let Some(plugins) = plugins {
            driver = driver.with_plugin_mcp_session(plugins)?;
        }
        let mut driver = driver
            .with_permission_handler(move |request, capability_id, resource, _reason, input| {
                // The committed driver may dispatch on a worker thread. Bind
                // the canonical owner in that thread before consulting the
                // permission trace registry or a surface resolver.
                let _permission_session =
                    crate::tools::enter_host_permission_session(&permission_session_id);
                let (permission_request_id, permission_instructions, allow_conversation) = match &request.payload {
                    nola_core::agent::host::HostRequest::RequestPermission {
                        permission_request_id,
                        permission_instructions,
                        allow_conversation,
                        ..
                    } => (permission_request_id.clone(), permission_instructions, *allow_conversation),
                    _ => {
                        return Err("Host permission handler received a non-permission request"
                            .to_string()
                            .into())
                    }
                };
                if let Some(decision) = crate::task_runtime::wait_for_background_agent_permission(
                    &permission_session_id,
                    capability_id,
                    input,
                    permission_instructions,
                    &permission_request_id,
                )? {
                    let decision = match decision {
                        crate::task_runtime::BackgroundPermissionWait::Decision(decision) => decision,
                        crate::task_runtime::BackgroundPermissionWait::StopRequested => {
                            return Err(crate::agent_host_driver::PermissionHandlerError::StopRequested);
                        }
                        crate::task_runtime::BackgroundPermissionWait::Settled => {
                            return Err(crate::agent_host_driver::PermissionHandlerError::Settled);
                        }
                    };
                    return Ok(match decision {
                        crate::task_runtime::BackgroundPermissionDecision::Allow => {
                            HostPermissionResolution::allow_once()
                        }
                        crate::task_runtime::BackgroundPermissionDecision::AllowTask => {
                            HostPermissionResolution::allow_conversation(None)
                        }
                        crate::task_runtime::BackgroundPermissionDecision::Custom(note) => {
                            HostPermissionResolution::provide_instruction(note)
                        }
                        crate::task_runtime::BackgroundPermissionDecision::ReturnToComposer => {
                            HostPermissionResolution::return_to_composer()
                        }
                        crate::task_runtime::BackgroundPermissionDecision::Deny => {
                            HostPermissionResolution::deny()
                        }
                    });
                }
                let request = ToolRequest {
                    id: permission_request_id,
                    name: capability_id.to_string(),
                    args: input
                        .as_object()
                        .cloned()
                        .unwrap_or_else(|| permission_resource_args(resource)),
                };
                if let Some(outcome) =
                    crate::tools::preflight_tool_request_with_skills(&request, &permission_skills)
                {
                    // Host validation is a failure, not user-authored guidance.
                    // Re-enqueuing it as an instruction would repeatedly ask
                    // permission for the same invalid invocation.
                    return Err(outcome.text.into());
                }
                let turn_resolver = permission_resolver_for_driver
                    .lock()
                    .ok()
                    .and_then(|resolver| resolver.clone());
                // Core emitted this request only after schema validation and
                // policy evaluation. The Host must render the pending request,
                // not re-evaluate it through CLI preflight or a second session
                // cache. Core alone persists and applies conversation grants.
                let resolution = present_agent_core_permission_with_resolver(
                    &request,
                    turn_resolver,
                    permission_instructions,
                    allow_conversation,
                );
                let resolution = match resolution {
                    ToolPermissionResolution::StopRequested => {
                        return Err(crate::agent_host_driver::PermissionHandlerError::StopRequested);
                    }
                    ToolPermissionResolution::TransportInterrupted(reason) => {
                        return Err(
                            crate::agent_host_driver::PermissionHandlerError::TransportInterrupted(
                                reason,
                            ),
                        )
                    }
                    ToolPermissionResolution::Allow => HostPermissionResolution::allow_once(),
                    ToolPermissionResolution::AllowConversation => {
                        HostPermissionResolution::allow_conversation(None)
                    }
                    ToolPermissionResolution::Custom(note) => {
                        HostPermissionResolution::provide_instruction(note)
                    }
                    ToolPermissionResolution::ReturnToComposer => {
                        HostPermissionResolution::return_to_composer()
                    }
                    ToolPermissionResolution::Deny => HostPermissionResolution::deny(),
                };
                Ok(resolution)
            })
            .with_runtime_tool_ids(runtime_tool_ids)
            .with_runtime_tool_replay_handler(move |_, name, call_id, _, _| {
                crate::task_runtime::replay_runtime_task_tool(name, call_id)
            })
            .with_runtime_tool_handler(move |request, name, call_id, args, context| {
                if crate::task_runtime::runtime_tool_ids().contains(&name) {
                    let mut scoped = runtime_tool_config.clone();
                    scoped.session.session_id = Some(runtime_tool_owner_session_id.clone());
                    return crate::task_runtime::execute_runtime_task_tool(
                        &scoped, &runtime_tool_owner_session_id, request, name, call_id, args, context,
                    );
                }
                // Permission-bound macOS tools must execute in the signed app
                // process even when the same schema is also advertised by the
                // generic device bridge. Checking the generic bridge first
                // silently routed computer_status and accessibility operations
                // through the Agent Host helper, so the model observed the
                // helper process's permission state instead of Nola's.
                if let Some(outcome) = crate::tools::execute_mac_native_computer_bridge(
                    &runtime_tool_owner_session_id,
                    name,
                    call_id,
                    args,
                ) {
                    return Some(crate::agent_host_driver::RuntimeToolResult::completed(
                        outcome,
                    ));
                }
                if current_device_bridge_tool_schema(name).is_some() {
                    return Some(crate::agent_host_driver::RuntimeToolResult::completed(
                        crate::tools::execute_authorized_tool(
                            &ToolRequest {
                                id: call_id.to_string(),
                                name: name.to_string(),
                                args: args.clone(),
                            },
                            context,
                            crate::tools::ExecutionAuthority::CoreApproved(match &request.payload {
                                nola_core::agent::host::HostRequest::ExecuteTool {permission_profile,..} => *permission_profile,
                                _ => return Some(crate::agent_host_driver::RuntimeToolResult::completed(
                                    crate::tools::permission_resource_execution_failure("Device bridge requires ExecuteTool admission".into()))),
                            }),
                        ),
                    ));
                }
                let mut scoped_runtime_tool_config = runtime_tool_config.clone();
                scoped_runtime_tool_config.session.session_id = Some(runtime_tool_owner_session_id.clone());
                crate::task_runtime::execute_runtime_task_tool(
                    &scoped_runtime_tool_config,
                    &runtime_tool_owner_session_id,
                    request,
                    name,
                    call_id,
                    args,
                    context,
                )
            });

        let context_budget = edited_resend_host::context_budget(config, &model_settings)?;
        let context_window_tokens = context_budget
            .context_window_tokens
            .unwrap_or_else(|| {
                context_budget
                    .prompt_token_limit
                    .saturating_add(context_budget.reserved_output_tokens)
            })
            .max(2_000);
        let model_capability = configured_model_capability_profile(config, context_window_tokens)?;
        let compaction_target_tokens = context_budget
            .token_threshold
            .saturating_mul(3)
            .saturating_div(5)
            .max(1);
        let stored_initial_items = session_store
            .as_mut()
            .map(|store| {
                store.load_messages().map(|messages| {
                    messages
                        .iter()
                        .enumerate()
                        .flat_map(|(index, message)| {
                            session_message_to_agent_items(message, &session_id, index)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .transpose()?
            .unwrap_or_default();
        let initial_items = if stored_initial_items.is_empty() {
            imported_initial_items
        } else {
            stored_initial_items
        };
        // Restore must use the configuration frozen in the checkpoint. A
        // settings change is a later, explicit Core configuration transition;
        // silently replacing it here makes valid sessions fail restoration.
        let memory_policy = saved_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.state.configuration.memory_policy.clone())
            .map(Ok)
            .unwrap_or_else(|| {
                config
                    .runtime
                    .admitted_memory_policy
                    .clone()
                    .ok_or_else(|| "memory policy was not admitted by the Host".to_string())
            })?;
        let configured_execution_backend =
            nola_domain::execution_backend::ExecutionBackendSelection {
                schema_version: nola_domain::execution_backend::EXECUTION_BACKEND_SCHEMA_VERSION,
                backend_id: config.execution.execution_backend.clone(),
                backend_session_id: config.execution.external_execution_session_id.clone(),
                context_anchor_item_id: None,
                context_identity_digest: None,
            };
        let restored_execution_backend = saved_checkpoint
            .as_ref()
            .map(|checkpoint| {
                checkpoint
                    .state
                    .configuration
                    .turn_identity
                    .execution_backend
                    .clone()
            })
            .unwrap_or(configured_execution_backend);
        if let Some(requested) = config.execution.external_execution_session_id.as_deref() {
            if restored_execution_backend.backend_session_id.as_deref() != Some(requested) {
                return Err("Requested external session differs from the frozen conversation session; inspect the prior execution instead of creating a replacement session".into());
            }
        }

        if should_restore_checkpoint && (!plugin_revisions.is_empty() || saved_checkpoint.as_ref().is_some_and(|checkpoint|
            checkpoint.state.host.catalog.capabilities.values().any(|capability|capability.input_normalization.mcp_route.is_some()))) {
            let checkpoint = saved_checkpoint.as_ref().expect("validated active checkpoint");
            driver.restore_catalog_fence(&checkpoint.state.host.catalog,
                checkpoint.state.host.pending_effects.iter()
                    .filter(|effect| effect.capability_id == "model.stream")
                    .map(|effect| (effect.catalog_revision, effect.availability_epoch)))?;
        }
        let host_policy = host_policy_context_components_with_skills(config, &skills, true, driver.code_mode_discoverable())?;
        let runtime_config = AgentCoreConfig {
            // A terminal Thread keeps its actual Core identity when the next
            // Turn replaces idle execution settings. The Store/registry key
            // continues to address the canonical conversation independently.
            session_id: Some(match saved_checkpoint.as_ref() {
                Some(checkpoint) => {
                    if checkpoint.session_id.trim().is_empty()
                        || checkpoint.state.session_id.as_str() != checkpoint.session_id
                    {
                        return Err(
                            "Host invocation unavailable: inconsistent owned Core session".into(),
                        );
                    }
                    checkpoint.state.session_id.clone()
                }
                None => SessionId::from_string(session_id.clone()),
            }),
            model: model_settings.core_model(),
            max_model_iterations: config.execution.max_iterations,
            max_strategy_recovery_attempts: AgentCoreConfig::default().max_strategy_recovery_attempts,
            initial_items,
            // The active configuration is the schema source for exact typed
            // activation and cold recovery. Exposure still comes from Core's
            // capability projection; terminal checkpoint projection removes
            // the live tool schemas after retaining concrete calls/results.
            tools: tools.clone(),
            context_window: ContextWindowConfig {
                context_window_tokens,
                reserved_output_tokens: context_budget.reserved_output_tokens,
                prompt_token_limit: Some(context_budget.prompt_token_limit),
                auto_compact_tokens: Some(context_budget.token_threshold),
                compaction_target_tokens: Some(compaction_target_tokens),
                ..ContextWindowConfig::default()
            },
            // Active Turn recovery needs the complete frozen Host protocol.
            // AgentThreadState::checkpoint_projection removes live Tool
            // descriptors only after the Turn reaches a terminal state.
            host_catalog: Some(driver.catalog().clone()),
            permission_rules: core_permission_rules_with_skills(config, &skills),
            memory_policy,
            turn_identity: AgentTurnIdentity {
                workspace: Some(host_execution_workspace_identity(config)?),
                delegation: crate::delegated_invocation::frozen_parent(config)?,
                remote_tool_plan: config.tool_settings.remote_tool_plan.clone(),
                plugin_revisions,
                supports_reviewed_text_parts: config.session.supports_reviewed_text_parts,
                host_invocation: Some(invocation_binding),
                host_recovery: host_recovery::capture(config, &session_id, &host_invocation_binding(config)?)?,
                prompt: host_policy.prompt_identity,
                instruction_manifest: host_policy.instruction_manifest,
                model_capability,
                execution_backend: restored_execution_backend,
                ..AgentTurnIdentity::default()
            },
            automation_execution_spec: actual_automation_execution_spec(config)?,
        };
        // The active Run owns its Core configuration. Current Host credentials
        // were proven above, but must not replace model/context/catalog policy.
        let runtime_config = if should_restore_checkpoint {
            let checkpoint = saved_checkpoint
                .as_ref()
                .expect("active checkpoint validated above");
            let frozen = &checkpoint.state.configuration;
            AgentCoreConfig {
                session_id: Some(checkpoint.state.session_id.clone()),
                model: frozen.model.clone(),
                max_model_iterations: frozen.max_model_iterations,
                max_strategy_recovery_attempts: frozen.max_strategy_recovery_attempts,
                initial_items: Vec::new(),
                tools: frozen.tools.clone(),
                context_window: frozen.context_window.clone(),
                host_catalog: Some(checkpoint.state.host.catalog.clone()),
                permission_rules: checkpoint.state.execution.permission_rules.clone(),
                memory_policy: frozen.memory_policy.clone(),
                turn_identity: frozen.turn_identity.clone(),
                automation_execution_spec: None,
            }
        } else {
            runtime_config
        };
        let (runtime, restored_from_checkpoint) =
            CommittedCoreRuntime::restore_or_new(runtime_config.clone(), saved_checkpoint, driver)
                .map_err(|error| format!("Agent Core failed: {error}"))?;

        Ok(Self {
            execution_config: config.clone(),
            skills,
            session_id,
            signature,
            runtime,
            session_store,
            permission_resolver,
            plan_state,
            tool_count: tools.len(),
            admission_tool_schemas: tools,
            resume_pending_once: restored_from_checkpoint && should_restore_checkpoint,
            last_used: Instant::now(),
            memory_llm,
            memory_inbox_directory,
        })
    }

    fn prepare_pending_effect_recovery(&mut self, emitter: &mut dyn crate::runtime_output::RuntimeOutputPort) -> Result<(), String> {
        let result = self.session_store.as_mut()
            .ok_or("Committed recovery requires its original Session Store")?
            .prepare_core_effect_recovery();
        match result {
            Ok(prepared) if prepared.rebound_count == 0 => Ok(()),
            Ok(prepared) => {
                self.runtime.requires_committed_restore = true;
                self.recover_projection_if_needed()?;
                self.runtime.effects.renew_delivery_identity(crate::process_observation::current_worker_identity()?)?;
                let mut presenter = CoreEventPresenter::new(emitter, self.tool_count)
                    .with_assistant_journal_notifications(self.session_store.as_mut().expect("recovery Session Store"),
                        self.runtime.execution.control.session_id.as_str());
                for trace in prepared.traces { presenter.publish(CoreHostOutput::Trace(trace)); }
                presenter.publish_execution(&self.runtime.execution);
                Ok(())
            }
            Err(error) => {
                // A transport failure may follow a committed rebind. Never
                // dispatch the retained execution projection after uncertainty.
                self.runtime.requires_committed_restore = true;
                Err(error)
            }
        }
    }

    fn run_turn_items(
        &mut self,
        config: &HostInvocationRequest,
        input_items: Vec<AgentItem>,
        turn_context_platform: nola_core::context_engine::ContextPlatform,
        turn_context_components: Vec<nola_core::context_engine::ContextComponent>,
        cancellation: RuntimeExecutionControl,
        emitter: &mut dyn crate::runtime_output::RuntimeOutputPort,
    ) -> Result<CoreRunOutcome, String> {
        self.run_turn_items_with_admission(config, input_items, turn_context_platform,
            turn_context_components, cancellation, emitter, None)
    }

    fn run_turn_items_with_admission(
        &mut self,
        config: &HostInvocationRequest,
        input_items: Vec<AgentItem>,
        turn_context_platform: nola_core::context_engine::ContextPlatform,
        mut turn_context_components: Vec<nola_core::context_engine::ContextComponent>,
        cancellation: RuntimeExecutionControl,
        emitter: &mut dyn crate::runtime_output::RuntimeOutputPort,
        queued: Option<QueuedUserAdmission>,
    ) -> Result<CoreRunOutcome, String> {
        if config.session.user_delegation.is_some() {
            if queued.is_some() || crate::application_port::CliApplicationPort::persistent_default()?.active_run(&self.session_id)?.is_some() {
                return Err("The conversation is busy; /agent was not queued or executed".into());
            }
        }
        let mut admitted_config = config.clone();
        if let Some(store) = &self.session_store {
            store.apply_initial_creation_boundary(&mut admitted_config.session)?;
        }
        freeze_host_output_admission(&mut admitted_config)?;
        let config = &admitted_config;
        if input_items.is_empty()
            && config
                .input.host_turn_context
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err("Agent Core turn requires at least one input item".to_string());
        }
        self.enforce_cumulative_task_budget()?;
        // Direct CoreAgentHost callers (tests, recovery entrypoints, and
        // non-terminal surfaces) do not necessarily pass through the CLI/Web
        // bootstrap guards. Bind the canonical conversation while Core asks
        // for permission so trace lookup and replay use the same owner.
        let _host_permission_session =
            crate::tools::enter_host_permission_session(&self.session_id);
        emitter.set_session_id(Some(self.session_id.clone()));
        cancellation.bind_session_input(self.session_id.clone(),
            crate::runtime_control::InputRetention::for_session(config.session.save_session))?;
        if let Ok(mut resolver) = self.permission_resolver.lock() {
            *resolver = current_host_permission_resolver();
        }
        let _permission_resolver_guard =
            TurnPermissionResolverGuard::new(Arc::clone(&self.permission_resolver));
        let turn_policy = host_turn_policy();
        let turn_cancellation = cancellation.clone();
        let mut host_policy =
            host_policy_context_components_with_skills(config, &self.skills, false, self.runtime.effects.driver_mut().code_mode_discoverable())?;
        host_policy.components.push(host_ui_presentation_context(
            if turn_context_platform == ContextPlatform::Unknown { host_context_platform() } else { turn_context_platform }
        ));
        if let Some(plan_context) = if config.session.edited_resend_operation_id.is_some() { None } else { self.plan_state.restored_context()? } {
            host_policy
                .components
                .push(prepare_dynamic_context_component(
                    ContextComponent {
                        kind: ContextComponentKind::System,
                        id: "conversation-plan-runtime".to_string(),
                        content: Value::String(plan_context),
                        source: Some(ContextComponentSource {
                            platform: Some(host_context_platform()),
                            uri: Some(format!("nola://conversation/{}/plan", self.session_id)),
                            title: Some("Canonical conversation plan runtime".to_string()),
                            mime_type: Some("text/plain".to_string()),
                            ..ContextComponentSource::default()
                        }),
                    },
                    host_context_platform(),
                )?);
        }
        let canonical_submission_id = config
            .session.host_submission_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // Session preparation may settle an older Run and mutate idle control
        // state. None of those facts belong to this request's transport. Keep
        // publication isolated until Application atomically admits its Run.
        emitter.begin_recovery_transport_isolation();
        let admission_result = (|| -> Result<_, String> {
            if self.resume_pending_once {
                let recovery_result = (|| -> Result<(), String> {
                    if !self
                        .session_store
                        .as_ref()
                        .is_some_and(SessionStore::has_active_core_turn)
                    {
                        return Err(
                        "restored Core checkpoint has no canonical active Run; refusing uncommitted Host replay"
                            .to_string(),
                    );
                    }
                    let recovery_snapshot = self.runtime.control();
                    if !self.runtime.execution.permissions.pending.is_empty() {
                        return Err("The current run is waiting for a permission response. Resume or explicitly cancel it before submitting another turn.".into());
                    }
                    if recovered_turn_blocks_new_admission(recovery_snapshot.turn_status) {
                        // Process loss does not revoke the admitted Run. A new
                        // message cannot impersonate an explicit cancellation.
                        return Err("The previous run still owns unfinished work. Resume it or explicitly cancel it before starting another turn.".into());
                    } else {
                        // A terminal Turn may still own a committed cancellation or
                        // cleanup effect. Drain it before opening the next Run.
                        self.prepare_pending_effect_recovery(emitter)?;
                        self.drain_committed_terminal_effects(emitter)?;
                    }
                    Ok(())
                })();
                recovery_result?;
                self.resume_pending_once = false;
            }
            let recovery_snapshot = self.runtime.control();
            if !self.runtime.execution.permissions.pending.is_empty() {
                return Err("The current run is waiting for a permission response. Resume or explicitly cancel it before submitting another turn.".into());
            }
            if recovered_turn_blocks_new_admission(recovery_snapshot.turn_status) {
                return Err("The previous run still owns unfinished work. Resume it or explicitly cancel it before starting another turn.".into());
            }
            // Retire the previous Turn's concrete effects before replacing their
            // cancellation context with the new Turn's token and policy.
            self.runtime
                .driver_mut()
                .begin_turn(cancellation, turn_policy.clone());
            self.runtime
                .driver_mut()
                .set_execution_settings(config.host_execution_settings());
            self.session_store
                .as_mut()
                .expect("canonical session store")
                .set_task_execution_authority(config.session.host_task_execution.clone());
            emitter.set_host_turn_identity(
                config.session.host_submission_id.clone(),
                config.session.host_assistant_message_id.clone(),
            );
            emitter.set_next_trace_message_id(config.session.host_assistant_message_id.clone());
            // Memory worker imports are idle session mutations. A process restart
            // can restore an active Run whose committed Host effect must be replayed
            // (or interrupted) through that Run's immutable invocation digest first.
            // Importing detached worker output before this boundary incorrectly
            // overlaps an idle submission with the active Run.
            self.ingest_memory_worker_results(emitter)?;
            let refreshed_context_budget = edited_resend_host::context_budget(config, &config.model_invocation_settings()?)?;
            let refreshed_context_window_tokens = refreshed_context_budget
                .context_window_tokens
                .unwrap_or_else(|| self.runtime.control_ref().context_window_tokens)
                .max(2_000);
            let refreshed_model_capability =
                configured_model_capability_profile(config, refreshed_context_window_tokens)?;
            let existing_execution_backend = self
                .runtime
                .config
                .turn_identity
                .execution_backend
                .clone();
            let selected_execution_backend = if existing_execution_backend.backend_id
                == config.execution.execution_backend
            {
                existing_execution_backend
            } else {
                nola_domain::execution_backend::ExecutionBackendSelection {
                    schema_version: nola_domain::execution_backend::EXECUTION_BACKEND_SCHEMA_VERSION,
                    backend_id: config.execution.execution_backend.clone(),
                    backend_session_id: config.execution.external_execution_session_id.clone(),
                    context_anchor_item_id: None,
                    context_identity_digest: None,
                }
            };
            let requested_memory_policy = Some(
                config
                    .runtime
                    .admitted_memory_policy
                    .clone()
                    .ok_or_else(|| "memory policy was not admitted by the Host".to_string())?,
            );
            {
                self.commit_core_submission(
                    "update-session-configuration",
                    AgentSubmission::UpdateSessionConfiguration {
                        patch: nola_core::agent::AgentSessionConfigurationPatch {
                            max_model_iterations: Some(config.execution.max_iterations),
                            context_window: Some(ContextWindowConfig {
                                context_window_tokens: refreshed_context_window_tokens,
                                reserved_output_tokens: refreshed_context_budget
                                    .effective_output_budget
                                    .tokens,
                                prompt_token_limit: Some(
                                    refreshed_context_budget.prompt_token_limit,
                                ),
                                auto_compact_tokens: Some(refreshed_context_budget.token_threshold),
                                compaction_target_tokens: Some(
                                    refreshed_context_budget
                                        .token_threshold
                                        .saturating_mul(3)
                                        .saturating_div(5)
                                        .max(1),
                                ),
                                ..ContextWindowConfig::default()
                            }),
                            memory_policy: requested_memory_policy,
                            turn_identity: Some(AgentTurnIdentity {
                                workspace: Some(host_execution_workspace_identity(config)?),
                                delegation: crate::delegated_invocation::frozen_parent(config)?,
                                remote_tool_plan: config.tool_settings.remote_tool_plan.clone(),
                                host_invocation: Some(host_invocation_binding(config)?),
                                host_recovery: host_recovery::capture(config, &self.session_id, &host_invocation_binding(config)?)?,
                                prompt: host_policy.prompt_identity.clone(),
                                instruction_manifest: host_policy.instruction_manifest.clone(),
                                model_capability: refreshed_model_capability,
                                execution_backend: selected_execution_backend,
                                ..AgentTurnIdentity::default()
                            }),
                            ..Default::default()
                        },
                    },
                    emitter,
                )?;
            }
            if config.session.edited_resend_operation_id.is_some() {
                let main_selection=turn_context_components.iter()
                    .filter(|component|component.kind==ContextComponentKind::MainExpertSelection)
                    .cloned().collect::<Vec<_>>();
                // Only freshly composed Host guidance plus the Core-frozen
                // definition data enters this new owner. No renderer resources,
                // previous history, resident memory or conversation knowledge.
                turn_context_components = host_policy.components.iter().filter(|component|
                    matches!(component.kind, ContextComponentKind::System | ContextComponentKind::Skill | ContextComponentKind::UiClientData))
                    .cloned().map(|mut component| {
                        if let Some(source)=&mut component.source {
                            source.uri=None; source.authorization=None; source.observed_at=None; source.maximum_age_minutes=None;
                        }
                        component
                    }).collect();
                turn_context_components.extend(config.runtime.edited_resend_prompt_definitions.clone());
                turn_context_components.extend(main_selection);
            } else {
            host_policy
                .components
                .extend(self.resident_memory_components(turn_context_platform.clone())?);
            // Interactive surfaces share the same committed conversation
            // documents. Scheduled work uses only its frozen input artifacts.
            if config.input.host_input_origin != nola_core::agent::AgentInputOrigin::ScheduledAction
                && config.session.automation_execution_lease.is_none()
            {
                host_policy.components.extend(self.session_store.as_mut()
                    .ok_or("Conversation knowledge requires the original Session Store")?
                    .conversation_knowledge_context()?);
            }
            // Runtime owns the available Skill context. An older surface or request
            // component cannot restore a catalog revoked in this captured context.
            turn_context_components
                .retain(|component| component.kind != ContextComponentKind::Skill);
            turn_context_components.extend(host_policy.components);
            if !config.input.expected_artifact_refs.is_empty() {
                let contract = AgentTurnArtifactContract {
                    schema_version: 1,
                    expected_artifact_refs: config.input.expected_artifact_refs.clone(),
                };
                contract.validate()?;
                turn_context_components.push(ContextComponent {
                    kind: ContextComponentKind::ProjectInstruction,
                    id: TURN_ARTIFACT_CONTRACT_COMPONENT_ID.to_string(),
                    content: serde_json::to_value(contract).map_err(|error| {
                        format!("failed to encode Turn artifact contract: {error}")
                    })?,
                    source: Some(ContextComponentSource {
                        platform: Some(turn_context_platform.clone()),
                        uri: Some("nola://turn/artifact-contract".to_string()),
                        title: Some("External artifact completion contract".to_string()),
                        mime_type: Some("application/json".to_string()),
                        ..ContextComponentSource::default()
                    }),
                });
            }
            }
            let turn_context_items = config
                .input.host_turn_context
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    vec![AgentItem::user_message(
                        ItemId::from_string(format!("{canonical_submission_id}:internal-context")),
                        value,
                    )]
                })
                .unwrap_or_default();
            if matches!(config.input.host_input_origin,nola_core::agent::AgentInputOrigin::GoalInteractiveUser | nola_core::agent::AgentInputOrigin::GoalContinuation) {
                return Err("Goal Run admission belongs to step_goal; execute its exact admitted Run".into());
            }
            let submission_id = Some(canonical_submission_id.clone());
            let submission = if let Some(delegation)=&config.session.user_delegation {
                delegation.clone()
            } else if config.input.host_input_origin.is_automated() {
                AgentSubmission::AutomatedInput {
                    origin: config.input.host_input_origin,
                    items: input_items.clone(),
                    turn_context_items: turn_context_items.clone(),
                    turn_context_platform,
                    turn_context_components,
                    submission_id,
                    submitted_at_epoch_ms: Some(now_epoch_ms()?),
                }
            } else {
                AgentSubmission::UserInput {
                    items: input_items.clone(),
                    turn_context_items,
                    turn_context_platform,
                    turn_context_components,
                    submission_id,
                    submitted_at_epoch_ms: Some(now_epoch_ms()?),
                    delivery: AgentInputDelivery::CurrentTurn,
                }
            };
            // Only this explicit new-submission boundary restores discovery
            // schemas. A terminal/idle checkpoint is historical and correctly
            // omits them; same-Run recovery must retain its frozen projection.
            let admission_core_config=self.runtime.new_run_admission_configuration(&self.admission_tool_schemas)?;
            let store = self
                .session_store
                .as_mut()
                .expect("every CLI execution mode owns a canonical transaction Store");
            let mut begin = canonical_begin_turn_input(
                config,
                &self.session_id,
                &input_items,
                admission_core_config,
                submission,
            )?;
            if let Some(operation)=&config.session.edited_resend_operation_id {
                if config.session.host_task_execution.is_some() || config.session.automation_execution_lease.is_some()
                    || queued.is_some() || !config.input.host_attachments.is_empty()
                    || !config.input.host_context_components.is_empty() || config.input.host_turn_context.is_some() {
                    return Err("edited resend must start only its edited user message in a fresh target".into());
                }
                store.attest_edited_resend_admission(operation,&mut begin)?;
            }
            if config.session.host_task_execution.is_some() && config.session.automation_execution_lease.is_some() {
                return Err("a Run cannot have both Task and automation admission owners".into());
            }
            let accepted = if let Some(queued) = queued {
                if config.session.host_task_execution.is_some() || config.session.automation_execution_lease.is_some() {
                    return Err("queued user input cannot take a task or automation lease".into());
                }
                let expected_configuration_revision = begin.expected_configuration_revision
                    .ok_or("queued input requires the observed conversation configuration revision")?;
                let AgentSubmission::UserInput { turn_context_items, turn_context_platform,
                    turn_context_components, .. } = begin.submission else {
                    return Err("queued input requires an interactive user context".into());
                };
                self.runtime.requires_committed_restore = true;
                let result = store.begin_queued_core_turn(&queued.request_id, nola_application::BeginQueuedUserTurn {
                    conversation_id: begin.conversation_id, command_id: queued.lease.command_id,
                    lease_owner: queued.lease.lease_owner, expected_configuration_revision,
                    delivery_attempt: queued.lease.delivery_attempt,
                    expected_parent_message_id: begin.expected_parent_message_id,
                    core_config: begin.core_config, now_ms: begin.created_at_ms,
                    turn_context_items, turn_context_platform, turn_context_components,
                });
                self.runtime.receive_admission(result)?
            } else if let Some(lease) = &config.session.automation_execution_lease {
                self.runtime.requires_committed_restore = true;
                let result = store.begin_automation_core_turn(lease.clone(), begin);
                self.runtime.receive_admission(result)?
            } else if let Some(authority) = &config.session.host_task_execution {
                // Keep both authority and lease stable through admission; a
                // heartbeat or permission transfer cannot stale the witness.
                authority
                    .with_active(|lease| {
                        lease.with_attempt(|attempt| {
                            crate::task_runtime::prepare_expert_task_admission(&attempt.task_id, &mut begin)?;
                            self.runtime.requires_committed_restore = true;
                            let result = store.begin_task_core_turn(
                                nola_application::RuntimeTaskLeaseWitness::from(&*attempt),
                                begin,
                            );
                            self.runtime.receive_admission(result)
                        })
                    })?
                    .ok_or("task execution authority is not active")?
            } else {
                self.runtime.requires_committed_restore = true;
                let result = store.begin_core_turn(begin);
                self.runtime.receive_admission(result)?
            };
            Ok(accepted)
        })();
        // Restore request-level error delivery even when admission fails.
        emitter.end_recovery_transport_isolation();
        let accepted = admission_result.map_err(|error| {
            // A typed rejected transaction clears this fence; lost commit ACK
            // keeps it set and must never become a retryable rejection.
            if !self.runtime.requires_committed_restore {
                report_unadmitted_submission(config, &error, emitter);
            }
            error
        })?;
        emitter.submission_admission(&accepted.begin.conversation_id, &accepted.begin.run_id,
            &accepted.begin.assistant_message_id, None);
        let turn_result = {
            let store = self
                .session_store
                .as_mut()
                .expect("every CLI execution mode owns a canonical transaction Store");
            self.execution_config = config.clone();
            // An occurrence admission may replay its original transaction
            // receipt after a lost acknowledgement. Its checkpoint and events
            // are historical, not the current dispatch or presentation state.
            let automation_admission = config.session.automation_execution_lease.is_some();
            let execution = if automation_admission {
                store.load_host_execution_state()?
            } else {
                accepted.execution
            };
            {
                let mut presenter = CoreEventPresenter::new(emitter, self.tool_count)
                    .with_assistant_journal_notifications(store, execution.control.session_id.as_str());
                if !automation_admission {
                    for event in accepted.events {
                        presenter.publish(CoreHostOutput::Event(event));
                    }
                    for trace in accepted.traces {
                        presenter.publish(CoreHostOutput::Trace(trace));
                    }
                }
                presenter.publish_execution(&execution);
            }
            self.runtime
                .adopt_execution_state(execution)
                .map_err(format_runtime_error)?;
            // Only the foreground request owns the post-Turn settlement
            // boundary. Other Presenter call sites (recovery, idle commands,
            // tests) retain immediate terminal delivery.
            let physical_verification = prepare_host_invocation_verification(config)?;
            emitter.defer_terminal_transport();
            let runtime = &mut self.runtime;
            let mut presenter = CoreEventPresenter::new(emitter, self.tool_count)
                .with_assistant_journal_notifications(
                    store,
                    runtime.control_ref().session_id.as_str(),
                );
            // An automation occurrence owns exactly one Run. Finishing it
            // cannot grant its worker authority over a queued user successor.
            // Store independently enforces this boundary across cold recovery.
            runtime.execute_committed_scope(store, &mut |output| {
                // Application has accepted and persisted this Task projection
                // before publishing it. This is the first safe point at which
                // the Supervisor may claim the durable Host task, and it still
                // precedes the next parent-model effect in the same Turn.
                let queued_task_id = match &output {
                    CoreHostOutput::Event(envelope) => match &envelope.event {
                        AgentEvent::TaskUpdated { task, .. }
                            if task.state == nola_core::agent::TaskState::Queued =>
                        {
                            Some(task.id.clone())
                        }
                        _ => None,
                    },
                    _ => None,
                };
                presenter.publish(output);
                if let Some(task_id) = queued_task_id {
                    crate::task_runtime::reconcile_committed_agent_task(config, &task_id);
                }
            }, !automation_admission, &mut || physical_verification.verify())
        };
        let (consumed_task_context, terminal_snapshot) = match turn_result {
            Ok(snapshot)
                if turn_cancellation.is_requested() && !snapshot.turn_status.is_terminal() =>
            {
                // The same boundary source admits the stop before its first
                // claim, even if the synchronous wait returned before polling.
                // Never manufacture another Interrupt identity here.
                let snapshot = match self.drain_committed_terminal_effects(emitter) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        emitter.discard_deferred_terminal_transport();
                        emitter.error(&error);
                        return Err(error);
                    }
                };
                (false, snapshot)
            }
            // A transport interruption yields a durable, nonterminal turn.
            // Its task evidence has not been consumed by a completed turn.
            Ok(snapshot) => (snapshot.turn_status.is_terminal(), snapshot),
            Err(dispatch_error) if turn_cancellation.is_requested()
                && !(self.runtime.requires_committed_restore
                    && self.session_store.as_ref().and_then(SessionStore::active_core_run_id)
                        != self.runtime.executed_run_id.as_deref()) => {
                let dispatch_error = format_runtime_error(dispatch_error);
                // Host stop can race request setup. Only the shared boundary
                // producer owns its canonical cancellation submission.
                // A boundary Interrupt may already have committed while the
                // Host response was rejected. Recover that authoritative state
                // before polling that source or dispatching cancel effects.
                if self.runtime.requires_committed_restore {
                    let state = self.session_store.as_mut()
                        .expect("Core Agent Host owns a Session Store")
                        .load_host_execution_state()
                        .map_err(|error| format!("{error}; original Host dispatch error: {dispatch_error}"))?;
                    self.runtime.adopt_execution_state(state).map_err(|error|
                        format!("{}; original Host dispatch error: {dispatch_error}", format_runtime_error(error)))?;
                }
                let snapshot = match self.drain_committed_terminal_effects(emitter) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        let error = format!("{error}; original Host dispatch error: {dispatch_error}");
                        emitter.discard_deferred_terminal_transport();
                        emitter.error(&error);
                        return Err(error);
                    }
                };
                (false, snapshot)
            }
            Err(error) => {
                emitter.discard_deferred_terminal_transport();
                return Err(format_runtime_error(error));
            }
        };

        if turn_cancellation.is_requested() && !terminal_snapshot.turn_status.is_terminal() {
            let error = format!("Cancellation did not reach a canonical terminal state ({:?}); Run remains recoverable", terminal_snapshot.turn_status);
            emitter.discard_deferred_terminal_transport();
            emitter.error(&error);
            return Err(error);
        }
        let seal = if terminal_snapshot.turn_status.is_terminal() {
            let terminal_run_id = self.runtime.terminal_run_id()?.to_owned();
            match self
            .session_store
            .as_mut()
            .expect("completed Core turn owns a canonical Application Store")
            .load_assistant_run_terminal_seal(&terminal_run_id)
        {
            Ok(Some(seal)) => Some(seal),
            Ok(None) => {
                let error = "Terminal Core snapshot has no exact canonical Run seal".to_string();
                emitter.discard_deferred_terminal_transport();
                emitter.error(&error);
                return Err(error);
            }
            Err(error) => {
                emitter.discard_deferred_terminal_transport();
                emitter.error(&error);
                return Err(error);
            }
            }
        } else { None };
        let goal_signal=if self.runtime.execution.has_running_goal {
            seal.as_ref().map(|seal| self.session_store.as_mut().expect("Goal owns Session Store").goal_claim_for_seal(seal)).transpose()?.flatten()
        } else {None};
        let run_result = seal.as_ref().map(|seal| seal.result.value.clone());
        if let Some(seal) = seal {
            emitter.committed_assistant_run_terminal(&seal);
        }

        // A Core turn is not necessarily the consumption boundary for durable
        // task evidence. Goal execution may open a verifier/continuation turn
        // after a model returned prose without a goal terminal signal. Keep
        // the mailbox visible throughout that running Goal slice; otherwise
        // the verifier observes a different world state from the turn it is
        // auditing. Non-Goal conversations still acknowledge after their
        // successful turn, while paused/terminal Goal states acknowledge only
        // after the final turn had access to the same evidence.
        let task_context_acknowledged = consumed_task_context
            && !self.runtime.execution.has_running_goal;
        if task_context_acknowledged {
            if !host_policy.from_resume_snapshot {
                if let Err(error) =
                    crate::task_runtime::mark_task_context_notified(&host_policy.pending_task_ids)
                {
                    emitter.discard_deferred_terminal_transport();
                    emitter.error(&error);
                    return Err(error);
                }
            }
        }

        let process_has_active_turn = self
            .session_store
            .as_ref()
            .is_some_and(SessionStore::has_active_core_turn);
        crate::llm::log_local_debug_payload(
            "turn_settlement",
            &json!({
                "type": "turn_settlement",
                "turn_status": format!("{:?}", terminal_snapshot.turn_status),
                "process_has_active_turn": process_has_active_turn,
                "consumed_task_context": consumed_task_context,
                "task_context_acknowledged": task_context_acknowledged,
            }),
        );
        if let Err(error) = self.start_claimable_memory_workers(emitter) {
            let error = format!(
                "start memory workers after Core turn settlement failed (turn_status={:?}, process_has_active_turn={}): {error}",
                terminal_snapshot.turn_status, process_has_active_turn
            );
            emitter.discard_deferred_terminal_transport();
            emitter.error(&error);
            return Err(error);
        }
        emitter.flush_deferred_terminal_transport();
        self.last_used = Instant::now();
        Ok(CoreRunOutcome {
            turn_status: terminal_snapshot.turn_status,
            run_result,
            goal_signal,
        })
    }

    fn enforce_cumulative_task_budget(&self) -> Result<(), String> {
        let Some(limit) = std::env::var("NOLA_TASK_TOKEN_BUDGET")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|value| *value > 0)
        else {
            return Ok(());
        };
        if !self.runtime.execution.has_active_goal {
            return Ok(());
        }
        let model_tokens = self.runtime.execution.cumulative_model_tokens;
        // Provider input usage already includes tool results sent to the model.
        // The committed Core checkpoint is the accounting authority.
        if let Some(error) = cumulative_task_budget_violation(true, limit, model_tokens) {
            return Err(error);
        }
        Ok(())
    }

    fn ingest_memory_worker_results(&mut self, emitter: &mut dyn crate::runtime_output::RuntimeOutputPort) -> Result<(), String> {
        let Some(directory) = self.memory_inbox_directory.clone() else {
            return Ok(());
        };
        // A detached worker can finish just before the Host is resumed after
        // its lease deadline. Its atomic result is stronger evidence than the
        // wall-clock lease: import matching owned results before reclaiming
        // workers that truly disappeared. Sweeping first clears the ownership
        // token, causing a valid late result to be discarded and leaving the
        // durable job stuck in a misleading retry/running lifecycle.
        let results = crate::memory_worker::load_results(&directory)?;
        for (path, result) in results {
            let Some(job) = self.session_store.as_mut().expect("memory maintenance Store")
                .memory_maintenance_state()?.0.job(&result.job_id).cloned()
            else {
                let _ = std::fs::remove_file(path);
                continue;
            };
            if job.ownership_token.as_deref() != Some(result.ownership_token.as_str()) {
                let _ = std::fs::remove_file(path);
                continue;
            }
            let now = now_epoch_ms()?;
            if let Some(output) = result.output {
                let completion = self.commit_idle_submission_observing(
                    AgentSubmission::MemoryWorkerCompleted {
                        operation_id: format!("complete-worker-{}", result.job_id),
                        job_id: result.job_id.clone(),
                        ownership_token: result.ownership_token.clone(),
                        now_epoch_ms: now,
                        completion_watermark: job.input_watermark.clone(),
                        output,
                    },
                    emitter,
                    &mut |_| {},
                );
                match isolate_memory_worker_completion(completion, |error| {
                    crate::llm::log_local_debug_payload(
                        "memory_worker_result_rejected",
                        &json!({
                            "type": "memory_worker_result_rejected",
                            "job_id": &result.job_id,
                            "error": &error,
                        }),
                    );
                    // A background worker is auxiliary to the foreground
                    // Turn. Keep Core's strict semantic validation, but
                    // project a rejected result into that worker's durable
                    // retry lifecycle instead of failing the user's chat.
                    self.commit_idle_submission_observing(
                        AgentSubmission::MemoryWorkerFailed {
                            operation_id: format!(
                                "reject-worker-{}-{}",
                                result.job_id, job.attempt_count
                            ),
                            job_id: result.job_id.clone(),
                            ownership_token: result.ownership_token.clone(),
                            now_epoch_ms: now,
                            retry_after_ms: None,
                            max_retry_count: 3,
                            error: format!("memory worker output rejected: {error}"),
                        },
                        emitter,
                        &mut |_| {},
                    )?;
                    Ok(())
                })? {
                    Some(_) => {}
                    None => {}
                }
            } else {
                self.commit_idle_submission_observing(
                    AgentSubmission::MemoryWorkerFailed {
                        operation_id: format!(
                            "retry-worker-{}-{}",
                            result.job_id, job.attempt_count
                        ),
                        job_id: result.job_id.clone(),
                        ownership_token: result.ownership_token.clone(),
                        now_epoch_ms: now,
                        retry_after_ms: None,
                        max_retry_count: 3,
                        error: result
                            .error
                            .unwrap_or_else(|| "memory worker failed".to_string()),
                    },
                    emitter,
                    &mut |_| {},
                )?;
            }
            std::fs::remove_file(path)
                .map_err(|error| format!("remove imported memory result failed: {error}"))?;
            let _ = std::fs::remove_file(directory.join(format!(
                "{}.{}.worker.log",
                result.job_id, result.ownership_token
            )));
            // Clean up logs from pre-attempt-qualified worker versions too.
            let _ = std::fs::remove_file(directory.join(format!("{}.worker.log", result.job_id)));
        }
        let now = now_epoch_ms()?;
        self.commit_idle_submission_observing(
            AgentSubmission::MemoryRuntime {
                envelope: nola_core::agent::MemoryRuntimeCommandEnvelope {
                    operation_id: format!("sweep-memory-leases-{now}"),
                    command: nola_core::agent::MemoryRuntimeCommand::SweepExpiredLeases {
                        now_epoch_ms: now,
                        max_retry_count: 3,
                    },
                },
            },
            emitter,
            &mut |_| {},
        )?;
        self.schedule_memory_consolidation(emitter)?;
        Ok(())
    }

    fn schedule_memory_consolidation(&mut self, emitter: &mut dyn crate::runtime_output::RuntimeOutputPort) -> Result<(), String> {
        if !automatic_memory_maintenance_enabled(std::env::var_os(
            "NOLA_AUTOMATIC_MEMORY_MAINTENANCE_ENABLED",
        )) {
            return Ok(());
        }
        let Some(store) = self.session_store.as_mut() else {
            return Ok(());
        };
        let (records, usage_counts) = store.recent_global_memory_records_with_usage(100)?;
        let (memory_runtime, memories) = store.memory_maintenance_state()?;
        let candidates = memory_runtime.consolidation_candidates_with_usage(&records, &usage_counts);
        if candidates.is_empty() {
            return Ok(());
        }
        let updates = {
            let projected = &memories.records;
            candidates
                .iter()
                .filter_map(|record| match projected.get(&record.id) {
                    Some(current) if current == record => None,
                    Some(_) => Some(AgentSubmission::UpdateMemory {
                        record: record.clone(),
                    }),
                    None => Some(AgentSubmission::CreateMemory {
                        record: record.clone(),
                    }),
                })
                .collect::<Vec<_>>()
        };
        for submission in updates {
            self.commit_idle_submission_observing(submission, emitter, &mut |_| {})?;
        }
        let selected_memory_ids = candidates
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        let input_digest =
            nola_core::agent::memory_runtime::consolidation_input_digest(&candidates)?;
        let job_id = format!("memory-consolidation-{}", &input_digest[7..31]);
        if self.session_store.as_mut().expect("memory maintenance Store")
            .memory_maintenance_state()?.0.job(&job_id).is_some() {
            return Ok(());
        }
        let now = now_epoch_ms()?;
        self.commit_idle_submission_observing(
            AgentSubmission::MemoryRuntime {
                envelope: nola_core::agent::MemoryRuntimeCommandEnvelope {
                    operation_id: format!("create-{job_id}"),
                    command: nola_core::agent::MemoryRuntimeCommand::CreateConsolidation {
                        job_id,
                        input_watermark: format!("memory-store:{now}"),
                        input_digest,
                        selected_memory_ids,
                        now_epoch_ms: now,
                    },
                },
            },
            emitter,
            &mut |_| {},
        )?;
        Ok(())
    }

    fn resident_memory_components(
        &mut self,
        platform: ContextPlatform,
    ) -> Result<Vec<ContextComponent>, String> {
        // Resident profile recall is independent from explicit memory tools and
        // background memory maintenance. Hosts that require a clean context
        // boundary can disable this injection while leaving memory_save and
        // memory_search available to the model.
        if !resident_memory_recall_enabled(std::env::var_os("NOLA_RESIDENT_MEMORY_RECALL_ENABLED"))
        {
            return Ok(Vec::new());
        }
        let Some(store) = self.session_store.as_mut() else {
            return Ok(Vec::new());
        };
        let policy = self
            .runtime
            .config
            .memory_policy
            .clone();
        let candidates = store.recent_global_memory_records(100)?;
        let plan = select_resident_memories(candidates, policy, chrono::Utc::now().to_rfc3339());
        crate::llm::log_local_debug_payload(
            "resident_memory_selection",
            &json!({
                "type": "resident_memory_selection",
                "plan_digest": &plan.plan_digest,
                "selected_memory_ids": plan.selected.iter().map(|entry| entry.record.id.as_str()).collect::<Vec<_>>(),
                "rejected": plan.rejected.iter().map(|entry| json!({
                    "memory_id": &entry.memory_id,
                    "reason": entry.reason,
                })).collect::<Vec<_>>(),
                "used_input_tokens": plan.used_input_tokens,
                "remaining_input_tokens": plan.remaining_input_tokens,
                "token_counts_are_estimated": plan.token_counts_are_estimated,
            }),
        );
        resident_memory_components_from_plan(plan, platform)
    }

    fn start_claimable_memory_workers(&mut self, emitter: &mut dyn crate::runtime_output::RuntimeOutputPort) -> Result<(), String> {
        if !automatic_memory_maintenance_enabled(std::env::var_os(
            "NOLA_AUTOMATIC_MEMORY_MAINTENANCE_ENABLED",
        )) {
            return Ok(());
        }
        let Some(directory) = self.memory_inbox_directory.clone() else {
            return Ok(());
        };
        let now = now_epoch_ms()?;
        let jobs = self.session_store.as_mut().expect("memory maintenance Store")
            .claimable_memory_jobs(now)?
            .into_iter()
            .take(2)
            .collect::<Vec<_>>();
        for job in jobs {
            let (lease_until_epoch_ms, deadline_epoch_ms, execution_timeout_ms) =
                memory_worker_lease_contract(now);
            let ownership_token = uuid::Uuid::new_v4().to_string();
            self.commit_idle_submission_observing(
                AgentSubmission::ClaimMemoryWorker {
                    operation_id: format!("claim-worker-{}-{}", job.id, job.attempt_count),
                    job_id: job.id.clone(),
                    worker_id: "nola-memory-worker".to_string(),
                    ownership_token: ownership_token.clone(),
                    now_epoch_ms: now,
                    // Detached workers retry short transport interruptions
                    // themselves. Keep enough time for semantic extraction,
                    // but reclaim a dead local-endpoint worker promptly.
                    lease_until_epoch_ms,
                },
                emitter,
                &mut |_| {},
            )?;
            let plan = self.session_store.as_mut().expect("memory maintenance Store")
                .build_memory_worker_plan(&job.id)?;
            let request = MemoryWorkerRequest {
                plan,
                llm: MemoryWorkerLlmConfig {
                    siwc: self.memory_llm.siwc.clone(), native_model_route: self.memory_llm.native_model_route.clone(),
                    provider: self.memory_llm.provider.clone(),
                    endpoint: self.memory_llm.endpoint.clone(),
                    api_key: self.memory_llm.api_key.clone(),
                    model: self.memory_llm.model.clone(),
                    temperature: self.memory_llm.temperature,
                    top_p: self.memory_llm.top_p,
                    max_output_tokens: self.memory_llm.max_output_tokens,
                },
                inbox_directory: directory.clone(),
                deadline_epoch_ms: Some(deadline_epoch_ms),
                execution_timeout_ms: Some(execution_timeout_ms),
            };
            if let Err(error) = crate::memory_worker::spawn(&request) {
                self.commit_idle_submission_observing(
                    AgentSubmission::MemoryWorkerFailed {
                        operation_id: format!(
                            "spawn-retry-worker-{}-{}",
                            job.id, job.attempt_count
                        ),
                        job_id: job.id,
                        ownership_token,
                        now_epoch_ms: now,
                        retry_after_ms: None,
                        max_retry_count: 3,
                        error,
                    },
                    emitter,
                    &mut |_| {},
                )?;
            }
        }
        Ok(())
    }

    fn compact_session(&mut self) -> Result<CoreManualCompactionResult, String> {
        self.runtime
            .driver_mut()
            .begin_turn(RuntimeExecutionControl::default(), HostTurnPolicy::default());
        let mut completed = None;
        let mut failed = None;
        let operation_id = uuid::Uuid::new_v4().to_string();
        let core_config = self.runtime.core_config()?.clone();
        self.runtime.requires_committed_restore = true;
        let accepted = self
            .session_store
            .as_mut()
            .expect("Core Agent Host owns a Session Store")
            .begin_maintenance_core_turn(BeginMaintenanceTurn {
                conversation_id: self.session_id.clone(),
                context_checkpoint_id: None,
                core_config: core_config.clone(),
                submission: AgentSubmission::Compact {
                    operation_id: operation_id.clone(),
                },
                created_at_ms: nola_runtime_driver::monotonic_epoch_millis(),
            })?;
        self.runtime
            .adopt_execution_state(accepted.execution)
            .map_err(format_runtime_error)?;
        let store = self
            .session_store
            .as_mut()
            .expect("Core Agent Host owns a Session Store");
        let snapshot = self
            .runtime
            .execute_committed_application_port(store, &mut |output| {
                if let CoreHostOutput::Event(envelope) = output {
                    if let Some(error) = manual_compaction_failure(&envelope.event) {
                        failed = Some(error.to_owned());
                    }
                    match envelope.event {
                        AgentEvent::CompactionCompleted {
                            replaced_item_count,
                            remaining_item_count,
                            ..
                        } => {
                            completed = Some((replaced_item_count, remaining_item_count));
                        }
                        _ => {}
                    }
                }
            })
            .map_err(format_runtime_error)?;
        if let Some(error) = failed {
            return Err(error);
        }
        let (replaced_item_count, remaining_item_count) =
            completed.ok_or_else(|| "Agent Core compaction did not complete".to_string())?;
        let context_usage_percent = if snapshot.context_window_tokens == 0 {
            0.0
        } else {
            snapshot.context_used_tokens as f64 / snapshot.context_window_tokens as f64 * 100.0
        };
        self.last_used = Instant::now();
        Ok(CoreManualCompactionResult {
            replaced_item_count,
            remaining_item_count,
            context_used_tokens: snapshot.context_used_tokens,
            context_window_tokens: snapshot.context_window_tokens,
            context_usage_percent,
        })
    }
}

fn manual_compaction_failure(event: &AgentEvent) -> Option<&str> {
    match event {
        AgentEvent::CompactionFailed { error }
        | AgentEvent::TurnFailed { error, .. } => Some(error),
        _ => None,
    }
}

#[cfg(test)]
#[test]
fn manual_compaction_reports_failed_maintenance_turn() {
    let error = "Compaction summary still exceeds its admitted budget; original context retained";
    let event = AgentEvent::TurnFailed {
        session_id: nola_core::agent::SessionId("isolated-session".into()),
        turn_id: nola_core::agent::TurnId("maintenance-run".into()),
        error: error.into(),
    };
    assert_eq!(manual_compaction_failure(&event), Some(error));
    assert_eq!(manual_compaction_failure(&AgentEvent::CompactionFailed {
        error: error.into(),
    }), Some(error));
}
