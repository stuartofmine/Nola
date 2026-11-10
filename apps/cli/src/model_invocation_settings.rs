// SPDX-License-Identifier: Apache-2.0
//! Immutable model settings after composition; no argv, prompt, environment,
//! terminal, or configuration-file loading. Credentials remain ephemeral.
use crate::model_route::{route_binding, validate_native_transport};
use nola_host_protocol::{HostModelRouteBinding, HostOutputAdmission};

#[derive(Clone)]
pub(crate) struct ModelInvocationOptions {
    pub siwc: Option<crate::model_siwc::SiwcExecutionContext>,
    pub provider: Option<String>,
    pub native_model_route: Option<nola_host_protocol::HostNativeModelRouteBinding>,
    pub llm_fallbacks: Vec<crate::runtime_config::RuntimeLlmProvider>,
    pub max_output_tokens: Option<u64>,
    pub chat_output_token_field: nola_application::ChatOutputTokenField,
    pub reasoning_selection: Option<nola_core::agent::reasoning::ReasoningSelection>,
    pub output_admission: Option<HostOutputAdmission>,
    pub compaction: crate::runtime_config::RuntimeCompaction,
}

#[derive(Clone)]
pub(crate) struct ModelInvocationSettings {
    endpoint: String,
    api_key: String,
    model: String,
    options: ModelInvocationOptions,
    swe_mode: bool,
    primary: HostModelRouteBinding,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ModelInvocationSettingsError {
    MissingEndpoint,
    InvalidRoute(String),
}
impl std::fmt::Display for ModelInvocationSettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEndpoint => f.write_str("Host invocation requires an explicit model endpoint"),
            Self::InvalidRoute(message) => f.write_str(message),
        }
    }
}
impl std::fmt::Debug for ModelInvocationSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ModelInvocationSettings { route_and_credentials: [redacted] }")
    }
}

impl ModelInvocationSettings {
    pub(crate) fn external(backend: String, model: Option<String>, provider: Option<String>,
        mut options: ModelInvocationOptions, swe_mode: bool,
    ) -> Result<Self, ModelInvocationSettingsError> {
        let primary = HostModelRouteBinding::External { backend, model: model.clone(), provider };
        primary.validate().map_err(|e| ModelInvocationSettingsError::InvalidRoute(e.into()))?;
        // These are HTTP transport settings, not executor configuration. Never
        // probe them or leak a selected native model's credentials to a delegate.
        options.provider = None;
        options.siwc = None;
        options.native_model_route = None;
        options.llm_fallbacks.clear();
        Ok(Self { endpoint: String::new(), api_key: String::new(), model: model.unwrap_or_default(),
            options, swe_mode, primary })
    }

