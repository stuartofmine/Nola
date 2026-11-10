// SPDX-License-Identifier: Apache-2.0
// Test modules extracted from model_conversion.rs; included at the same module scope.
#[cfg(test)]
mod sop_custom_permission_host_tests {
    use super::*;
    use nola_core::agent::host::{HostEffectId, HostId};
    use nola_core::agent::sop::*;
    struct DatabaseEnvironment(Option<std::ffi::OsString>);
    impl Drop for DatabaseEnvironment {
        fn drop(&mut self) {
            if let Some(value) = &self.0 {
                std::env::set_var("NOLA_DB_PATH", value)
            } else {
                std::env::remove_var("NOLA_DB_PATH")
            }
        }
    }
    fn definition() -> SopDefinition {
        SopDefinition {
            schema_version: SOP_SCHEMA_VERSION,
            id: "application.sop".into(),
            version: 1,
            title: "Application SOP".into(),
            inputs: json!({}),
            steps: vec![SopStep {
                id: "work".into(),
                title: "Work".into(),
                depends_on: Vec::new(),
                mode: SopExecutionMode::Inline,
                concurrent: false,
                inputs: json!({}),
                outputs: json!({}),
                preconditions: Vec::new(),
                completion: SopCompletionContract {
                    description: "work is complete".into(),
                    evidence_required: true,
                },
                required_capabilities: Vec::new(),
                required_permissions: Vec::new(),
                budget: SopStepBudget::default(),
                retry: SopRetryPolicy::default(),
                artifacts: Vec::new(),
                verifier: None,
                resume_key: Some("work-key".into()),
            }],
            verification: SopVerificationContract::default(),
            compensation: SopCompensationContract::default(),
        }
    }

    #[test]
    fn sop_custom_response_stays_waiting_and_successor_approval_resumes_same_step() {
        let _environment = crate::store::test_env_lock();
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        let _restore = DatabaseEnvironment(std::env::var_os("NOLA_DB_PATH"));
        std::env::set_var("NOLA_DB_PATH", root.join("sop.db"));
        let permission_scope = crate::tools::SessionPermissionScope::default();
        permission_scope.bind_owner_session("sop-owner");
        let context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
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
            endpoint: "http://127.0.0.1:1/v1".to_string(),
            api_key: String::new(),
            model: "unused".to_string(),
            temperature: 0.0,
            top_p: 1.0,
            max_output_tokens: 1,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: false, reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            openai_prompt_cache: false,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };

