// SPDX-License-Identifier: Apache-2.0
//! Shared generation override contract for configuration composition and Host input.
macro_rules! impl_model_request_overrides {
    ($target:ty) => {
        impl $target {
            /// Resolve a frozen request snapshot. Missing fields inherit; explicit null
            /// clears reasoning. Validate before returning so failures cannot partially
            /// mutate the caller's global configuration.
            pub fn with_model_request(&self, request: &serde_json::Value) -> Result<Self, String> {
                let model = nola_application::model_profile_resolution::ResolvedModelRuntime {
                    provider: self.provider.clone(),
                    temperature: self.temperature.clone(),
                    top_p: self.top_p.clone(),
                    max_output_tokens: self.max_output_tokens.clone(),
                    chat_output_token_field: self.chat_output_token_field.clone(),
                    reasoning_selection: self.reasoning_selection.clone(),
                    reasoning_tier: self.reasoning_tier.clone(),
                    sampling_override: self.sampling_override.clone(),
                    generation_options: self.generation_options.clone(),
                    model_capabilities: self.model_capabilities.clone(),
                    context_window_tokens: self.compaction.context_window_tokens,
                }
                .with_model_request(request)?;
                let mut result = self.clone();
                result.provider = model.provider;
                result.temperature = model.temperature;
                result.top_p = model.top_p;
                result.max_output_tokens = model.max_output_tokens;
                result.chat_output_token_field = model.chat_output_token_field;
                result.reasoning_selection = model.reasoning_selection;
                result.reasoning_tier = model.reasoning_tier;
                result.sampling_override = model.sampling_override;
                result.generation_options = model.generation_options;
                result.model_capabilities = model.model_capabilities;
                result.compaction.context_window_tokens = model.context_window_tokens;
                Ok(result)
            }
        }
    };
}
pub(crate) use impl_model_request_overrides;
