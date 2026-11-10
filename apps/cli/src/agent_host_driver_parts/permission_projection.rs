// SPDX-License-Identifier: Apache-2.0
// Permission wait projection and delegated permission identity contracts.
// Included by the Host facade; no separate dispatch loop or stop authority.

impl CliAgentHostDriver {
    fn dispatch_permission_request(
        &mut self,
        request: &HostRequestEnvelope,
        permission: HostRequest,
        permission_wait_returned: &mut bool,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        match permission {
            HostRequest::RequestPermission {
                allow_conversation: _,
                permission_request_id,
                capability_id,
                resource,
                reason,
                mut input,
                evaluation: _,
                permission_instructions,
            } => {
                let mut sequence = Self::response_sequence(&request);
                if let Err(message) = crate::tools::shell_permission_preflight(&capability_id, &input) {
                    Self::emit_failure(&request, &mut sequence, "tool_preflight_failed", message, false, emit)?;
                    return Ok(());
                }
                // A custom instruction rotates the delivery request, while
                // the SOP step stays suspended on the original permission.
                let original_request_id = permission_instructions
                    .first()
                    .map(|instruction| instruction.permission_request_id.as_str())
                    .unwrap_or(&permission_request_id);
                let wait_reference = format!("permission:{original_request_id}");
                let invocation = match validate_and_strip_sop_permission_context(
                    &mut input,
                    &self.tool_context,
                    &request.session_id,
                    &wait_reference,
                ) {
                    Ok(invocation) => invocation,
                    Err(error) => {
                        Self::emit_failure(
                            &request,
                            &mut sequence,
                            "invalid_sop_execution_lineage",
                            error,
                            false,
                            emit,
                        )?;
                        return Ok(());
                    }
                };
                if let Some(invocation) = invocation.as_ref() {
                    let transition = crate::store::NolaStore::open_default().and_then(|store| {
                        store.ensure_sop_permission_wait(
                            invocation.sop_run_id.clone(),
                            invocation.sop_step_id.clone(),
                            wait_reference.clone(),
                            reason.clone(),
                        )
                    });
                    if let Err(error) = transition {
                        Self::emit_failure(
                            &request,
                            &mut sequence,
                            "sop_permission_transition_failed",
                            error,
                            false,
                            emit,
                        )?;
                        return Ok(());
                    }
                }
                let descriptor = self.catalog.resolve_available(&capability_id)
                    .ok_or("permission capability is unavailable in the current Host catalog")?;
                if request.catalog_revision != self.catalog.revision {
                    return Err("permission request catalog revision mismatch".into());
                }
                let mcp_descriptor = (descriptor.input_normalization.origin.kind == nola_core::agent::host::HostCapabilityOriginKind::Mcp)
                    .then(|| serde_json::to_value(descriptor)).transpose().map_err(|error| error.to_string())?;
                let _catalog_evidence = crate::host_tool_permission_evidence::enter_with_mcp(json!({
                    "catalog_revision": self.catalog.revision,
                    "tool": {"tool_id": capability_id, "route": if descriptor.input_normalization.origin.kind
                        == nola_core::agent::host::HostCapabilityOriginKind::Device {"device"} else {"agent_host"},
                        "schema": discovery_tool_schema(descriptor),
                        "mcp_route": descriptor.input_normalization.mcp_route}
                }), mcp_descriptor);
                let wait_cancellation = self.tool_context.cancellation.child();
                self.active_effect_cancellations.lock()
                    .map_err(|_| "effect cancellation registry poisoned")?
                    .insert(request.effect_id.0.clone(), wait_cancellation.context());
                let wait_guard = ActiveEffectCancellationGuard {
                    effect_id: request.effect_id.0.clone(),
                    registry: std::sync::Arc::clone(&self.active_effect_cancellations),
                };
                let resolution_result = (self.permission_handler)(
                    &request,
                    &capability_id,
                    &resource,
                    &reason,
                    &input,
                );
                drop(wait_guard);
                // This synchronous invocation has returned and its exact waiter
                // registration is gone. Retain physical quiescence, including
                // transport loss, without turning EOF into a permission decision
                // or a cancellation acknowledgement.
                *permission_wait_returned = true;
                // A disconnected native transport is still unknown unless
                // this exact execution requested stop AND its synchronous wait
                // has actually returned. Do not infer cancellation from EOF.
                let resolution_result = match resolution_result {
                    Err(PermissionHandlerError::TransportInterrupted(
                        nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
                    )) if wait_cancellation.is_requested() => {
                        Err(PermissionHandlerError::StopRequested)
                    }
                    result => result,
                };
                let mut resolution = match resolution_result {
                    Ok(resolution) => resolution,
                    Err(PermissionHandlerError::Settled) => {
                        // Do not replay a choice or fabricate failure for an
                        // effect already consumed by the canonical reducer.
                        *permission_wait_returned = true;
                        return Ok(());
                    }
                    Err(PermissionHandlerError::StopRequested) => {
                        // Core's canonical cancellation still owns the pending
                        // effect. Neither Deny nor UI failure describes this.
                        // The exact synchronous wait did return: retain only
                        // that physical fact for a later separate Cancel.
                        *permission_wait_returned = true;
                        return Ok(());
                    }
                    Err(PermissionHandlerError::TransportInterrupted(reason)) => {
                        Self::emit(
                            &request,
                            &mut sequence,
                            HostResponse::PermissionTransportInterrupted { reason },
                            emit,
                        )?;
                        return Ok(());
                    }
                    Err(PermissionHandlerError::Failed(error)) => {
                        Self::emit_failure(
                            &request,
                            &mut sequence,
                            "permission_ui_failed",
                            error,
                            false,
                            emit,
                        )?;
                        return Ok(());
                    }
                };
                if matches!(
                    &resolution,
                    HostPermissionResolution::Intervention {
                        intervention: HostPermissionIntervention::ProvideInstruction { instruction }
                    } if instruction.trim().is_empty()
                ) {
                    resolution = HostPermissionResolution::return_to_composer();
                }
                if let Some(invocation) = invocation.filter(|_| {
                    !matches!(
                        &resolution,
                        HostPermissionResolution::Intervention {
                            intervention: HostPermissionIntervention::ProvideInstruction { .. }
                        }
                    )
                }) {
                    let allowed = resolution.is_allowed();
                    let disposition = if allowed {
                        nola_core::agent::SopWaitDisposition::Continue
                    } else {
                        nola_core::agent::SopWaitDisposition::Replan
                    };
                    let evidence = format!(
                        "permission-result:{permission_request_id}:{}",
                        if allowed { "allowed" } else { "not_allowed" }
                    );
                    let transition = crate::store::NolaStore::open_default().and_then(|store| {
                        store.resolve_sop_permission_wait(
                            invocation.sop_run_id,
                            invocation.sop_step_id,
                            wait_reference,
                            disposition,
                            vec![evidence],
                        )
                    });
                    if let Err(error) = transition {
                        Self::emit_failure(
                            &request,
                            &mut sequence,
                            "sop_permission_transition_failed",
                            error,
                            false,
                            emit,
                        )?;
                        return Ok(());
                    }
                }
                Self::emit(
                    &request,
                    &mut sequence,
                    HostResponse::PermissionResolved {
                        request_id: permission_request_id,
                        resolution,
                    },
                    emit,
                )?
            },
            _ => return Err("permission projection requires a permission request".into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod delegated_invocation_identity_tests {
    use super::*;

    #[test]
    fn delegated_permission_duplicate_is_rejected_before_worker_enqueue() {
        let root = std::env::temp_dir().join(format!(
            "nola-permission-duplicate-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "deny".into(), Vec::new()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: crate::tools::BrowserRuntime::new(root.join("browser")),
        };
        let llm = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "http://127.0.0.1:1/v1".into(),
            api_key: String::new(),
            model: "unused".into(),
            temperature: 0.0,
            top_p: 1.0,
            max_output_tokens: 1,
            thinking_enabled: false, reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            max_retries: crate::llm::default_llm_max_retries(),
            openai_prompt_cache: false,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };
        let mut driver = CliAgentHostDriver::new("host", llm, context);
        let control: HostRequestEnvelope = serde_json::from_value(json!({
            "request_id":"control", "effect_id":"control-effect", "host_id":"host", "session_id":"conversation",
            "catalog_revision":driver.catalog.revision,"availability_epoch":driver.catalog.availability_epoch,"created_at_ms":0,
            "payload":{"type":"resolve_delegated_permission","target_effect_id":"parent-effect","backend_id":"codex",
                "permission_request_id":"rpc","occurrence":{"occurrence_id":"occurrence","worker_generation":"generation"},
                "delivery":{"delivery_id":"delivery","invocation_id":"invocation","worker_generation":"generation","occurrence_id":"occurrence","call_id":"call","request_digest":"digest"},
                "resolution":{"type":"authorization","decision":"allow_once"}}
        })).unwrap();
        let (control_tx, control_rx) = std::sync::mpsc::channel();
        let (output_tx, output_rx) = std::sync::mpsc::sync_channel(1);
        output_tx
            .send(DelegatedAgentWorkerOutput::Suspended)
            .unwrap();
        let mut parent = control.clone();
        parent.effect_id.0 = "parent-effect".into();
        parent.delegated_invocation_id = Some("invocation".into());
        driver.insert_delegated_agent_session(
            "parent-effect".into(),
            DelegatedAgentWorkerSession {
                effect_id: "parent-effect".into(),
                permission_delivery_ids: Default::default(),
                request: parent,
                next_sequence: 1,
                control_tx,
                output_rx,
                cancellation: Default::default(),
                worker: None,
                admission_failure: Default::default(),
                cancellation_registry: std::sync::Arc::clone(&driver.active_effect_cancellations),
            },
        );
        driver
            .dispatch(control.clone(), &mut |_| {
                panic!("enqueue is not delivery evidence")
            })
            .unwrap();
        assert!(matches!(
            control_rx.try_recv().unwrap(),
            DelegatedAgentWorkerControl::ResolvePermission { .. }
        ));
        let error = driver
            .dispatch(control, &mut |_| {
                panic!("duplicate cannot produce evidence")
            })
            .unwrap_err();
        assert!(error.contains("already been submitted"), "{error}");
        assert!(matches!(
            control_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        drop(driver);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delegated_dispatch_requires_committed_identity_and_echoes_cold_interruption() {
        let root =
            std::env::temp_dir().join(format!("nola-delegated-identity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "deny".into(), Vec::new()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: crate::tools::BrowserRuntime::new(root.join("browser")),
        };
        let llm = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "http://127.0.0.1:1/v1".into(),
            api_key: String::new(),
            model: "unused".into(),
            temperature: 0.0,
            top_p: 1.0,
            max_output_tokens: 1,
            thinking_enabled: false, reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            max_retries: crate::llm::default_llm_max_retries(),
            openai_prompt_cache: false,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };
        let mut driver = CliAgentHostDriver::new("identity-host", llm, context);
        let mut request: HostRequestEnvelope = serde_json::from_value(json!({
            "request_id":"delivery-1", "effect_id":"effect-1", "host_id":"identity-host",
            "session_id":"conversation-1", "turn_id":"turn-1", "catalog_revision": driver.catalog.revision,
            "availability_epoch": driver.catalog.availability_epoch, "created_at_ms":0, "deadline_ms":null,
            "payload":{"type":"delegated_agent","request":{
                "contract_version":1,"backend_id":"claude","text_part_id":"text-1","reasoning_part_id":"reasoning-1",
                "context":{"mode":"bootstrap","identity_digest":"frozen","target_anchor_item_id":"input-1",
                    "messages":[{"role":"user","content":"fixture"}]}, "tools":[],"settings":{}
            }}
        })).unwrap();
        let mut emitted = Vec::new();
        for identity in [None, Some(" ".to_string())] {
            request.delegated_invocation_id = identity;
            let error = driver
                .dispatch(request.clone(), &mut |response| {
                    emitted.push(response);
                    Ok(())
                })
                .unwrap_err();
            assert!(error.contains("committed invocation identity"), "{error}");
            assert!(driver.delegated_agent_sessions.is_empty());
            assert!(emitted.is_empty());
        }
        request.delegated_invocation_id = Some(uuid::Uuid::new_v4().to_string());
        request.attempt_id = Some("incorrect-model-attempt".into());
        let error = driver
            .dispatch(request.clone(), &mut |response| {
                emitted.push(response);
                Ok(())
            })
            .unwrap_err();
        assert!(error.contains("cannot claim a model Attempt"), "{error}");
        assert!(emitted.is_empty());
        request.attempt_id = None;
        request.resume_after_sequence = Some(1);
        driver
            .dispatch(request.clone(), &mut |response| {
                emitted.push(response);
                Ok(())
            })
            .unwrap();
        assert_eq!(emitted.len(), 1);
        assert_eq!(
            emitted[0].delegated_invocation_id,
            request.delegated_invocation_id
        );
        assert_eq!(emitted[0].attempt_id, None);
        assert!(matches!(
            emitted[0].payload,
            HostResponse::DelegatedExecutionInterrupted { .. }
        ));
        assert!(driver.delegated_agent_sessions.is_empty());
        assert!(driver.quiescent_effects.is_empty(), "cold interruption proves no worker stop");
        let mut cancel = request.clone();
        cancel.effect_id.0 = "cancel-cold".into();
        cancel.request_id = "cancel-cold-request".into();
        cancel.resume_after_sequence = None;
        cancel.delegated_invocation_id = None;
        cancel.payload = HostRequest::Cancel { target_effect_id: request.effect_id, target_not_started: None };
        emitted.clear();
        let error = driver.dispatch(cancel, &mut |response| { emitted.push(response); Ok(()) }).unwrap_err();
        assert!(error.contains("stop is unproven"), "{error}");
        assert!(emitted.is_empty(), "a cold registry must not manufacture a stop confirmation");
        drop(driver);
        std::fs::remove_dir_all(root).unwrap();
    }
}
