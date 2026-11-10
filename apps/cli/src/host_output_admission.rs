// SPDX-License-Identifier: Apache-2.0
//! Admission observes provider facts once; execution and recovery only consume
//! the resulting non-secret, route-bound snapshot.

use crate::model_invocation_settings::ModelInvocationSettings;
use crate::model_route::route_binding;
use crate::context_budget::{provider_context_limits, ContextBudget, ProviderContextLimits};
use nola_domain::generation::{resolve_output_limit, OutputTokenLimits, OutputTokenPreference};
use nola_host_protocol::{HostModelRouteBinding, HostOutputAdmission, HostRouteTokenLimits};

pub(crate) fn primary_route_binding(config: &ModelInvocationSettings) -> Result<HostModelRouteBinding, String> {
    Ok(config.primary().clone())
}

fn candidates(config: &ModelInvocationSettings) -> Vec<(Option<&str>, &str, &str, &str)> {
    let mut routes = vec![(
        config.options().provider.as_deref(),
        config.endpoint().as_str(),
        config.model().as_str(),
        config.api_key().as_str(),
    )];
    if crate::llm::supports_provider_fallbacks(
        config.options().provider.as_deref(),
        config.options().reasoning_selection.is_some(),
    ) {
        routes.extend(config.options().llm_fallbacks.iter().map(|fallback| {
            (
                fallback.provider.as_deref(),
                fallback.endpoint.as_str(),
                fallback.model.as_deref().unwrap_or(config.model()),
                fallback.api_key.as_deref().unwrap_or(config.api_key()),
            )
        }));
    }
    routes
}

fn preference(config: &ModelInvocationSettings) -> OutputTokenPreference {
    config.options()
        .max_output_tokens
        .map(OutputTokenPreference::Explicit)
        .unwrap_or(OutputTokenPreference::Default)
}

