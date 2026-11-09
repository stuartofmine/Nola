#[cfg(test)]
mod dynamic_discovery_allowlist_tests {
    use super::*;

    #[test]
    fn read_only_agent_allowlist_cannot_rediscover_spawn_agent() {
        let root =
            std::env::temp_dir().join(format!("nola-read-only-discovery-{}", uuid::Uuid::new_v4()));
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
        let mut driver = CliAgentHostDriver::new("host", llm, context)
            .with_tool_allowlist(Some(["tool_search".to_string(), "read_file".to_string()]));

        driver.refresh_current_turn_available_tool_ids();

        assert!(driver
            .current_turn_available_tool_ids
            .contains("tool_search"));
        assert!(driver.current_turn_available_tool_ids.contains("read_file"));
        assert!(!driver
            .current_turn_available_tool_ids
            .contains("spawn_agent"));
        assert!(!driver
            .current_turn_available_tool_ids
            .contains("write_file"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_authorization_allowlist_preserves_deferred_model_exposure() {
        let root = std::env::temp_dir().join(format!(
            "nola-authorized-discovery-{}",
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
        let mut driver = CliAgentHostDriver::new("host", llm, context)
            .with_authorized_tool_allowlist(Some([
                "tool_search".to_string(),
                "read_file".to_string(),
                "edit_file".to_string(),
                "memory_save".to_string(),
            ]));

        driver.refresh_current_turn_available_tool_ids();

        assert_eq!(
            driver.catalog.capabilities["read_file"].exposure,
            HostCapabilityExposure::Visible
        );
        assert_eq!(
            driver.catalog.capabilities["edit_file"].exposure,
            HostCapabilityExposure::Visible
        );
        assert_eq!(
            driver.catalog.capabilities["memory_save"].exposure,
            HostCapabilityExposure::Deferred
        );
        assert!(driver.current_turn_available_tool_ids.contains("read_file"));
        assert!(driver.current_turn_available_tool_ids.contains("edit_file"));
        assert!(driver.current_turn_available_tool_ids.contains("memory_save"));
        assert!(!driver.current_turn_available_tool_ids.contains("bash"));
        assert!(driver
            .current_turn_discovery_tool_schemas()
            .iter()
            .any(|schema| {
                schema.pointer("/function/name").and_then(Value::as_str) == Some("read_file")
            }));
        let full = crate::agent_registry::AgentRegistry::from_profiles(&Default::default())
            .tool_schemas_for("general", &driver.catalog.capabilities.values()
                .map(discovery_tool_schema).collect::<Vec<_>>());
        let allowed = full.iter().filter_map(|tool| tool.pointer("/function/name")
            .and_then(Value::as_str).map(str::to_string)).collect::<Vec<_>>();
        driver.configure_tool_allowlist(Some(allowed.clone()), false);
        driver.refresh_current_turn_available_tool_ids();
        let visible = driver.catalog.capabilities.values().filter(|tool|
            allowed.contains(&tool.id) && tool.exposure.is_model_visible())
            .map(discovery_tool_schema).collect::<Vec<_>>();
        assert!(visible.iter().any(|tool| tool.pointer("/function/name").and_then(Value::as_str) == Some("tool_search")));
        assert!(!visible.iter().any(|tool| tool.pointer("/function/name").and_then(Value::as_str) == Some("memory_save")));
        assert!(driver.current_turn_discovery_tool_schemas().iter().any(|tool|
            tool.pointer("/function/name").and_then(Value::as_str) == Some("memory_save")));
        assert!(!driver.current_turn_available_tool_ids.contains("spawn_agent"));
        println!("CHILD_PROJECTION full={} bytes={} visible={} bytes={}", full.len(),
            serde_json::to_vec(&full).unwrap().len(), visible.len(), serde_json::to_vec(&visible).unwrap().len());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_projects_native_host_provider_schemas_without_provider_special_cases() {
        let root = std::env::temp_dir().join(format!(
            "nola-native-provider-discovery-{}",
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
        let mut driver = CliAgentHostDriver::new("host", llm, context);
        let mut native = driver
            .catalog
            .capabilities
            .get("read_file")
            .expect("seed descriptor")
            .clone();
        native.id = "repository_memory_search".to_string();
        native.title = "Search the native repository memory graph".to_string();
        native.input_schema["description"] = json!("Query the repository memory graph using the configured native provider.");
        native.exposure = HostCapabilityExposure::Deferred;
        driver
            .catalog
            .capabilities
            .insert(native.id.clone(), native);

        driver.refresh_current_turn_available_tool_ids();
        let schemas = driver.current_turn_discovery_tool_schemas();
        let projected = schemas.iter().find(|schema| {
            schema.pointer("/function/name").and_then(Value::as_str)
                == Some("repository_memory_search")
        });

        assert!(projected.is_some());
        assert_eq!(
            projected.and_then(|schema| {
                schema
                    .pointer("/function/description")
                    .and_then(Value::as_str)
            }),
            Some("Query the repository memory graph using the configured native provider.")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}


#[cfg(test)]
mod effect_delivery_start_barrier_tests {
    use super::*;
    use nola_core::agent::host::{EffectDeliveryLease, HostEffectId};
    use std::io::{BufRead, Read, Write};
    use std::sync::Arc;

    #[test]
    fn workspace_rejection_keeps_its_cause_without_starting_the_effect() {
        let root = tempfile::tempdir().unwrap();
        let settings = crate::host_execution_settings::HostExecutionSettings {
            memory_workspace_id: None,
            bounded_file_writes: true, task_worktree: None, task_sandbox_requirement: None,
            external: crate::external_execution_settings::ExternalExecutionSettings {
                project_root: root.path().into(), execution_backend: "nola".into(),
                external_execution_session_id: None, external_execution_model: None,
                external_execution_model_provider: None, host_context_components: Vec::new(),
                host_attachments: Vec::new(), budget: Default::default(),
                permission_profile: crate::permissions::PermissionProfile::Cautious,
                host_input_origin: nola_core::agent::AgentInputOrigin::SystemRecovery,
            },
            reasoning_tier: Default::default(), native_model_route: false,
            admitted_memory_policy: None,
        };
        let mut driver = CliAgentHostDriver::new("delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()))
            .with_execution_settings(settings)
            .with_context_handler(|_, _, _| panic!("rejected effect must not start"));
        for fenced in [true, false] {
            let mut request = fenced_request(&driver, HostRequest::FetchContext {
                source_id: "fixture".into(), query: Value::Null, freshness_deadline_ms: None,
            });
            if !fenced { request.delivery = None; }
            let mut responses = Vec::new();
            driver.dispatch(request, &mut |response| { responses.push(response); Ok(()) }).unwrap();
            assert_eq!(responses.len(), 1);
            match &responses[0].payload {
                HostResponse::ExecutionRejected { code, .. } => assert_eq!(code, "task_workspace_unavailable"),
                HostResponse::Failed { error } => assert_eq!(error.code, "task_workspace_unavailable"),
                other => panic!("unexpected workspace response: {other:?}"),
            }
            assert!(driver.started_effect_deliveries.is_empty());
        }
    }

    #[test]
    fn revoked_memory_authority_rejects_work_before_start_but_allows_cancellation() {
        let _environment = crate::store::test_env_lock();
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match &self.0 {
                    Some(value) => std::env::set_var("NOLA_LONG_TERM_MEMORY_ENABLED", value),
                    None => std::env::remove_var("NOLA_LONG_TERM_MEMORY_ENABLED"),
                }
            }
        }
        let _restore = Restore(std::env::var_os("NOLA_LONG_TERM_MEMORY_ENABLED"));
        std::env::set_var("NOLA_LONG_TERM_MEMORY_ENABLED", "false");
        let root = tempfile::tempdir().unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let mut config = Config::for_test().unwrap();
        config.runtime.admitted_memory_policy = Some(nola_core::memory::MemoryPolicy::default());
        let mut driver = CliAgentHostDriver::new(
            "delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()),
            test_context(root.path()),
        )
        .with_execution_settings(config.host_execution_settings())
        .with_context_handler(move |_, _, _| {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Vec::new())
        });
        let request = fenced_request(
            &driver,
            HostRequest::FetchContext {
                source_id: "fixture".into(),
                query: Value::Null,
                freshness_deadline_ms: None,
            },
        );
        let mut responses = Vec::new();
        driver
            .dispatch(request.clone(), &mut |response| {
                responses.push(response);
                Ok(())
            })
            .unwrap();
        assert_eq!(responses.len(), 1);
        assert!(
            matches!(&responses[0].payload, HostResponse::ExecutionRejected { code, .. }
            if code == "memory_authority_unavailable")
        );
        assert!(driver.started_effect_deliveries.is_empty());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);

        let mut unfenced = request;
        unfenced.delivery = None;
        unfenced.batch = Some(nola_host_protocol::HostRequestBatchIdentity {
            batch_id: "memory-revocation-batch".into(),
            index: 0,
            size: 1,
        });
        responses.clear();
        driver
            .dispatch_batch(vec![unfenced], &mut |response| {
                responses.push(response);
                Ok(())
            })
            .unwrap();
        assert_eq!(responses.len(), 1);
        assert!(
            matches!(&responses[0].payload, HostResponse::Failed { error }
            if error.code == "memory_authority_unavailable")
        );
        assert!(driver.started_effect_deliveries.is_empty());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);

        let mut cancel = fenced_request(
            &driver,
            HostRequest::Cancel {
                target_effect_id: HostEffectId("effect-fenced".into()),
                target_not_started: None,
            },
        );
        cancel.effect_id = HostEffectId("effect-cancel".into());
        cancel.request_id = "request-cancel".into();
        responses.clear();
        driver
            .dispatch(cancel, &mut |response| {
                responses.push(response);
                Ok(())
            })
            .unwrap();
        assert!(responses.iter().any(|response| matches!(&response.payload,
            HostResponse::CancellationConfirmed { target_effect_id } if target_effect_id.0 == "effect-fenced")));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn cancellation_requires_positive_local_completion_and_the_exact_owner() {
        let root = tempfile::tempdir().unwrap();
        let mut driver = CliAgentHostDriver::new("delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()))
            .with_context_handler(|_, _, _| Ok(Vec::new()));
        let target = fenced_request(&driver, HostRequest::FetchContext {
            source_id: "fixture".into(), query: Value::Null, freshness_deadline_ms: None,
        });
        let mut cancel = target.clone();
        cancel.effect_id.0 = "cancel-known".into();
        cancel.request_id = "cancel-known-request".into();
        cancel.delivery.as_mut().unwrap().attempt_id = "cancel-known-attempt".into();
        cancel.payload = HostRequest::Cancel { target_effect_id: target.effect_id.clone(), target_not_started: None };
        // An empty driver cannot prove an unseen target is stopped.
        let mut unseen = cancel.clone();
        unseen.delivery.as_mut().unwrap().attempt_id = "cancel-unseen-attempt".into();
        let mut unproven_responses = Vec::new();
        assert!(driver.dispatch(unseen, &mut |response| {
            unproven_responses.push(response); Ok(())
        }).unwrap_err().contains("stop is unproven"));
        assert!(unproven_responses.is_empty(), "unproven cancellation must not consume ExecutionStarted");
        assert!(!driver.started_effect_deliveries.contains("cancel-unseen-attempt"));
        driver.dispatch(target, &mut |_| Ok(())).unwrap();
        assert_eq!(driver.quiescent_effects.len(), 1);
        for (index, foreign) in ["session", "turn"].into_iter().enumerate() {
            let mut wrong = cancel.clone();
            wrong.delivery.as_mut().unwrap().attempt_id = format!("cancel-wrong-{index}");
            if foreign == "session" { wrong.session_id = "foreign".into(); }
            else { wrong.turn_id = Some("foreign".into()); }
            let mut confirmed = false;
            assert!(driver.dispatch(wrong, &mut |response| {
                confirmed |= matches!(response.payload, HostResponse::CancellationConfirmed { .. }); Ok(())
            }).is_err());
            assert!(!confirmed);
        }
        let mut confirmed = false;
        driver.dispatch(cancel.clone(), &mut |response| {
            confirmed |= matches!(response.payload, HostResponse::CancellationConfirmed { .. }); Ok(())
        }).unwrap();
        assert!(confirmed);
        driver.begin_successor_turn();
        cancel.delivery.as_mut().unwrap().attempt_id = "cancel-after-reset".into();
        assert!(driver.dispatch(cancel, &mut |_| Ok(())).unwrap_err().contains("stop is unproven"));
    }

    #[test]
    fn returned_permission_wait_preserves_exact_stop_witness_across_same_run_resume() {
        for stop_requested in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut driver = CliAgentHostDriver::new("delivery-host",
                test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()))
                .with_boundary_session_id("session-fenced");
            driver.bind_file_read_run("turn-fenced");
            let control = driver.tool_context.cancellation.clone();
            driver = driver.with_permission_handler(move |_, _, _, _, _| {
                if stop_requested { control.request(); }
                Err(PermissionHandlerError::TransportInterrupted(
                    nola_host_protocol::PermissionTransportInterruptionReason::Disconnected))
            });
            let target = fenced_request(&driver, HostRequest::RequestPermission {
                allow_conversation: true,
                permission_request_id: "permission-fixture".into(), capability_id: "read_file".into(),
                resource: root.path().join("fixture.txt").to_string_lossy().into_owned(),
                reason: "explicit fixture ask".into(), input: json!({"path":"fixture.txt"}),
                evaluation: None, permission_instructions: Vec::new(),
            });
            let identity = (target.host_id.0.clone(), target.session_id.clone(),
                target.turn_id.clone(), target.effect_id.0.clone());
            let mut responses = Vec::new();
            driver.dispatch(target.clone(), &mut |response| { responses.push(response); Ok(()) }).unwrap();
            assert!(driver.quiescent_effects.contains_key(&identity));
            assert_eq!(responses.iter().any(|response| matches!(response.payload,
                HostResponse::PermissionTransportInterrupted { .. })), !stop_requested);
            assert!(!responses.iter().any(|response| matches!(response.payload,
                HostResponse::PermissionResolved { .. } | HostResponse::CancellationConfirmed { .. })));
            // A disconnected waiter is not itself Cancelled. A subsequent
            // explicit control may use the physical return fact after warm resume.
            // This driver unit test supplies an exact authority port. SQLite
            // recovery tests exercise the production port and reject rebinding.
            driver.cancellation_stop_validator = |conversation, run, _, request_id, delivery| {
                assert_eq!(conversation, "session-fenced"); assert_eq!(run, "turn-fenced");
                assert!(!request_id.is_empty()); assert_eq!(delivery.unwrap().attempt_id, "attempt-fenced"); Ok(())
            };
            driver.begin_resumed_turn("turn-fenced", driver.tool_context.cancellation.clone(), HostTurnPolicy::default());
            assert!(driver.quiescent_effects.contains_key(&identity));
            let mut exact_cancel = target.clone();
            exact_cancel.effect_id.0 = "resumed-wait-cancel".into();
            exact_cancel.request_id = "resumed-wait-cancel-request".into();
            exact_cancel.delivery.as_mut().unwrap().attempt_id = "resumed-wait-cancel-attempt".into();
            exact_cancel.payload = HostRequest::Cancel { target_effect_id: target.effect_id.clone(), target_not_started: None };
            let mut resumed_responses = Vec::new();
            driver.dispatch(exact_cancel, &mut |response| { resumed_responses.push(response); Ok(()) }).unwrap();
            assert!(resumed_responses.iter().any(|response| matches!(response.payload, HostResponse::CancellationConfirmed { .. })));
            if stop_requested {
                let mut business = fenced_request(&driver, HostRequest::FetchContext {
                    source_id: "must-not-dispatch".into(), query: Value::Null, freshness_deadline_ms: None,
                });
                business.effect_id.0 = "skipped-business".into();
                business.delivery.as_mut().unwrap().attempt_id = "skipped-attempt".into();
                let mut cancel = target.clone();
                cancel.effect_id.0 = "wait-cancel".into();
                cancel.request_id = "wait-cancel-request".into();
                cancel.delivery.as_mut().unwrap().attempt_id = "wait-cancel-attempt".into();
                cancel.payload = HostRequest::Cancel { target_effect_id: target.effect_id.clone(), target_not_started: None };
                responses.clear();
                driver.dispatch_batch(vec![business, cancel], &mut |response| { responses.push(response); Ok(()) }).unwrap();
                assert!(responses.iter().all(|response| response.effect_id.0 == "wait-cancel"));
                assert!(responses.iter().any(|response| matches!(response.payload,
                    HostResponse::CancellationConfirmed { .. })));
                assert!(!driver.started_effect_deliveries.contains("skipped-attempt"));
            }
            driver.begin_resumed_turn("successor-run", RuntimeExecutionControl::default(), HostTurnPolicy::default());
            assert!(!driver.quiescent_effects.contains_key(&identity));
        }
    }

    #[test]
    fn cancel_of_the_exact_skipped_target_uses_only_current_queued_not_started_proof() {
        let root = tempfile::tempdir().unwrap();
        let mut driver = CliAgentHostDriver::new("delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()))
            .with_boundary_session_id("session-fenced")
            .with_context_handler(|_, _, _| panic!("cancelled queued target must not execute"));
        driver.bind_file_read_run("turn-fenced");
        driver.tool_context.cancellation.request();
        let target = fenced_request(&driver, HostRequest::FetchContext {
            source_id: "queued-only".into(), query: Value::Null, freshness_deadline_ms: None,
        });
        let mut cancel = target.clone();
        cancel.request_id = "cancel-queued-request".into();
        cancel.effect_id.0 = "cancel-queued-effect".into();
        cancel.delivery.as_mut().unwrap().attempt_id = "cancel-queued-attempt".into();
        cancel.payload = HostRequest::Cancel { target_effect_id: target.effect_id.clone(), target_not_started: None };
        let mut responses = Vec::new();
        driver.dispatch_batch(vec![target.clone(),cancel.clone()], &mut |response| {
            responses.push(response); Ok(())
        }).unwrap();
        assert_eq!(responses.len(), 2);
        assert!(responses.iter().all(|response| response.effect_id == cancel.effect_id));
        assert!(matches!(responses[0].payload, HostResponse::ExecutionStarted));
        assert!(matches!(&responses[1].payload, HostResponse::CancellationConfirmed { target_effect_id }
            if target_effect_id == &target.effect_id));
        assert!(!driver.started_effect_deliveries.contains("attempt-fenced"));

        // Identical persisted identities in a fresh Host do not prove that an
        // absent old target stopped. Only the observed queued request above did.
        let mut cold = CliAgentHostDriver::new("delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()))
            .with_boundary_session_id("session-fenced");
        cold.bind_file_read_run("turn-fenced");
        cold.tool_context.cancellation.request();
        responses.clear();
        assert!(cold.dispatch_batch(vec![cancel], &mut |response| {
            responses.push(response); Ok(())
        }).unwrap_err().contains("stop is unproven"));
        assert!(responses.is_empty());
    }

    #[test]
    fn skipped_started_resumed_or_foreign_targets_never_gain_not_started_proof() {
        for invalid in ["started", "host", "session", "turn", "catalog", "resumed", "model_started", "active", "missing_lease", "expired", "unbound"] {
            let root = tempfile::tempdir().unwrap();
            let mut driver = CliAgentHostDriver::new("delivery-host",
                test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()))
                .with_boundary_session_id("session-fenced");
            driver.bind_file_read_run("turn-fenced");
            driver.tool_context.cancellation.request();
            let mut target = fenced_request(&driver, HostRequest::FetchContext {
                source_id: "never-run".into(), query: Value::Null, freshness_deadline_ms: None,
            });
            let mut cancel = target.clone();
            cancel.request_id = "cancel-unproven-queued".into();
            cancel.effect_id.0 = "cancel-unproven-queued-effect".into();
            cancel.delivery.as_mut().unwrap().attempt_id = "cancel-unproven-queued-attempt".into();
            cancel.payload = HostRequest::Cancel { target_effect_id: target.effect_id.clone(), target_not_started: None };
            match invalid {
                "started" => { driver.started_effect_deliveries.insert("attempt-fenced".into()); }
                "host" => target.host_id.0 = "foreign-host".into(),
                "session" => target.session_id = "foreign-session".into(),
                "turn" => target.turn_id = Some("foreign-run".into()),
                "catalog" => target.catalog_revision = target.catalog_revision.saturating_add(1),
                "resumed" => target.resume_after_sequence = Some(0),
                "model_started" => target.attempt_started = true,
                "active" => { driver.active_effect_cancellations.lock().unwrap()
                    .insert(target.effect_id.0.clone(), RuntimeExecutionControl::default().context()); }
                "missing_lease" => target.delivery = None,
                "expired" => target.delivery.as_mut().unwrap().lease_expires_at_ms = 0,
                "unbound" => driver.bound_dispatch_run_id = None,
                _ => unreachable!(),
            }
            let mut emitted = Vec::new();
            assert!(driver.dispatch_batch(vec![target,cancel], &mut |response| {
                emitted.push(response); Ok(())
            }).is_err(), "{invalid}");
            assert!(emitted.is_empty(), "{invalid}: no skipped target may emit a start or stop ACK");
            assert!(driver.quiescent_effects.is_empty(), "{invalid}");
        }
    }

    #[test]
    fn returned_local_model_worker_keeps_stop_proof_when_its_response_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut driver = CliAgentHostDriver::new("delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()));
        let projection = AgentModelToolProjection::compile(1, "turn-fenced", Vec::new()).unwrap().projection;
        let invocation_policy_digest = "b".repeat(64);
        let stable_prefix_digest = "e".repeat(64);
        let turn_context = serde_json::from_value(json!({"stable_prefix_digest":"a".repeat(64)})).unwrap();
        let settings = json!({"step_context": {
            "schema_version":1,"step_index":1,"turn_context":turn_context,
            "model_prefix_digest":projection.model_prefix_digest_with_invocation_policy(&stable_prefix_digest,&invocation_policy_digest),
            "request_snapshot_digest":projection.request_snapshot_digest(&turn_context,&invocation_policy_digest),
            "tool_projection":projection,"invocation_policy_digest":invocation_policy_digest,
            "stable_prefix_digest":stable_prefix_digest,
        }});
        let mut request = fenced_request(&driver, HostRequest::ModelStream {
            model_id: "fixture".into(), text_part_id: "text-fixture".into(), reasoning_part_id: "reasoning-fixture".into(),
            response_mode: ModelResponseMode::default(), messages: Vec::new(), tools: Vec::new(), settings,
        });
        request.attempt_id = Some("prepared-local-model".into());
        let control = driver.tool_context.cancellation.clone();
        let error = driver.dispatch(request.clone(), &mut |response| {
            if matches!(response.payload, HostResponse::ExecutionStarted) { return Ok(()); }
            control.request();
            Err("fixture rejected local model admission".into())
        }).unwrap_err();
        assert!(error.contains("fixture rejected local model admission"), "{error}");
        let identity = (request.host_id.0.clone(),request.session_id.clone(),request.turn_id.clone(),request.effect_id.0.clone());
        assert!(driver.quiescent_effects.contains_key(&identity), "returned local worker is independent of canonical result ACK");
        assert!(driver.active_effect_cancellations.lock().unwrap().is_empty());
        let mut cancel = request.clone();
        cancel.attempt_id = None;
        cancel.request_id = "cancel-returned-model".into();
        cancel.effect_id.0 = "cancel-returned-model-effect".into();
        cancel.delivery.as_mut().unwrap().attempt_id = "cancel-returned-model-attempt".into();
        cancel.payload = HostRequest::Cancel { target_effect_id: request.effect_id, target_not_started: None };
        let mut confirmed = false;
        driver.dispatch(cancel, &mut |response| {
            confirmed |= matches!(response.payload, HostResponse::CancellationConfirmed { .. }); Ok(())
        }).unwrap();
        assert!(confirmed);
    }

    #[test]
    fn uncertain_typed_tool_receipt_cannot_become_stop_proof_after_the_handler_returns() {
        use nola_host_protocol::tool_result::{ExecutionReceipt, ExecutionState, ToolResult};
        let _environment = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        struct Database(Option<std::ffi::OsString>);
        impl Drop for Database { fn drop(&mut self) {
            if let Some(value) = self.0.take() { std::env::set_var("NOLA_DB_PATH", value); }
            else { std::env::remove_var("NOLA_DB_PATH"); }
        }}
        let _database = Database(std::env::var_os("NOLA_DB_PATH"));
        std::env::set_var("NOLA_DB_PATH", root.path().join("stop-proof.db"));
        for reject_response in [false, true] {
        for receipt_state in [None, Some(ExecutionState::Unknown), Some(ExecutionState::Partial),
            Some(ExecutionState::Deferred), Some(ExecutionState::Completed), Some(ExecutionState::NotStarted)] {
            let mut driver = CliAgentHostDriver::new("delivery-host",
                test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()))
                .with_runtime_tool_ids(["stop_fixture".to_string()])
                .with_runtime_tool_handler(move |_, _, _, _, _| Some(RuntimeToolResult {
                    outcome: ToolOutcome {
                        full_output_ref: None,
                        data: Some(json!({"_nola_execution":{"state":"completed"},"process_tree_stop":"confirmed"})),
                        execution: receipt_state.map(|state| ExecutionReceipt { state, ..Default::default() }),
                        text: "completed successfully; child stopped".into(), display: "Done".into(),
                        error: receipt_state != Some(ExecutionState::Deferred),
                        artifacts: Vec::new(), activation: None,
                    }, task_promotion: None, task_update: None,
                }));
            let mut capability = driver.catalog.capabilities["read_file"].clone();
            capability.id = "stop_fixture".into();
            capability.input_schema = json!({"type":"object"});
            capability.input_normalization = Default::default();
            capability.input_normalization.effect = nola_core::agent::host::HostCapabilityEffectDeclaration::fixed(
                nola_core::agent::host::HostCapabilityEffect::Investigate);
            driver.catalog.capabilities.insert(capability.id.clone(), capability);
            let target = fenced_request(&driver, HostRequest::ExecuteTool {
                capability_id: "stop_fixture".into(), call_id: "stop-fixture-call".into(), input: json!({}),
                grant: None, permission_instructions: Vec::new(),
                permission_profile: nola_domain::PermissionProfile::Cautious,
            });
            let mut responses = Vec::new();
            let dispatched = driver.dispatch(target.clone(), &mut |response| {
                let reject = reject_response && matches!(response.payload, HostResponse::ToolCompleted { .. });
                responses.push(response);
                if reject { Err("canonical result rejected after Interrupt".into()) } else { Ok(()) }
            });
            assert_eq!(dispatched.is_err(), reject_response);
            let output = responses.iter().find_map(|response| match &response.payload {
                HostResponse::ToolCompleted { output, .. } => Some(output.clone()), _ => None,
            }).expect("actual runtime handler must return a typed tool result");
            let decoded: ToolResult = serde_json::from_value(output).unwrap();
            assert_eq!(decoded.execution.as_ref().map(|receipt| receipt.state), receipt_state);
            assert!(driver.active_effect_cancellations.lock().unwrap().is_empty(), "Rust handler has returned");
            let stopped = matches!(receipt_state, None | Some(ExecutionState::Completed) | Some(ExecutionState::NotStarted));
            let identity = (target.host_id.0.clone(), target.session_id.clone(), target.turn_id.clone(), target.effect_id.0.clone());
            assert_eq!(driver.quiescent_effects.contains_key(&identity), stopped, "{receipt_state:?}");
            let mut cancel = target.clone();
            cancel.request_id = "cancel-uncertain-tool".into();
            cancel.effect_id.0 = "cancel-uncertain-tool-effect".into();
            cancel.delivery.as_mut().unwrap().attempt_id = "cancel-uncertain-tool-attempt".into();
            cancel.payload = HostRequest::Cancel { target_effect_id: target.effect_id, target_not_started: None };
            responses.clear();
            let result = driver.dispatch(cancel, &mut |response| { responses.push(response); Ok(()) });
            assert_eq!(result.is_ok(), stopped, "{receipt_state:?}");
            assert_eq!(responses.iter().any(|response| matches!(response.payload,
                HostResponse::CancellationConfirmed { .. })), stopped);
            if !stopped { assert!(responses.is_empty(), "uncertain control must not start"); }
        }
        }
        assert!(!terminal_response_proves_local_stop(&HostResponse::Failed { error: HostError {
            code: "stopped".into(), message: "worker exited".into(), retryable: false, http_status: None,
        }}, false, None));
    }

    #[test]
    fn missing_receipt_stop_boundary_uses_frozen_effect_not_business_data() {
        use nola_core::agent::host::{HostCapabilityEffect, HostCapabilityEffectDeclaration};
        use nola_host_protocol::tool_result::{ToolResult, ToolResultContract, ToolPresentation};
        let root = tempfile::tempdir().unwrap();
        let output = serde_json::to_value(ToolResult {
            contract: ToolResultContract::V1,
            data: Some(json!({"effect":"investigate","_nola_execution":{"state":"completed"}})),
            presentation: ToolPresentation {text:"Read completed; process stopped".into(),summary:"Done".into()},
            execution: None, full_output_ref: None, memory_usage: None,
        }).unwrap();
        let response = HostResponse::ToolCompleted {
            call_id:"call".into(),output,outcome:HostToolOutcome::Succeeded,
            activation:None,full_output_ref:None,artifacts:Vec::new(),
        };
        for (effect, expected) in [(HostCapabilityEffect::Investigate,true),(HostCapabilityEffect::Mutate,false)] {
            let mut driver = CliAgentHostDriver::new("delivery-host",
                test_llm("http://127.0.0.1:1/v1".into()),test_context(root.path()));
            // The same input/output is evaluated against two independently
            // frozen fixture catalogs. No tool is executed or reclassified by
            // its name, prose or the forged business-data receipt.
            driver.catalog.capabilities.get_mut("read_file").unwrap().input_normalization.effect =
                HostCapabilityEffectDeclaration::fixed(effect);
            let request = fenced_request(&driver,HostRequest::ExecuteTool {
                capability_id:"read_file".into(),call_id:"call".into(),input:json!({"path":"fixture.txt"}),
                grant:None,permission_instructions:Vec::new(),permission_profile:nola_domain::PermissionProfile::Cautious,
            });
            let ordinary = driver.allows_absent_tool_stop_receipt(&request);
            assert_eq!(ordinary,expected);
            assert_eq!(terminal_response_proves_local_stop(&response,ordinary,None),expected);
        }
    }

    #[test]
    fn delegated_physical_stop_survives_a_missing_terminal_commit() {
        let root = tempfile::tempdir().unwrap();
        let mut driver = CliAgentHostDriver::new("delivery-host", test_llm("http://127.0.0.1:1/v1".into()),
            test_context(root.path())).with_boundary_session_id("session-fenced");
        driver.bind_file_read_run("turn-fenced");
        let target = fenced_request(&driver, HostRequest::FetchContext {source_id:"fixture".into(),
            query:Value::Null, freshness_deadline_ms:None});
        let identity=(target.host_id.0.clone(),target.session_id.clone(),target.turn_id.clone(),target.effect_id.0.clone());
        let (control_tx, _) = std::sync::mpsc::channel();
        let (output_tx, output_rx) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || { output_tx.send(DelegatedAgentWorkerOutput::PhysicalStopObserved).unwrap(); });
        driver.insert_delegated_agent_session(target.effect_id.0.clone(), DelegatedAgentWorkerSession {
            effect_id:target.effect_id.0.clone(),permission_delivery_ids:Default::default(),request:target.clone(),
            next_sequence:0,control_tx,output_rx,cancellation:Default::default(),worker:Some(worker),
            admission_failure:Default::default(),cancellation_registry:Arc::clone(&driver.active_effect_cancellations),
        });
        assert!(driver.drain_delegated_agent_session(&target.effect_id.0,&mut |_| Ok(())).is_err());
        assert!(driver.delegated_agent_sessions.is_empty());
        assert!(driver.quiescent_effects.get(&identity).is_some_and(|proof| proof.matches_owner(&identity)));
        let mut cancel=target.clone();cancel.effect_id.0="exact-stop-control".into();
        cancel.request_id="exact-stop-control-request".into();
        cancel.payload=HostRequest::Cancel {target_effect_id:target.effect_id.clone(),target_not_started:None};
        let mut confirmed=false;
        driver.dispatch_batch(vec![cancel],&mut |response| {
            confirmed |= matches!(response.payload,HostResponse::CancellationConfirmed {..}); Ok(())
        }).unwrap();
        assert!(confirmed);
    }

    #[test]
    fn delegated_cancellation_confirms_only_after_the_actual_worker_is_joined() {
        use std::sync::atomic::{AtomicBool, Ordering};
        for finished in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let mut driver = CliAgentHostDriver::new("delivery-host",
                test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()))
                .with_boundary_session_id("session-fenced");
            driver.bind_file_read_run("turn-fenced");
            let target = fenced_request(&driver, HostRequest::FetchContext {
                source_id: "fixture".into(), query: Value::Null, freshness_deadline_ms: None,
            });
            let (control_tx, control_rx) = std::sync::mpsc::channel();
            let (output_tx, output_rx) = std::sync::mpsc::sync_channel(1);
            let exited = Arc::new(AtomicBool::new(false));
            let worker_exited = Arc::clone(&exited);
            let worker = std::thread::spawn(move || {
                assert!(matches!(control_rx.recv().unwrap(), DelegatedAgentWorkerControl::Cancel));
                output_tx.send(if finished { DelegatedAgentWorkerOutput::Finished }
                    else { DelegatedAgentWorkerOutput::Suspended }).unwrap();
                worker_exited.store(true, Ordering::Release);
            });
            driver.insert_delegated_agent_session(target.effect_id.0.clone(), DelegatedAgentWorkerSession {
                effect_id: target.effect_id.0.clone(), permission_delivery_ids: Default::default(),
                request: target.clone(), next_sequence: 0, control_tx, output_rx,
                cancellation: Default::default(), worker: Some(worker), admission_failure: Default::default(),
                cancellation_registry: Arc::clone(&driver.active_effect_cancellations),
            });
            let mut cancel = target.clone();
            cancel.effect_id.0 = "cancel-worker".into();
            cancel.request_id = "cancel-worker-request".into();
            cancel.delivery.as_mut().unwrap().attempt_id = "cancel-worker-attempt".into();
            cancel.payload = HostRequest::Cancel { target_effect_id: target.effect_id.clone(), target_not_started: None };
            driver.tool_context.cancellation.request();
            let mut confirmed = false;
            // The already-owned worker cannot gain NotStarted proof merely
            // because its old request is skipped. Its following control must
            // still execute and wait for that actual worker.
            let result = driver.dispatch_batch(vec![target, cancel], &mut |response| {
                if matches!(response.payload, HostResponse::CancellationConfirmed { .. }) {
                    assert!(exited.load(Ordering::Acquire), "confirmation preceded worker join");
                    confirmed = true;
                }
                Ok(())
            });
            assert_eq!(result.is_ok(), finished);
            assert_eq!(confirmed, finished);
            assert_eq!(driver.delegated_agent_sessions.is_empty(), finished);
        }
    }

    struct RejectingRenewalPort {
        request: Option<HostRequestEnvelope>,
        reject_on_renewal: usize,
        renewals: usize,
        claim_started: Option<std::time::Instant>,
        accepted: Vec<HostResponse>,
    }

    impl nola_runtime_driver::CommittedRuntimeApplicationPort for RejectingRenewalPort {
    type Protocol = nola_core::agent::host::AgentRuntimeProtocol;
        fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope>, String> {
            Ok(Vec::new())
        }

        fn claim_committed_requests(
            &mut self,
            claim: nola_runtime_driver::RuntimeEffectDeliveryClaim,
        ) -> Result<Vec<HostRequestEnvelope>, String> {
            let Some(mut request) = self.request.take() else {
                return Ok(Vec::new());
            };
            request.delivery = Some(EffectDeliveryLease {
                attempt_id: "attempt-child-wait".into(),
                worker_generation: claim.worker_generation,
                lease_owner: claim.lease_owner,
                lease_token: "token-child-wait".into(),
                lease_expires_at_ms: claim.lease_expires_at_ms,
            });
            self.claim_started = Some(std::time::Instant::now());
            Ok(vec![request])
        }

        fn renew_effect_delivery(
            &mut self,
            renewal: nola_runtime_driver::RuntimeEffectDeliveryRenewal,
        ) -> Result<EffectDeliveryLease, String> {
            self.renewals += 1;
            if self.renewals == self.reject_on_renewal {
                return Err("fixture rejected child-process lease renewal".into());
            }
            let mut renewed = renewal.delivery;
            renewed.lease_expires_at_ms = renewal.lease_expires_at_ms;
            Ok(renewed)
        }


        fn accept_raw_host_response(
            &mut self,
            response: HostResponseEnvelope,
        ) -> Result<nola_runtime_driver::CommittedRuntimeAcceptance<nola_core::agent::host::AgentRuntimeProtocol>, String> {
            self.accepted.push(response.payload);
            Ok(nola_runtime_driver::CommittedRuntimeAcceptance {
                events: Vec::new(),
                traces: Vec::new(),
                control: None,
            })
        }

        fn accept_boundary_input(
            &mut self,
            _: nola_core::agent::host::AgentBoundaryInput,
        ) -> Result<nola_runtime_driver::CommittedRuntimeAcceptance<nola_core::agent::host::AgentRuntimeProtocol>, String> {
            Err("child-process fixture has no boundary submissions".into())
        }
    }

    #[test]
    fn runtime_renewal_abort_targets_a_delegated_session_without_cancelling_others() {
        let root = std::env::temp_dir().join(format!(
            "nola-effect-renewal-abort-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut driver = CliAgentHostDriver::new(
            "delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()),
            test_context(&root),
        );
        let first = RuntimeExecutionControl::default();
        let second = RuntimeExecutionControl::default();
        let mut request = fenced_request(
            &driver,
            HostRequest::FetchContext {
                source_id: "delegated-fixture".into(),
                query: Value::Null,
                freshness_deadline_ms: None,
            },
        );
        request.effect_id.0 = "effect-first".into();
        let (control_tx, _) = std::sync::mpsc::channel();
        let (_, output_rx) = std::sync::mpsc::sync_channel(1);
        driver.insert_delegated_agent_session(
            "effect-first".into(),
            DelegatedAgentWorkerSession {
                effect_id: "effect-first".into(),
                permission_delivery_ids: Default::default(),
                request,
                next_sequence: 0,
                control_tx,
                output_rx,
                cancellation: first.clone(),
                worker: None,
                admission_failure: Default::default(),
                cancellation_registry: std::sync::Arc::clone(&driver.active_effect_cancellations),
            },
        );
        driver
            .active_effect_cancellations
            .lock()
            .unwrap()
            .insert("effect-second".into(), second.context());

        driver.effect_delivery_abort_handle()("effect-first");

        assert!(first.is_requested());
        assert!(!second.is_requested());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejected_renewal_cancels_a_real_silent_child_without_committing_a_fake_terminal() {
        let root = std::env::temp_dir().join(format!(
            "nola-effect-renewal-child-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let pid_path = root.join("child.pid");
        let finished_path = root.join("child.finished");
        let context = test_context(&root);
        let observed_cancellation =
            std::sync::Arc::new(std::sync::Mutex::new(None::<RuntimeExecutionControl>));
        let handler_observed_cancellation = std::sync::Arc::clone(&observed_cancellation);
        let handler_pid_path = pid_path.clone();
        let handler_finished_path = finished_path.clone();
        let driver = CliAgentHostDriver::new(
            "delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()),
            context,
        )
        .with_runtime_tool_ids(["grep".to_string()])
        .with_runtime_tool_handler(move |_, capability_id, _, _, context| {
            assert_eq!(capability_id, "grep");
            *handler_observed_cancellation.lock().unwrap() = Some(context.cancellation.clone());
            use std::os::unix::process::CommandExt;
            let mut child = std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    "printf '%s\\n' $$ > \"$1\"; sleep 30; printf 'finished\\n' > \"$2\"",
                    "effect-delivery-child",
                ])
                .arg(&handler_pid_path)
                .arg(&handler_finished_path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap();
            let cancelled = loop {
                if context.cancellation.is_requested() {
                    let process_group = i32::try_from(child.id()).unwrap();
                    unsafe {
                        libc::kill(-process_group, libc::SIGTERM);
                    }
                    let _ = child.wait();
                    break true;
                }
                if child.try_wait().unwrap().is_some() {
                    break false;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            Some(RuntimeToolResult {
                outcome: ToolOutcome {
                    full_output_ref: None,
                    data: None,
                    execution: None,
                    text: if cancelled {
                        "child cancelled after lease rejection".into()
                    } else {
                        "child exited without cancellation".into()
                    },
                    display: "silent child fixture".into(),
                    error: true,
                    artifacts: Vec::new(),
                    activation: None,
                },
                task_promotion: None,
                task_update: None,
            })
        });
        let unrelated = RuntimeExecutionControl::default();
        driver
            .active_effect_cancellations
            .lock()
            .unwrap()
            .insert("effect-unrelated".into(), unrelated.context());
        let abort = driver.effect_delivery_abort_handle();
        let tool_input = json!({"pattern":"unused", "path":"."});
        let contract = &driver.catalog().capabilities["grep"].input_normalization.permission_contract;
        let plan = contract.resource_plan_for(&tool_input).unwrap();
        let resolution = nola_domain::permission_resource_plan::PermissionResolutionRequest {
            resolution_id: "silent-child-resource".into(),
            execution_target: "delivery-host".into(),
            invocation_digest: nola_domain::permission_resource_plan::PermissionResolutionRequest::input_digest(&tool_input),
            capability: plan.capability.clone(), operation: plan.operation.clone(),
            demands: plan.extract(&tool_input).unwrap(),
        };
        let facts = crate::tools::resolve_permission_resources(
            &driver.tool_context, &resolution, &tool_input,
        ).unwrap();
        let request = fenced_request(
            &driver,
            HostRequest::ExecuteTool {
                permission_profile: nola_domain::PermissionProfile::Cautious,
                capability_id: "grep".into(),
                call_id: "call-child-wait".into(),
                input: tool_input,
                grant: Some(json!({"permission_resolution":{"request":resolution,"facts":facts}})),
                permission_instructions: Vec::new(),
            },
        );
        let mut port = RejectingRenewalPort {
            request: Some(request),
            reject_on_renewal: 20,
            renewals: 0,
            claim_started: None,
            accepted: Vec::new(),
        };
        let initial_ttl = std::time::Duration::from_millis(120);
        let mut runtime =
            nola_runtime_driver::CommittedRuntimeDriver::with_delivery_identity_and_timing(
                driver,
                "generation-child-wait",
                "owner-child-wait",
                nola_runtime_driver::RuntimeEffectDeliveryTiming {
                    lease_duration: initial_ttl,
                    heartbeat_cadence: std::time::Duration::from_millis(25),
                },
            )
            .with_delivery_abort(abort);

        let result = runtime.execute(&mut port, &mut |_| {});
        assert!(
            matches!(result, Err(nola_runtime_driver::RuntimeDriverError::Core(ref error))
                if error == "fixture rejected child-process lease renewal"),
            "unexpected Runtime result: {result:?}; renewals={}; accepted={:?}",
            port.renewals,
            port.accepted
        );
        assert_eq!(port.renewals, 20);
        assert!(
            !unrelated.is_requested(),
            "scoped lease abort cancelled an unrelated Host effect"
        );
        assert!(
            observed_cancellation
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(RuntimeExecutionControl::is_requested),
            "runtime handler did not receive the effect-scoped cancellation registered by Host"
        );
        assert!(
            port.claim_started.unwrap().elapsed() > initial_ttl,
            "real child did not remain silent beyond its initial lease TTL"
        );
        assert!(
            pid_path.exists(),
            "bash child never crossed its process boundary"
        );
        let child_pid = std::fs::read_to_string(&pid_path)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while unsafe { libc::kill(child_pid, 0) } == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_ne!(
            unsafe { libc::kill(child_pid, 0) },
            0,
            "lease-renewal rejection left the real bash child alive"
        );
        assert!(
            !finished_path.exists(),
            "cancelled child completed after losing its lease"
        );
        assert_eq!(
            port.accepted
                .iter()
                .filter(|payload| payload.is_terminal())
                .count(),
            0,
            "renewal rejection committed a fabricated terminal response"
        );
        assert!(matches!(
            port.accepted.first(),
            Some(HostResponse::ExecutionStarted)
        ));
        assert!(port
            .accepted
            .iter()
            .any(|payload| matches!(payload, HostResponse::ToolCallProgress { .. })));
        assert_eq!(
            port.accepted.len(),
            2,
            "a response escaped admission after renewal rejection"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    pub(super) fn test_context(root: &std::path::Path) -> ToolContext {
        ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs: vec![root.to_path_buf()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "allow".into(), Vec::new()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::new(root.join("tool-outputs")),
            browser: crate::tools::BrowserRuntime::new(root.join("browser")),
        }
    }

    #[test]
    fn native_subagents_require_connection_and_cannot_be_discovered() {
        let root = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::for_test().unwrap();
        config.runtime.native_model_route = Some(serde_json::from_value(json!({
            "adapter":"mlx", "adapter_contract_revision":1, "model_id":"native",
            "profile_ref":{"profile_id":"native", "admitted_revision":1}
        })).unwrap());
        let mut driver = CliAgentHostDriver::new("host", test_llm("http://127.0.0.1:1/v1".into()), test_context(root.path()))
            .with_execution_settings(config.host_execution_settings());
        assert_eq!(driver.catalog().capabilities["spawn_agent"].availability,
            CapabilityAvailability::RequiresConnection);
        driver.refresh_current_turn_available_tool_ids();
        assert!(!driver.current_turn_available_tool_ids.contains("spawn_agent"));
        assert!(!driver.capability_catalog.builtin_tools.contains("spawn_agent"));
        assert!(driver.current_turn_discovery_tool_schemas().iter()
            .all(|schema| schema.pointer("/function/name").and_then(Value::as_str) != Some("spawn_agent")));
        let stale_catalog = build_cli_host_catalog("host");
        assert_eq!(driver.prepare_committed_catalog(stale_catalog).unwrap().capabilities["spawn_agent"].availability,
            CapabilityAvailability::RequiresConnection);
        config.runtime.native_model_route = None;
        driver.set_execution_settings(config.host_execution_settings());
        driver.refresh_current_turn_available_tool_ids();
        assert_eq!(driver.catalog().capabilities["spawn_agent"].availability, CapabilityAvailability::Available);
        assert!(driver.current_turn_available_tool_ids.contains("spawn_agent"));
    }

    fn llm_with_admitted_reasoning_route() -> LlmConfig {
        let mut config = crate::config::Config::for_test().unwrap();
        config.runtime.reasoning_selection =
            Some(nola_core::agent::reasoning::ReasoningSelection::Effort {
                value: "high".into(),
            });
        config
            .runtime
            .llm_fallbacks
            .push(crate::runtime_config::RuntimeLlmProvider {
                provider: Some("openai".into()),
                endpoint: "https://unadmitted.invalid/v1".into(),
                model: Some("unadmitted".into()),
                api_key: None,
            });
        config.runtime.output_admission =
            Some(crate::host_output_admission::admit(&config.model_invocation_settings().unwrap(), false).unwrap());
        let mut llm = test_llm(config.endpoint.clone());
        llm.reasoning_selection = config.runtime.reasoning_selection.clone();
        llm.fallbacks = crate::host_output_admission::admitted_fallbacks(&config.model_invocation_settings().unwrap()).unwrap();
        llm
    }

    #[test]
    fn disabling_reasoning_for_an_effect_cannot_revive_unadmitted_fallbacks() {
        let mut llm = llm_with_admitted_reasoning_route();
        llm.reasoning_selection = None;
        llm.thinking_enabled = false;
        assert!(llm.fallbacks.is_empty());
        assert_eq!(CliAgentHostDriver::model_target_keys(&llm).len(), 1);
    }

    #[test]
    fn unavailable_primary_cannot_promote_an_unadmitted_fallback() {
        let mut llm = llm_with_admitted_reasoning_route();
        let original_endpoint = llm.endpoint.clone();
        let root = std::env::temp_dir().join(format!("nola-frozen-route-{}", uuid::Uuid::new_v4()));
        let mut driver = CliAgentHostDriver::new("frozen-route", llm.clone(), test_context(&root));
        driver
            .current_turn_unavailable_model_targets
            .insert(CliAgentHostDriver::model_target_key(
                llm.provider.as_deref(),
                &llm.endpoint,
                &llm.model,
            ));
        driver.omit_unavailable_model_targets(&mut llm);
        assert_eq!(llm.endpoint, original_endpoint);
        assert!(llm.fallbacks.is_empty());
    }

    pub(super) fn test_llm(endpoint: String) -> LlmConfig {
        LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint,
            api_key: "fixture".into(),
            model: "fixture-model".into(),
            temperature: 0.0,
            top_p: 1.0,
            max_output_tokens: 128,
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
        }
    }

    pub(super) fn fenced_request(driver: &CliAgentHostDriver, payload: HostRequest) -> HostRequestEnvelope {
        serde_json::from_value(json!({
            "request_id": "request-fenced",
            "effect_id": "effect-fenced",
            "host_id": "delivery-host",
            "session_id": "session-fenced",
            "turn_id": "turn-fenced",
            "catalog_revision": driver.catalog.revision,
            "availability_epoch": driver.catalog.availability_epoch,
            "created_at_ms": 0,
            "delivery": {
                "attempt_id": "attempt-fenced",
                "worker_generation": "generation-fenced",
                "lease_owner": "owner-fenced",
                "lease_token": "token-fenced",
                "lease_expires_at_ms": i64::MAX
            },
            "payload": payload
        }))
        .unwrap()
    }

    #[test]
    fn compaction_snapshot_restore_preserves_failure_categories() {
        let root = tempfile::tempdir().unwrap();
        let cfg = test_llm("http://127.0.0.1:1/v1".into());
        let path = root.path().join("compaction-request.json");
        let check = |expected| {
            assert_eq!(CompactionRequestWitness::restore(root.path(), "session", &cfg).err(), Some(expected));
        };
        check(CompactionWitnessRestoreError::Missing);
        std::fs::create_dir(&path).unwrap();
        check(CompactionWitnessRestoreError::Unreadable);
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"{broken").unwrap();
        check(CompactionWitnessRestoreError::Malformed);
        std::fs::write(&path, b"{}").unwrap();
        check(CompactionWitnessRestoreError::Malformed);
        std::fs::File::create(&path).unwrap().set_len(32 * 1024 * 1024 + 1).unwrap();
        check(CompactionWitnessRestoreError::TooLarge);
        let identity = crate::llm::request_configuration_identity(&cfg);
        std::fs::write(&path, json!({"session_id":"other", "configuration_identity":identity}).to_string()).unwrap();
        check(CompactionWitnessRestoreError::SessionChanged);
        std::fs::write(&path, json!({"session_id":"session", "configuration_identity":"changed"}).to_string()).unwrap();
        check(CompactionWitnessRestoreError::ConfigurationChanged);
        std::fs::write(&path, json!({"session_id":"session", "configuration_identity":identity,"request":{}}).to_string()).unwrap();
        check(CompactionWitnessRestoreError::InvalidRequest);
    }

    #[test]
    fn compaction_cache_fork_recovers_without_executing_tools_and_accounts_both_attempts() {
        for case in ["tool_call", "empty", "truncated", "oversized", "oversized_twice", "configuration_changed", "session_changed", "request_snapshot_malformed"] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
            let cold = matches!(case, "configuration_changed" | "session_changed" | "request_snapshot_malformed");
            let server = std::thread::spawn(move || {
                let mut bodies = Vec::new();
                for index in 0..if cold { 2 } else { 3 } {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    let (mut stream, _) = loop {
                        match listener.accept() {
                            Ok(socket) => break socket,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(std::time::Instant::now() < deadline, "missing request: {case}/{index}");
                                std::thread::sleep(std::time::Duration::from_millis(5));
                            }
                            Err(error) => panic!("{error}"),
                        }
                    };
                    // The listener polls nonblocking; the accepted connection
                    // is read synchronously and must not inherit that mode.
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                    stream.set_write_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut first = String::new(); reader.read_line(&mut first).unwrap();
                    assert!(first.starts_with("POST /v1/chat/completions "));
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap_or_else(|error| panic!(
                            "compaction fixture header read failed: case={case}, request={index}, content_length={length}: {error}"
                        ));
                        if line == "\r\n" { break; }
                        assert!(!line.is_empty());
                        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    let mut bytes = vec![0; length]; reader.read_exact(&mut bytes).unwrap();
                    bodies.push(serde_json::from_slice::<Value>(&bytes).unwrap());
                    let rejected = index == 1 && !cold;
                    let delta = if rejected && case == "tool_call" {
                        json!({"content":"Do not accept this summary","tool_calls":[{"index":0,"id":"forbidden", "type":"function", "function":{"name":"write_file","arguments":"{}"}}]})
                    } else if (rejected && case == "oversized") || (index > 0 && case == "oversized_twice") {
                        json!({"content":(0..600).map(|n| format!("Constraint {n}: retain evidence for item {n}.\n")).collect::<String>()})
                    } else { json!({"content":if rejected && case == "empty" { "" } else { "durable summary" }}) };
                    let reason = if rejected && case == "truncated" { "length" }
                        else if rejected && case == "tool_call" { "tool_calls" } else { "stop" };
                    let event = json!({"choices":[{"index":0,"delta":delta,"finish_reason":reason}],
                        "usage":{"prompt_tokens":12000,"completion_tokens":5,"total_tokens":12005,"prompt_tokens_details":{"cached_tokens":11000}}});
                    let body = format!("data: {event}\n\ndata: [DONE]\n\n");
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
                bodies
            });
            let root = tempfile::tempdir().unwrap();
            let mut cfg = test_llm(endpoint);
            cfg.provider = Some("openai".into()); cfg.max_output_tokens = 8192;
            cfg.openai_prompt_cache = true;
            cfg.openai_prompt_cache_key = Some("fixture-cache".into());
            cfg.max_retries = 0;
            let archive = vec![json!({"type":"message","id":"archive-1","role":"user","content":"archived evidence ".repeat(2500)})];
            let mut wire = vec![json!({"role":"system","content":"stable system"})];
            wire.extend(provider_messages_for_wire_protocol(&archive, &cfg));
            wire.push(json!({"role":"assistant","content":"retained tail"}));
            let tools = vec![json!({"type":"function","function":{"name":"write_file","parameters":{"type":"object","properties":{}}}})];
            let choice = stream_chat_checked(&cfg, &wire, &tools, "auto", |_| Ok(())).unwrap();
            let mut host = CliAgentHostDriver::new("delivery-host", cfg.clone(), test_context(root.path()));
            host.compaction_request_witness = Some(CompactionRequestWitness {
                request: choice.request_snapshot.unwrap(), session_id: if case == "session_changed" { "other-session" } else { "session-fenced" }.into(),
                configuration_identity: crate::llm::request_configuration_identity(&cfg), context_window:128000, usage:choice.usage,
            });
            if !cold {
                host.compaction_request_witness.as_ref().unwrap()
                    .persist(host.tool_context.output_store.root()).unwrap();
                assert!(matches!(CompactionRequestWitness::restore(host.tool_context.output_store.root(), "another-session", &cfg), Err(CompactionWitnessRestoreError::SessionChanged)));
                let restored = CompactionRequestWitness::restore(host.tool_context.output_store.root(), "session-fenced", &cfg).unwrap();
                assert_eq!(restored.usage.output_tokens, Some(5), "restart must preserve measured append budget");
                let isolated = tempfile::tempdir().unwrap();
                let mut partial = restored;
                partial.usage.input_tokens = None;
                partial.usage.output_tokens = Some(0);
                partial.usage.total_tokens = None;
                partial.persist(isolated.path()).unwrap();
                let partial = CompactionRequestWitness::restore(isolated.path(), "session-fenced", &cfg).unwrap();
                assert_eq!(partial.usage.input_tokens, None);
                assert_eq!(partial.usage.output_tokens, Some(0));
                assert_eq!(partial.usage.total_tokens, None);
                host.compaction_request_witness = None;
            }
            if case == "configuration_changed" { host.llm.temperature = 0.8; }
            if case == "request_snapshot_malformed" {
                std::fs::create_dir_all(host.tool_context.output_store.root()).unwrap();
                std::fs::write(host.tool_context.output_store.root().join("compaction-request.json"), b"{broken").unwrap();
                host.compaction_request_witness = None;
            }
            let mut request = fenced_request(&host, HostRequest::Compact {
                context_window_tokens: 128000,
                strategy:"fixture".into(), instructions:"Summarize the archive".into(), messages:archive.clone(), target_tokens:2048,
            });
            request.attempt_id = Some("prepared-compaction".into());
            let mut events = Vec::new();
            host.execute_compaction(&request,"Summarize the archive",&archive,2048,&mut |event| { events.push(event); Ok(()) }).unwrap();
            let bodies = server.join().unwrap();
            let last = bodies.last().unwrap();
            if matches!(case, "truncated" | "oversized" | "oversized_twice") {
                let count = bodies[1]["messages"].as_array().unwrap().len();
                assert!(last["messages"].as_array().unwrap().len() >= count, "{case}: expected an appended physical request");
                assert_eq!(&last["messages"].as_array().unwrap()[..count], bodies[1]["messages"].as_array().unwrap());
                assert_eq!(last["tools"], bodies[1]["tools"]);
                assert_eq!(last["prompt_cache_key"], bodies[1]["prompt_cache_key"]);
            } else {
                assert!(last.get("tools").is_none());
                assert!(last.get("prompt_cache_key").is_none());
            }
            if !cold {
                assert_eq!(bodies[1]["tools"], bodies[0]["tools"]);
                assert_eq!(bodies[1]["tool_choice"], bodies[0]["tool_choice"]);
                assert_eq!(bodies[1]["messages"].as_array().unwrap().len(), bodies[0]["messages"].as_array().unwrap().len()+1);
            }
            let metrics = events.iter().filter_map(|event| match &event.payload {
                HostResponse::ToolProgress { title, metadata, .. } if title == "compaction_request_metrics" => Some(metadata), _=>None,
            }).collect::<Vec<_>>();
            assert_eq!(metrics.len(), if cold { 1 } else { 2 });
            if cold { assert_eq!(metrics[0]["fallback_reason"], case); }
            else {
                assert_eq!(metrics[0]["strategy"],"cached_request_fork");
                assert_eq!(metrics[0]["accepted_summary"], matches!(case, "oversized" | "oversized_twice"));
            }
            if matches!(case, "oversized" | "oversized_twice") {
                assert_eq!(metrics[1]["strategy"], "summary_budget_rewrite");
                assert_eq!(metrics[1]["summary_rewrite_count"], 1);
                assert!(last.to_string().contains("COMPLETE replacement"));
            }
            if case == "oversized_twice" {
                assert!(!events.iter().any(|event| matches!(event.payload, HostResponse::CompactionCompleted { .. })));
                assert!(events.iter().any(|event| matches!(&event.payload, HostResponse::Failed { error }
                    if error.code == "compaction_summary_budget_exceeded")));
                assert_eq!(events.iter().filter(|event| matches!(event.payload, HostResponse::ModelUsageReported { .. })).count(), 2);
                continue;
            }
            let completed = events.iter().find_map(|event| match &event.payload {
                HostResponse::CompactionCompleted { summary, usage } => Some((summary,usage)), _=>None,
            }).expect("valid fallback summary must complete");
            assert_eq!(completed.0, if case == "truncated" { "durable summarydurable summary" } else { "durable summary" });
            assert_eq!(completed.1["output_tokens"], 5, "Completion belongs only to its physical attempt");
            let usages = events.iter().filter_map(|event| match &event.payload {
                HostResponse::ModelUsageReported { usage } => Some(usage), _ => None,
            }).collect::<Vec<_>>();
            assert_eq!(usages.len(), if cold { 1 } else { 2 });
            assert!(usages.iter().all(|usage| usage["output_tokens"] == 5));
            assert!(host.compaction_request_witness.is_none());
            assert!(!events.iter().any(|event| matches!(&event.payload,HostResponse::ToolCompleted { .. })));
        }
    }

    #[test]
    fn standalone_compaction_continues_truncated_output_and_keeps_each_requests_usage() {
        for provider in ["openai", "openai-responses", "anthropic"] {
        for empty_continuation in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let mut bodies = Vec::new();
                while bodies.len() < 2 {
                    let attempt = bodies.len();
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    let (mut stream, _) = loop {
                        match listener.accept() {
                            Ok(value) => break value,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(std::time::Instant::now() < deadline);
                                std::thread::sleep(std::time::Duration::from_millis(5));
                            }
                            Err(error) => panic!("{error}"),
                        }
                    };
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    stream.set_nonblocking(false).unwrap();
                    let mut length = 0;
                    let mut metadata_request = false;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap_or_else(|error| panic!(
                            "compaction fixture header read failed: provider={provider}, empty_continuation={empty_continuation}, model_attempt={attempt}, metadata_request={metadata_request}, content_length={length}, partial_line={line:?}: {error}"
                        ));
                        if line == "\r\n" { break; }
                        assert!(!line.is_empty(), "compaction fixture connection closed before headers: provider={provider}, empty_continuation={empty_continuation}, model_attempt={attempt}, metadata_request={metadata_request}");
                        metadata_request |= line.starts_with("GET /v1/status ");
                        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    if metadata_request {
                        // Supported adapters opt in; unknown adapters retain defaults.
                        let body = if empty_continuation { "{}" } else {
                            r#"{"service":"strata","model":"fixture-model","dialects":["/v1/responses"]}"#
                        };
                        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                        continue;
                    }
                    let mut bytes = vec![0; length]; reader.read_exact(&mut bytes).unwrap();
                    bodies.push(serde_json::from_slice::<Value>(&bytes).unwrap());
                    let truncated = attempt == 0;
                    let text = if truncated {"unfinished code"} else if empty_continuation {""} else {" complete checkpoint"};
                    let event = json!({"choices":[{"index":0,"delta":{"content":text},
                        "finish_reason":if truncated {"length"} else {"stop"}}],
                        "usage":{"prompt_tokens":7014,"completion_tokens":if truncated {8000} else {12},
                        "prompt_tokens_details":{"cached_tokens":0}}});
                    let events = match provider {
                        "openai-responses" => vec![json!({"type":if truncated {"response.incomplete"} else {"response.completed"},"response":{
                            "object":"response","status":if truncated {"incomplete"} else {"completed"},
                            "incomplete_details":if truncated {json!({"reason":"max_output_tokens"})} else {Value::Null},
                            "output":[{"type":"message","role":"assistant","status":if truncated {"incomplete"} else {"completed"},
                                "content":[{"type":"output_text","text":text,"annotations":[]}]}],
                            "usage":{"input_tokens":7014,"output_tokens":if truncated {8000} else {12},"input_tokens_details":{"cached_tokens":0}}}})],
                        "anthropic" => vec![
                            json!({"type":"message_start","message":{"usage":{"input_tokens":7014,"cache_read_input_tokens":0}}}),
                            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
                            json!({"type":"message_delta","delta":{"stop_reason":if truncated {"max_tokens"} else {"end_turn"}},"usage":{"output_tokens":if truncated {8000} else {12}}}),
                            json!({"type":"message_stop"}),
                        ],
                        _ => vec![event],
                    };
                    let body = events.iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
                bodies
            });
            let root = tempfile::tempdir().unwrap();
            let mut cfg = test_llm(endpoint); cfg.max_output_tokens = 8192; cfg.max_retries = 0;
            cfg.provider = Some(provider.into());
            let mut host = CliAgentHostDriver::new("delivery-host", cfg, test_context(root.path()));
            let archive = vec![json!({"type":"message","id":"old","role":"user","content":"Keep my constraints".repeat(224)})];
            let mut request = fenced_request(&host, HostRequest::Compact { strategy:"fixture".into(),
                context_window_tokens: 32768,
                instructions:"Keep all active constraints".into(),messages:archive.clone(),target_tokens:48000 });
            request.attempt_id = Some("prepared-compaction".into());
            let mut events = Vec::new();
            host.execute_compaction(&request,"Keep all active constraints",&archive,48000,
                &mut |event| { events.push(event); Ok(()) }).unwrap();
            let bodies = server.join().unwrap();
            let key = if provider == "openai-responses" { "input" } else { "messages" };
            if provider == "openai-responses" && !empty_continuation {
                assert_eq!(bodies[0]["reasoning"]["effort"], "none");
            } else if provider == "openai-responses" {
                assert!(bodies[0].get("reasoning").is_none());
            }
            let prefix = bodies[0][key].as_array().unwrap();
            assert_eq!(&bodies[1][key].as_array().unwrap()[..prefix.len()], prefix);
            assert!(bodies[1][key][prefix.len()]["content"].to_string().contains("unfinished code"));
            let mut continued = bodies[1].clone();
            continued[key] = bodies[0][key].clone();
            assert_eq!(continued, bodies[0], "Continuation must preserve tools, system, model and generation settings");
            assert!(!bodies[0].to_string().contains("1024 tokens"));
            assert!(!bodies[1].to_string().contains("1024 tokens"));
            for body in &bodies {
                assert_eq!(body.get("max_completion_tokens").or_else(||body.get("max_tokens")).or_else(||body.get("max_output_tokens")),Some(&json!(8192)));
            }
            let usages = events.iter().filter_map(|event| match &event.payload {
                HostResponse::ModelUsageReported { usage } => Some(usage), _ => None,
            }).collect::<Vec<_>>();
            assert_eq!(usages.len(),2);
            assert_eq!(usages[0]["output_tokens"],8000);
            assert_eq!(usages[0]["cache_usage"]["read_tokens"],0);
            assert_eq!(events.iter().filter(|event| matches!(event.payload,HostResponse::ModelAttemptStarted { .. })).count(),usages.len());
            assert_eq!(events.iter().any(|event| matches!(event.payload,HostResponse::CompactionCompleted { .. })),!empty_continuation);
            assert_eq!(events.iter().any(|event| matches!(event.payload,HostResponse::Failed { .. })),empty_continuation);
            if !empty_continuation {
                assert!(events.iter().any(|event| matches!(&event.payload,
                    HostResponse::CompactionCompleted { summary, .. } if summary == "unfinished code complete checkpoint")));
            }
        }
        }
    }

    #[test]
    fn rejected_execution_start_makes_zero_real_provider_connections() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let root =
            std::env::temp_dir().join(format!("nola-effect-start-http-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut driver =
            CliAgentHostDriver::new("delivery-host", test_llm(endpoint), test_context(&root));
        let request = fenced_request(
            &driver,
            HostRequest::Compact {
                strategy: "fixture".into(),
                instructions: "summarize".into(),
                messages: vec![json!({"role":"user","content":"fixture"})],
                target_tokens: 64,
                context_window_tokens: 128000,
            },
        );
        let expected_delivery = request.delivery.clone();
        let mut responses = Vec::new();
        let error = driver
            .dispatch(request, &mut |response| {
                responses.push(response.clone());
                assert!(matches!(response.payload, HostResponse::ExecutionStarted));
                assert_eq!(response.sequence, 0);
                assert_eq!(response.delivery, expected_delivery);
                Err("Application rejected execution start".into())
            })
            .unwrap_err();
        assert!(error.contains("rejected execution start"), "{error}");
        assert_eq!(responses.len(), 1);
        assert!(matches!(
            listener.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        assert!(driver.started_effect_deliveries.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepted_start_is_emitted_once_and_every_response_echoes_the_lease() {
        let root =
            std::env::temp_dir().join(format!("nola-effect-start-once-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let mut driver = CliAgentHostDriver::new(
            "delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()),
            test_context(&root),
        )
        .with_context_handler(move |_, _, _| {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(vec![json!({"text":"context"})])
        });
        let request = fenced_request(
            &driver,
            HostRequest::FetchContext {
                source_id: "fixture".into(),
                query: json!({"q":"current"}),
                freshness_deadline_ms: None,
            },
        );
        let expected_delivery = request.delivery.clone();
        let mut responses = Vec::new();
        driver
            .dispatch(request.clone(), &mut |response| {
                responses.push(response);
                Ok(())
            })
            .unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(responses.len(), 2);
        assert!(matches!(
            responses[0].payload,
            HostResponse::ExecutionStarted
        ));
        assert!(matches!(
            responses[1].payload,
            HostResponse::ContextFetched { .. }
        ));
        assert_eq!(responses[0].sequence, 0);
        assert_eq!(responses[1].sequence, 1);
        assert!(responses
            .iter()
            .all(|response| response.delivery == expected_delivery));

        let error = driver
            .dispatch(request, &mut |_| {
                panic!("duplicate delivery must emit nothing")
            })
            .unwrap_err();
        assert!(error.contains("already started"), "{error}");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expired_fenced_request_emits_rejection_without_execution_start() {
        let root =
            std::env::temp_dir().join(format!("nola-effect-expired-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let mut driver = CliAgentHostDriver::new(
            "delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()),
            test_context(&root),
        )
        .with_context_handler(move |_, _, _| {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Vec::new())
        });
        let mut request = fenced_request(
            &driver,
            HostRequest::FetchContext {
                source_id: "fixture".into(),
                query: json!({"q":"expired"}),
                freshness_deadline_ms: None,
            },
        );
        request.deadline_ms = Some(1);
        let expected_delivery = request.delivery.clone();
        let mut responses = Vec::new();

        driver
            .dispatch(request, &mut |response| {
                responses.push(response);
                Ok(())
            })
            .unwrap();

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(responses.len(), 1);
        assert!(matches!(
            &responses[0].payload,
            HostResponse::ExecutionRejected { code, retryable: false, .. }
                if code == "deadline_exceeded"
        ));
        assert_eq!(responses[0].delivery, expected_delivery);
        assert!(driver.started_effect_deliveries.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expired_request_with_invalid_fencing_identity_is_rejected_as_host_error() {
        let root = std::env::temp_dir().join(format!(
            "nola-effect-invalid-expired-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut driver = CliAgentHostDriver::new(
            "delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()),
            test_context(&root),
        );
        let mut request = fenced_request(
            &driver,
            HostRequest::FetchContext {
                source_id: "fixture".into(),
                query: json!({}),
                freshness_deadline_ms: None,
            },
        );
        request.deadline_ms = Some(1);
        request.delivery.as_mut().unwrap().lease_token.clear();

        let error = driver
            .dispatch(request, &mut |_| {
                panic!("invalid identity must emit no response")
            })
            .unwrap_err();
        assert!(error.contains("invalid fencing identity"), "{error}");
        assert!(driver.started_effect_deliveries.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expired_legacy_request_keeps_the_failed_response_family() {
        let root = std::env::temp_dir().join(format!(
            "nola-effect-legacy-expired-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut driver = CliAgentHostDriver::new(
            "delivery-host",
            test_llm("http://127.0.0.1:1/v1".into()),
            test_context(&root),
        );
        let mut request = fenced_request(
            &driver,
            HostRequest::FetchContext {
                source_id: "fixture".into(),
                query: json!({}),
                freshness_deadline_ms: None,
            },
        );
        request.delivery = None;
        request.deadline_ms = Some(1);
        let mut responses = Vec::new();

        driver
            .dispatch(request, &mut |response| {
                responses.push(response);
                Ok(())
            })
            .unwrap();

        assert_eq!(responses.len(), 1);
        assert!(matches!(
            &responses[0].payload,
            HostResponse::Failed { error }
                if error.code == "deadline_exceeded" && !error.retryable
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    struct DelayedHttpLeasePort {
        request: Option<HostRequestEnvelope>,
        lease: Option<nola_core::agent::host::EffectDeliveryLease>,
        initial_expiry: Option<i64>,
        terminal_expiry: Option<i64>,
        execution_started: bool,
        renewals: usize,
        renewals_before_headers: usize,
        headers_sent: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl nola_runtime_driver::CommittedRuntimeApplicationPort for DelayedHttpLeasePort {
    type Protocol = nola_core::agent::host::AgentRuntimeProtocol;
        fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope>, String> {
            Ok(Vec::new())
        }

        fn claim_committed_requests(
            &mut self,
            claim: nola_runtime_driver::RuntimeEffectDeliveryClaim,
        ) -> Result<Vec<HostRequestEnvelope>, String> {
            let Some(mut request) = self.request.take() else {
                return Ok(Vec::new());
            };
            let lease = nola_core::agent::host::EffectDeliveryLease {
                attempt_id: "delayed-http-attempt".into(),
                worker_generation: claim.worker_generation,
                lease_owner: claim.lease_owner,
                lease_token: "delayed-http-token".into(),
                lease_expires_at_ms: claim.lease_expires_at_ms,
            };
            self.initial_expiry = Some(lease.lease_expires_at_ms);
            self.lease = Some(lease.clone());
            request.delivery = Some(lease);
            Ok(vec![request])
        }

        fn renew_effect_delivery(
            &mut self,
            renewal: nola_runtime_driver::RuntimeEffectDeliveryRenewal,
        ) -> Result<nola_core::agent::host::EffectDeliveryLease, String> {
            let current = self.lease.as_ref().expect("claimed lease");
            assert_eq!(&renewal.delivery, current);
            assert!(renewal.lease_expires_at_ms > current.lease_expires_at_ms);
            let mut renewed = current.clone();
            renewed.lease_expires_at_ms = renewal.lease_expires_at_ms;
            self.lease = Some(renewed.clone());
            self.renewals += 1;
            if !self.headers_sent.load(std::sync::atomic::Ordering::SeqCst) {
                self.renewals_before_headers += 1;
            }
            Ok(renewed)
        }


        fn accept_raw_host_response(
            &mut self,
            response: HostResponseEnvelope,
        ) -> Result<nola_runtime_driver::CommittedRuntimeAcceptance<nola_core::agent::host::AgentRuntimeProtocol>, String> {
            assert_eq!(response.delivery.as_ref(), self.lease.as_ref());
            if matches!(response.payload, HostResponse::ExecutionStarted) {
                assert!(!self.execution_started);
                self.execution_started = true;
            } else {
                assert!(
                    self.execution_started,
                    "provider crossed the start ACK barrier"
                );
            }
            if response.payload.is_terminal() {
                self.terminal_expiry = response
                    .delivery
                    .as_ref()
                    .map(|delivery| delivery.lease_expires_at_ms);
            }
            Ok(nola_runtime_driver::CommittedRuntimeAcceptance {
                events: Vec::new(),
                traces: Vec::new(),
                control: None,
            })
        }

        fn accept_boundary_input(
            &mut self,
            _submission: nola_core::agent::host::AgentBoundaryInput,
        ) -> Result<nola_runtime_driver::CommittedRuntimeAcceptance<nola_core::agent::host::AgentRuntimeProtocol>, String> {
            Err("fixture does not accept boundary submissions".into())
        }
    }

    #[test]
    fn delayed_provider_headers_renew_delivery_and_terminal_uses_latest_lease() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let headers_sent = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_headers_sent = std::sync::Arc::clone(&headers_sent);
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "Host did not contact provider"
                            );
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Err(error) => panic!("accept delayed provider: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                let path = request_line.split_whitespace().nth(1).unwrap().to_string();
                let mut content_length = 0_usize;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        content_length = value.trim().parse().unwrap();
                    }
                }
                let mut request_body = vec![0_u8; content_length];
                reader.read_exact(&mut request_body).unwrap();
                if path == "/v1/models" {
                    let body =
                        json!({"data":[{"id":"fixture-model","context_length":32768}]}).to_string();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    continue;
                }
                assert_eq!(path, "/v1/chat/completions");
                assert!(request_line.starts_with("POST "));
                assert_eq!(
                    serde_json::from_slice::<Value>(&request_body).unwrap()["model"],
                    "fixture-model"
                );
                std::thread::sleep(std::time::Duration::from_millis(170));
                let delta = json!({"choices":[{"index":0,"delta":{"content":"durable summary"},"finish_reason":null}]});
                let finish = json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
                let body = format!("data: {delta}\n\ndata: {finish}\n\ndata: [DONE]\n\n");
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                server_headers_sent.store(true, std::sync::atomic::Ordering::SeqCst);
                return;
            }
        });

        let root =
            std::env::temp_dir().join(format!("nola-effect-renew-http-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let host = CliAgentHostDriver::new(
            "delivery-host",
            test_llm(format!("http://{address}/v1")),
            test_context(&root),
        );
        let mut request = fenced_request(
            &host,
            HostRequest::Compact {
                strategy: "fixture".into(),
                instructions: "summarize".into(),
                messages: vec![json!({"role":"user","content":"fixture"})],
                target_tokens: 64,
                context_window_tokens: 128000,
            },
        );
        request.delivery = None;
        request.attempt_id = Some("prepared-compaction-attempt".into());
        let mut runtime =
            nola_runtime_driver::CommittedRuntimeDriver::with_delivery_identity_and_timing(
                host,
                "delayed-http-generation",
                "delayed-http-owner",
                nola_runtime_driver::RuntimeEffectDeliveryTiming {
                    lease_duration: std::time::Duration::from_millis(100),
                    heartbeat_cadence: std::time::Duration::from_millis(25),
                },
            );
        let mut port = DelayedHttpLeasePort {
            request: Some(request),
            lease: None,
            initial_expiry: None,
            terminal_expiry: None,
            execution_started: false,
            renewals: 0,
            renewals_before_headers: 0,
            headers_sent,
        };

        let mut published_snapshot = false;
        runtime.execute(&mut port, &mut |output| {
            if matches!(output, nola_runtime_driver::CoreHostOutput::Control(_)) {
                published_snapshot = true;
            }
        }).unwrap();
        assert!(!published_snapshot);
        server.join().unwrap();
        assert!(port.execution_started);
        assert!(
            port.renewals >= 2,
            "expected repeated renewal during provider silence"
        );
        assert!(port.renewals_before_headers >= 1);
        assert!(port.terminal_expiry.unwrap() > port.initial_expiry.unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod provider_reasoning_channel_http_tests {
    use super::*;
    use nola_core::agent::host::{HostEffectId, HostId};
    use std::io::{BufRead, Read, Write};

    #[test]
    fn tool_preamble_never_becomes_provider_reasoning_through_real_host_http() {
        for reasoning in ["", "provider-reasoning-only"] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let reasoning = reasoning.to_string();
            let server_reasoning = reasoning.clone();
            let server = std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                let mut paths = Vec::new();
                for _ in 0..4 {
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(
                                    std::time::Instant::now() < deadline,
                                    "Host did not contact provider"
                                );
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            }
                            Err(error) => panic!("accept: {error}"),
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut request_line = String::new();
                    assert!(reader.read_line(&mut request_line).unwrap() > 0);
                    let path = request_line.split_whitespace().nth(1).unwrap().to_string();
                    paths.push(path.clone());
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        assert!(reader.read_line(&mut line).unwrap() > 0);
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(value) =
                            line.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    assert!(length < 1_000_000);
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    if path == "/v1/models" {
                        assert!(request_line.starts_with("GET "));
                        let body = json!({"data":[{"id":"fixture-model","context_length":32768}]})
                            .to_string();
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                        continue;
                    }
                    assert_eq!(
                        path, "/v1/chat/completions",
                        "unexpected provider request: {paths:?}"
                    );
                    assert!(request_line.starts_with("POST "));
                    let input: Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(input["model"], "fixture-model");
                    let delta = json!({"id":"response-42","model":"publisher/resolved-model","choices":[{"index":0,"delta":{
                    "content":"ordinary-preamble-not-reasoning",
                    "reasoning_content":server_reasoning,
                    "tool_calls":[{"index":0,"id":"question-1","type":"function","function":{
                        "name":"request_user_input","arguments":"{\"questions\":[]}"}}]
                },"finish_reason":null}]});
                    let finish =
                        json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
                    let body = format!("data: {delta}\n\ndata: {finish}\n\ndata: [DONE]\n\n");
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                    return paths;
                }
                panic!("provider request limit exceeded: {paths:?}");
            });
            let root = std::env::temp_dir()
                .join(format!("nola-provider-channels-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let context = ToolContext {
                memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                language_services: Default::default(),
                skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
                allowed_dirs: vec![root.clone()],
                policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "allow".into(), Vec::new()),
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
                endpoint: format!("http://{address}/v1"),
                api_key: "fixture".into(),
                model: "fixture-model".into(),
                temperature: 0.0,
                top_p: 1.0,
                max_output_tokens: 512,
                thinking_enabled: !reasoning.is_empty(), reasoning_summary_supported: None,
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
            let mut driver = CliAgentHostDriver::new("channel-host", llm, context);
            let request = HostRequestEnvelope {
                attempt_started: false,
                delegated_invocation_id: None,
                attempt_id: Some("planned-channel-attempt".into()),
                request_id: "channel-request".into(),
                effect_id: HostEffectId("channel-effect".into()),
                host_id: HostId("channel-host".into()),
                lease_id: None,
                session_id: "channel-session".into(),
                turn_id: Some("channel-turn".into()),
                catalog_revision: driver.catalog.revision,
                availability_epoch: driver.catalog.availability_epoch,
                created_at_ms: 0,
                deadline_ms: None,
                batch: None,
                resume_after_sequence: None,
                delivery: None,
                payload: HostRequest::Compact {
                    strategy: "fixture".into(),
                    instructions: String::new(),
                    messages: Vec::new(),
                    target_tokens: 512,
                    context_window_tokens: 128000,
                },
            };
            let mut responses = Vec::new();
            driver.execute_model(&request, "fixture-model", "text-1", "reasoning-1",
                &[json!({"role":"user","content":"write DFS"})],
                &[json!({"type":"function","function":{"name":"request_user_input","parameters":{"type":"object"}}})],
                &ModelInvocationPlan {
                    native_tool_calls: true,
                    reasoning: !reasoning.is_empty(),
                    reasoning_continuity: true,
                    stream_text: true,
                    stream_reasoning: true,
                    stream_tool_calls: true,
                    ..ModelInvocationPlan::default()
                }, "", BTreeMap::new(), None,
                ModelResponseMode::Conversational, false, &mut |response| { responses.push(response); Ok(()) }).unwrap();
            let started = responses
                .iter()
                .filter(|response| {
                    matches!(response.payload, HostResponse::ModelAttemptStarted { .. })
                })
                .collect::<Vec<_>>();
            assert_eq!(
                started.len(),
                1,
                "fixture must accept the prepared Attempt before sampling"
            );
            assert_eq!(
                started[0].attempt_id.as_deref(),
                Some("planned-channel-attempt")
            );
            assert!(responses
                .iter()
                .all(|response| response.attempt_id.as_deref() == Some("planned-channel-attempt")));
            let provenance = responses
                .iter()
                .find_map(|response| match &response.payload {
                    HostResponse::ToolProgress {
                        title,
                        detail: Some(detail),
                        ..
                    } if title == "model_response_provenance" => {
                        serde_json::from_str::<Value>(detail).ok()
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!("real Host must persist provider provenance; responses: {responses:#?}")
                });
            assert_eq!(provenance["configured_model"], "fixture-model");
            assert_eq!(provenance["response_model"], "publisher/resolved-model");
            assert_eq!(provenance["response_id"], "response-42");
            assert_eq!(provenance["attempt"], 1);
            let paths = server.join().unwrap();
            assert_eq!(
                paths.last().map(String::as_str),
                Some("/v1/chat/completions")
            );
            let items = responses
                .iter()
                .find_map(|response| match &response.payload {
                    HostResponse::ModelCompleted {
                        items, tool_calls, ..
                    } => {
                        assert_eq!(tool_calls.len(), 1);
                        Some(items)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!("real Host must finish the model call; responses: {responses:#?}")
                });
            let reasoning_items: Vec<_> = items
                .iter()
                .filter(|item| item["type"] == "reasoning")
                .collect();
            assert_eq!(reasoning_items.len(), usize::from(!reasoning.is_empty()));
            for item in reasoning_items {
                assert_eq!(item["content"], reasoning);
            }
            for response in &responses {
                if let HostResponse::ModelContinuation { continuation } = &response.payload {
                    assert!(!continuation
                        .state
                        .to_string()
                        .contains("ordinary-preamble-not-reasoning"));
                }
            }
            drop(driver);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

fn memory_usage_metadata(capability_id: &str, outcome: &ToolOutcome) -> Value {
    if capability_id != "memory_search" || outcome.error {
        return Value::Array(Vec::new());
    }
    let Ok(payload) = serde_json::from_str::<Value>(&outcome.text) else {
        return Value::Array(Vec::new());
    };
    let digest = format!("sha256:{:x}", sha2::Sha256::digest(outcome.text.as_bytes()));
    let used_at_epoch_ms = chrono::Utc::now().timestamp_millis();
    Value::Array(
        payload
            .get("memories")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|memory| memory.get("id").and_then(Value::as_str))
            .map(|memory_id| {
                json!({
                    "memory_id": memory_id,
                    "used_at_epoch_ms": used_at_epoch_ms,
                    "context_payload_digest": digest,
                    "citation_ids": [format!("memory-search:{memory_id}")],
                })
            })
            .collect(),
    )
}

fn is_deterministic_tool_failure(detail: &str) -> bool {
    fn known_category(category: &str) -> bool {
        matches!(
            category,
            "invalid_tool_arguments"
                | "truncated_validation_output"
                | "write_payload_too_large"
                | "append_chunk_too_large"
                | "edit_no_effect"
        )
    }

    if let Ok(value) = serde_json::from_str::<Value>(detail) {
        if let Some(category) = value
            .get("deterministic_failure_category")
            .and_then(Value::as_str)
            .or_else(|| value.get("error").and_then(Value::as_str))
        {
            return known_category(category);
        }
        return value
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(is_deterministic_tool_failure);
    }

    detail.lines().any(|line| {
        line.trim()
            .strip_prefix("deterministic_failure_category:")
            .map(str::trim)
            .is_some_and(known_category)
    })
}

#[cfg(test)]
mod model_recovery_budget_tests {
    use super::{ModelRecoveryBudget, ModelTextPresentationGate};

    #[test]
    fn protocol_corrections_are_bounded() {
        let mut budget = ModelRecoveryBudget::new();
        assert!(budget.take_protocol_correction());
        assert!(budget.take_protocol_correction());
        assert!(!budget.take_protocol_correction());
    }

    #[test]
    fn tool_capable_answer_streams_after_the_initial_semantic_gate() {
        let mut gate = ModelTextPresentationGate::for_request(false, true);
        assert_eq!(
            gate.push_text("a".repeat(159)).concat().chars().count(),
            159
        );
        // Once the initial gate opens, transport-sized deltas still pass
        // through the semantic presentation buffer. A one-character provider
        // callback must not become a one-character persisted/UI update.
        assert!(gate.push_text(".".to_string()).is_empty());
        assert_eq!(gate.finish_answer(), vec!["."]);
    }

    #[test]
    fn retry_after_partial_tool_transport_does_not_suppress_the_clean_answer() {
        let mut gate = ModelTextPresentationGate::for_request(false, true);
        gate.observe_tool_call();
        gate.reset_after_transport_retry();

        assert!(gate
            .push_text("Clean answer after retry.".to_string())
            .is_empty());
        assert_eq!(gate.finish_answer(), vec!["Clean answer after retry."]);
    }
}

#[cfg(test)]
mod deterministic_tool_failure_tests {
    use super::is_deterministic_tool_failure;

    #[test]
    fn classifies_only_strict_machine_failure_fields() {
        for detail in [
            r#"{"error":"invalid_tool_arguments"}"#,
            "参数无效\ndeterministic_failure_category: invalid_tool_arguments\nnext_action: retry",
            r#"{"text":"写入过大\ndeterministic_failure_category: write_payload_too_large","display":"写入失败"}"#,
        ] {
            assert!(is_deterministic_tool_failure(detail), "{detail}");
        }
        for detail in [
            "connection reset by peer",
            "bash approval required",
            "Permission denied. Approve interactively.",
            "HTTP 503 Service Unavailable",
            "未知工具 invalid_tool_arguments_search，请检查 current 目录",
            "検索結果: Exact edit preflight failed: old_text_not_found",
            "command too long はユーザーの引用で、ツール状態ではない",
            r#"{"error":"unknown_tool","message":"invalid_tool_arguments; search current"}"#,
            r#"{"error":"permission_denied","message":"deterministic_failure_category: edit_no_effect"}"#,
            "prefix deterministic_failure_category: invalid_tool_arguments",
            "deterministic_failure_category: unknown_tool",
        ] {
            assert!(!is_deterministic_tool_failure(detail), "{detail}");
        }
    }
}

fn can_salvage_text_only_completion(tool_projection_is_empty: bool, content: &str) -> bool {
    tool_projection_is_empty && !content.trim().is_empty()
}

#[cfg(test)]
mod text_only_completion_contract_tests {
    use super::can_salvage_text_only_completion;

    #[test]
    fn repeated_unavailable_call_can_preserve_nonempty_text_at_tool_free_boundary() {
        assert!(can_salvage_text_only_completion(
            true,
            "The requested cleanup was denied; the validated artifacts remain available."
        ));
    }

    #[test]
    fn unavailable_call_never_turns_an_empty_or_executable_projection_into_success() {
        assert!(!can_salvage_text_only_completion(true, "  \n"));
        assert!(!can_salvage_text_only_completion(
            false,
            "I will call a different tool."
        ));
    }
}

fn normalize_host_tool_args(
    normalization: &HostInputNormalization,
    args: &mut serde_json::Map<String, Value>,
) {
    for rule in &normalization.root_relative_patterns {
        for pattern in &rule.patterns {
            normalize_root_relative_pattern(args, rule, pattern);
        }
    }
    for (field, maximum) in &normalization.integer_maximums {
        if args
            .get(field)
            .and_then(Value::as_u64)
            .is_some_and(|value| value > *maximum)
        {
            args.insert(field.clone(), Value::from(*maximum));
        }
    }
}

fn normalize_root_relative_pattern(
    args: &mut serde_json::Map<String, Value>,
    rule: &HostRootRelativePatternNormalization,
    pattern_field: &str,
) {
    let Some(root) = args
        .get(&rule.root)
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let Some(pattern) = args
        .get(pattern_field)
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let root = root.trim().trim_end_matches('/');
    let pattern = pattern.trim().trim_start_matches("./");
    let relative_root = root.trim_start_matches("./");
    let root_name = std::path::Path::new(root)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let prefix = if !relative_root.is_empty()
        && !std::path::Path::new(relative_root).is_absolute()
        && pattern.starts_with(&format!("{relative_root}/"))
    {
        Some(relative_root)
    } else if !root_name.is_empty() && pattern.starts_with(&format!("{root_name}/")) {
        Some(root_name)
    } else {
        None
    };
    if let Some(prefix) = prefix {
        args.insert(
            pattern_field.to_string(),
            Value::String(pattern[prefix.len() + 1..].to_string()),
        );
    }
}

struct PreparedCompactionArchive {
    prompt: String,
}

fn prepare_compaction_archive(messages: &[Value]) -> Result<PreparedCompactionArchive, String> {
    const PREFIX: &[u8] = b"Archive JSON:\n";
    let mut bytes = PREFIX.to_vec();
    serde_json::to_writer(&mut bytes, messages)
        .map_err(|error| format!("serialize compaction archive failed: {error}"))?;
    // serde_json writes UTF-8. Reuse its allocation instead of retaining an
    // archive String alongside a second, prefixed prompt String.
    let prompt = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    Ok(PreparedCompactionArchive { prompt })
}

fn compaction_model_messages(instructions: &str, prompt: String) -> Vec<Value> {
    let mut user = json!({"role": "user"});
    // json!({"content": prompt}) serializes a borrow and clones the large
    // string. Assign the owned Value to transfer the existing allocation.
    user["content"] = Value::String(prompt);
    vec![json!({"role":"system", "content":instructions}), user]
}

fn compaction_summary_instructions(instructions: &str, _output_limit: u64, _recovery: bool) -> String {
    nola_core::compaction_maintenance::maintenance_instructions(instructions)
}

#[cfg(test)]
mod model_text_presentation_gate_tests {
    use super::{
        agent_items_to_llm_messages, apply_provider_continuation_to_messages,
        build_cli_capability_catalog, build_cli_host_catalog, compaction_model_messages,
        host_runtime_fact, model_response_requires_protocol_recovery, has_tool_result_since_latest_user,
        initial_model_tool_schemas, inject_tool_artifact_media,
        prepare_compaction_archive,
        project_overview_file_count_from_value, project_overview_file_count_since_latest_user,
        provider_messages_for_active_turn, provider_messages_for_wire_protocol,
        skill_resource_directory_from_outcome, tools_for_model_turn,
        tools_for_model_turn_with_skill_state, tools_for_turn, HostTurnPolicy,
        ModelTextPresentationGate, INITIAL_MODEL_TOOL_IDS,
    };
    use crate::tools::{bash_command_paths_stay_in_allowed_dirs, tool_schemas_for_host};
    use serde_json::{json, Value};

    #[test]
    fn web_budget_exhaustion_does_not_finalize_the_whole_agent_turn() {
        let catalog = build_cli_host_catalog("test-host");
        for capability_id in ["web_search", "web_fetch"] {
            assert!(
                catalog.capabilities[capability_id]
                    .input_normalization
                    .turn_budget
                    .is_none(),
                "web retrieval must remain available for long mixed research and artifact workflows"
            );
        }
        assert!(catalog.capabilities.contains_key("write_file"));
        assert!(catalog.capabilities.contains_key("bash"));
        assert!(catalog.capabilities.contains_key("Skill"));
    }

    #[test]
    fn host_projection_preserves_composed_prompt_and_canonical_user_provenance() {
        let composition = nola_core::agent::compose_agent_prompt(vec![
            nola_core::agent::AgentPromptFragment {
                id: "workflow".to_string(),
                layer: nola_core::agent::AgentPromptLayer::ExecutionPolicy,
                trust: nola_core::agent::AgentPromptTrust::HostInstruction,
                cacheable: true,
                content: "Stable Host workflow".to_string(),
            },
            nola_core::agent::AgentPromptFragment {
                id: "host-fact".to_string(),
                layer: nola_core::agent::AgentPromptLayer::HostCapabilities,
                trust: nola_core::agent::AgentPromptTrust::HostFact,
                cacheable: false,
                content: "Tools exposed: read_file".to_string(),
            },
        ])
        .expect("prompt composition");
        let items = vec![
            json!({"type":"message","id":"s1","role":"system","content":composition.rendered}),
            json!({"type":"message","id":"u1","role":"user","content":"原始 query search current","_nola_user_input":true}),
            json!({"type":"message","id":"runtime1","role":"user","content":"Host runtime observation","_nola_user_input":false}),
        ];

        let messages = provider_messages_for_active_turn(agent_items_to_llm_messages(&items));

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "system");
        assert!(messages[0]["content"]
            .as_str()
            .expect("system content")
            .contains("Stable Host workflow"));
        assert_eq!(
            messages[1],
            json!({"role":"user","content":"原始 query search current","_nola_user_input":true})
        );
        assert_eq!(messages[2], json!({"role":"user","content":"Host runtime observation","_nola_user_input":false}));
    }

    #[test]
    fn tool_capable_answer_keeps_a_short_prefix_private_until_completion() {
        let mut gate = ModelTextPresentationGate::for_request(false, true);
        assert!(gate
            .push_text("A concise opening that is not complete".to_string())
            .is_empty());
        assert!(gate
            .push_text(" until this boundary arrives.".to_string())
            .is_empty());
        assert!(gate
            .push_text(" The next phrase streams directly.".to_string())
            .is_empty());
        assert_eq!(
            gate.finish_answer().concat(),
            "A concise opening that is not complete until this boundary arrives. The next phrase streams directly."
        );
    }

    #[test]
    fn tool_round_discards_an_ambiguous_preamble_before_it_reaches_the_ui() {
        let mut gate = ModelTextPresentationGate::for_request(false, true);
        assert!(gate
            .push_text("I will inspect the workspace first.".to_string())
            .is_empty());
        gate.observe_tool_call();
        assert!(gate.finish_answer().is_empty());
        assert!(gate.push_text("must stay hidden".to_string()).is_empty());
    }

    #[test]
    fn long_answer_opens_before_model_completion_even_when_tools_are_available() {
        let mut gate = ModelTextPresentationGate::for_request(false, true);
        let long_preamble = "I will inspect the available sources before answering. ".repeat(12);

        assert!(!gate.push_text(long_preamble).is_empty());
        gate.observe_tool_call();
        assert!(gate.finish_answer().is_empty());
    }

    #[test]
    fn completed_tool_capable_answer_is_released_as_semantic_chunks() {
        let mut gate = ModelTextPresentationGate::for_request(false, true);

        assert!(gate
            .push_text("First verified point. Second verified point.".to_string())
            .is_empty());
        assert_eq!(
            gate.finish_answer().concat(),
            "First verified point. Second verified point."
        );
    }

    #[test]
    fn short_final_answer_is_released_when_the_model_completes() {
        let mut gate = ModelTextPresentationGate::for_request(false, true);
        assert!(gate.push_text("Short answer.".to_string()).is_empty());
        assert_eq!(gate.finish_answer(), vec!["Short answer."]);
    }

    #[test]
    fn private_answer_never_releases_presentation_deltas() {
        let mut gate = ModelTextPresentationGate::for_request(true, true);
        assert!(gate
            .push_text("A complete but still unverified answer.".to_string())
            .is_empty());
        assert!(gate.finish_answer().is_empty());
    }

    #[test]
    fn deferred_tool_round_never_releases_provisional_text() {
        let mut gate = ModelTextPresentationGate::for_request(true, true);
        assert!(gate
            .push_text(
                "A long provisional tool preamble that crosses several semantic boundaries."
                    .repeat(4)
            )
            .is_empty());
        gate.observe_tool_call();
        assert!(gate.finish_answer().is_empty());
    }

    #[test]
    fn long_private_answer_does_not_fan_out_into_durable_presentation_events() {
        let mut gate = ModelTextPresentationGate::for_request(true, true);
        let answer = concat!(
            "## Verified result\n\n",
            "The first verified point is complete and remains stable. ",
            "The second verified point is also complete and remains stable. ",
            "The final recommendation preserves the exact Markdown body."
        );

        for delta in answer.as_bytes().chunks(7) {
            assert!(gate
                .push_text(String::from_utf8(delta.to_vec()).expect("ASCII fixture"))
                .is_empty());
        }
        let batches = gate.finish_answer();

        assert!(
            batches.is_empty(),
            "private text belongs only in ModelCompleted"
        );
    }

    #[test]
    fn markup_examples_do_not_decide_provider_protocol_validity() {
        let mut gate = ModelTextPresentationGate::default();
        assert!(gate
            .push_text("我来检查 CLI。\n\n<ba".to_string())
            .is_empty());
        assert!(gate
            .push_text("sh>\nproject_overview -p /workspace\n</bash>".to_string())
            .is_empty());
        assert!(!model_response_requires_protocol_recovery("stop", false));
        assert!(model_response_requires_protocol_recovery("protocol_error", false));
    }

    #[test]
    fn prose_is_not_an_execution_or_protocol_verdict() {
        let content = "我来检查 CLI。\n\n```bash\nls -la /workspace/cli\n```\n\n它的输出显示：\n\n```\nFAKE-LISTING\n```";
        let mut gate = ModelTextPresentationGate::default();
        assert!(gate.push_text(content.to_string()).is_empty());
        assert!(!model_response_requires_protocol_recovery("stop", false));
        assert_eq!(gate.finish_answer().concat(), content);
    }

    #[test]
    fn tool_evidence_is_scoped_to_the_latest_user_turn() {
        let completed_tool_turn = vec![
            json!({"role":"user","content":"inspect","_nola_user_input":true}),
            json!({"role":"assistant","content":"","tool_calls":[]}),
            json!({"role":"tool","content":"real output"}),
        ];
        assert!(has_tool_result_since_latest_user(&completed_tool_turn));

        let next_turn = vec![
            json!({"role":"user","content":"inspect","_nola_user_input":true}),
            json!({"role":"tool","content":"real output"}),
            json!({"role":"user","content":"inspect something else","_nola_user_input":true}),
        ];
        assert!(!has_tool_result_since_latest_user(&next_turn));
    }

    #[test]
    fn project_overview_count_is_read_only_from_the_current_user_turn() {
        let messages = vec![
            json!({"role":"user","content":"inspect cli","_nola_user_input":true}),
            json!({
                "role":"tool",
                "name":"project_overview",
                "content": serde_json::to_string(&json!({
                    "display":"Project overview",
                    "text":"# Project overview\nscanned_files: 87\n"
                })).unwrap()
            }),
        ];
        assert_eq!(
            project_overview_file_count_since_latest_user(&messages),
            Some(87)
        );

        let later_turn = vec![
            messages[0].clone(),
            messages[1].clone(),
            json!({"role":"user","content":"now inspect src/llm.rs","_nola_user_input":true}),
        ];
        assert_eq!(
            project_overview_file_count_since_latest_user(&later_turn),
            None
        );
    }

    #[test]
    fn project_overview_count_unwraps_host_output_envelopes() {
        let output = json!({
            "output_preview": {
                "display": "Project overview",
                "text": "# Project overview\nscanned_files: 87\n"
            }
        });
        assert_eq!(project_overview_file_count_from_value(&output), Some(87));

        let serialized = Value::String(output.to_string());
        assert_eq!(
            project_overview_file_count_from_value(&serialized),
            Some(87)
        );
    }

    #[test]
    fn overview_fact_does_not_choose_a_workflow_or_rewrite_policy() {
        let message = host_runtime_fact("project-overview", json!({"sampled_file_count":87}));
        assert_eq!(message["role"], "user");
        assert!(message["content"].as_str().unwrap().contains("sampled_file_count"));
        assert!(!message.to_string().contains("ask exactly one"));
    }

    #[test]
    fn dynamic_facts_preserve_the_entire_existing_message_prefix() {
        let mut messages = vec![
            json!({"role":"system","content":"base"}),
            json!({"role":"user","content":"inspect"}),
            json!({"role":"tool","content":"overview"}),
        ];
        let prefix = messages.clone();
        messages.push(host_runtime_fact("recovery", json!({"executed_calls":0})));
        assert_eq!(messages.len(), 4);
        assert_eq!(&messages[..3], prefix.as_slice());
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "base");
        assert_eq!(messages[1]["role"], "user");
    }

    #[test]
    fn reasoning_and_parallel_tool_calls_stay_in_one_assistant_message() {
        let serial_request_plan = nola_core::agent::ModelInvocationPlan::default();
        assert!(!serial_request_plan.parallel_tool_calls);
        let messages = agent_items_to_llm_messages(&[
            json!({"type":"message","id":"u1","role":"user","content":"inspect"}),
            json!({"type":"reasoning","id":"r1","summary":"","content":"I should inspect the workspace."}),
            json!({"type":"tool_call","id":"t1","call_id":"call-1","name":"workspace_status","arguments":{}}),
            json!({"type":"tool_call","id":"t2","call_id":"call-2","name":"list_dir","arguments":{"path":"."}}),
            json!({"type":"tool_result","id":"tr1","call_id":"call-1","name":"workspace_status","output":{"ok":true}}),
        ]);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(
            messages[1]["reasoning_content"],
            "I should inspect the workspace."
        );
        assert_eq!(messages[1]["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(messages[2]["role"], "tool");
    }

    #[test]
    fn opaque_responses_continuations_keep_each_tool_step_in_order() {
        let continuation = |call_id: &str| {
            json!({
                "provider_namespace":"openai.responses",
                "model_id":"fixture-model",
                "state":{"route_identity":"route", "response_items":[
                    {"type":"function_call","call_id":call_id,"name":"read","arguments":"{}"}
                ]},
                "integrity_kind":"encrypted"
            })
        };
        let messages = agent_items_to_llm_messages(&[
            json!({"type":"message","id":"u1","role":"user","content":"inspect"}),
            json!({"type":"reasoning","id":"r1","content":"",
                "provider_continuation":continuation("call-1")}),
            json!({"type":"tool_call","id":"t1","call_id":"call-1","name":"read","arguments":{}}),
            json!({"type":"tool_result","id":"o1","call_id":"call-1","name":"read","output":"first"}),
            json!({"type":"reasoning","id":"r2","content":"",
                "provider_continuation":continuation("call-2")}),
            json!({"type":"tool_call","id":"t2","call_id":"call-2","name":"read","arguments":{}}),
            json!({"type":"tool_result","id":"o2","call_id":"call-2","name":"read","output":"second"}),
        ]);
        assert_eq!(
            messages[1]["_nola_provider_continuation"]["state"]["response_items"][0]["call_id"],
            "call-1"
        );
        assert_eq!(
            messages[3]["_nola_provider_continuation"]["state"]["response_items"][0]["call_id"],
            "call-2"
        );
    }

    #[test]
    fn responses_replay_attaches_every_step_across_turns() {
        let cfg = crate::llm::LlmConfig {
            siwc: None, native_model_route: None,
            provider: Some("openai-responses".into()),
            fallbacks: Vec::new(),
            endpoint: "http://localhost/v1".into(),
            api_key: String::new(),
            model: "fixture-model".into(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 4096,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            max_retries: 0,
            parallel_tool_calls: None,
            thinking_enabled: true, reasoning_summary_supported: None,
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
        };
        let route = crate::llm::responses_route_identity(&cfg);
        let attached = |call_id: &str| {
            json!({
                "provider_namespace":"openai.responses",
                "model_id":"fixture-model",
                "state":{"route_identity":route, "response_items":[
                    {"type":"function_call","call_id":call_id,"name":"read","arguments":"{}"}
                ]},
                "integrity_kind":"encrypted"
            })
        };
        let mut messages = vec![
            json!({"role":"assistant","content":"old","_nola_provider_continuation":attached("old")}),
            json!({"role":"user","content":"new task"}),
            json!({"role":"assistant","content":"","_nola_provider_continuation":attached("call-1")}),
            json!({"role":"tool","tool_call_id":"call-1","content":"first"}),
            json!({"role":"assistant","content":"","_nola_provider_continuation":attached("call-2")}),
            json!({"role":"tool","tool_call_id":"call-2","content":"second"}),
        ];
        apply_provider_continuation_to_messages(&mut messages, None, &cfg);
        assert_eq!(messages[0]["nola_response_items"][0]["call_id"], "old");
        assert!(messages[0].get("_nola_provider_continuation").is_none());
        assert_eq!(messages[2]["nola_response_items"][0]["call_id"], "call-1");
        assert_eq!(messages[4]["nola_response_items"][0]["call_id"], "call-2");

        let items = vec![
            json!({"type":"message","id":"u1","role":"user","content":"first task"}),
            json!({"type":"reasoning","id":"r1","content":"",
                "provider_continuation":attached("old")}),
            json!({"type":"tool_call","id":"c1","call_id":"old","name":"read","arguments":{}}),
            json!({"type":"tool_result","id":"o1","call_id":"old","name":"read","output":"prior result"}),
            json!({"type":"message","id":"a1","role":"assistant","content":"done"}),
            json!({"type":"message","id":"u2","role":"user","content":"follow up"}),
        ];
        let mut responses_history = provider_messages_for_wire_protocol(&items, &cfg);
        apply_provider_continuation_to_messages(&mut responses_history, None, &cfg);
        assert_eq!(
            responses_history[1]["nola_response_items"][0]["call_id"],
            "old"
        );
        assert_eq!(responses_history[2]["role"], "tool");
        assert_eq!(responses_history[2]["content"], "\"prior result\"");

        let mut chat_cfg = cfg.clone();
        chat_cfg.provider = Some("openai".into());
        let chat_history = provider_messages_for_wire_protocol(&items, &chat_cfg);
        assert!(chat_history.iter().any(|message| message["role"] == "tool"));
        assert!(chat_history.iter().any(|message| message.get("tool_calls").is_some()));
    }

    #[test]
    fn multimodal_transport_injects_only_media_that_is_actually_present() {
        let store = crate::tools::ToolOutputStore::for_test();
        let path = store.persist_media(b"\x89PNG\r\n\x1a\nfixture").expect("snapshot fixture");
        let uri = url::Url::from_file_path(&path)
            .expect("file URL")
            .to_string();
        let items = vec![
            json!({"type":"message","id":"u1","role":"user","content":"inspect layout"}),
            json!({"type":"tool_call","id":"t1","call_id":"call-1","name":"browser_screenshot","arguments":{"target":"managed","session_id":"test"}}),
            json!({
                "type":"tool_result",
                "id":"tr1",
                "call_id":"call-1",
                "name":"browser_screenshot",
                "output":{"text":"screenshot captured"},
                "artifacts":[{"uri":uri,"name":"page.png","mime_type":"image/png","operation":"observe"}]
            }),
        ];

        let mut vision = provider_messages_for_active_turn(agent_items_to_llm_messages(&items));
        inject_tool_artifact_media(&mut vision, true, false);
        assert_eq!(vision.len(), 4);
        assert_eq!(vision[3]["role"], "user");
        assert!(vision[3]["content"][1]["image_url"]["url"]
            .as_str()
            .is_some_and(|value| value.starts_with("data:image/png;base64,")));

        let text_items = vec![json!({
            "type":"message",
            "id":"u2",
            "role":"user",
            "content":"text only"
        })];
        let mut text_only =
            provider_messages_for_active_turn(agent_items_to_llm_messages(&text_items));
        inject_tool_artifact_media(&mut text_only, true, false);
        assert_eq!(text_only.len(), 1);
        assert_eq!(text_only[0]["content"], "text only");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn provider_history_preserves_completed_tools_across_new_user_turns() {
        let messages = vec![
            json!({"role":"system","content":"instructions"}),
            json!({"role":"user","content":"inspect the project"}),
            json!({"role":"assistant","content":"","tool_calls":[{
                "id":"old-call",
                "type":"function",
                "function":{"name":"read_file","arguments":"{\"path\":\"status.txt\"}"}
            }]}),
            json!({"role":"tool","tool_call_id":"old-call","name":"read_file","content":"old result marker"}),
            json!({"role":"assistant","content":"The project is healthy."}),
            json!({"role":"user","content":"now inspect the tests"}),
            json!({"role":"assistant","content":"","tool_calls":[{"id":"current-call"}]}),
            json!({"role":"tool","tool_call_id":"current-call","content":"current result"}),
        ];

        let prepared = super::provider_messages_for_active_turn(messages.clone());
        assert_eq!(prepared, messages);
        let roles = prepared
            .iter()
            .filter_map(|message| message.get("role").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert_eq!(
            roles,
            vec![
                "system",
                "user",
                "assistant",
                "tool",
                "assistant",
                "user",
                "assistant",
                "tool"
            ]
        );
        assert!(prepared.iter().any(|message| {
            message.get("tool_call_id").and_then(Value::as_str) == Some("old-call")
        }));
        assert_eq!(prepared[3]["content"], "old result marker");
        assert!(prepared.iter().any(|message| {
            message.get("tool_call_id").and_then(Value::as_str) == Some("current-call")
        }));
    }

    #[test]
    fn no_tool_surface_skips_the_skill_decision_preflight() {
        assert!(!super::skill_decision_tools_available(&[]));
        assert!(super::skill_decision_tools_available(&[json!({
            "type":"function",
            "function":{"name":"skill_skip","parameters":{"type":"object"}}
        })]));
    }

    #[test]
    fn catalog_declares_initial_and_deferred_tools_without_hiding_discovery() {
        let host = build_cli_host_catalog("projection-contract");
        assert_eq!(
            host.capabilities["read_file"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Visible
        );
        assert_eq!(
            host.capabilities["read_file"].effect_for(&serde_json::json!({"path": "notes.md"})),
            nola_core::agent::host::HostCapabilityEffect::Investigate
        );
        assert_eq!(
            host.capabilities["read_file"].effect_for(&serde_json::json!({
                "path": "notes.md",
                "operation_kind": "validate"
            })),
            nola_core::agent::host::HostCapabilityEffect::Validate
        );
        assert_eq!(
            host.capabilities["edit_file"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Visible
        );
        assert_eq!(
            host.capabilities["bash"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Visible
        );
        let bash = &host.capabilities["bash"];
        assert_eq!(
            bash.effect_for(&serde_json::json!({
                "command": "words that resemble a test or file mutation"
            })),
            nola_core::agent::host::HostCapabilityEffect::Execute
        );
        assert!(!bash.mutation_requires_validation_for(&serde_json::json!({
            "command": "words that resemble a test or file mutation"
        })));
        assert!(!bash.mutation_requires_validation_for(&serde_json::json!({
            "command": "opaque-command",
            "operation_kind": "mutate"
        })));
        assert!(!bash.mutation_requires_validation_for(&serde_json::json!({
            "command": "opaque-command",
            "operation_kind": "validate"
        })));
        let writer = &host.capabilities["write_file"];
        assert!(
            !writer.mutation_requires_validation_for(&serde_json::json!({
                "path": "any.extension",
                "content": "value"
            }))
        );
        assert_eq!(
            host.capabilities["spawn_agent"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Deferred
        );
        assert_eq!(
            host.capabilities["goal_complete"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Deferred
        );
        assert_eq!(
            host.capabilities["diff_review"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Deferred
        );
        for capability_id in [
            "Skill",
            "SOP",
            "tool_search",
            "memory_search",
            "conversation_recall",
            "request_user_input",
            "repo_map",
            "code_search",
            "code_navigation",
            "failure_context",
            "read_file",
            "write_file",
            "edit_file",
            "apply_patch",
            "workspace_status",
            "bash",
        ] {
            assert_eq!(
                host.capabilities[capability_id].exposure,
                nola_core::agent::host::HostCapabilityExposure::Visible,
                "protocol entry capability must remain directly visible: {capability_id}"
            );
        }
        assert_eq!(
            host.capabilities["memory_save"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Deferred
        );
        assert_eq!(
            host.capabilities["workspace_status"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Visible
        );
        assert_eq!(
            host.capabilities["tool_output_search"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Deferred
        );

        let discovery = build_cli_capability_catalog(&host);
        assert!(discovery.builtin_tools.contains("read_file"));
        assert!(discovery.builtin_tools.contains("edit_file"));
        assert_eq!(
            super::mcp_tool_exposure(),
            nola_core::agent::host::HostCapabilityExposure::Deferred
        );
    }

    #[test]
    fn initial_surface_contains_protocol_entry_capabilities() {
        let schemas = tool_schemas_for_host()
            .into_iter()
            .chain(crate::code_intelligence_provider::model_tool_schemas())
            .collect();
        let initial = initial_model_tool_schemas(schemas);
        for schema in &initial {
            let description = schema
                .pointer("/function/description")
                .and_then(Value::as_str)
                .expect("initial tool description");
            assert!(
                !description.is_empty() && description.chars().count() <= 1_600,
                "initial tool contract must be nonempty and bounded: {description}"
            );
        }
        let names = initial
            .into_iter()
            .filter_map(|schema| {
                schema
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<std::collections::BTreeSet<_>>();

        for required in [
            "SOP",
            "Skill",
            "conversation_recall",
            "memory_search",
            "request_user_input",
            "tool_search",
            "repo_map",
            "code_search",
            "code_navigation",
            "failure_context",
            "read_file",
            "workspace_status",
            "bash",
        ] {
            assert!(
                names.contains(required),
                "missing protocol entry {required}"
            );
        }
        assert_eq!(
            names.len(),
            INITIAL_MODEL_TOOL_IDS.len(),
            "the first request must stay bounded to the declared entry surface"
        );
        for deferred in [
            "file_tree",
            "web_search",
            "memory_save",
        ] {
            assert!(
                !names.contains(deferred),
                "management capability should remain discoverable: {deferred}"
            );
        }
    }

    #[test]
    fn skill_reminder_skips_specialized_workflows_for_atomic_direct_tool_tasks() {
        let mut messages = vec![json!({
            "role": "system",
            "content": "NOLA_SKILL_CATALOG_V1\n- organize-files: inventory, classify, rename, move, deduplicate, archive, or quarantine files"
        })];

        super::move_skill_catalog_to_turn_reminder(&mut messages, true);

        let reminder = messages
            .iter()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("system"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("skill decision reminder");
        assert!(reminder.contains("<runtime_state id=\"available-skills-current-step\">"));
        assert!(reminder.contains("do not override policy"));
        assert!(reminder.contains("optional reusable workflows"));
        assert!(reminder.contains("Compose ordinary tools directly"));
        assert!(reminder.contains("does not require activation"));
        assert!(!reminder.contains("mandatory even for a one-shot request"));
        assert!(!reminder.contains("call skill_skip instead"));
    }

    #[test]
    fn provider_call_ids_are_scoped_to_the_model_effect() {
        let first = super::core_scoped_model_call_id("host_effect:17", "call_text_0");
        let repeated_in_same_effect =
            super::core_scoped_model_call_id("host_effect:17", "call_text_0");
        let reused_in_later_effect =
            super::core_scoped_model_call_id("host_effect:42", "call_text_0");

        assert_eq!(first, repeated_in_same_effect);
        assert_ne!(first, reused_in_later_effect);
        assert!(first.ends_with("::call_text_0"));
    }

    #[test]
    fn compaction_prompt_preserves_source_anchors_and_current_evidence_contract() {
        let instructions = nola_core::agent::compaction_instructions(4_096);
        let prompt = prepare_compaction_archive(&[
            json!({"role":"user","content":"Remember HISTORICAL_MARKER_7319"}),
        ])
        .unwrap()
        .prompt;
        assert!(instructions.contains("16384 Unicode characters"));
        assert!(instructions.contains("not provider token usage"));
        assert!(instructions.contains("identifiers and source references exactly"));
        assert!(instructions.contains("canonical user-role messages"));
        assert!(instructions.contains("never user approval"));
        assert!(prompt.contains("HISTORICAL_MARKER_7319"));
        assert!(instructions.contains("Previously committed summary blocks"));
        assert!(instructions.contains("same requirement and artifact version"));
        assert!(instructions.contains("Omit empty headings"));
    }

    #[test]
    fn compacted_history_is_untrusted_user_data_with_canonical_source_anchors() {
        let messages = agent_items_to_llm_messages(&[json!({
            "type": "compaction",
            "id": "checkpoint-part-7",
            "summary": "Keep the original constraint.",
            "source_item_ids": ["message-1", "message-2:body"],
            "source_fingerprint": "abc123",
            "original_token_count": 9001
        })]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        let content = messages[0]["content"].as_str().unwrap();
        assert!(content.contains("<conversation_checkpoint_data>"));
        assert!(content.contains("checkpoint-part-7"));
        assert!(content.contains("not a new user instruction or runtime authority"));
        assert!(content.contains("message-1"));
        assert!(content.contains("message-2:body"));
        assert!(content.contains("abc123"));
        assert!(content.contains("conversation_recall"));

        let many_ids = (0..100)
            .map(|index| Value::String(format!("source-{index}")))
            .collect::<Vec<_>>();
        let bounded = agent_items_to_llm_messages(&[json!({
            "type": "compaction",
            "id": "checkpoint-many",
            "summary": "bounded",
            "source_item_ids": many_ids,
            "source_fingerprint": "def456",
            "original_token_count": 9999
        })]);
        let bounded_content = bounded[0]["content"].as_str().unwrap();
        assert!(bounded_content.contains("source-0"));
        assert!(bounded_content.contains("source-99"));
        assert!(!bounded_content.contains("source-50\""));
        assert!(bounded_content.contains("source_item_ids_omitted\":36"));
    }

    #[test]
    fn compaction_archive_preserves_ten_million_characters_and_tool_protocol() {
        let messages = vec![
            json!({"role":"user","content":"x".repeat(10_000_000)}),
            json!({"role":"assistant","tool_calls":[{"id":"call-1","type":"function",
                "function":{"name":"read_file","arguments":"{\"path\":\"notes.md\"}"}}]}),
            json!({"role":"tool","tool_call_id":"call-1","content":"约束\nKeep exact paths and IDs."}),
        ];
        let started = std::time::Instant::now();
        let archive = prepare_compaction_archive(&messages).unwrap();
        eprintln!(
            "10M compaction archive preparation: {:?}, prompt_bytes={}",
            started.elapsed(),
            archive.prompt.len()
        );
        let source = archive.prompt.strip_prefix("Archive JSON:\n").unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(source).unwrap(),
            messages
        );
        let pointer = archive.prompt.as_ptr();
        let llm_messages = compaction_model_messages("Instructions", archive.prompt);
        assert_eq!(
            llm_messages[1]["content"].as_str().unwrap().as_ptr(),
            pointer
        );
        assert_eq!(llm_messages[0]["content"], "Instructions");
        assert_eq!(llm_messages[1]["role"], "user");
    }

    #[test]
    fn compaction_archive_empty_input_is_explicit_json_not_an_empty_fallback() {
        let archive = prepare_compaction_archive(&[]).unwrap();
        assert_eq!(archive.prompt, "Archive JSON:\n[]");
    }

    #[test]
    fn background_inspection_is_read_only_but_stop_still_requires_permission() {
        let catalog = build_cli_host_catalog("test-host");
        for name in [
            "system_time",
            "bash_background_status",
            "bash_background_wait",
        ] {
            let capability = catalog.capabilities.get(name).unwrap();
            assert_eq!(capability.permission_domain, None, "{name}");
        }
        let stop = catalog.capabilities.get("bash_background_stop").unwrap();
        assert_eq!(stop.permission_domain.as_deref(), Some("bash"));
        #[cfg(unix)]
        {
        let input = catalog.capabilities.get("terminal_input").unwrap();
        assert_eq!(input.permission_domain.as_deref(), Some("bash"));
        assert_eq!(input.input_normalization.permission_contract.risk,
            nola_domain::permission_resource_plan::PermissionOperationRisk::Unknown);
        }
        #[cfg(windows)]
        assert!(!catalog.capabilities.contains_key("terminal_input"));
        #[cfg(windows)]
        assert!(!catalog.capabilities.contains_key("language_service_query"));
        #[cfg(unix)]
        {
        let semantic_query = catalog.capabilities.get("language_service_query").unwrap();
        assert_eq!(semantic_query.permission_domain.as_deref(), Some("bash"));
        assert_eq!(semantic_query.input_normalization.permission_contract.risk,
            nola_domain::permission_resource_plan::PermissionOperationRisk::Unknown);
        }
    }

    #[test]
    fn git_diff_does_not_offer_destructive_evidence_truncation() {
        let catalog = build_cli_host_catalog("test-host");
        let diff = &catalog.capabilities["git_diff"];
        assert!(diff.validate_input(&json!({"root":"/workspace"})).is_ok());
        assert!(diff
            .validate_input(&json!({"root":"/workspace", "max_diff_chars":40000}))
            .is_err());
    }

    #[test]
    fn obsolete_search_and_read_arguments_are_rejected() {
        let catalog = build_cli_host_catalog("test-host");
        let grep = catalog.capabilities.get("grep").unwrap();
        assert!(grep
            .validate_input(&json!({"path":"core/src/lib.rs","query":"cfg(test)"}))
            .is_err());
        let read = catalog.capabilities.get("read_file").unwrap();
        assert!(read
            .validate_input(&json!({"file_path":"core/src/lib.rs","start_line":2,"end_line":8}))
            .is_err());
    }

    #[test]
    fn normalizes_only_current_root_relative_patterns() {
        let catalog = build_cli_host_catalog("test-host");
        let mut grep = serde_json::Map::from_iter([
            ("root".to_string(), Value::String("core".to_string())),
            (
                "pattern".to_string(),
                Value::String("cfg(test)".to_string()),
            ),
            (
                "glob".to_string(),
                Value::String("core/src/**/*.rs".to_string()),
            ),
        ]);
        super::normalize_host_tool_args(
            &catalog.capabilities["grep"].input_normalization,
            &mut grep,
        );
        assert_eq!(grep["root"], "core");
        assert_eq!(grep["pattern"], "cfg(test)");
        assert_eq!(grep["glob"], "src/**/*.rs");

        let mut glob = serde_json::Map::from_iter([
            ("root".to_string(), Value::String("core".to_string())),
            (
                "pattern".to_string(),
                Value::String("core/tests/**/*.rs".to_string()),
            ),
        ]);
        super::normalize_host_tool_args(
            &catalog.capabilities["glob"].input_normalization,
            &mut glob,
        );
        assert_eq!(glob["pattern"], "tests/**/*.rs");
    }

    #[test]
    fn host_metadata_is_not_exposed_to_the_model_schema() {
        let catalog = build_cli_host_catalog("test-host");
        let grep = &catalog.capabilities["grep"];
        assert!(grep
            .input_schema
            .get(crate::tools::TOOL_NORMALIZATION_SCHEMA_KEY)
            .is_none());

        let model_grep = crate::tools::tool_schemas()
            .into_iter()
            .find(|schema| schema.pointer("/function/name").and_then(Value::as_str) == Some("grep"))
            .unwrap();
        assert!(model_grep
            .pointer(&format!(
                "/function/parameters/{}",
                crate::tools::TOOL_NORMALIZATION_SCHEMA_KEY
            ))
            .is_none());
    }

    #[test]
    fn shell_directory_changes_resolve_paths_without_imposing_a_workspace_boundary() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("workspace");
        std::fs::create_dir_all(root.join("src")).unwrap();
        assert!(bash_command_paths_stay_in_allowed_dirs(
            "cd src && cargo test --quiet",
            Some(root.to_string_lossy().as_ref()),
            std::slice::from_ref(&root),
        ));
        // Ordinary access outside the initial directory is not a sensitive
        // resource approval. This is syntax/path admission, not a tool grant.
        assert!(bash_command_paths_stay_in_allowed_dirs(
            "cd .. && cargo test --quiet",
            Some(root.to_string_lossy().as_ref()),
            std::slice::from_ref(&root),
        ));
    }

    #[test]
    fn shell_path_preflight_distinguishes_sensitive_content_from_ordinary_paths() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().to_path_buf();
        std::fs::write(root.join(".env"), "TEST_SECRET=fixture").unwrap();
        assert!(bash_command_paths_stay_in_allowed_dirs(
            "ls missing 2>/dev/null || true",
            Some(root.to_string_lossy().as_ref()),
            std::slice::from_ref(&root),
        ));
        assert!(bash_command_paths_stay_in_allowed_dirs(
            "cat /dev/random",
            Some(root.to_string_lossy().as_ref()),
            std::slice::from_ref(&root),
        ));
        assert!(!bash_command_paths_stay_in_allowed_dirs(
            "cat .env",
            Some(root.to_string_lossy().as_ref()),
            std::slice::from_ref(&root),
        ));
    }

    #[test]
    fn loaded_skill_result_exposes_only_its_typed_resource_directory() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("actual directory");
        let display_only = fixture.path().join("display-only directory");
        let outcome = skill_directory_outcome(
            Some(json!({
                "type": "invoked_skill",
                "resource_directory_uri": url::Url::from_directory_path(&root).unwrap().as_str(),
            })),
            json!({"base_directory": display_only}).to_string(),
        );
        assert_eq!(skill_resource_directory_from_outcome(&outcome), Some(root));
    }

    fn skill_directory_outcome(data: Option<Value>, text: String) -> crate::tools::ToolOutcome {
        crate::tools::ToolOutcome {
            full_output_ref: None,
            data,
            execution: None,
            text,
            display: "Skill displayed directory".to_string(),
            error: false,
            artifacts: Vec::new(),
            activation: None,
        }
    }

    #[test]
    fn skill_resource_directory_rejects_presentation_missing_data_errors_and_non_file_uris() {
        let root = std::env::current_dir().unwrap();
        let uri = url::Url::from_directory_path(&root).unwrap().to_string();
        let displayed = json!({"type":"invoked_skill", "base_directory":root,
            "resource_directory_uri":uri})
        .to_string();
        for data in [
            None,
            Some(Value::Null),
            Some(json!({"type":"invoked_skill", "base_directory":root})),
            Some(
                json!({"type":"invoked_skill", "resource_directory_uri":"https://example.test/resource/"}),
            ),
            Some(json!({"type":"invoked_skill", "resource_directory_uri":"not a URI"})),
            Some(json!({"type":"invoked_sop", "resource_directory_uri":uri})),
        ] {
            let outcome = skill_directory_outcome(data, displayed.clone());
            assert_eq!(skill_resource_directory_from_outcome(&outcome), None);
        }
        let mut errored = skill_directory_outcome(
            Some(json!({
                "type":"invoked_skill", "resource_directory_uri":uri,
            })),
            displayed,
        );
        errored.error = true;
        assert_eq!(skill_resource_directory_from_outcome(&errored), None);
    }

    #[cfg(unix)]
    #[test]
    fn skill_resource_directory_preserves_non_utf8_paths_despite_identical_lossy_display() {
        use std::os::unix::ffi::OsStringExt;
        let fixture = tempfile::tempdir().unwrap();
        let exact = fixture
            .path()
            .join(std::ffi::OsString::from_vec(b"resource-\xff".to_vec()));
        let same_display = fixture.path().join("resource-\u{fffd}");
        assert_ne!(exact, same_display);
        assert_eq!(exact.to_string_lossy(), same_display.to_string_lossy());
        let display = json!({"base_directory":exact.to_string_lossy()}).to_string();
        for root in [&exact, &same_display] {
            let outcome = skill_directory_outcome(
                Some(json!({
                    "type":"invoked_skill",
                    "resource_directory_uri":url::Url::from_directory_path(root).unwrap().as_str(),
                })),
                display.clone(),
            );
            assert_eq!(
                skill_resource_directory_from_outcome(&outcome).as_ref(),
                Some(root)
            );
        }
    }

    #[test]
    fn cli_turn_keeps_the_host_curated_tool_surface() {
        let tools = vec![
            json!({"type":"function","function":{"name":"project_overview"}}),
            json!({"type":"function","function":{"name":"workspace_status"}}),
            json!({"type":"function","function":{"name":"change_review"}}),
            json!({"type":"function","function":{"name":"read_file"}}),
            json!({"type":"function","function":{"name":"bash"}}),
            json!({"type":"function","function":{"name":"file_tree"}}),
            json!({"type":"function","function":{"name":"code_diagnostics"}}),
        ];
        let filtered = tools_for_turn(&tools, &HostTurnPolicy::default());
        let names = filtered
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "project_overview",
                "workspace_status",
                "change_review",
                "read_file",
                "bash",
                "file_tree",
                "code_diagnostics"
            ]
        );
    }

    #[test]
    fn word_turn_uses_the_same_real_host_catalog_as_other_wording() {
        let tools = tool_schemas_for_host();
        let word_messages = vec![json!({
            "role": "user",
            "content": "Create a polished Word project brief from the supplied source and include an accessibility review."
        })];
        let neutral_messages = vec![json!({
            "role": "user",
            "content": "Continue with the task."
        })];
        let word_names = tools_for_model_turn(&tools, &HostTurnPolicy::default(), &word_messages)
            .iter()
            .filter_map(|schema| {
                schema
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();
        let neutral_names =
            tools_for_model_turn(&tools, &HostTurnPolicy::default(), &neutral_messages)
                .iter()
                .filter_map(|schema| {
                    schema
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>();

        assert_eq!(word_names, neutral_names);
        for required in ["Skill", "SOP", "skill_skip", "request_user_input"] {
            assert!(
                word_names.iter().any(|name| name == required),
                "missing {required}: {word_names:?}"
            );
        }
        for direct in ["Skill", "tool_search", "request_user_input"] {
            assert!(
                word_names.iter().any(|name| name == direct),
                "protocol entry tool {direct} must be available beside optional Skills: {word_names:?}"
            );
        }
    }

    #[test]
    fn unified_browser_action_remains_available_to_the_model() {
        let tools = vec![
            json!({"type":"function","function":{"name":"Skill"}}),
            json!({"type":"function","function":{"name":"skill_skip"}}),
            json!({"type":"function","function":{"name":"system_time"}}),
            json!({"type":"function","function":{"name":"web_search"}}),
            json!({"type":"function","function":{"name":"web_fetch"}}),
            json!({"type":"function","function":{"name":"browser_navigate"}}),
        ];
        let selected = tools_for_model_turn(
            &tools,
            &HostTurnPolicy::default(),
            &[
                json!({"role":"user","content":"打开 GitHub 网页"}),
                json!({"role":"tool","name":"skill_skip","content":"{\"decision\":\"none\"}"}),
            ],
        );
        let names = selected
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(names.contains(&"browser_navigate"));
    }

    #[test]
    fn browser_observe_and_act_tools_are_discoverable_unified_contracts() {
        for required in [
            "browser_status",
            "browser_start",
            "browser_snapshot",
            "browser_navigate",
            "browser_click",
            "browser_type_text",
        ] {
            assert!(
                !INITIAL_MODEL_TOOL_IDS.contains(&required),
                "browser observe/act tool must be discovered only when needed: {required}"
            );
        }
        assert!(!INITIAL_MODEL_TOOL_IDS.contains(&"chrome_native_snapshot"));
        assert!(!INITIAL_MODEL_TOOL_IDS.contains(&"chrome_native_type"));
        assert!(!INITIAL_MODEL_TOOL_IDS.contains(&"chrome_native_open_page"));
        assert!(INITIAL_MODEL_TOOL_IDS.contains(&"bash"));
        assert!(!INITIAL_MODEL_TOOL_IDS.contains(&"spawn_agent"));
        assert!(!INITIAL_MODEL_TOOL_IDS.contains(&"goal_complete"));
        assert!(INITIAL_MODEL_TOOL_IDS.contains(&"repo_map"));
        assert!(INITIAL_MODEL_TOOL_IDS.contains(&"code_search"));
        assert!(INITIAL_MODEL_TOOL_IDS.contains(&"edit_file"));
        assert!(INITIAL_MODEL_TOOL_IDS.contains(&"apply_patch"));
        assert!(!INITIAL_MODEL_TOOL_IDS.contains(&"bash_background_status"));
        assert!(!INITIAL_MODEL_TOOL_IDS.contains(&"bash_background_wait"));
        assert!(!INITIAL_MODEL_TOOL_IDS.contains(&"bash_background_stop"));
    }

    #[test]
    fn model_tool_surface_does_not_depend_on_user_wording_or_language() {
        let tools = vec![
            json!({"type":"function","function":{"name":"Skill"}}),
            json!({"type":"function","function":{"name":"skill_skip"}}),
            json!({"type":"function","function":{"name":"tool_search"}}),
            json!({"type":"function","function":{"name":"read_file"}}),
            json!({"type":"function","function":{"name":"write_file"}}),
            json!({"type":"function","function":{"name":"web_search"}}),
            json!({"type":"function","function":{"name":"memory_search"}}),
            json!({"type":"function","function":{"name":"chrome_native_open_page"}}),
        ];
        let policy = HostTurnPolicy::default();
        let chinese = tools_for_model_turn(
            &tools,
            &policy,
            &[json!({"role":"user","content":"检索当前长期记忆"})],
        );
        let japanese = tools_for_model_turn(
            &tools,
            &policy,
            &[json!({"role":"user","content":"最新情報を調べて"})],
        );
        let unrelated = tools_for_model_turn(
            &tools,
            &policy,
            &[json!({"role":"user","content":"continue"})],
        );
        assert_eq!(chinese, japanese);
        assert_eq!(japanese, unrelated);
    }

    #[test]
    fn host_does_not_rebuild_core_tool_projection_from_message_history() {
        let tools = vec![
            json!({"type":"function","function":{"name":"Skill"}}),
            json!({"type":"function","function":{"name":"tool_search"}}),
            json!({"type":"function","function":{"name":"calendar_create"}}),
        ];
        let visible = tools_for_model_turn(
            &tools,
            &HostTurnPolicy::default(),
            &[
                json!({"role":"user","content":"do the next thing"}),
                json!({
                    "role":"tool",
                    "name":"skill_skip",
                    "content":"{\"decision\":\"none\",\"reason\":\"no matching skill\"}"
                }),
                json!({
                    "role":"tool",
                    "name":"tool_search",
                    "content": "{\"display\":\"1 tool match\",\"text\":\"matches: calendar_create\\ntool_search_meta: {\\\"matches\\\":[\\\"calendar_create\\\"]}\"}"
                }),
            ],
        );
        let names = visible
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(names.contains(&"calendar_create"));
    }

    #[test]
    fn host_does_not_remove_mutation_tools_from_the_model_turn() {
        let tools = vec![
            json!({"type":"function","function":{"name":"read_file"}}),
            json!({"type":"function","function":{"name":"web_search"}}),
            json!({"type":"function","function":{"name":"Skill"}}),
            json!({"type":"function","function":{"name":"write_file"}}),
            json!({"type":"function","function":{"name":"apply_patch"}}),
            json!({"type":"function","function":{"name":"bash"}}),
        ];
        let filtered = tools_for_turn(&tools, &HostTurnPolicy::default());
        let names = filtered
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "read_file",
                "web_search",
                "Skill",
                "write_file",
                "apply_patch",
                "bash"
            ]
        );
    }

    #[test]
    fn skill_decision_survives_completed_tool_transport_compaction() {
        let tools = vec![
            json!({"type":"function","function":{"name":"Skill"}}),
            json!({"type":"function","function":{"name":"skill_skip"}}),
            json!({"type":"function","function":{"name":"read_file"}}),
            json!({"type":"function","function":{"name":"write_file"}}),
            json!({"type":"function","function":{"name":"bash"}}),
        ];
        let messages = vec![
            json!({"role":"user","content":"Create the requested draft."}),
            json!({"role":"tool","name":"read_file","content":"source contents"}),
        ];

        let visible = tools_for_model_turn_with_skill_state(
            &tools,
            &HostTurnPolicy::default(),
            &messages,
            true,
        );
        let names = visible
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"bash"));
    }
}

impl CliAgentHostDriver {
    // No access to a driver/Turn cancellation token: this control operation
    // can only address the durable task selected by Core.
    fn spawn_delegated_agent_session(
        &self,
        envelope: HostRequestEnvelope,
        delegated: DelegatedAgentRequestV1,
    ) -> Result<DelegatedAgentWorkerSession, String> {
        let config = self
            .execution_settings
            .clone()
            .ok_or_else(|| "delegated Agent Host configuration is unavailable".to_string())?;
        let (control_tx, control_rx) = std::sync::mpsc::channel();
        let (output_tx, output_rx) = std::sync::mpsc::sync_channel(1);
        let admission_failure = std::sync::Arc::new(std::sync::Mutex::new(None));
        let worker_admission_failure = admission_failure.clone();
        let cancellation = self.tool_context.cancellation.child();
        let worker_cancellation = cancellation.clone();
        let text_part_id = delegated.text_part_id.clone();
        let reasoning_part_id = delegated.reasoning_part_id.clone();
        let backend_id = delegated.backend_id.clone();
        let parent_effect_id = envelope.effect_id.0.clone();
        let tool_context = self.tool_context.clone();
        let invocation_id = envelope
            .delegated_invocation_id
            .clone()
            .ok_or("missing delegated invocation")?;
        let worker = std::thread::spawn(move || {
            let owner_session_id = tool_context.permission_scope.owner_session_id();
            let lease = match crate::task_runtime::ExternalExecutionTaskLease::acquire(
                &config.external,
                owner_session_id,
                &parent_effect_id,
                &backend_id,
                &worker_cancellation,
            ) {
                Ok(lease) => lease,
                Err(error) => {
                    let event = if worker_cancellation.is_requested() {
                        DelegatedAgentEventV1::CancellationConfirmed
                    } else {
                        DelegatedAgentEventV1::Failed {
                            error: HostError {
                                code: "delegated_agent_supervisor_failed".to_string(),
                                message: error,
                                retryable: false,
                                http_status: None,
                            },
                        }
                    };
                    if send_delegated_event(&output_tx, &worker_admission_failure, event).is_err() {
                        return;
                    }
                    if output_tx
                        .send(DelegatedAgentWorkerOutput::Finished)
                        .is_err()
                    {
                        return;
                    }
                    return;
                }
            };
            let mut observer = DelegatedAgentWorkerObserver {
                physical_stop_confirmed: false,
                accounting: None,
                worker_generation: uuid::Uuid::new_v4().to_string(),
                invocation_id,
                pending_delivery: None,
                backend_id: backend_id.clone(),
                backend_session_id: None,
                parent_effect_id,
                text_part_id,
                reasoning_part_id,
                output_tx: output_tx.clone(),
                admission_failure: worker_admission_failure.clone(),
                control_rx,
                tool_context,
                work_step_transactions: BTreeMap::new(),
                work_step_transaction_failures: BTreeMap::new(),
                text: String::new(),
                reasoning: String::new(),
                used_tokens: 0,
                context_window_tokens: 0,
            };
            let authorized_roots=observer.tool_context.allowed_dirs.clone();
            let result = crate::external_executor::run_delegated(
                &config.external,
                &delegated,
                &authorized_roots,
                &mut observer,
                Some(worker_cancellation.clone()),
            );
            if worker_admission_failure
                .lock()
                .map_or(true, |failure| failure.is_some())
            {
                // Application rejected an event. Do not reinterpret that as
                // provider failure or user cancellation; lease Drop records reconciliation.
                return;
            }
            if let Some((envelope, target, _, _, sequence)) = observer.pending_delivery.take() {
                if observer
                    .send_control(
                        &envelope,
                        sequence,
                        HostResponse::DelegatedPermissionInterrupted {
                            delivery_id: target.delivery_id,
                        },
                    )
                    .is_err()
                {
                    return;
                }
            }
            match result {
                Ok(()) => {
                    if let Err(error) = lease.finish_completed() {
                        if send_delegated_event(
                            &output_tx,
                            &worker_admission_failure,
                            DelegatedAgentEventV1::Failed {
                                error: HostError {
                                    code: "delegated_agent_commit_failed".to_string(),
                                    message: error,
                                    retryable: false,
                                    http_status: None,
                                },
                            },
                        )
                        .is_err()
                        {
                            return;
                        }
                    }
                }
                Err(error) => {
                    let cancelled = worker_cancellation.is_requested();
                    if cancelled && !observer.physical_stop_confirmed {
                        // A stop request is not physical ACK. Leave the started
                        // delivery for the existing reconciliation path.
                        if let Ok(mut failure) = worker_admission_failure.lock() {
                            *failure = Some(format!("delegated executor stop unconfirmed: {error}"));
                        }
                        return;
                    }
                    let terminal = if cancelled {
                        lease.finish_cancelled()
                    } else {
                        lease.finish_failed(&error)
                    };
                    let event = if cancelled {
                        DelegatedAgentEventV1::CancellationConfirmed
                    } else {
                        DelegatedAgentEventV1::Failed {
                            error: HostError {
                                code: "delegated_agent_failed".to_string(),
                                message: error,
                                retryable: false,
                                http_status: None,
                            },
                        }
                    };
                    if send_delegated_event(&output_tx, &worker_admission_failure, event).is_err() {
                        return;
                    }
                    if let Err(error) = terminal {
                        if send_delegated_event(
                            &output_tx,
                            &worker_admission_failure,
                            DelegatedAgentEventV1::Failed {
                                error: HostError {
                                    code: "delegated_agent_commit_failed".to_string(),
                                    message: error,
                                    retryable: false,
                                    http_status: None,
                                },
                            },
                        )
                        .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            if output_tx
                .send(DelegatedAgentWorkerOutput::Finished)
                .is_err()
            {
                return;
            }
        });
        Ok(DelegatedAgentWorkerSession {
            effect_id: envelope.effect_id.0.clone(),
            permission_delivery_ids: Default::default(),
            next_sequence: Self::response_sequence(&envelope),
            request: envelope,
            control_tx,
            output_rx,
            cancellation,
            worker: Some(worker),
            admission_failure,
            cancellation_registry: std::sync::Arc::clone(&self.active_effect_cancellations),
        })
    }

    fn drain_delegated_agent_session(
        &mut self,
        effect_id: &str,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut session = self
            .delegated_agent_sessions
            .remove(effect_id)
            .ok_or_else(|| "delegated Agent session is not active".to_string())?;
        loop {
            match session.output_rx.recv() {
                Ok(DelegatedAgentWorkerOutput::Control {
                    envelope,
                    mut sequence,
                    response,
                    acknowledgement,
                }) => {
                    let accepted = Self::emit(&envelope, &mut sequence, response, emit);
                    if let Err(error) = &accepted {
                        // Stop cleanup must be visible before the worker is
                        // released by the rejected commit ACK. Drop is later
                        // and races native protocol cancellation/EOF cleanup.
                        session.cancellation.request();
                        *session
                            .admission_failure
                            .lock()
                            .map_err(|_| "delegated admission state poisoned")? =
                            Some(error.clone());
                    }
                    acknowledgement
                        .send(accepted.clone())
                        .map_err(|_| "delegated control acknowledgement disconnected")?;
                    accepted?;
                }
                Ok(DelegatedAgentWorkerOutput::Event {
                    event,
                    acknowledgement,
                }) => {
                    let accepted = Self::emit(
                        &session.request,
                        &mut session.next_sequence,
                        HostResponse::DelegatedAgentEvent { event },
                        emit,
                    );
                    if let Err(error) = &accepted {
                        // Stop cleanup must be visible before the worker is
                        // released by the rejected commit ACK. Drop is later
                        // and races native protocol cancellation/EOF cleanup.
                        session.cancellation.request();
                        *session
                            .admission_failure
                            .lock()
                            .map_err(|_| "delegated admission state poisoned")? =
                            Some(error.clone());
                    }
                    acknowledgement.send(accepted.clone()).map_err(|_| {
                        "delegated worker stopped before commit acknowledgement".to_string()
                    })?;
                    accepted?;
                }
                Ok(DelegatedAgentWorkerOutput::PhysicalStopObserved) => {
                    // The owned adapter proved its process group exited. A
                    // rejected result event must not erase that physical fact.
                    // This driver is exclusive; a later control sees it only
                    // after the session's Drop/Finished path joins the worker.
                    self.quiescent_effects.insert((session.request.host_id.0.clone(),
                        session.request.session_id.clone(), session.request.turn_id.clone(),
                        session.request.effect_id.0.clone()), ObservedHostStop::from_request(&session.request, None));
                }
                Ok(DelegatedAgentWorkerOutput::Suspended) => {
                    self.insert_delegated_agent_session(effect_id.to_string(), session);
                    return Ok(());
                }
                Ok(DelegatedAgentWorkerOutput::Finished) => {
                    if let Some(worker) = session.worker.take() {
                        worker
                            .join()
                            .map_err(|_| "delegated worker panicked".to_string())?;
                    }
                    return Ok(());
                }
                Err(_) => {
                    let failure = session
                        .admission_failure
                        .lock()
                        .map_err(|_| "delegated admission state poisoned")?
                        .clone()
                        .unwrap_or_else(|| {
                            "delegated worker closed without a committed terminal event".into()
                        });
                    return Err(failure);
                }
            }
        }
    }

    fn dispatch_isolated_tool_batch(
        &mut self,
        requests: Vec<HostRequestEnvelope>,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<bool, String> {
        if requests.len() < 2 {
            return Ok(false);
        }
        let Some(expected_batch) = requests.first().and_then(|request| request.batch.clone())
        else {
            return Ok(false);
        };
        if expected_batch.index != 0 || expected_batch.size != requests.len() {
            return Ok(false);
        }

        let mut jobs = Vec::with_capacity(requests.len());
        for (index, request) in requests.into_iter().enumerate() {
            if request.delivery.is_some() { return Ok(false); }
            let Some(batch) = request.batch.as_ref() else {
                return Ok(false);
            };
            if batch.batch_id != expected_batch.batch_id
                || batch.index != index
                || batch.size != expected_batch.size
            {
                return Ok(false);
            }
            if request.host_id != self.catalog.host_id
                || request.catalog_revision != self.catalog.revision
            {
                return Ok(false);
            }
            let HostRequest::ExecuteTool {
                capability_id,
                call_id,
                input,
                ..
            } = &request.payload
            else {
                return Ok(false);
            };
            let capability_id = capability_id.clone();
            let call_id = call_id.clone();
            if !crate::tools::tool_supports_isolated_parallel_execution(&capability_id) {
                return Ok(false);
            }
            let Some(capability) = self.catalog.resolve_available(&capability_id) else {
                return Ok(false);
            };
            if capability.parallelism != HostCapabilityParallelism::ConcurrentSafe {
                return Ok(false);
            }
            let input_normalization = capability.input_normalization.clone();
            if capability.validate_input(input).is_err() { return Ok(false); }
            let Some(mut args) = input.as_object().cloned() else {
                return Ok(false);
            };
            normalize_host_tool_args(&input_normalization, &mut args);
            let failure_key = deterministic_tool_failure_key(&capability_id, &args);
            if self
                .current_turn_deterministic_tool_failures
                .get(&failure_key)
                .copied()
                .unwrap_or_default()
                >= 3
            {
                return Ok(false);
            }
            let mut context = self.tool_context.clone();
            context.policy = context.policy.without_auto_approval();
            context.cancellation = self.tool_context.cancellation.child();
            if let Ok(mut cancellations) = self.active_effect_cancellations.lock() {
                cancellations.insert(request.effect_id.0.clone(), context.cancellation.context());
            }
            let local_execution_identity = self.fresh_local_execution_identity(&request);
            self.quiescent_effects.remove(&(request.host_id.0.clone(), request.session_id.clone(),
                request.turn_id.clone(), request.effect_id.0.clone()));
            jobs.push(IsolatedToolJob {
                request,
                capability_id,
                call_id,
                args,
                context,
                failure_key,
                permission_contract:input_normalization.permission_contract,
                local_execution_identity,
            });
        }

        for job in &jobs {
            let mut sequence = Self::response_sequence(&job.request);
            Self::emit(
                &job.request,
                &mut sequence,
                HostResponse::ToolCallProgress {
                    call_id: job.call_id.clone(),
                    channel: nola_core::agent::ToolProgressChannel::Status,
                    title: Some(job.capability_id.clone()),
                    detail: Some("running".to_string()),
                    delta: None,
                    metadata: json!({
                        "call_id": job.call_id,
                        "batch_id": expected_batch.batch_id.clone(),
                        "batch_index": job.request.batch.as_ref().map(|batch| batch.index),
                    }),
                },
                emit,
            )?;
        }

        let completions = std::thread::scope(|scope| {
            let handles = jobs
                .into_iter()
                .map(|job| {
                    scope.spawn(move || {
                        let _local_execution = job.local_execution_identity.clone()
                            .map(crate::tools::enter_local_execution_identity);
                        let _permission_scope_guard = job.context.permission_scope.enter();
                        let (permission_profile, checked) = match &job.request.payload {
                            HostRequest::ExecuteTool { permission_profile, input, grant, .. } => (*permission_profile,
                                crate::tools::validate_permission_resource_execution(&job.context,&job.permission_contract,
                                    &job.request.host_id.0,input,grant.as_ref())),
                            _ => return Err("isolated tool job has no admitted permission policy".to_string()),
                        };
                        let expected = match checked {
                            Ok(expected) => expected,
                            Err(error) => return Ok(IsolatedToolCompletion {outcome:crate::tools::permission_resource_execution_failure(error),job}),
                        };
                        let _permission_resource_guard = crate::tools::enter_permission_resource_expectations(expected);
                        let outcome = crate::tools::execute_authorized_tool(
                            &ToolRequest {
                                id: job.call_id.clone(),
                                name: job.capability_id.clone(),
                                args: job.args.clone(),
                            },
                            &job.context,
                            crate::tools::ExecutionAuthority::CoreApproved(permission_profile),
                        );
                        Ok(IsolatedToolCompletion { job, outcome })
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| "isolated tool worker panicked".to_string())
                        .and_then(|result| result)
                })
                .collect::<Result<Vec<_>, _>>()
        })?;

        for completion in completions {
            let IsolatedToolCompletion { job, outcome } = completion;
            let full_output_ref = full_output_ref_from_outcome(&outcome);
            if let Ok(mut cancellations) = self.active_effect_cancellations.lock() {
                cancellations.remove(&job.request.effect_id.0);
            }
            self.record_tool_outcome(&job.failure_key, &outcome);
            let mut sequence = Self::response_sequence(&job.request).saturating_add(1);
            let now_ms = nola_runtime_driver::monotonic_epoch_millis() as u64;
            if job.request.is_expired_at(now_ms) {
                Self::emit_failure(
                    &job.request,
                    &mut sequence,
                    "deadline_exceeded",
                    "Host request exceeded its execution deadline",
                    false,
                    emit,
                )?;
                continue;
            }
            let (artifacts, media_failures) =
                host_artifacts_for_tool(&job.capability_id, &job.args, &outcome, &job.context);
            let mut result = tool_result_contract(&outcome, None);
            annotate_media_publication_failures(&mut result, media_failures);
            let response = HostResponse::ToolCompleted {
                    call_id: job.call_id,
                    output: serde_json::to_value(result)
                        .map_err(|error| error.to_string())?,
                    outcome: if job.context.cancellation.is_requested() {
                        HostToolOutcome::Cancelled
                    } else if outcome.error {
                        HostToolOutcome::Failed
                    } else {
                        successful_tool_completion(&job.capability_id, &outcome)
                    },
                    activation: outcome.activation,
                    full_output_ref,
                    artifacts,
                };
            let observed_stop = terminal_response_current_local_stop(
                &response, job.local_execution_identity.as_ref());
            let accepted = Self::emit(&job.request, &mut sequence, response, emit);
            // These scoped workers have already joined. A current typed OS
            // stop is independent of response delivery, not business success.
            if observed_stop.is_some() && !self.delegated_agent_sessions.contains_key(&job.request.effect_id.0) {
                self.quiescent_effects.insert((job.request.host_id.0.clone(),
                    job.request.session_id.clone(), job.request.turn_id.clone(), job.request.effect_id.0.clone()),
                    ObservedHostStop::from_request(&job.request, observed_stop));
            }
            accepted?;
        }
        Ok(true)
    }
}

impl AgentHostDriver for CliAgentHostDriver {
    type Protocol = nola_core::agent::host::AgentRuntimeProtocol;
    type Error = String;

    fn boundary_submission_source(
        &self,
    ) -> Option<std::sync::Arc<dyn nola_core::agent::host::AgentBoundarySubmissionSource<nola_core::agent::host::AgentBoundaryInput>>> {
        Some(std::sync::Arc::new(CliBoundarySubmissionSource {
            cancellation: self.tool_context.cancellation.clone(),
            session_id: self.boundary_session_id.clone(),
            run_id: self.bound_dispatch_run_id.clone(),
            durable_lease_owner: format!(
                "boundary-consumer:{}:{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ),
            durable_state: std::sync::Arc::clone(&self.boundary_submission_state),
        }))
    }

    fn dispatch(
        &mut self,
        request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), Self::Error> {
        let completed_identity = (
            request.host_id.0.clone(), request.session_id.clone(),
            request.turn_id.clone(), request.effect_id.0.clone(),
        );
        let is_cancellation = matches!(&request.payload, HostRequest::Cancel { .. } | HostRequest::CancelTask { .. });
        if !is_cancellation { self.quiescent_effects.remove(&completed_identity); }
        let mut terminal_accepted = false;
        let mut terminal_stop_observed = false;
        let mut permission_wait_returned = false;
        let mut model_worker_returned = false;
        let mut locally_not_started = false;
        let expected_local_execution = self.fresh_local_execution_identity(&request);
        let mut owned_local_stop_observed = None;
        let ordinary_investigation = self.allows_absent_tool_stop_receipt(&request);
        let admitted_not_started = self.has_admitted_not_started_target(&request);
        let result = (|| {
        let mut observe_completion = |response: HostResponseEnvelope| {
            let matching_terminal = response.effect_id == request.effect_id
                && response.request_id == request.request_id && response.payload.is_terminal();
            let completed = matching_terminal && terminal_response_proves_local_stop(
                &response.payload, ordinary_investigation, expected_local_execution.as_ref());
            if matching_terminal {
                owned_local_stop_observed = terminal_response_current_local_stop(
                    &response.payload, expected_local_execution.as_ref()).or(owned_local_stop_observed.take());
            }
            // Physical completion survives a rejected canonical response (for
            // example a concurrent Interrupt). It is not a success/commit ACK.
            terminal_stop_observed |= completed;
            let accepted = emit(response);
            if matching_terminal { terminal_accepted = completed && accepted.is_ok(); }
            accepted
        };
        let emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String> = &mut observe_completion;
        let _local_execution = expected_local_execution.clone()
            .map(crate::tools::enter_local_execution_identity);
        let _execution_scope_guard = self.tool_context.cancellation.enter();
        let _permission_scope_guard = self.tool_context.permission_scope.enter();
        if request.host_id != self.catalog.host_id {
            return Err("Core selected a different Host".to_string());
        }
        if !self.accepts_catalog_revision(request.catalog_revision, request.availability_epoch,
            matches!(&request.payload, HostRequest::ModelStream { .. })) {
            return Err(format!("Core used a stale Host capability catalog (request={}, host={}, effect={})",
                request.catalog_revision, self.catalog.revision, request.effect_id.0));
        }
        self.validate_effect_delivery_request(&request)?;
        // Revalidate live Host authority at the side-effect boundary, not only
        // when a Run is created/restored. Cancellation must remain available
        // after revocation so Core can settle its durable execution journal.
        if !matches!(
            &request.payload,
            HostRequest::Cancel { .. } | HostRequest::CancelTask { .. }
        ) {
            if let Some(config) = self.execution_settings.as_ref() {
                let workspace_authority=config.validate_workspace_authority().and_then(|()| {
                    if let HostRequest::DelegatedAgent {request:delegated}=&request.payload {
                        if self.tool_context.policy.process_sandbox().is_some() {
                            return Err("Required native shell sandbox cannot cover an external execution unit; execution was not started".into());
                        }
                        if let Some(primary)=delegated.settings.get("primary_worktree") {
                            let primary=serde_json::from_value(primary.clone()).map_err(|_|"Invalid primary workspace binding")?;
                            crate::tools::primary_execution_root(&primary,&config.external.project_root,&self.tool_context.allowed_dirs)?;
                        }
                    }
                    Ok(())
                });
                let unavailable=workspace_authority.err().map(|message|("task_workspace_unavailable",message))
                    .or_else(||config.validate_memory_authority().err().map(|message|("memory_authority_unavailable",message)));
                if let Some((code,message)) = unavailable {
                    locally_not_started = true;
                    let mut sequence = Self::response_sequence(&request);
                    if request.delivery.is_some() {
                        Self::emit(
                            &request,
                            &mut sequence,
                            HostResponse::ExecutionRejected {
                                code: code.into(),
                                message,
                                retryable: false,
                            },
                            emit,
                        )?;
                    } else {
                        Self::emit_failure(
                            &request,
                            &mut sequence,
                            code,
                            &message,
                            false,
                            emit,
                        )?;
                    }
                    return Ok(());
                }
            }
        }
        // Core creates execution deadlines from the admitted runtime time.
        // Comparing them to a freshly sampled wall clock reintroduces the
        // clock-step expiry bug even when delivery heartbeats are healthy.
        let now_ms = nola_runtime_driver::monotonic_epoch_millis() as u64;
        if request.is_expired_at(now_ms) {
            locally_not_started = true;
            let mut sequence = Self::response_sequence(&request);
            if request.delivery.is_some() {
                Self::emit(
                    &request,
                    &mut sequence,
                    HostResponse::ExecutionRejected {
                        code: "deadline_exceeded".into(),
                        message: "Host request expired before execution started".into(),
                        retryable: false,
                    },
                    emit,
                )?;
            } else {
                Self::emit_failure(
                    &request,
                    &mut sequence,
                    "deadline_exceeded",
                    "Host request expired before execution started",
                    false,
                    emit,
                )?;
            }
            return Ok(());
        }
        let has_effect_delivery = request.delivery.is_some();
        // Validate local stop/ownership before consuming the control's durable
        // start barrier. A missing proof must not strand an already-started
        // Cancel. A live delegated worker is stopped and joined below, after
        // that barrier; registry emptiness alone never proves termination.
        if let HostRequest::Cancel { target_effect_id, .. } = &request.payload {
            let target_identity = (request.host_id.0.clone(), request.session_id.clone(),
                request.turn_id.clone(), target_effect_id.0.clone());
            let owned_worker = self.delegated_agent_sessions.get(&target_effect_id.0)
                .is_some_and(|session| session.request.host_id == request.host_id
                    && session.request.session_id == request.session_id
                    && session.request.turn_id == request.turn_id
                    && session.worker.is_some());
            let local_stop = self.quiescent_effects.get(&target_identity)
                .is_some_and(|witness| witness.matches_owner(&target_identity))
                && !self.delegated_agent_sessions.contains_key(&target_effect_id.0)
                && !self.active_effect_cancellations.lock()
                    .map_err(|_| "effect cancellation registry poisoned")?
                    .contains_key(&target_effect_id.0);
            if local_stop {
                let witness = self.quiescent_effects.get(&target_identity).expect("checked stop witness");
                if witness.requires_current_target_validation {
                    (self.cancellation_stop_validator)(
                        self.boundary_session_id.as_deref().ok_or("cancellation conversation binding missing")?,
                        self.bound_dispatch_run_id.as_deref().ok_or("cancellation Run binding missing")?,
                        &request, &witness.request_id, witness.delivery.as_ref())?;
                }
            }
            let owned_code_worker = self.code_cells.values().any(|cell|
                cell.request.effect_id == *target_effect_id && cell.validate_control(&request).is_ok());
            if !local_stop && !owned_worker && !owned_code_worker && !admitted_not_started {
                let recovered = (|| {
                    let conversation = self.boundary_session_id.as_deref()
                        .ok_or("cancellation conversation binding missing")?;
                    let run = self.bound_dispatch_run_id.as_deref()
                        .ok_or("cancellation Run binding missing")?;
                    crate::core_agent_runtime::observe_recovered_cancellation_target(conversation, run, &request)
                })();
                let target = recovered.map_err(|error: String| format!(
                    "cancellation target stop is unproven in this Host; durable owner reconciliation is required: {error}"))?;
                self.quiescent_effects.insert(target_identity,
                    ObservedHostStop::from_request(&target, None));
            }
        }
        self.start_effect_delivery(&request, emit)?;
        // Existing execution helpers derive their first sequence from the
        // durable request cursor. A fenced dispatch has already consumed that
        // first sequence for ExecutionStarted, so shift every later response
        // without changing resume semantics observed by the payload handler.
        let mut emit_after_start = |mut response: HostResponseEnvelope| {
            if has_effect_delivery {
                response.sequence = response.sequence.saturating_add(1);
            }
            emit(response)
        };
        let emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String> =
            &mut emit_after_start;
        match request.payload.clone() {
            HostRequest::ReadCodeCellHistory { .. } => {
                return Err("historical code cell observation belongs to Application, not Host".into());
            },
            HostRequest::PollCodeCell { cell } => {
                let worker = self.code_cells.get(&cell.cell_id)
                    .ok_or("code cell worker unavailable; source replay is forbidden")?;
                worker.validate_control(&request)?;
                if worker.worker.owner() != &cell { return Err("code poll owner mismatch".into()); }
                self.drain_code_cell_until(&cell.cell_id,
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(1)), true, emit)?;
                self.settle_code_observations(&cell.cell_id, emit)?;
                let mut sequence = Self::response_sequence(&request);
                Self::emit(&request, &mut sequence, HostResponse::Completed {
                    output:serde_json::json!({"cell":cell}), usage:None,
                    full_output_ref:None, artifacts:Vec::new(),
                }, emit)?;
            },
            HostRequest::StartCodeCell { identity, limits, initial_wait_ms, source, tools } => {
                self.start_code_cell(&request, identity, limits, initial_wait_ms, source, tools, emit)?;
            },
            HostRequest::ObserveCodeCell { call_id, observation } => {
                self.observe_code_cell(&request, call_id, observation, emit)?;
            },
            HostRequest::ResolveCodeTool { result } => {
                let cell_id = result.cell.cell_id.clone();
                let cell = self.code_cells.get_mut(&cell_id).ok_or("code cell worker unavailable; source replay is forbidden")?;
                cell.validate_control(&request)?;
                let nested_sequence = result.sequence;
                let acknowledgement = cell.worker.resolve(result)?;
                cell.awaiting.remove(&nested_sequence);
                // Publish immediately available output/child callbacks before
                // acknowledging the result-delivery control. Its ACK may wake
                // the next model step, which must not precede these observations.
                self.drain_code_cell(&cell_id, emit)?;
                self.settle_code_observations(&cell_id, emit)?;
                let mut sequence = Self::response_sequence(&request);
                Self::emit(&request, &mut sequence, HostResponse::Completed {
                    output: acknowledgement, usage: None, full_output_ref: None, artifacts: Vec::new(),
                }, emit)?;
            },
            HostRequest::ResolvePermissionResources { call_id, capability_id, input, request: resolution } => {
                let mut sequence = Self::response_sequence(&request);
                let result = (|| {
                    let capability = self.catalog.resolve_available(&capability_id).ok_or("resolution capability is unavailable")?;
                    capability.validate_input(&input)?;
                    let plan = capability.input_normalization.permission_contract.resource_plan_for(&input)
                        .ok_or("capability has no resource resolution contract")?;
                    if resolution.execution_target != request.host_id.0 || resolution.capability != plan.capability
                        || resolution.operation != plan.operation
                        || resolution.demands != plan.extract(&input).map_err(|error|error.to_string())? {
                        return Err("permission resource resolution contract mismatch".to_string());
                    }
                    crate::tools::resolve_permission_resources(&self.tool_context, &resolution, &input)
                })();
                match result {
                    Ok(facts) => Self::emit(&request, &mut sequence, HostResponse::PermissionResourcesResolved {call_id,facts}, emit)?,
                    Err(error) => Self::emit_failure(&request,&mut sequence,"permission_resource_resolution_failed",error,false,emit)?,
                }
            }
            HostRequest::RunLifecycleHook { event } => {
                let mut sequence = Self::response_sequence(&request);
                if event.name != nola_core::lifecycle_hooks::LifecycleHookEventName::SessionEnd
                    || event.session_id != request.session_id
                    || event.turn_id != request.turn_id
                {
                    Self::emit_failure(
                        &request,
                        &mut sequence,
                        "invalid_lifecycle_event",
                        "Lifecycle event identity does not match the Core request",
                        false,
                        emit,
                    )?;
                    return Ok(());
                }
                self.run_lifecycle_hooks(
                    &request,
                    &mut sequence,
                    event.name,
                    Some(&event.id),
                    event.tool_name.as_deref(),
                    event.tool_call_id.as_deref(),
                    true,
                    emit,
                )?;
                Self::emit(
                    &request,
                    &mut sequence,
                    HostResponse::Completed {
                        output: json!({"lifecycle": "session_end"}),
                        usage: None,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    },
                    emit,
                )?;
            }
            HostRequest::CancelTask { .. } => {
                return Err("Task cancellation must execute through its committed Application control port".into());
            }
            HostRequest::RefreshCapabilityCatalog { .. } => {
                match self.refresh_mcp_capability_catalog() {
                    Ok(catalog) => emit(capability_catalog_refresh_response(&request, catalog, self.catalog.clone()))?,
                    Err(error) => {
                        let mut sequence = Self::response_sequence(&request);
                        Self::emit_failure(
                            &request,
                            &mut sequence,
                            "capability_catalog_refresh_failed",
                            &error,
                            true,
                            emit,
                        )?;
                    }
                }
            }
            HostRequest::ModelStream {
                model_id,
                text_part_id,
                reasoning_part_id,
                response_mode,
                messages,
                tools,
                settings,
            } => {
                let step_context = validate_model_tool_projection(&settings, &tools)?;
                let tool_projection = step_context.tool_projection;
                let active_deferred_capability_ids = tool_projection
                    .capability_ids
                    .iter()
                    .filter(|capability_id| {
                        self.catalog
                            .capabilities
                            .get(*capability_id)
                            .is_some_and(|capability| {
                                capability.route == HostCapabilityRoute::Tool
                                    && capability.exposure == HostCapabilityExposure::Deferred
                            })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let deferred_available_count = self
                    .catalog
                    .capabilities
                    .values()
                    .filter(|capability| {
                        capability.route == HostCapabilityRoute::Tool
                            && capability.availability == CapabilityAvailability::Available
                            && capability.exposure == HostCapabilityExposure::Deferred
                    })
                    .count();
                crate::llm::log_local_debug_payload(
                    "model_tool_projection",
                    &json!({
                        "type": "model_tool_projection",
                        "catalog_revision": self.catalog.revision,
                        "availability_epoch": self.catalog.availability_epoch,
                        "projection_revision": tool_projection.revision,
                        "projection_digest": tool_projection.schema_digest,
                        "active_tool_count": tool_projection.capability_ids.len(),
                        "deferred_available_count": deferred_available_count,
                        "active_deferred_capability_ids": active_deferred_capability_ids,
                    }),
                );
                let invocation_plan = settings
                    .get("model_invocation_plan")
                    .cloned()
                    .and_then(|value| serde_json::from_value::<ModelInvocationPlan>(value).ok())
                    .unwrap_or_default();
                let model_result = self.execute_model(
                    &request,
                    &model_id,
                    &text_part_id,
                    &reasoning_part_id,
                    &messages,
                    &tools,
                    &invocation_plan,
                    settings
                        .pointer("/step_context/model_prefix_digest")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    settings
                        .pointer("/step_context/turn_context/stable_prompt_fragment_digests")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok())
                        .unwrap_or_default(),
                    settings
                        .get("previous_prompt_cache_identity")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok()),
                    response_mode,
                    settings
                        .get("defer_text_presentation")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    emit,
                );
                // execute_model is synchronous and its cancellation registry
                // guard has been dropped on return, including admission error.
                // This proves only this local stream worker returned, not that
                // a remote provider or arbitrary external tool was rolled back.
                model_worker_returned = true;
                model_result?;
            }
            HostRequest::DelegatedAgent { request: delegated } => {
                if request
                    .delegated_invocation_id
                    .as_deref()
                    .is_none_or(|id| id.trim().is_empty())
                {
                    return Err(
                        "delegated execution requires a committed invocation identity".into(),
                    );
                }
                if request.attempt_id.is_some() {
                    return Err("delegated execution cannot claim a model Attempt identity".into());
                }
                delegated.validate().map_err(str::to_string)?;
                let effect_id = request.effect_id.0.clone();
                if let Some(session) = self.delegated_agent_sessions.get(&effect_id) {
                    if session.request.delegated_invocation_id != request.delegated_invocation_id
                        || session.request.request_id != request.request_id
                        || session.request.session_id != request.session_id
                        || session.request.turn_id != request.turn_id
                        || session.request.host_id != request.host_id
                        || session.request.lease_id != request.lease_id
                        || session.request.catalog_revision != request.catalog_revision
                        || session.request.availability_epoch != request.availability_epoch
                        || session.request.payload != request.payload
                    {
                        return Err("delegated warm worker identity mismatch".into());
                    }
                } else {
                    let mut sequence = Self::response_sequence(&request);
                    if request.resume_after_sequence.is_some() {
                        Self::emit(&request, &mut sequence, HostResponse::DelegatedExecutionInterrupted {
                            reason: nola_host_protocol::DelegatedExecutionInterruptionReason::WorkerUnavailable,
                        }, emit)?;
                        return Ok(());
                    }
                    // ExecutionStarted is the single durable admission for a
                    // fenced request. Emitting the legacy Accepted event as
                    // well would admit the same delegated invocation twice.
                    if !has_effect_delivery {
                        Self::emit(&request, &mut sequence, HostResponse::Accepted, emit)?;
                    }
                    let mut accepted_request = request.clone();
                    accepted_request.resume_after_sequence = sequence.checked_sub(1);
                    let session =
                        self.spawn_delegated_agent_session(accepted_request, delegated)?;
                    self.insert_delegated_agent_session(effect_id.clone(), session);
                }
                self.drain_delegated_agent_session(&effect_id, emit)?;
            }
            HostRequest::ExecuteTool {
                capability_id,
                call_id,
                mut input,
                grant: _,
                permission_instructions: _,
                permission_profile: _,
            } => {
                let invocation = match validate_and_strip_sop_invocation_context(
                    &mut input,
                    &self.tool_context,
                    &request.session_id,
                ) {
                    Ok(invocation) => invocation,
                    Err(error) => {
                        let mut sequence = Self::response_sequence(&request);
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
                self.execute_tool_with_sop(
                    &request,
                    &capability_id,
                    &call_id,
                    &input,
                    invocation,
                    emit,
                )?;
            }
            permission @ HostRequest::RequestPermission { .. } => {
                self.dispatch_permission_request(&request, permission, &mut permission_wait_returned, emit)?;
            }
            HostRequest::ResolveDelegatedPermission {
                target_effect_id,
                backend_id,
                backend_session_id,
                permission_request_id,
                resolution,
                occurrence,
                delivery,
                permission_instructions: _,
            } => {
                let target =
                    delivery.ok_or("delegated permission delivery is not canonically prepared")?;
                let decision = match resolution {
                    HostPermissionResolution::Authorization { decision, .. } => decision,
                    HostPermissionResolution::Intervention { .. } => {
                        return Err("delegated permission delivery requires authorization".into())
                    }
                };
                let mut sequence = Self::response_sequence(&request);
                let Some(session) = self.delegated_agent_sessions.get_mut(&target_effect_id.0)
                else {
                    Self::emit(
                        &request,
                        &mut sequence,
                        HostResponse::DelegatedPermissionInterrupted {
                            delivery_id: target.delivery_id,
                        },
                        emit,
                    )?;
                    return Ok(());
                };
                if request.resume_after_sequence.is_some() {
                    Self::emit(
                        &request,
                        &mut sequence,
                        HostResponse::DelegatedPermissionInterrupted {
                            delivery_id: target.delivery_id,
                        },
                        emit,
                    )?;
                    return Ok(());
                }
                if session.request.delegated_invocation_id.as_deref()
                    != Some(target.invocation_id.as_str())
                {
                    return Err("delegated permission invocation mismatch".into());
                }
                if !session
                    .permission_delivery_ids
                    .insert(target.delivery_id.clone())
                {
                    return Err(
                        "delegated permission delivery has already been submitted to this worker"
                            .into(),
                    );
                }
                session
                    .control_tx
                    .send(DelegatedAgentWorkerControl::ResolvePermission {
                        request_id: permission_request_id,
                        decision,
                        envelope: request.clone(),
                        target,
                        occurrence,
                        backend_id,
                        backend_session_id,
                    })
                    .map_err(|_| "delegated permission channel closed")?;
                self.drain_delegated_agent_session(&target_effect_id.0, emit)?;
            }
            HostRequest::FetchContext {
                source_id,
                query,
                freshness_deadline_ms: _,
            } => {
                let mut sequence = Self::response_sequence(&request);
                match (self.context_handler)(&request, &source_id, &query) {
                    Ok(components) => Self::emit(
                        &request,
                        &mut sequence,
                        HostResponse::ContextFetched { components },
                        emit,
                    )?,
                    Err(error) => Self::emit_failure(
                        &request,
                        &mut sequence,
                        "context_fetch_failed",
                        error,
                        true,
                        emit,
                    )?,
                }
            }
            HostRequest::Compact {
                context_window_tokens: _,
                strategy: _,
                instructions,
                messages,
                target_tokens,
            } => {
                self.execute_compaction(&request, &instructions, &messages, target_tokens, emit)?
            }
            HostRequest::Cancel { target_effect_id, .. } => {
                let delegated_effect_id = target_effect_id.0.clone();
                let target_identity = (request.host_id.0.clone(), request.session_id.clone(),
                    request.turn_id.clone(), delegated_effect_id.clone());
                if let Some(witness) = self.stop_code_cell(&request, &delegated_effect_id)? {
                    self.quiescent_effects.insert(target_identity.clone(), witness);
                }
                let stopped_delegated = if let Some(session) =
                    self.delegated_agent_sessions.get(&delegated_effect_id)
                {
                    if session.request.host_id != request.host_id
                        || session.request.session_id != request.session_id
                        || session.request.turn_id != request.turn_id {
                        return Err("cancellation target belongs to a different Host owner".into());
                    }
                    if session.worker.is_none() {
                        return Err("delegated cancellation has no owned worker handle to confirm".into());
                    }
                    session.cancellation.request();
                    let _ = session.control_tx.send(DelegatedAgentWorkerControl::Cancel);
                    Some(ObservedHostStop::from_request(&session.request, None))
                } else {
                    None
                };
                if let Some(cancellation) = self
                    .active_effect_cancellations
                    .lock()
                    .ok()
                    .and_then(|cancellations| cancellations.get(&target_effect_id.0).cloned())
                {
                    cancellation.request();
                }
                if let Some(witness) = stopped_delegated {
                    self.drain_delegated_agent_session(&delegated_effect_id, emit)?;
                    if self.delegated_agent_sessions.contains_key(&delegated_effect_id) {
                        return Err("delegated cancellation is still suspended; worker stop is unconfirmed".into());
                    }
                    self.quiescent_effects.insert(target_identity.clone(), witness);
                }
                if !admitted_not_started && !self.quiescent_effects.get(&target_identity)
                    .is_some_and(|witness| witness.matches_owner(&target_identity)) {
                    return Err("cancellation target stop is unproven in this Host; durable owner reconciliation is required".into());
                }
                let mut sequence = Self::response_sequence(&request);
                Self::emit(
                    &request,
                    &mut sequence,
                    HostResponse::CancellationConfirmed {
                        target_effect_id: target_effect_id.clone(),
                    },
                    emit,
                )?;
            }
        }
        Ok(())
        })();
        let local_stop_observed = terminal_stop_observed || locally_not_started || permission_wait_returned || owned_local_stop_observed.is_some()
            || model_worker_returned;
        if ((result.is_ok() && terminal_accepted) || local_stop_observed) && !is_cancellation
            && !self.delegated_agent_sessions.contains_key(&completed_identity.3)
            && self.active_effect_cancellations.lock()
                .map_err(|_| "effect cancellation registry poisoned")?
                .get(&completed_identity.3).is_none() {
            self.quiescent_effects.insert(completed_identity,
                ObservedHostStop::from_request(&request, owned_local_stop_observed));
        }
        result
    }

    fn dispatch_batch(
        &mut self,
        requests: Vec<HostRequestEnvelope>,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), Self::Error> {
        let mut pending = std::collections::VecDeque::from(requests);
        while let Some(request) = pending.pop_front() {
            // Execution cancellation stops model/tool data-plane work, but it
            // must never stop the control-plane requests that settle that
            // cancellation. Otherwise Core durably emits `host.cancel` while
            // Host refuses to acknowledge it, leaving a terminal Turn with a
            // permanently live effect journal.
            let is_terminal_control_request = matches!(
                &request.payload,
                HostRequest::Cancel { .. } | HostRequest::CancelTask { .. }
            );
            if self.tool_context.cancellation.is_requested() && !is_terminal_control_request {
                self.record_queued_not_started(&request)?;
                continue;
            }
            // Isolated batching bypasses `dispatch`, so fenced effects use the
            // ordinary path where each execution-start barrier is committed
            // synchronously before its tool begins.
            // Configured Runs also require the live authority gate in dispatch.
            if request.delivery.is_some() || self.execution_settings.is_some() {
                self.dispatch(request, emit)?;
                continue;
            }
            let Some(batch) = request.batch.clone() else {
                self.dispatch(request, emit)?;
                continue;
            };
            let mut grouped = vec![request];
            while grouped.len() < batch.size {
                let Some(next) = pending.front() else {
                    break;
                };
                if next
                    .batch
                    .as_ref()
                    .is_some_and(|identity| identity.batch_id == batch.batch_id)
                {
                    grouped.push(pending.pop_front().expect("front request exists"));
                } else {
                    break;
                }
            }
            if !self.dispatch_isolated_tool_batch(grouped.clone(), emit)? {
                for request in grouped {
                    self.dispatch(request, emit)?;
                }
            }
        }
        Ok(())
    }
}
