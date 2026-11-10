// SPDX-License-Identifier: Apache-2.0
// Test modules extracted from tool_projection.rs; included at the same module scope.
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