        let mut stopped = false;
        let mut settled = false;
        let mut interrupted = false;
        let mut driver = CliAgentHostDriver::new("sop-host", llm, context).with_permission_handler(
            move |request, _, _, _, _| {
                if !stopped {
                    stopped = true;
                    return Err(PermissionHandlerError::StopRequested);
                }
                if !settled {
                    settled = true;
                    return Err(PermissionHandlerError::Settled);
                }
                if !interrupted {
                    interrupted = true;
                    return Err(PermissionHandlerError::TransportInterrupted(
                        nola_host_protocol::PermissionTransportInterruptionReason::Timeout,
                    ));
                }
                if request.request_id == "delivery-p1" {
                    Ok(HostPermissionResolution::provide_instruction(
                        "Keep the original encoding",
                    ))
                } else {
                    Ok(HostPermissionResolution::allow_once())
                }
            },
        );
        let store = crate::store::NolaStore::open_default().unwrap();
        store
            .create_sop_run(
                definition(),
                "sop-run".into(),
                json!({}),
                Some("sop-owner".into()),
                root.to_string_lossy().into_owned(),
            )
            .unwrap();
        store
            .apply_sop_run_event(
                "sop-run".into(),
                1,
                SopRunEvent::StartStep {
                    step_id: "work".into(),
                    executor_ref: "host-tool".into(),
                },
            )
            .unwrap();
        let input = json!({"path":root.join("file.txt"),"content":"original", nola_core::agent::tool_lifecycle::TOOL_INVOCATION_CONTEXT_ARGUMENT:{"sopRunId":"sop-run","sopStepId":"work"}});
        let request =
            |id: &str, instructions: Vec<nola_core::agent::host::PermissionInstruction>| {
                HostRequestEnvelope {
                    attempt_started: false,
                    delegated_invocation_id: None,
                    attempt_id: None,
                    request_id: format!("delivery-{id}"),
                    effect_id: HostEffectId(format!("effect-{id}")),
                    host_id: HostId("sop-host".into()),
                    lease_id: None,
                    session_id: "sop-owner".into(),
                    turn_id: Some("turn".into()),
                    catalog_revision: 1,
                    availability_epoch: 1,
                    created_at_ms: 0,
                    deadline_ms: None,
                    batch: None,
                    resume_after_sequence: None,
                    delivery: None,
                    payload: HostRequest::RequestPermission {
                        allow_conversation: true,
                        permission_request_id: id.into(),
                        capability_id: "write_file".into(),
                        resource: root.join("file.txt").to_string_lossy().into_owned(),
                        reason: "write requires approval".into(),
                        input: input.clone(),
                        evaluation: None,
                        permission_instructions: instructions,
                    },
                }
            };
        let mut foreign = request("foreign", vec![]);
        foreign.session_id = "another-session".into();
        foreign.catalog_revision = driver.catalog.revision;
        foreign.availability_epoch = driver.catalog.availability_epoch;
        let mut foreign_responses = Vec::new();
        driver
            .dispatch(foreign, &mut |response| {
                foreign_responses.push(response);
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            &foreign_responses.last().unwrap().payload,
            HostResponse::Failed { error }
                if error.code == "invalid_sop_execution_lineage"
                    && error.message.contains("another session")
        ));
        assert_eq!(
            store
                .sop_run("sop-run".into())
                .unwrap()
                .unwrap()
                .record
                .status,
            SopRunStatus::Running
        );
        let mut p1 = request("p1", vec![]);
        p1.catalog_revision = driver.catalog.revision;
        p1.availability_epoch = driver.catalog.availability_epoch;
        let mut stopped_responses = vec![];
        driver.dispatch(p1.clone(), &mut |response| {
            stopped_responses.push(response);
            Ok(())
        }).unwrap();
        assert!(stopped_responses.iter().all(|response| matches!(
            response.payload, HostResponse::ExecutionStarted
        )), "stopping the transport must not emit a decision, failure or cancellation acknowledgement");
        let stopped_identity = (p1.host_id.0.clone(), p1.session_id.clone(), p1.turn_id.clone(), p1.effect_id.0.clone());
        assert!(driver.quiescent_effects.contains_key(&stopped_identity), "the exact synchronous wait returned, without deciding the permission");
        let stopped_wait = store.sop_run("sop-run".into()).unwrap().unwrap();
        assert_eq!(stopped_wait.record.status, SopRunStatus::WaitingPermission);
        assert!(stopped_wait.record.steps["work"].current_wait.as_ref().unwrap().disposition.is_none());
        p1.resume_after_sequence = stopped_responses.last().map(|response| response.sequence);
        let mut settled_responses = vec![];
        driver.dispatch(p1.clone(), &mut |response| {
            settled_responses.push(response);
            Ok(())
        }).unwrap();
        assert!(settled_responses.iter().all(|response| matches!(
            response.payload, HostResponse::ExecutionStarted
        )), "a Core-settled occurrence must not emit another decision or failure");
        assert!(driver.quiescent_effects.contains_key(&stopped_identity));
        p1.resume_after_sequence = settled_responses.last().map(|response| response.sequence)
            .or(p1.resume_after_sequence);
        let mut interrupted_responses = vec![];
        driver
            .dispatch(p1.clone(), &mut |response| {
                interrupted_responses.push(response);
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            interrupted_responses.last().unwrap().payload,
            HostResponse::PermissionTransportInterrupted {
                reason: nola_host_protocol::PermissionTransportInterruptionReason::Timeout
            }
        ));
        assert!(interrupted_responses
            .iter()
            .all(|response| !response.payload.is_terminal()));
        // The new synchronous wait really returned. Its physical fact is not
        // a permission choice or completed SOP step, and must retain this fence.
        let returned = driver.quiescent_effects.get(&stopped_identity).unwrap();
        assert_eq!(returned.request_id, p1.request_id);
        assert_eq!(returned.delivery_attempt_id, p1.delivery.as_ref().map(|lease| lease.attempt_id.clone()));
        assert_eq!(returned.delivery, p1.delivery);
        assert!(returned.local_stop.is_none(), "wait return cannot invent a tool execution receipt");
        let waiting = store.sop_run("sop-run".into()).unwrap().unwrap();
        assert_eq!(waiting.record.status, SopRunStatus::WaitingPermission);
        assert!(waiting.record.steps["work"]
            .current_wait
            .as_ref()
            .unwrap()
            .disposition
            .is_none());
        p1.resume_after_sequence = Some(interrupted_responses.last().unwrap().sequence);
        let mut responses = vec![];
        driver
            .dispatch(p1, &mut |response| {
                responses.push(response);
                Ok(())
            })
            .unwrap();
        assert!(
            matches!(
                &responses.last().unwrap().payload,
                HostResponse::PermissionResolved {
                    resolution: HostPermissionResolution::Intervention { .. },
                    ..
                }
            ),
            "{responses:?}"
        );
        let waiting = store.sop_run("sop-run".into()).unwrap().unwrap();
        assert_eq!(waiting.record.status, SopRunStatus::WaitingPermission);
        let wait = waiting.record.steps["work"].current_wait.as_ref().unwrap();
        assert_eq!(wait.reference_id, "permission:p1");
        assert!(wait.disposition.is_none());
        // Reopen the persisted SOP through the real CLI Application port before the new delivery.
        let store = crate::store::NolaStore::open_default().unwrap();
        assert!(
            store
                .validate_sop_invocation(
                    "sop-run".into(),
                    "work".into(),
                    root.to_string_lossy().into_owned(),
                    "sop-owner".into()
                )
                .is_err(),
            "waiting permission never authorizes execution"
        );
        assert!(
            store
                .validate_sop_permission_invocation(
                    "sop-run".into(),
                    "work".into(),
                    root.to_string_lossy().into_owned(),
                    "sop-owner".into(),
                    "permission:wrong".into()
                )
                .is_err(),
            "another wait cannot resume this step"
        );
        let mut p2 = request(
            "p2",
            vec![nola_core::agent::host::PermissionInstruction {
                permission_request_id: "p1".into(),
                instruction: "Keep the original encoding".into(),
            }],
        );
        p2.catalog_revision = driver.catalog.revision;
        p2.availability_epoch = driver.catalog.availability_epoch;
        let mut responses = vec![];
        driver
            .dispatch(p2, &mut |response| {
                responses.push(response);
                Ok(())
            })
            .unwrap();
        assert!(
            matches!(
                &responses.last().unwrap().payload,
                HostResponse::PermissionResolved {
                    resolution: HostPermissionResolution::Authorization { .. },
                    ..
                }
            ),
            "{responses:?}"
        );
        let resumed = store.sop_run("sop-run".into()).unwrap().unwrap();
        let step = &resumed.record.steps["work"];
        assert_eq!(step.status, SopStepRunStatus::Running);
        assert!(step.current_wait.is_none());
        assert_eq!(step.wait_history.len(), 1);
        assert_eq!(step.wait_history[0].reference_id, "permission:p1");
        assert_eq!(
            step.wait_history[0].disposition,
            Some(SopWaitDisposition::Continue)
        );
        assert_eq!(step.attempt, 1);
        assert!(
            !root.join("file.txt").exists(),
            "permission Host must not execute tool"
        );
    }
}
