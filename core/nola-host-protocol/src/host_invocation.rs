// SPDX-License-Identifier: Apache-2.0
//! Non-secret, immutable Host execution identity committed with a Run.
use nola_domain::generation::{
    resolve_output_limit_with_window, OutputTokenLimits, OutputTokenPreference, ResolvedOutputLimit,
};
use serde::{Deserialize, Serialize};
use crate::{HostSecretPurpose, HostSecretSource, EMPTY_CREDENTIAL_SHA256};

// Version 8 adds an explicit owned SIWC account binding. Managed references are
// structural identity only: cold reconstruction still needs a trusted local
// resolver with live revocation checks. Ephemeral secrets cannot be rebuilt
// from this snapshot alone; a Host may independently obtain and verify them.
pub const HOST_INVOCATION_BINDING_VERSION: u32 = 1;
pub const HOST_OUTPUT_ADMISSION_VERSION: u32 = 1;

/// Opaque Host-private reconstruction material, authenticated by the owning
/// canonical Run configuration. The reference itself grants no authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRunRecoveryReference {
    pub adapter: String,
    pub handle: String,
    pub sha256: String,
}

/// Resolved wire field, not a user preference such as Chat's `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelOutputTokenField { MaxTokens, MaxCompletionTokens, MaxOutputTokens, Omitted }