    pub(crate) fn new(endpoint: String, api_key: String, model: String,
        options: ModelInvocationOptions, swe_mode: bool,
    ) -> Result<Self, ModelInvocationSettingsError> {
        use ModelInvocationSettingsError::{InvalidRoute, MissingEndpoint};
        if endpoint.trim().is_empty() { return Err(MissingEndpoint); }
        if options.llm_fallbacks.len() > 16 {
            return Err(InvalidRoute("Host output admission supports at most 16 fallback routes".into()));
        }
        let primary = if let Some(siwc) = &options.siwc {
            if !api_key.is_empty() || !options.llm_fallbacks.is_empty() || options.native_model_route.is_some()
                || options.provider.as_deref() != Some("siwc_responses") {
                return Err(InvalidRoute("SIWC cannot use API credentials or fallback routes".into()));
            }
            let mut route = route_binding(options.provider.as_deref(), &endpoint, &model, "",
                options.chat_output_token_field).map_err(InvalidRoute)?;
            if let HostModelRouteBinding::Http(http) = &mut route {
                http.protocol = nola_domain::generation::ModelProtocol::SiwcResponses;
                http.output_token_field = nola_host_protocol::ModelOutputTokenField::Omitted;
                http.siwc_account = Some(siwc.account.clone());
            }
            route
        } else if let Some(route) = &options.native_model_route {
            validate_native_transport(route, &endpoint, &model, &api_key).map_err(InvalidRoute)?;
            if crate::llm::resolved_model_protocol(options.provider.as_deref(), &endpoint)
                != nola_domain::generation::ModelProtocol::ChatCompletions {
                return Err(InvalidRoute("native MLX channel requires its Chat Completions adapter".into()));
            }
            HostModelRouteBinding::Native(route.clone())
        } else {
            route_binding(options.provider.as_deref(), &endpoint, &model, &api_key,
                options.chat_output_token_field).map_err(InvalidRoute)?
        };
        primary.validate().map_err(|error| InvalidRoute(error.into()))?;
        let mut identities = vec![primary.clone()];
        if crate::llm::supports_provider_fallbacks(options.provider.as_deref(), options.reasoning_selection.is_some()) {
            for fallback in &options.llm_fallbacks {
                if fallback.endpoint.trim().is_empty() { return Err(MissingEndpoint); }
                let route = route_binding(fallback.provider.as_deref(), &fallback.endpoint,
                    fallback.model.as_deref().unwrap_or(&model), fallback.api_key.as_deref().unwrap_or(&api_key),
                    options.chat_output_token_field).map_err(InvalidRoute)?;
                route.validate().map_err(|error| InvalidRoute(error.into()))?;
                if identities.contains(&route) {
                    return Err(InvalidRoute("Host output admission contains a duplicate route".into()));
                }
                identities.push(route);
            }
        }
        Ok(Self { endpoint, api_key, model, options, swe_mode, primary })
    }
    pub(crate) fn endpoint(&self) -> &String { &self.endpoint }
    pub(crate) fn api_key(&self) -> &String { &self.api_key }
    pub(crate) fn model(&self) -> &String { &self.model }
    /// Delegated executors may select their own default model. Preserve absence
    /// across the Core boundary instead of turning it into an empty identity.
    pub(crate) fn core_model(&self) -> Option<String> {
        match &self.primary {
            HostModelRouteBinding::External { model, .. } => model.clone(),
            _ => Some(self.model.clone()),
        }
    }
    pub(crate) fn options(&self) -> &ModelInvocationOptions { &self.options }
    pub(crate) fn swe_mode(&self) -> bool { self.swe_mode }
    pub(crate) fn primary(&self) -> &HostModelRouteBinding { &self.primary }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> ModelInvocationOptions {
        ModelInvocationOptions {
            provider: Some("openai".into()), siwc: None, native_model_route: None, llm_fallbacks: vec![],
            max_output_tokens: Some(1024), chat_output_token_field: nola_application::ChatOutputTokenField::Auto,
            reasoning_selection: None, output_admission: None,
            compaction: crate::runtime_config::RuntimeCompaction::default(),
        }
    }
    #[test]
    fn siwc_model_route_requires_owned_identity_and_forbids_api_fallbacks() {
        let mut options = options();
        options.provider = Some("siwc_responses".into());
        assert!(ModelInvocationSettings::new("https://api.openai.com/v1".into(),String::new(),"fixture".into(),options.clone(),false).is_err());
        options.siwc = Some(crate::model_siwc::SiwcExecutionContext {database:std::path::PathBuf::from("/fixture/never-opened"),
            account:nola_host_protocol::HostSiwcAccountBinding {account_id:"fixture".into(),account_revision:1,
                credential_ref:"siwc:fixture".into(),client_id:"fixture-client".into(),subject:"fixture-subject".into()}});
        let settings = ModelInvocationSettings::new("https://api.openai.com/v1".into(),String::new(),"fixture".into(),options.clone(),false).unwrap();
        let HostModelRouteBinding::Http(route) = settings.primary() else {panic!("http route")};
        assert_eq!(route.output_token_field,nola_host_protocol::ModelOutputTokenField::Omitted);
        assert_eq!(route.siwc_account.as_ref().unwrap().account_revision,1);
        assert!(ModelInvocationSettings::new("https://api.openai.com/v1".into(),"fixture-api-key".into(),"fixture".into(),options.clone(),false).is_err());
        options.llm_fallbacks.push(crate::runtime_config::RuntimeLlmProvider {endpoint:"https://api.openai.com/v1".into(),..Default::default()});
        assert!(ModelInvocationSettings::new("https://api.openai.com/v1".into(),String::new(),"fixture".into(),options,false).is_err());
    }
    #[test]
    fn host_model_settings_require_no_cli_configuration_and_missing_endpoint_is_typed() {
        assert_eq!(ModelInvocationSettings::new(String::new(), String::new(), "fixture".into(), options(), false).unwrap_err(),
            ModelInvocationSettingsError::MissingEndpoint);
        let settings = ModelInvocationSettings::new("https://fixture.invalid/v1".into(), "secret".into(),
            "fixture".into(), options(), false).unwrap();
        let admitted = crate::host_output_admission::admit(&settings, false).unwrap();
        assert_eq!(admitted.resolved.tokens, 1024);
        assert!(!format!("{settings:?}").contains("secret"));
    }
    #[test]
    fn external_executor_needs_no_http_endpoint_or_credentials() {
        let settings = ModelInvocationSettings::external("codex".into(), Some("gpt-6.1-sol".into()),
            None, options(), false).unwrap();
        assert!(settings.endpoint().is_empty());
        assert!(settings.api_key().is_empty());
        assert!(settings.options().provider.is_none());
        assert!(settings.options().llm_fallbacks.is_empty());
        assert!(matches!(settings.primary(), HostModelRouteBinding::External { backend, .. } if backend == "codex"));
        assert!(crate::host_output_admission::admit(&settings, true).is_ok());
        assert_eq!(settings.core_model().as_deref(), Some("gpt-6.1-sol"));
    }
    #[test]
    fn external_default_model_remains_absent_at_core_boundary() {
        let settings = ModelInvocationSettings::external("codex".into(), None,
            None, options(), false).unwrap();
        assert_eq!(settings.core_model(), None);
        assert!(matches!(settings.primary(), HostModelRouteBinding::External { model: None, .. }));
    }
    #[test]
    fn invalid_fallback_is_rejected_before_provider_metadata_io() {
        let mut input = options();
        input.llm_fallbacks.push(crate::runtime_config::RuntimeLlmProvider {
            provider: Some("openai".into()), endpoint: "file:///private/config".into(), api_key: None, model: None,
        });
        assert!(matches!(ModelInvocationSettings::new("https://fixture.invalid/v1".into(), String::new(),
            "fixture".into(), input, false), Err(ModelInvocationSettingsError::InvalidRoute(_))));
    }
}
