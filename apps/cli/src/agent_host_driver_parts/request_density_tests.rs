// SPDX-License-Identifier: Apache-2.0
// Current catalog-density and authority-invariance regressions.
#[cfg(test)]
mod request_density_tests {
    use super::*;
    use nola_domain::reasoning_tiers::ReasoningTier;

    #[test]
    fn product_catalog_contains_recovery_without_benchmark_configuration() {
        let catalog = build_cli_host_catalog("ordinary-product-host");
        let probe = &catalog.capabilities["environment_probe"];
        assert_eq!(probe.exposure, HostCapabilityExposure::Visible);
        assert!(probe.input_schema.to_string().contains("executables"));
        let read = &catalog.capabilities["read_file"];
        assert!(read.input_schema.to_string().contains("pdf_page"));
        let defaults = crate::runtime_config::RuntimeConfig::default();
        // The isolated evaluator's installation grant must not become a
        // product default on a user's machine.
        assert!(defaults.instructions.is_empty());
        assert_ne!(defaults.model_capabilities.native_pdf_input, Some(true));
        let policy = nola_core::agent::builtin_agent_policy_fragments();
        assert!(policy.iter().any(|fragment| fragment.id == "nola.core.environment-recovery"));
    }

    #[test]
    fn tier_changes_initial_surface_without_removing_discovery_or_authority_metadata() {
        let base = build_cli_host_catalog("tier-test");
        let mut counts = Vec::new();
        for tier in [ReasoningTier::Concise, ReasoningTier::Balanced, ReasoningTier::Detailed] {
            let mut catalog = base.clone();
            apply_initial_tool_tier(&mut catalog, tier);
            assert_eq!(catalog.capabilities.len(), base.capabilities.len());
            for (id, capability) in &catalog.capabilities {
                let original = &base.capabilities[id];
                assert_eq!(capability.availability, original.availability);
                assert_eq!(capability.input_schema, original.input_schema);
                assert_eq!(capability.input_normalization, original.input_normalization);
                assert_eq!(capability.permission_domain, original.permission_domain);
                assert_eq!(capability.exposure.is_model_discoverable(), original.exposure.is_model_discoverable());
            }
            assert_eq!(catalog.capabilities["tool_search"].exposure, HostCapabilityExposure::Visible);
            counts.push(catalog.capabilities.values().filter(|c| c.exposure.is_model_visible()).count());
        }
        eprintln!("initial catalog tool counts: {counts:?}");
        assert!(counts[0] < counts[1] && counts[1] < counts[2]);
    }

    #[test]
    fn concise_keeps_deferred_mutation_tools_discoverable_not_granted() {
        for name in ["write_file", "bash", "web_search", "spawn_agent"] {
            assert_eq!(initial_tool_exposure_for_tier(name, ReasoningTier::Concise), HostCapabilityExposure::Deferred);
        }
        // No prompt, language, or inferred task category enters this policy.
        assert_eq!(initial_tool_exposure_for_tier("unknown_tool", ReasoningTier::Detailed), HostCapabilityExposure::Deferred);
    }

    #[test]
    fn registered_ui_publisher_stays_visible_in_every_tier_without_changing_authority() {
        let base = build_cli_host_catalog("ui-publisher");
        assert!(!base.capabilities.contains_key("render_ui"));
        let descriptor = nola_core::agent::generative_ui::capability_descriptor();
        for tier in [ReasoningTier::Concise, ReasoningTier::Balanced, ReasoningTier::Detailed] {
            let mut catalog = base.clone();
            apply_initial_tool_tier(&mut catalog, tier);
            assert!(!catalog.capabilities.contains_key("render_ui"));
            catalog.capabilities.insert(descriptor.id.clone(), descriptor.clone());
            apply_initial_tool_tier(&mut catalog, tier);
            assert_eq!(catalog.capabilities["render_ui"], descriptor);
        }
    }

    #[test]
    fn balanced_exposes_narrow_file_mutations_alongside_shell() {
        let base = build_cli_host_catalog("foundational-file-operations");
        let mut catalog = base.clone();
        apply_initial_tool_tier(&mut catalog, ReasoningTier::Balanced);
        for name in ["read_file", "write_file", "edit_file", "apply_patch", "bash"] {
            let capability = &catalog.capabilities[name];
            assert_eq!(capability.exposure, HostCapabilityExposure::Visible);
            assert_eq!(capability.permission_domain, base.capabilities[name].permission_domain);
            assert_eq!(capability.input_schema, base.capabilities[name].input_schema);
        }
    }
}