impl ModelOutputTokenField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MaxTokens => "max_tokens",
            Self::MaxCompletionTokens => "max_completion_tokens",
            Self::MaxOutputTokens => "max_output_tokens",
            Self::Omitted => "omitted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSiwcAccountBinding {
    pub account_id: String,
    pub account_revision: i64,
    pub credential_ref: String,
    pub client_id: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostHttpModelRouteBinding {
    pub siwc_account: Option<HostSiwcAccountBinding>,
    /// Exact wire adapter selected at admission, not inferred during recovery.
    pub protocol: nola_domain::generation::ModelProtocol,
    pub output_token_field: ModelOutputTokenField,
    pub provider: Option<String>,
    /// Normalized endpoint without query/user-info/fragment. Query contents are
    /// represented only by a digest because providers may place secrets there.
    pub endpoint: String,
    pub endpoint_query_sha256: Option<String>,
    pub endpoint_query_source: HostSecretSource,
    pub model: String,
    /// A comparison identity, never a credential usable for authentication.
    pub credential_sha256: String,
    pub credential_source: HostSecretSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostModelRouteBinding {
    Http(HostHttpModelRouteBinding),
    Native(HostNativeModelRouteBinding),
    /// The executor owns its model transport and authentication.
    External { backend: String, model: Option<String>, provider: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostNativeModelAdapter { Mlx, LlamaCpp }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostModelProfileReference {
    pub profile_id: String,
    pub admitted_revision: i64,
}

/// Logical model selection, deliberately not a model weights/content snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostNativeModelRouteBinding {
    pub adapter: HostNativeModelAdapter,
    pub adapter_contract_revision: u32,
    pub model_id: String,
    #[serde(deserialize_with = "required_profile_reference")]
    pub profile_ref: Option<HostModelProfileReference>,
}

fn required_profile_reference<'de, D: serde::Deserializer<'de>>(deserializer: D)
    -> Result<Option<HostModelProfileReference>, D::Error> {
    Option::deserialize(deserializer)
}

impl HostModelRouteBinding {
    pub fn validate(&self) -> Result<(), &'static str> {
        fn identity(value: &str) -> bool {
            !value.trim().is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
        }
        fn digest(value: &str) -> bool {
            value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }
        let route = match self {
            Self::External { backend, model, provider } => {
                if !identity(backend) || backend == "nola" || model.as_ref().is_some_and(|v| !identity(v))
                    || provider.as_ref().is_some_and(|v| !identity(v)) {
                    return Err("invalid external executor identity");
                }
                return Ok(());
            }
            Self::Native(native) => {
                if native.adapter_contract_revision != 1 || !identity(&native.model_id)
                    || native.profile_ref.as_ref().is_some_and(|profile|
                        !identity(&profile.profile_id) || profile.admitted_revision <= 0) {
                    return Err("invalid native Host model identity");
                }
                return Ok(());
            }
            Self::Http(http) => http,
        };
        use nola_domain::generation::ModelProtocol;
        let valid_field = match route.protocol {
            ModelProtocol::Responses => route.output_token_field == ModelOutputTokenField::MaxOutputTokens,
            ModelProtocol::SiwcResponses => route.output_token_field == ModelOutputTokenField::Omitted,
            ModelProtocol::Anthropic => route.output_token_field == ModelOutputTokenField::MaxTokens,
            ModelProtocol::ChatCompletions => matches!(route.output_token_field, ModelOutputTokenField::MaxTokens | ModelOutputTokenField::MaxCompletionTokens),
        };
        if !valid_field { return Err("Host output token field does not match its protocol"); }
        if let Some(account) = &route.siwc_account {
            if route.protocol != ModelProtocol::SiwcResponses || account.account_revision < 1
                || !identity(&account.account_id) || !identity(&account.client_id) || !identity(&account.subject)
                || account.credential_ref != format!("siwc:{}", account.account_id)
                || route.credential_source != HostSecretSource::None
                || route.credential_sha256 != EMPTY_CREDENTIAL_SHA256
                || route.endpoint_query_sha256.is_some()
                || route.endpoint != "https://api.openai.com/v1"
            { return Err("invalid SIWC account route binding"); }
        } else if route.protocol == ModelProtocol::SiwcResponses {
            return Err("SIWC route requires an exact account binding");
        }
        route.credential_source.validate(HostSecretPurpose::ModelCredential)?;
        route.endpoint_query_source.validate(HostSecretPurpose::EndpointQuery)?;
        if matches!(route.credential_source, HostSecretSource::None)
            != (route.credential_sha256 == EMPTY_CREDENTIAL_SHA256)
            || matches!(route.endpoint_query_source, HostSecretSource::None)
                != route.endpoint_query_sha256.is_none()
        {
            return Err("Host secret provenance does not match its digest");
        }
        if route.endpoint.is_empty() {
            if route.endpoint_query_sha256.is_some() { return Err("Host route query identity requires an endpoint"); }
        } else {
            let endpoint = url::Url::parse(&route.endpoint).map_err(|_| "invalid Host endpoint identity")?;
            if !matches!(endpoint.scheme(), "http" | "https") || endpoint.host_str().is_none()
                || !endpoint.username().is_empty() || endpoint.password().is_some()
                || endpoint.query().is_some() || endpoint.fragment().is_some()
                || endpoint.as_str() != route.endpoint {
                return Err("Host endpoint identity must be normalized and non-secret");
            }
        }
        if route.endpoint.len() > 4096 || route.model.len() > 1024
            || route.provider.as_ref().is_some_and(|v| v.len() > 128)
            || !digest(&route.credential_sha256)
            || route.endpoint_query_sha256.as_ref().is_some_and(|v| !digest(v)) {
            return Err("invalid Host route identity");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostGenerationBinding {
    pub max_retries: u32,
    #[serde(deserialize_with = "required_parallel_tool_calls")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(deserialize_with = "required_reasoning_selection")]
    pub reasoning_selection: Option<nola_domain::generation::ReasoningSelection>,
    pub reasoning_tier: nola_domain::reasoning_tiers::ReasoningTier,
    pub sampling_override: bool,
    pub generation_options: nola_domain::generation::ValidatedGenerationOptions,
    pub temperature: serde_json::Number,
    pub top_p: serde_json::Number,
    pub max_output_tokens: u64,
    pub thinking_enabled: bool,
    #[serde(default)]
    pub reasoning_summary_supported: Option<bool>,
    pub prompt_cache: bool,
    pub prompt_cache_key_sha256: Option<String>,
    pub prompt_cache_key_prefix_sha256: Option<String>,
    pub prompt_cache_retention: Option<String>,
}

fn required_reasoning_selection<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<nola_domain::generation::ReasoningSelection>, D::Error> {
    Option::deserialize(deserializer)
}

fn required_parallel_tool_calls<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<bool>, D::Error> {
    Option::deserialize(deserializer)
}

/// Observations captured for an exact non-secret route identity. Output caps
/// are admission evidence; input/total fields currently feed context planning
/// only and may originate from legacy context hints. Reservation estimates
/// themselves must never be stored as declared output caps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRouteTokenLimits {
    pub route: HostModelRouteBinding,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub max_total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostOutputAdmission {
    pub version: u32,
    pub configured_context_window: Option<u64>,
    /// Primary first, then executable fallbacks in their execution order.
    pub routes: Vec<HostRouteTokenLimits>,
    pub preference: OutputTokenPreference,
    pub resolved: ResolvedOutputLimit,
}

impl HostOutputAdmission {
    /// Admit already-observed, exact route facts without transport or IO.
    /// Callers provide preferences/facts, never a guessed resolution source or
    /// schema version. Missing caps stay unknown; an explicit zero is invalid.
    pub fn admit(
        routes: Vec<HostRouteTokenLimits>,
        preference: OutputTokenPreference,
    ) -> Result<Self, &'static str> {
        Self::admit_with_window(routes, preference, None)
    }

    pub fn admit_with_window(
        routes: Vec<HostRouteTokenLimits>,
        preference: OutputTokenPreference,
        configured_context_window: Option<u64>,
    ) -> Result<Self, &'static str> {
        let window = configured_context_window.into_iter()
            .chain(routes.iter().filter_map(|route| route.max_total_tokens)).min();
        let resolved = resolve_output_limit_with_window(preference, OutputTokenLimits {
            known_output_cap: routes.iter().filter_map(|route| route.max_output_tokens).min(),
        }, window).map_err(|_| "invalid Host output admission preference, capability or context window")?;
        let admission = Self {
            version: HOST_OUTPUT_ADMISSION_VERSION,
            configured_context_window,
            routes,
            preference,
            resolved,
        };
        admission.validate()?;
        Ok(admission)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != HOST_OUTPUT_ADMISSION_VERSION
            || self.routes.is_empty()
            || self.routes.len() > 17
        {
            return Err("unsupported Host output admission");
        }
        for (index, candidate) in self.routes.iter().enumerate() {
            candidate.route.validate()?;
            if self.routes[..index]
                .iter()
                .any(|previous| previous.route == candidate.route)
            {
                return Err("duplicate Host output admission route");
            }
        }
        let limits = OutputTokenLimits {
            known_output_cap: self
                .routes
                .iter()
                .filter_map(|candidate| candidate.max_output_tokens)
                .min(),
        };
        let window = self.configured_context_window.into_iter()
            .chain(self.routes.iter().filter_map(|route| route.max_total_tokens)).min();
        let resolved = resolve_output_limit_with_window(self.preference, limits, window)
            .map_err(|_| "invalid Host output admission preference or capability")?;
        if self.resolved != resolved {
            return Err("Host output admission does not match its frozen facts");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostInvocationBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_launch: Option<crate::HostMacLaunchDescriptor>,
    pub version: u32,
    pub primary: HostModelRouteBinding,
    pub fallbacks: Vec<HostModelRouteBinding>,
    pub generation: HostGenerationBinding,
    pub output_admission: HostOutputAdmission,
    /// Host-side capability constraints that are not represented by a model
    /// route: workspace, allowed paths, backend selection and tool policy.
    pub execution_policy_sha256: String,
}

impl HostInvocationBinding {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(launch) = &self.mac_launch { launch.validate()?; }
        fn digest(value: &str) -> bool {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }
        if self.version != HOST_INVOCATION_BINDING_VERSION || self.fallbacks.len() > 16 {
            return Err("unsupported Host invocation binding");
        }
        if !digest(&self.execution_policy_sha256) {
            return Err("invalid Host policy identity");
        }
        self.output_admission.validate()?;
        if self.output_admission.routes[0].route != self.primary {
            return Err("Host output admission primary route mismatch");
        }
        let mut remaining_fallbacks = self.fallbacks.as_slice();
        for candidate in &self.output_admission.routes[1..] {
            let index = remaining_fallbacks
                .iter()
                .position(|route| route == &candidate.route)
                .ok_or("Host output admission fallback route mismatch")?;
            remaining_fallbacks = &remaining_fallbacks[index + 1..];
        }
        if self.generation.max_output_tokens != self.output_admission.resolved.tokens {
            return Err("Host generation differs from admitted output limit");
        }
        for route in std::iter::once(&self.primary).chain(&self.fallbacks) {
            route.validate()?;
        }
        let generation = &self.generation;
        if generation.max_retries > 6 {
            return Err("invalid Host retry budget");
        }
        if let Some(selection) = &generation.reasoning_selection {
            selection.validate(generation.max_output_tokens)
                .map_err(|_| "invalid Host reasoning selection")?;
        }
        if !generation
            .temperature
            .as_f64()
            .is_some_and(|v| (0.0..=2.0).contains(&v))
            || !generation
                .top_p
                .as_f64()
                .is_some_and(|v| (0.0..=1.0).contains(&v))
            || [
                &generation.prompt_cache_key_sha256,
                &generation.prompt_cache_key_prefix_sha256,
            ]
            .into_iter()
            .any(|v| v.as_ref().is_some_and(|v| !digest(v)))
            || generation
                .prompt_cache_retention
                .as_ref()
                .is_some_and(|v| v.len() > 128)
        {
            return Err("invalid Host generation identity");
        }
        Ok(())
    }

    pub fn require_exact(&self, current: &Self) -> Result<(), &'static str> {
        self.validate()?;
        current.validate()?;
        if self != current {
            return Err("Host invocation unavailable: frozen route, credentials or execution configuration changed");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::generation::resolve_output_limit;

    #[test]
    fn external_executor_identity_is_closed_and_transport_independent() {
        let route = HostModelRouteBinding::External {
            backend: "codex".into(), model: Some("fixture".into()), provider: None,
        };
        route.validate().unwrap();
        let mut wire = serde_json::to_value(&route).unwrap();
        assert_eq!(serde_json::from_value::<HostModelRouteBinding>(wire.clone()).unwrap(), route);
        wire["endpoint"] = serde_json::json!("https://unrelated.invalid");
        assert!(serde_json::from_value::<HostModelRouteBinding>(wire).is_err());
        for backend in ["", " ", "nola", "codex\n"] {
            assert!(HostModelRouteBinding::External { backend: backend.into(), model: None, provider: None }.validate().is_err());
        }
    }

    #[test]
    fn android_native_adapter_has_distinct_round_trip_identity() {
        let native = HostModelRouteBinding::Native(HostNativeModelRouteBinding {
            adapter: HostNativeModelAdapter::LlamaCpp,
            adapter_contract_revision: 1,
            model_id: "installed-gguf".into(),
            profile_ref: None,
        });
        native.validate().unwrap();
        let wire = serde_json::to_value(&native).unwrap();
        assert_eq!(wire["adapter"], "llama_cpp");
        assert_eq!(serde_json::from_value::<HostModelRouteBinding>(wire.clone()).unwrap(), native);
        let mut other = wire;
        other["adapter"] = serde_json::json!("mlx");
        assert_ne!(serde_json::from_value::<HostModelRouteBinding>(other).unwrap(), native);
    }

    #[test]
    fn shared_output_admission_resolves_exact_route_facts_and_preserves_order() {
        let routes = binding().output_admission.routes;
        let admitted = HostOutputAdmission::admit(routes.clone(), OutputTokenPreference::Default).unwrap();
        assert_eq!(admitted.version, HOST_OUTPUT_ADMISSION_VERSION);
        assert_eq!(admitted.routes, routes);
        assert_eq!(admitted.resolved.tokens, 8192);
        assert_eq!(admitted.resolved, resolve_output_limit(OutputTokenPreference::Default,
            OutputTokenLimits { known_output_cap: Some(8192) }).unwrap());
        let explicit = HostOutputAdmission::admit(routes.clone(), OutputTokenPreference::Explicit(64)).unwrap();
        assert_eq!(explicit.resolved.tokens, 64);
        assert_eq!(explicit.preference, OutputTokenPreference::Explicit(64));
        assert!(HostOutputAdmission::admit(routes, OutputTokenPreference::Explicit(8193)).is_err());
    }

    #[test]
    fn shared_output_admission_supports_native_unknown_caps_without_inventing_evidence() {
        let native = HostModelRouteBinding::Native(HostNativeModelRouteBinding {
            adapter: HostNativeModelAdapter::Mlx, adapter_contract_revision: 1,
            model_id: "catalog/model".into(), profile_ref: None,
        });
        let routes = vec![HostRouteTokenLimits {
            route: native, max_input_tokens: Some(4096), max_output_tokens: None,
            max_total_tokens: Some(4096),
        }];
        let admitted = HostOutputAdmission::admit(routes.clone(), OutputTokenPreference::Default).unwrap();
        assert_eq!(admitted.resolved, resolve_output_limit_with_window(OutputTokenPreference::Default,
            OutputTokenLimits { known_output_cap: None }, Some(4096)).unwrap());
        assert_eq!(admitted.resolved.source, nola_domain::generation::OutputTokenLimitSource::ContextWindowDefault);
        assert_eq!(admitted.routes, routes);
        assert_eq!(admitted.routes[0].max_output_tokens, None);
        let wire = serde_json::to_vec(&admitted).unwrap();
        let restored: HostOutputAdmission = serde_json::from_slice(&wire).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored, admitted);
    }

    #[test]
    fn shared_output_admission_rejects_invalid_routes_duplicates_and_zero_cap() {
        assert!(HostOutputAdmission::admit(vec![], OutputTokenPreference::Default).is_err());
        let candidate = binding().output_admission.routes[0].clone();
        assert!(HostOutputAdmission::admit(vec![candidate.clone(); 2], OutputTokenPreference::Default).is_err());
        assert!(HostOutputAdmission::admit(vec![candidate.clone(); 18], OutputTokenPreference::Default).is_err());
        let mut invalid = candidate.clone();
        invalid.max_output_tokens = Some(0);
        assert!(HostOutputAdmission::admit(vec![invalid], OutputTokenPreference::Default).is_err());
        let mut invalid = candidate;
        http_mut(&mut invalid.route).endpoint = "https://fixture.invalid/?secret=private".into();
        let error = HostOutputAdmission::admit(vec![invalid], OutputTokenPreference::Default).unwrap_err();
        assert!(!error.contains("private"));
    }

    #[test]
    fn native_route_has_no_http_identity_and_is_validated_at_both_boundaries() {
        let route: HostModelRouteBinding = serde_json::from_value(serde_json::json!({
            "kind":"native", "adapter":"mlx", "adapter_contract_revision":1,
            "model_id":"catalog/logical-model", "profile_ref":null
        })).unwrap();
        route.validate().unwrap();
        let mut value = binding();
        value.primary = route.clone();
        value.fallbacks.clear();
        value.output_admission.routes.truncate(1);
        value.output_admission.routes[0].route = route.clone();
        value.output_admission.routes[0].max_output_tokens = Some(8192);
        value.validate().unwrap();
        let encoded = serde_json::to_value(&value).unwrap();
        assert!(encoded["primary"].get("protocol").is_none());
        let decoded: HostInvocationBinding = serde_json::from_value(encoded).unwrap();
        decoded.require_exact(&value).unwrap();
        for key in ["endpoint", "protocol", "output_token_field", "credential_sha256"] {
            let mut forged = serde_json::to_value(&route).unwrap();
            forged[key] = serde_json::json!("http-imposter");
            assert!(serde_json::from_value::<HostModelRouteBinding>(forged).is_err(), "{key}");
        }
        for bad in [
            serde_json::json!({"kind":"native","adapter":"mlx","adapter_contract_revision":2,"model_id":"model","profile_ref":null}),
            serde_json::json!({"kind":"native","adapter":"mlx","adapter_contract_revision":1,"model_id":" ","profile_ref":null}),
            serde_json::json!({"kind":"native","adapter":"mlx","adapter_contract_revision":1,"model_id":"model","profile_ref":{"profile_id":"profile","admitted_revision":0}}),
        ] {
            let invalid: HostModelRouteBinding = serde_json::from_value(bad).unwrap();
            value.primary = invalid.clone();
            value.output_admission.routes[0].route = invalid;
            assert!(value.output_admission.validate().is_err());
            assert!(value.validate().is_err());
        }
        let mut missing_kind = serde_json::to_value(route).unwrap();
        missing_kind.as_object_mut().unwrap().remove("kind");
        assert!(serde_json::from_value::<HostModelRouteBinding>(missing_kind).is_err());
    }

    #[test]
    fn resolved_generation_is_required_and_part_of_exact_binding() {
        let original = binding();
        let wire = serde_json::to_value(&original).unwrap();
        for field in ["reasoning_selection", "reasoning_tier", "sampling_override", "generation_options", "max_retries", "parallel_tool_calls"] {
            let mut missing = wire.clone();
            missing["generation"].as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<HostInvocationBinding>(missing).is_err(), "{field}");
        }
        let mut changed = original.clone();
        changed.generation.reasoning_tier = nola_domain::reasoning_tiers::ReasoningTier::Concise;
        assert!(original.require_exact(&changed).is_err());
        let restored: HostInvocationBinding = serde_json::from_value(serde_json::to_value(&changed).unwrap()).unwrap();
        assert_eq!(restored.generation.reasoning_tier, nola_domain::reasoning_tiers::ReasoningTier::Concise);
        changed = original.clone();
        changed.generation.reasoning_selection = Some(nola_domain::generation::ReasoningSelection::Effort { value: "high".into() });
        assert!(original.require_exact(&changed).is_err());
        changed = original.clone();
        changed.generation.sampling_override = true;
        assert!(original.require_exact(&changed).is_err());
        changed = original.clone();
        changed.generation.generation_options = nola_domain::generation::GenerationOptions { seed: Some(0), ..Default::default() }.try_into().unwrap();
        assert!(original.require_exact(&changed).is_err());
        let restored: HostInvocationBinding = serde_json::from_value(serde_json::to_value(&changed).unwrap()).unwrap();
        assert_eq!(restored, changed);
        changed = original.clone();
        changed.generation.max_retries = 7;
        assert!(changed.validate().is_err());
        changed = original.clone();
        changed.generation.parallel_tool_calls = Some(false);
        assert!(original.require_exact(&changed).is_err());
        changed.version = 2;
        assert!(changed.validate().is_err());
    }

    fn route(model: &str) -> HostModelRouteBinding {
        HostModelRouteBinding::Http(HostHttpModelRouteBinding {
            siwc_account: None,
            protocol: nola_domain::generation::ModelProtocol::ChatCompletions,
            output_token_field: ModelOutputTokenField::MaxTokens,
            provider: Some("fixture".into()),
            endpoint: "https://fixture.invalid/v1".into(),
            endpoint_query_sha256: None,
            endpoint_query_source: HostSecretSource::None,
            model: model.into(),
            credential_sha256: "a".repeat(64),
            credential_source: HostSecretSource::Ephemeral,
        })
    }

    #[test]
    fn siwc_route_binds_account_without_api_key_or_output_wire_field() {
        let mut value = route("fixture");
        let http = http_mut(&mut value);
        http.protocol = nola_domain::generation::ModelProtocol::SiwcResponses;
        http.output_token_field = ModelOutputTokenField::Omitted;
        http.endpoint = "https://api.openai.com/v1".into();
        http.credential_source = HostSecretSource::None;
        http.credential_sha256 = EMPTY_CREDENTIAL_SHA256.into();
        http.siwc_account = Some(HostSiwcAccountBinding {account_id:"fixture".into(),account_revision:1,
            credential_ref:"siwc:fixture".into(),client_id:"fixture-client".into(),subject:"fixture-subject".into()});
        assert!(value.validate().is_ok());
        let original = value.clone();
        http_mut(&mut value).output_token_field = ModelOutputTokenField::MaxOutputTokens;
        assert!(value.validate().is_err());
        value = original.clone(); http_mut(&mut value).siwc_account = None;
        assert!(value.validate().is_err());
        value = original.clone(); http_mut(&mut value).protocol = nola_domain::generation::ModelProtocol::Responses;
        assert!(value.validate().is_err());
        value = original.clone(); http_mut(&mut value).credential_source = HostSecretSource::Ephemeral;
        http_mut(&mut value).credential_sha256 = "a".repeat(64);
        assert!(value.validate().is_err());
        value = original; http_mut(&mut value).siwc_account.as_mut().unwrap().credential_ref = "siwc:other".into();
        assert!(value.validate().is_err());
    }

    fn http_mut(route: &mut HostModelRouteBinding) -> &mut HostHttpModelRouteBinding {
        match route { HostModelRouteBinding::Http(http) => http, _ => panic!("HTTP fixture") }
    }

    fn binding() -> HostInvocationBinding {
        let primary = route("primary");
        let fallback = route("fallback");
        let preference = OutputTokenPreference::Default;
        let resolved = resolve_output_limit(
            preference,
            OutputTokenLimits {
                known_output_cap: Some(8_192),
            },
        )
        .unwrap();
        HostInvocationBinding {
            mac_launch: None,
            version: HOST_INVOCATION_BINDING_VERSION,
            primary: primary.clone(),
            fallbacks: vec![fallback.clone()],
            generation: HostGenerationBinding {
                max_retries: 2,
                parallel_tool_calls: None,
                reasoning_selection: None,
                reasoning_tier: Default::default(),
                sampling_override: false,
                generation_options: nola_domain::generation::GenerationOptions::default().try_into().unwrap(),
                temperature: serde_json::Number::from_f64(0.7).unwrap(),
                top_p: serde_json::Number::from_f64(0.9).unwrap(),
                max_output_tokens: resolved.tokens,
                thinking_enabled: false, reasoning_summary_supported: None,
                prompt_cache: false,
                prompt_cache_key_sha256: None,
                prompt_cache_key_prefix_sha256: None,
                prompt_cache_retention: None,
            },
            output_admission: HostOutputAdmission {
                version: HOST_OUTPUT_ADMISSION_VERSION,
                configured_context_window: None,
                preference,
                resolved,
                routes: vec![
                    HostRouteTokenLimits {
                        route: primary,
                        max_input_tokens: None,
                        max_output_tokens: None,
                        max_total_tokens: None,
                    },
                    HostRouteTokenLimits {
                        route: fallback,
                        max_input_tokens: None,
                        max_output_tokens: Some(8_192),
                        max_total_tokens: Some(128_000),
                    },
                ],
            },
            execution_policy_sha256: "b".repeat(64),
        }
    }

    #[test]
    fn frozen_output_facts_round_trip_without_reprobing() {
        let original = binding();
        original.validate().unwrap();
        let encoded = serde_json::to_vec(&original).unwrap();
        let restored: HostInvocationBinding = serde_json::from_slice(&encoded).unwrap();
        restored.require_exact(&original).unwrap();
        assert_eq!(restored.generation.max_output_tokens, 8_192);
    }

    #[test]
    fn protocol_is_required_and_part_of_exact_route_identity() {
        let original = binding();
        let mut wire = serde_json::to_value(&original).unwrap();
        wire["primary"].as_object_mut().unwrap().remove("protocol");
        assert!(serde_json::from_value::<HostInvocationBinding>(wire).is_err());
        let mut changed = original.clone();
        http_mut(&mut changed.primary).protocol = nola_domain::generation::ModelProtocol::Responses;
        http_mut(&mut changed.primary).output_token_field = ModelOutputTokenField::MaxOutputTokens;
        assert!(changed.validate().is_err(), "admission must match the protocol too");
        changed.output_admission.routes[0].route = changed.primary.clone();
        changed.validate().unwrap();
        assert!(original.require_exact(&changed).is_err());
        let mut old = original;
        old.version = 3;
        assert!(old.validate().is_err());
        old.version = HOST_INVOCATION_BINDING_VERSION;
        old.output_admission.version = 99;
        assert!(old.validate().is_err());
    }

    #[test]
    fn output_field_is_frozen_and_validated_against_wire_protocol() {
        let original = binding();
        let mut wire = serde_json::to_value(&original).unwrap();
        wire["primary"].as_object_mut().unwrap().remove("output_token_field");
        assert!(serde_json::from_value::<HostInvocationBinding>(wire).is_err());
        let mut changed = original.clone();
        http_mut(&mut changed.primary).output_token_field = ModelOutputTokenField::MaxCompletionTokens;
        changed.output_admission.routes[0].route = changed.primary.clone();
        changed.validate().unwrap();
        assert!(original.require_exact(&changed).is_err());
        for (protocol, field) in [
            (nola_domain::generation::ModelProtocol::Responses, ModelOutputTokenField::MaxTokens),
            (nola_domain::generation::ModelProtocol::Anthropic, ModelOutputTokenField::MaxCompletionTokens),
            (nola_domain::generation::ModelProtocol::ChatCompletions, ModelOutputTokenField::MaxOutputTokens),
        ] {
            http_mut(&mut changed.primary).protocol = protocol;
            http_mut(&mut changed.primary).output_token_field = field;
            changed.output_admission.routes[0].route = changed.primary.clone();
            assert!(changed.validate().is_err());
        }
    }

    #[test]
    fn persisted_routes_reject_sensitive_or_unnormalized_endpoints() {
        for endpoint in [
            "https://fixture.invalid/v1?key=fixture-secret",
            "https://user:fixture-secret@fixture.invalid/v1",
            "https://user@fixture.invalid/v1",
            "https://fixture.invalid/v1#fixture-secret",
            "file:///tmp/model", "not a URL", " https://fixture.invalid/v1",
            "https://FIXTURE.invalid/v1", "https://fixture.invalid/v1?",
        ] {
            // Keep admission evidence consistent so validation cannot pass
            // merely because a different route-identity check rejected it.
            for fallback in [false, true] {
                let mut value = binding();
                if fallback {
                    http_mut(&mut value.fallbacks[0]).endpoint = endpoint.into();
                    value.output_admission.routes[1].route = value.fallbacks[0].clone();
                } else {
                    http_mut(&mut value.primary).endpoint = endpoint.into();
                    value.output_admission.routes[0].route = value.primary.clone();
                }
                let error = value.validate().unwrap_err();
                assert!(!error.contains("fixture-secret"));
            }
        }
    }

    #[test]
    fn empty_local_endpoint_and_query_digest_remain_distinct() {
        let mut value = binding();
        http_mut(&mut value.primary).endpoint.clear();
        value.output_admission.routes[0].route = value.primary.clone();
        value.validate().unwrap();
        http_mut(&mut value.primary).endpoint_query_sha256 = Some("c".repeat(64));
        http_mut(&mut value.primary).endpoint_query_source = HostSecretSource::Ephemeral;
        value.output_admission.routes[0].route = value.primary.clone();
        assert!(value.validate().is_err());
        http_mut(&mut value.primary).endpoint = "http://127.0.0.1:1234/v1".into();
        value.output_admission.routes[0].route = value.primary.clone();
        value.validate().unwrap();
    }

    #[test]
    fn http_secret_sources_are_required_and_match_digest_and_purpose() {
        let original = route("model");
        for key in ["credential_source", "endpoint_query_source"] {
            let mut encoded = serde_json::to_value(&original).unwrap();
            encoded.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<HostModelRouteBinding>(encoded).is_err());
        }
        let mut value = original.clone();
        http_mut(&mut value).credential_source = HostSecretSource::None;
        assert!(value.validate().is_err());
        http_mut(&mut value).credential_sha256 = EMPTY_CREDENTIAL_SHA256.into();
        value.validate().unwrap();
        http_mut(&mut value).credential_source = HostSecretSource::Ephemeral;
        assert!(value.validate().is_err());

        let mut value = original;
        http_mut(&mut value).endpoint_query_source = HostSecretSource::Ephemeral;
        assert!(value.validate().is_err());
        http_mut(&mut value).endpoint_query_sha256 = Some("b".repeat(64));
        value.validate().unwrap();
        let reference = HostSecretSource::Managed {
            authority: crate::HostSecretAuthority::IosApp,
            owner_id: "a".repeat(32), handle_id: "b".repeat(32), revision: 1,
            purpose: HostSecretPurpose::ModelCredential,
        };
        http_mut(&mut value).credential_source = reference.clone();
        value.validate().unwrap();
        http_mut(&mut value).endpoint_query_source = reference;
        assert!(value.validate().is_err());
    }

    #[test]
    fn output_snapshot_rejects_forged_resolution_and_generation() {
        let original = binding();
        let mut changed = original.clone();
        changed.output_admission.resolved.tokens += 1;
        assert!(changed.validate().is_err());
        let mut changed = original.clone();
        changed.output_admission.resolved.source =
            nola_domain::generation::OutputTokenLimitSource::ProductDefault;
        assert!(changed.validate().is_err());
        let mut changed = original.clone();
        changed.generation.max_output_tokens += 1;
        assert!(changed.validate().is_err());
        let mut changed = original;
        changed.output_admission.preference = OutputTokenPreference::Explicit(16_384);
        assert!(changed.validate().is_err());
    }

    #[test]
    fn output_snapshot_rejects_foreign_duplicate_and_reordered_routes() {
        let original = binding();
        for field in ["model", "endpoint", "credential"] {
            let mut changed = original.clone();
            let route = &mut changed.output_admission.routes[1].route;
            match field {
                "model" => http_mut(route).model = "unrelated".into(),
                "endpoint" => http_mut(route).endpoint = "https://other.invalid/v1".into(),
                _ => http_mut(route).credential_sha256 = "c".repeat(64),
            }
            assert!(changed.validate().is_err());
        }
        let mut changed = original.clone();
        changed.output_admission.routes.swap(0, 1);
        assert!(changed.validate().is_err());
        let mut changed = original;
        changed
            .output_admission
            .routes
            .push(changed.output_admission.routes[1].clone());
        assert!(changed.validate().is_err());
    }

    #[test]
    fn output_snapshot_rejects_empty_invalid_or_unversioned_facts() {
        for cap in [0, 63] {
            let mut changed = binding();
            changed.output_admission.routes[1].max_output_tokens = Some(cap);
            assert!(changed.validate().is_err());
        }
        let mut changed = binding();
        changed.output_admission.routes.clear();
        assert!(changed.validate().is_err());
        let mut changed = binding();
        changed.output_admission.version = 0;
        assert!(changed.validate().is_err());
        let mut changed = binding();
        changed.version = 99;
        assert!(changed.validate().is_err());
        let mut encoded = serde_json::to_value(binding()).unwrap();
        encoded.as_object_mut().unwrap().remove("output_admission");
        assert!(serde_json::from_value::<HostInvocationBinding>(encoded).is_err());
    }

    #[test]
    fn snapshot_metadata_change_is_a_frozen_identity_change() {
        let original = binding();
        let mut changed = original.clone();
        changed.output_admission.routes[0].max_total_tokens = Some(256_000);
        changed.validate().unwrap();
        assert!(original.require_exact(&changed).is_err());
    }

    #[test]
    fn output_snapshot_unknown_fields_are_rejected() {
        let mut encoded = serde_json::to_value(binding()).unwrap();
        encoded["output_admission"]["routes"][0]["credentials"] = "not-a-contract-field".into();
        assert!(serde_json::from_value::<HostInvocationBinding>(encoded).is_err());
    }
}
