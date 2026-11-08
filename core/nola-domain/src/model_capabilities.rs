// SPDX-License-Identifier: Apache-2.0
//! Platform-neutral execution capabilities for a concrete model route.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatOutputTokenField {
    #[default]
    Auto,
    MaxTokens,
    MaxCompletionTokens,
}

/// What this concrete model route can execute. Product uses such as `chat`
/// and `retrieval` are deliberately separate from transport capabilities.
/// Catalogs without metadata receive the explicit agent baseline at import.
/// Runtime profiles still require a resolved declaration. An explicit all-false
/// declaration is a valid text-only route, not the same fact as unknown.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionCapabilities {
    pub tool_calls: bool,
    pub image_input: bool,
    pub reasoning: bool,
    pub sampling_parameters: bool,
    /// Some reasoning models accept temperature/top_p only with effort=none.
    /// Automatic effort is not equivalent to none because defaults vary by model.
    #[serde(default)]
    pub sampling_requires_no_reasoning: bool,
    /// Chat Completions output-limit spelling is a model-route contract.
    /// Auto uses the official OpenAI adapter's modern field, otherwise the
    /// generic compatible adapter's legacy field. It never inspects model text.
    #[serde(default)]
    pub chat_output_token_field: ChatOutputTokenField,
}

impl ModelExecutionCapabilities {
    /// Unknown provider metadata is usable for agent work; this is a product
    /// default, not evidence of provider support or authorization to execute.
    pub fn agent_baseline() -> Self {
        Self::for_import(None)
    }

    /// Import defaults are distinct from a runtime declaration. A provider's
    /// explicit false must override the agent default, not be treated as absent.
    pub fn for_import(provider_tool_calls: Option<bool>) -> Self {
        Self { tool_calls: provider_tool_calls.unwrap_or(true), ..Self::default() }
    }

    pub fn from_source(source: &Value) -> Result<Self, String> {
        match source.get("executionCapabilities") {
            Some(value) if !value.is_null() => {
                let capabilities: Self = serde_json::from_value(value.clone())
                    .map_err(|error| format!("invalid model execution capabilities: {error}"))?;
                if capabilities.sampling_requires_no_reasoning
                    && (!capabilities.sampling_parameters || !capabilities.reasoning)
                {
                    return Err(
                        "sampling effort restriction requires sampling and reasoning support"
                            .into(),
                    );
                }
                Ok(capabilities)
            }
            None | Some(_) => Err("model_execution_capabilities_required: Confirm execution capabilities for this exact endpoint and model in Model Center before using it. Allowed uses do not declare tool support.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn import_tools_default_on_and_explicit_provider_values_win() {
        assert!(ModelExecutionCapabilities::for_import(None).tool_calls);
        assert!(ModelExecutionCapabilities::for_import(Some(true)).tool_calls);
        assert!(!ModelExecutionCapabilities::for_import(Some(false)).tool_calls);
        assert!(!ModelExecutionCapabilities::for_import(None).image_input);
    }

    fn declared_capabilities() -> Value {
        json!({
            "toolCalls": true,
            "imageInput": false,
            "reasoning": true,
            "samplingParameters": true
        })
    }

    #[test]
    fn absent_metadata_never_infers_support_from_provider_or_model_names() {
        for source in [
            json!({}),
            json!({"executionCapabilities":null}),
            json!({"provider": "openai", "model": "reasoning-model"}),
        ] {
            assert!(ModelExecutionCapabilities::from_source(&source)
                .unwrap_err().starts_with("model_execution_capabilities_required:"));
        }
    }

    #[test]
    fn explicit_unsupported_capabilities_admit_text_only_without_inference() {
        let source = json!({"capabilities":["chat","issue_repair"], "executionCapabilities":{
            "toolCalls":false,"imageInput":false,"reasoning":false,"samplingParameters":false
        }});
        assert_eq!(ModelExecutionCapabilities::from_source(&source).unwrap(), ModelExecutionCapabilities::default());
    }

    #[test]
    fn sampling_restriction_requires_both_capabilities() {
        for sampling in [false, true] {
            for reasoning in [false, true] {
                let mut value = declared_capabilities();
                value["samplingRequiresNoReasoning"] = json!(true);
                value["samplingParameters"] = json!(sampling);
                value["reasoning"] = json!(reasoning);
                let result = ModelExecutionCapabilities::from_source(
                    &json!({"executionCapabilities": value}),
                );
                assert_eq!(result.is_ok(), sampling && reasoning);
            }
        }
    }

    #[test]
    fn wire_contract_preserves_names_and_optional_defaults() {
        let value = declared_capabilities();
        let capabilities =
            ModelExecutionCapabilities::from_source(&json!({"executionCapabilities": value}))
                .unwrap();
        assert!(!capabilities.sampling_requires_no_reasoning);
        assert_eq!(
            capabilities.chat_output_token_field,
            ChatOutputTokenField::Auto
        );
        assert_eq!(
            serde_json::to_value(capabilities).unwrap(),
            json!({
                "toolCalls": true,
                "imageInput": false,
                "reasoning": true,
                "samplingParameters": true,
                "samplingRequiresNoReasoning": false,
                "chatOutputTokenField": "auto"
            })
        );
        for (field, wire) in [
            (ChatOutputTokenField::Auto, "auto"),
            (ChatOutputTokenField::MaxTokens, "max_tokens"),
            (
                ChatOutputTokenField::MaxCompletionTokens,
                "max_completion_tokens",
            ),
        ] {
            assert_eq!(serde_json::to_value(field).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<ChatOutputTokenField>(json!(wire)).unwrap(),
                field
            );
        }
    }

    #[test]
    fn explicit_invalid_metadata_does_not_fall_back_to_defaults() {
        let mut unknown_field = declared_capabilities();
        unknown_field["arbitraryCapability"] = json!(true);
        let mut missing_field = declared_capabilities();
        missing_field.as_object_mut().unwrap().remove("reasoning");
        let mut invalid_type = declared_capabilities();
        invalid_type["toolCalls"] = json!("true");
        let mut invalid_token_field = declared_capabilities();
        invalid_token_field["chatOutputTokenField"] = json!("max_completion");
        for value in [
            Value::Null,
            json!({}),
            unknown_field,
            missing_field,
            invalid_type,
            invalid_token_field,
        ] {
            assert!(ModelExecutionCapabilities::from_source(
                &json!({"executionCapabilities": value})
            )
            .is_err());
        }
    }
}