/// Passing false is only for callers constructing an uncommitted binding with
/// no provider discovery. Production acquisition always admits with discovery.
pub(crate) fn admit(config: &ModelInvocationSettings, probe: bool) -> Result<HostOutputAdmission, String> {
    if let Some(snapshot) = &config.options().output_admission {
        validate_snapshot(config, snapshot)?;
        return Ok(snapshot.clone());
    }
    resolve_output_limit(
        preference(config),
        OutputTokenLimits {
            known_output_cap: None,
        },
    )
    .map_err(|error| error.to_string())?;
    if config.options().llm_fallbacks.len() > 16 {
        return Err("Host output admission supports at most 16 fallback routes".into());
    }
    // Validate every candidate identity before the first metadata request.
    let mut validated_routes = Vec::new();
    for (index, (provider, endpoint, model, key)) in candidates(config).into_iter().enumerate() {
        let route = if index == 0 { primary_route_binding(config)? } else {
            route_binding(provider, endpoint, model, key, config.options().chat_output_token_field)?
        };
        if validated_routes.contains(&route) {
            return Err("Host output admission contains a duplicate route".into());
        }
        validated_routes.push(route);
    }
    let routes = candidates(config)
        .into_iter()
        .enumerate()
        .map(|(index, (provider, endpoint, model, key))| {
            let route = validated_routes[index].clone();
            let limits = if matches!(route, HostModelRouteBinding::External { .. }) || config.options().siwc.is_some() { None }
                else { provider_context_limits(endpoint, model, key, provider, probe) };
            Ok(HostRouteTokenLimits {
                route,
                max_input_tokens: limits.and_then(|v| v.max_input_tokens).map(|v| v as u64),
                max_output_tokens: limits.and_then(|v| v.max_output_tokens).map(|v| v as u64),
                max_total_tokens: limits.and_then(|v| v.max_total_tokens).map(|v| v as u64),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let configured_window = crate::context_budget::configured_context_window_tokens(&config.options().compaction)
        .map(|value| value as u64);
    let admission = HostOutputAdmission::admit_with_window(routes, preference(config), configured_window)
        .map_err(|error| error.to_string())?;
    if let Some(selection) = &config.options().reasoning_selection {
        selection.validate(admission.resolved.tokens)?;
    }
    Ok(admission)
}

pub(crate) fn validate_snapshot(
    config: &ModelInvocationSettings,
    snapshot: &HostOutputAdmission,
) -> Result<(), String> {
    snapshot.validate().map_err(str::to_string)?;
    if let Some(selection) = &config.options().reasoning_selection {
        selection.validate(snapshot.resolved.tokens)?;
    }
    let expected = candidates(config)
        .into_iter()
        .enumerate()
        .map(|(index, (p, e, m, k))| if index == 0 { primary_route_binding(config) }
            else { route_binding(p, e, m, k, config.options().chat_output_token_field) })
        .collect::<Result<Vec<_>, _>>()?;
    if snapshot.version != nola_host_protocol::HOST_OUTPUT_ADMISSION_VERSION
        || snapshot.configured_context_window != crate::context_budget::configured_context_window_tokens(&config.options().compaction).map(|value| value as u64)
        || snapshot.preference != preference(config)
        || snapshot.routes.iter().map(|r| &r.route).ne(expected.iter())
    {
        return Err(
            "Host invocation unavailable: output admission does not match the frozen model candidates or output preference"
                .into(),
        );
    }
    // Shared validation above owns resolved-limit/source consistency. This
    // layer only compares the executable route set and frozen preferences.
    Ok(())
}

pub(crate) fn admitted_output_tokens(config: &ModelInvocationSettings) -> Result<u64, String> {
    Ok(admit(config, false)?.resolved.tokens)
}

pub(crate) fn admitted_fallbacks(
    config: &ModelInvocationSettings,
) -> Result<Vec<crate::llm::LlmProviderTarget>, String> {
    let admission = admit(config, false)?;
    config.options()
        .llm_fallbacks
        .iter()
        .filter_map(|fallback| {
            let identity = route_binding(
                fallback.provider.as_deref(),
                &fallback.endpoint,
                fallback.model.as_deref().unwrap_or(config.model()),
                fallback.api_key.as_deref().unwrap_or(config.api_key()),
                config.options().chat_output_token_field,
            );
            match identity {
                Err(error) => Some(Err(error)),
                Ok(identity)
                    if admission.routes[1..]
                        .iter()
                        .any(|candidate| candidate.route == identity) =>
                {
                    Some(Ok(crate::llm::LlmProviderTarget {
                        provider: fallback.provider.clone(),
                        endpoint: fallback.endpoint.clone(),
                        api_key: fallback.api_key.clone(),
                        model: fallback.model.clone(),
                    }))
                }
                Ok(_) => None,
            }
        })
        .collect()
}

pub(crate) fn context_budget(config: &ModelInvocationSettings) -> Result<ContextBudget, String> {
    let snapshot = admit(config, false)?;
    let primary = snapshot
        .routes
        .first()
        .ok_or("Host admission has no primary route")?;
    let limits = ProviderContextLimits {
        max_input_tokens: primary.max_input_tokens.map(|v| v as usize),
        max_output_tokens: primary.max_output_tokens.map(|v| v as usize),
        max_total_tokens: primary.max_total_tokens.map(|v| v as usize),
    };
    let budget = ContextBudget::from_frozen_provider_limits(
        &config.options().compaction,
        config.swe_mode(),
        Some(limits),
        snapshot.resolved.tokens,
    );
    validate_window_allocation(budget.context_window_tokens, snapshot.resolved.tokens)?;
    Ok(budget)
}

fn validate_window_allocation(window: Option<usize>, output: u64) -> Result<(), String> {
    let Some(window) = window else { return Ok(()); };
    let margin = (window / 20).clamp(512, 8_192);
    let available = u64::try_from(window.saturating_sub(margin)).unwrap_or(u64::MAX);
    if output > available || available - output < 512 {
        return Err(format!(
            "context_output_budget_conflict: context_window={window}, admitted_output_tokens={output}, safety_margin={margin}; fewer than 512 input tokens remain. Adjust the context window or the model output preference before starting the task."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn small_window_default_is_frozen_and_reused_by_context_planning() {
        let _guard = crate::store::test_env_lock();
        let mut config = native_config();
        config.runtime.compaction.context_window_tokens = Some(16_000);
        config.runtime.max_output_tokens = None;
        let admitted = admit(&config, false).unwrap();
        assert_eq!(admitted.resolved.tokens, 5_600);
        assert_eq!(admitted.configured_context_window, Some(16_000));
        config.runtime.output_admission = Some(admitted);
        assert_eq!(admitted_output_tokens(&config).unwrap(), 5_600);
        let budget = context_budget(&config).unwrap();
        assert_eq!(budget.reserved_output_tokens, 5_600);
        assert_eq!(budget.prompt_token_limit, 9_600);
        config.runtime.compaction.context_window_tokens = Some(32_000);
        assert!(admit(&config, false).is_err());
    }
    #[test]
    fn incompatible_output_is_rejected_before_compaction_without_clamping() {
        let error = super::validate_window_allocation(Some(16_000), 16_384).unwrap_err();
        assert!(error.contains("context_output_budget_conflict"));
        assert!(error.contains("admitted_output_tokens=16384"));
        assert!(error.contains("safety_margin=800"));
        assert!(super::validate_window_allocation(Some(16_000), 15_200).is_err());
        assert!(super::validate_window_allocation(Some(16_000), 14_689).is_err());
        assert!(super::validate_window_allocation(Some(16_000), 14_688).is_ok());
        assert!(super::validate_window_allocation(Some(16_000), 4_096).is_ok());
        assert!(super::validate_window_allocation(Some(262_144), 16_384).is_ok());
    }
    use crate::config::Config;
    fn admit(config: &Config, probe: bool) -> Result<HostOutputAdmission, String> {
        super::admit(&config.model_invocation_settings()?, probe)
    }
    fn admitted_output_tokens(config: &Config) -> Result<u64, String> {
        super::admitted_output_tokens(&config.model_invocation_settings()?)
    }
    fn context_budget(config: &Config) -> Result<ContextBudget, String> {
        super::context_budget(&config.model_invocation_settings()?)
    }

    fn native_config() -> crate::config::Config {
        let mut config = crate::config::Config::for_test().unwrap();
        config.endpoint = "http://127.0.0.1:32101/v1".into();
        config.api_key = "ephemeral-native-secret".into();
        config.model = "fixture-native-model".into();
        config.runtime.provider = Some("openai-compatible".into());
        config.runtime.native_model_route = Some(serde_json::from_value(serde_json::json!({
            "adapter": "mlx", "adapter_contract_revision": 1, "model_id": config.model,
            "profile_ref": {"profile_id": "native-profile", "admitted_revision": 8}
        })).unwrap());
        config
    }

    #[test]
    fn native_channel_restart_keeps_logical_binding_and_frozen_admission() {
        let _guard = crate::store::test_env_lock();
        let mut config = native_config().host_invocation_request().unwrap();
        config.runtime.llm_fallbacks.push(crate::runtime_config::RuntimeLlmProvider {
            provider: Some("openai-compatible".into()), endpoint: "https://fixture.invalid/v1".into(),
            api_key: Some("fallback-secret".into()), model: Some("fallback-model".into()),
        });
        let original = crate::core_agent_runtime::fixture_host_invocation_binding(&mut config).unwrap();
        config.runtime.output_admission = Some(original.output_admission.clone());
        config.model_settings.endpoint = "http://127.0.0.1:32102/v1".into();
        config.model_settings.api_key = "replacement-native-secret".into();
        let restarted = crate::core_agent_runtime::fixture_host_invocation_binding(&mut config).unwrap();
        original.require_exact(&restarted).unwrap();
        assert!(matches!(restarted.primary, HostModelRouteBinding::Native(_)));
        assert!(matches!(restarted.fallbacks[0], HostModelRouteBinding::Http(_)));
        let persisted = serde_json::to_string(&restarted).unwrap();
        assert!(!persisted.contains("32102"));
        assert!(!persisted.contains("replacement-native-secret"));
        config.runtime.native_model_route.as_mut().unwrap().profile_ref.as_mut().unwrap().admitted_revision += 1;
        assert!(super::validate_snapshot(&config.model_invocation_settings().unwrap(), &original.output_admission).is_err());
    }

    #[test]
    fn native_identity_cannot_relabel_a_remote_or_mismatched_channel() {
        let config = native_config();
        let route = config.runtime.native_model_route.as_ref().unwrap();
        for endpoint in ["https://fixture.invalid/v1", "http://localhost:123/v1",
            "http://127.0.0.1/v1?key=x", "http://user@127.0.0.1/v1", "http://127.0.0.1/v1#x"] {
            assert!(crate::model_route::validate_native_transport(route, endpoint, &config.model, &config.api_key).is_err());
        }
        assert!(crate::model_route::validate_native_transport(route, &config.endpoint, "other", &config.api_key).is_err());
        assert!(crate::model_route::validate_native_transport(route, &config.endpoint, &config.model, "").is_err());
        let mut persisted = serde_json::to_value(crate::runtime_config::RuntimeConfig::default()).unwrap();
        persisted["native_model_route"] = serde_json::to_value(route).unwrap();
        let runtime: crate::runtime_config::RuntimeConfig = serde_json::from_value(persisted).unwrap();
        assert!(runtime.native_model_route.is_none());
    }

    #[test]
    fn runtime_secret_material_is_ephemeral_not_a_managed_reference() {
        use nola_host_protocol::{HostModelRouteBinding, HostSecretSource};
        for (endpoint, secret, credential_source, query_source, has_query) in [
            ("https://fixture.invalid/v1", "", HostSecretSource::None, HostSecretSource::None, false),
            ("https://fixture.invalid/v1", "fixture-secret", HostSecretSource::Ephemeral, HostSecretSource::None, false),
            ("https://fixture.invalid/v1?token=private-query", "fixture-secret", HostSecretSource::Ephemeral, HostSecretSource::Ephemeral, true),
            // Preserve the original URL fact: an explicit empty query differs
            // from no query and has the digest of an empty byte sequence.
            ("https://fixture.invalid/v1?", "", HostSecretSource::None, HostSecretSource::Ephemeral, true),
        ] {
            let route = super::route_binding(Some("openai"), endpoint, "fixture", secret,
                nola_application::ChatOutputTokenField::Auto).unwrap();
            route.validate().unwrap();
            let HostModelRouteBinding::Http(http) = &route else { panic!("expected HTTP"); };
            assert_eq!(http.credential_source, credential_source);
            assert_eq!(http.endpoint_query_source, query_source);
            assert_eq!(http.endpoint_query_sha256.is_some(), has_query);
            assert!(!http.endpoint.contains('?'));
            let persisted = serde_json::to_string(&route).unwrap();
            assert!(!persisted.contains("fixture-secret"));
            assert!(!persisted.contains("private-query"));
            assert!(!persisted.contains("managed"));
        }
        let without = super::route_binding(Some("openai"), "https://fixture.invalid/v1", "fixture", "",
            nola_application::ChatOutputTokenField::Auto).unwrap();
        let empty = super::route_binding(Some("openai"), "https://fixture.invalid/v1?", "fixture", "",
            nola_application::ChatOutputTokenField::Auto).unwrap();
        assert_ne!(without, empty);
    }

    #[test]
    fn frozen_output_field_resolves_preference_without_losing_explicit_choice() {
        use nola_application::ChatOutputTokenField as Preference;
        use nola_host_protocol::ModelOutputTokenField as Field;
        for (endpoint, preference, expected) in [
            ("https://api.openai.com/v1", Preference::Auto, Field::MaxCompletionTokens),
            ("https://fixture.invalid/v1", Preference::Auto, Field::MaxTokens),
            ("https://api.openai.com/v1", Preference::MaxTokens, Field::MaxTokens),
            ("https://fixture.invalid/v1", Preference::MaxCompletionTokens, Field::MaxCompletionTokens),
        ] {
            let binding = super::route_binding(Some("openai"), endpoint, "fixture", "fixture-key", preference).unwrap();
            let HostModelRouteBinding::Http(binding) = binding else { panic!("HTTP fixture") };
            assert_eq!(binding.output_token_field, expected);
        }
    }
    #[test]
    fn frozen_route_uses_the_same_protocol_resolver_as_physical_dispatch() {
        use nola_domain::generation::ModelProtocol;
        for (provider, endpoint, expected) in [
            (Some(" OpenAI-Responses "), "https://api.anthropic.com/v1", ModelProtocol::Responses),
            (Some("openai"), "https://api.anthropic.com/v1", ModelProtocol::ChatCompletions),
            (None, "https://api.anthropic.com/v1", ModelProtocol::Anthropic),
            (None, "https://fixture.invalid/v1?next=api.anthropic.com", ModelProtocol::ChatCompletions),
        ] {
            let binding = super::route_binding(provider, endpoint, "fixture", "fixture-key", nola_application::ChatOutputTokenField::Auto).unwrap();
            let HostModelRouteBinding::Http(binding) = binding else { panic!("HTTP fixture") };
            assert_eq!(binding.protocol, expected);
            assert_eq!(binding.protocol, crate::llm::resolved_model_protocol(provider, endpoint));
            assert!(!binding.endpoint.contains('?'));
        }
    }
    use super::*;
    use nola_domain::generation::OutputTokenLimitSource;

    fn frozen(config: &Config, caps: &[Option<u64>]) -> HostOutputAdmission {
        let mut snapshot = admit(config, false).unwrap();
        assert_eq!(snapshot.routes.len(), caps.len());
        for (route, cap) in snapshot.routes.iter_mut().zip(caps) {
            route.max_output_tokens = *cap;
        }
        HostOutputAdmission::admit(snapshot.routes, snapshot.preference).unwrap()
    }

    #[test]
    fn default_is_resolved_once_and_context_planning_cannot_rewrite_it() {
        let mut config = Config::for_test().unwrap();
        config.runtime.max_output_tokens = None;
        config.runtime.output_admission = Some(frozen(&config, &[Some(2_048)]));
        let snapshot = admit(&config, true).unwrap();
        assert_eq!(snapshot.resolved.tokens, 2_048);
        assert_eq!(
            snapshot.resolved.source,
            OutputTokenLimitSource::KnownCapabilityDefault
        );
        assert_eq!(admitted_output_tokens(&config).unwrap(), 2_048);
        assert_eq!(
            context_budget(&config)
                .unwrap()
                .effective_output_budget
                .requested_tokens,
            2_048
        );
        let encoded = serde_json::to_string(&snapshot).unwrap();
        config.runtime.output_admission = Some(serde_json::from_str(&encoded).unwrap());
        assert_eq!(admit(&config, true).unwrap(), snapshot);
    }

    #[test]
    fn changed_route_or_preference_cannot_reuse_a_frozen_admission() {
        let mut config = Config::for_test().unwrap();
        config.runtime.max_output_tokens = Some(1_024);
        config.runtime.output_admission = Some(frozen(&config, &[Some(2_048)]));
        config.runtime.max_output_tokens = Some(4_096);
        assert!(admit(&config, false).is_err());
        config.runtime.max_output_tokens = Some(1_024);
        config.model.push_str("-different");
        assert!(admit(&config, false).is_err());
    }

    #[test]
    fn shared_constructor_owns_provenance_and_recovery_rejects_forged_source() {
        let mut config = Config::for_test().unwrap();
        config.runtime.max_output_tokens = Some(1_024);
        let admitted = admit(&config, false).unwrap();
        assert_eq!(admitted, HostOutputAdmission::admit(
            admitted.routes.clone(), OutputTokenPreference::Explicit(1_024),
        ).unwrap());
        assert_eq!(admitted.resolved.source, OutputTokenLimitSource::ExplicitPreference);
        let mut forged = admitted;
        // The tokens still match: a changed provenance is independently invalid.
        forged.resolved.source = OutputTokenLimitSource::ProductDefault;
        config.runtime.output_admission = Some(forged);
        assert!(admit(&config, false).is_err());
    }

    #[test]
    fn fallback_caps_participate_only_when_dispatch_can_select_them() {
        let mut config = Config::for_test().unwrap();
        config.runtime.max_output_tokens = None;
        config
            .runtime
            .llm_fallbacks
            .push(crate::runtime_config::RuntimeLlmProvider {
                provider: Some("openai".into()),
                endpoint: "https://fallback.invalid/v1".into(),
                api_key: None,
                model: Some("fallback-model".into()),
            });
        config.runtime.output_admission = Some(frozen(&config, &[Some(8_192), Some(1_024)]));
        assert_eq!(admitted_output_tokens(&config).unwrap(), 1_024);
        config.runtime.output_admission = None;
        config.runtime.provider = Some("openai-responses".into());
        assert_eq!(admit(&config, false).unwrap().routes.len(), 1);
        config.runtime.provider = Some("openai".into());
        config.runtime.reasoning_selection =
            Some(nola_core::agent::reasoning::ReasoningSelection::Effort {
                value: "high".into(),
            });
        assert_eq!(admit(&config, false).unwrap().routes.len(), 1);
    }

    #[test]
    fn serialized_runtime_settings_cannot_supply_provider_admission_evidence() {
        let mut config = Config::for_test().unwrap();
        config.runtime.output_admission = Some(admit(&config, false).unwrap());
        let encoded = serde_json::to_value(&config.runtime).unwrap();
        assert!(encoded.get("output_admission").is_some());
        let parsed: crate::runtime_config::RuntimeConfig = serde_json::from_value(encoded).unwrap();
        assert!(parsed.output_admission.is_none());
    }

    #[test]
    fn capability_default_cannot_make_selected_reasoning_budget_invalid() {
        let mut config = Config::for_test().unwrap();
        config.runtime.max_output_tokens = None;
        config.runtime.output_admission = Some(frozen(&config, &[Some(2_048)]));
        config.runtime.reasoning_selection =
            Some(nola_core::agent::reasoning::ReasoningSelection::Budget { tokens: 4_096 });
        assert!(admit(&config, false).is_err());
    }

    #[test]
    fn forged_or_over_cap_snapshot_is_rejected_before_execution() {
        let mut config = Config::for_test().unwrap();
        config.runtime.max_output_tokens = Some(4_096);
        let mut snapshot = admit(&config, false).unwrap();
        snapshot.routes[0].max_output_tokens = Some(2_048);
        config.runtime.output_admission = Some(snapshot);
        assert!(admit(&config, false).is_err());
        config.runtime.output_admission.as_mut().unwrap().routes[0].max_output_tokens = Some(0);
        assert!(admit(&config, false).is_err());
    }

    #[test]
    fn provider_metadata_admits_default_and_rejects_explicit_before_model_http() {
        use std::io::{Read, Write};
        use std::time::{Duration, Instant};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    other => panic!("metadata request not received: {other:?}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 2048];
            while !request.windows(4).any(|value| value == b"\r\n\r\n") {
                let size = stream.read(&mut bytes).unwrap();
                assert_ne!(size, 0);
                request.extend_from_slice(&bytes[..size]);
            }
            assert!(String::from_utf8_lossy(&request).starts_with("GET /v1/models "));
            let body = r#"{"data":[{"id":"bounded-model","max_output_tokens":2048,"context_length":32768}]}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let mut config = Config::for_test().unwrap();
        config.endpoint = endpoint;
        config.model = "bounded-model".into();
        config.runtime.provider = Some("openai".into());
        config.runtime.max_output_tokens = None;
        let snapshot = admit(&config, true).unwrap();
        server.join().unwrap();
        assert_eq!(snapshot.resolved.tokens, 2_048);
        assert_eq!(
            snapshot.resolved.source,
            OutputTokenLimitSource::KnownCapabilityDefault
        );
        config.runtime.max_output_tokens = Some(4_096);
        assert!(admit(&config, true).is_err());
        config.runtime.max_output_tokens = None;
        config.runtime.output_admission = Some(snapshot.clone());
        assert_eq!(admit(&config, true).unwrap(), snapshot);
    }
}
