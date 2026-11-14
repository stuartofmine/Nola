// SPDX-License-Identifier: Apache-2.0
//! Remote user intent, not an executable provider configuration. The receiving
//! host must resolve profile identity/revision and capabilities from its store.

use crate::generation::GenerationOptions;
use crate::reasoning_tiers::ReasoningTier;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunGenerationIntent {
    pub profile_id: String,
    pub expected_revision: i64,
    pub tier: ReasoningTier,
    pub overrides: RunGenerationOverrides,
}

/// Missing scalar fields inherit host/session values. Null scalars are rejected,
/// not silently treated as absence. Nested generation nulls explicitly clear a
/// stored option and must survive serialization until the host merges the patch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunGenerationOverrides {
    /// Optional exact choice for peers advertising expert-role catalog v1.
    /// Absent preserves the published tier contract and serializes identically.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub reasoning_selection: Option<crate::generation::ReasoningSelection>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub temperature: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub top_p: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub max_output_tokens: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub max_context_tokens: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub sampling_override: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub generation_options: Option<Map<String, Value>>,
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl RunGenerationIntent {
    pub fn validate(&self) -> Result<(), String> {
        if self.profile_id.trim().is_empty()
            || self.profile_id.len() > 512
            || self.expected_revision < 0
        {
            return Err("invalid model intent profile identity or revision".into());
        }
        self.overrides.validate()
    }
}

impl RunGenerationOverrides {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(selection) = &self.reasoning_selection {
            selection.validate(self.max_output_tokens.unwrap_or(16384))?;
        }
        if self.temperature.is_some_and(|v| !(0.0..=2.0).contains(&v))
            || self.top_p.is_some_and(|v| !(0.0..=1.0).contains(&v))
            || self.max_output_tokens.is_some_and(|v| {
                crate::generation::resolve_output_token_limit(Some(v)).is_err()
            })
            || self
                .max_context_tokens
                .is_some_and(|v| !(512..=4_000_000).contains(&v))
        {
            return Err("invalid model intent generation override".into());
        }
        if let Some(options) = &self.generation_options {
            let parsed: GenerationOptions = serde_json::from_value(Value::Object(options.clone()))
                .map_err(|error| format!("invalid model intent generation options: {error}"))?;
            parsed.validate()?;
        }
        Ok(())
    }

    pub fn to_request(&self) -> Result<Value, String> {
        self.validate()?;
        serde_json::to_value(self).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn optional_exact_reasoning_preserves_the_published_absent_wire() {
        let old = json!({"profile_id":"model","expected_revision":1,"tier":"balanced","overrides":{}});
        let decoded: RunGenerationIntent = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), old);
        for effort in crate::generation::REASONING_EFFORT_VALUES {
            let exact = json!({"reasoning_selection":{"kind":"effort","value":effort}});
            let overrides: RunGenerationOverrides = serde_json::from_value(exact.clone()).unwrap();
            assert_eq!(overrides.to_request().unwrap(), exact);
        }
        for value in [json!(null), json!({"kind":"effort","value":"invented"})] {
            let decoded = serde_json::from_value::<RunGenerationOverrides>(json!({"reasoning_selection":value}));
            assert!(decoded.is_err() || decoded.unwrap().validate().is_err());
        }
    }

    #[test]
    fn sparse_intent_preserves_zero_false_and_nested_clear() {
        let value = json!({"profile_id":"host:profile","expected_revision":0,"tier":"balanced",
            "overrides":{"temperature":0,"sampling_override":false,"generation_options":{"seed":0,"presence_penalty":null}}});
        let intent: RunGenerationIntent = serde_json::from_value(value.clone()).unwrap();
        intent.validate().unwrap();
        let encoded = intent.overrides.to_request().unwrap();
        assert_eq!(encoded["temperature"].as_f64(), Some(0.0));
        assert_eq!(encoded["sampling_override"], false);
        assert_eq!(
            encoded["generation_options"],
            value["overrides"]["generation_options"]
        );
        assert!(intent.overrides.top_p.is_none());
        for seed in [i64::MIN, i64::MAX] {
            let overrides: RunGenerationOverrides =
                serde_json::from_value(json!({"generation_options":{"seed":seed}})).unwrap();
            assert_eq!(
                overrides.to_request().unwrap()["generation_options"]["seed"].as_i64(),
                Some(seed)
            );
        }
    }

    #[test]
    fn client_cannot_supply_route_secrets_capabilities_or_wire_selection() {
        for field in [
            "endpoint",
            "api_key",
            "model",
            "model_protocol",
            "executionCapabilities",
            "reasoning_selection",
        ] {
            let mut value = json!({"profile_id":"host:profile","expected_revision":1,"tier":"balanced","overrides":{}});
            value["overrides"][field] = json!("untrusted");
            assert!(
                serde_json::from_value::<RunGenerationIntent>(value).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn explicit_invalid_values_do_not_become_inheritance() {
        for overrides in [
            json!({"temperature":null}),
            json!({"sampling_override":null}),
            json!({"generation_options":null}),
            json!({"temperature":3}),
            json!({"max_output_tokens":0}),
            json!({"generation_options":{"top_k":0}}),
            json!({"generation_options":{"unknown":null}}),
        ] {
            let result = serde_json::from_value::<RunGenerationOverrides>(overrides.clone());
            assert!(
                result.is_err() || result.unwrap().validate().is_err(),
                "{overrides}"
            );
        }
        assert!(RunGenerationOverrides {
            temperature: Some(f64::NAN),
            ..Default::default()
        }
        .validate()
        .is_err());
        for (id, revision) in [("", 1), ("host:profile", -1)] {
            assert!(RunGenerationIntent {
                profile_id: id.into(),
                expected_revision: revision,
                tier: ReasoningTier::Balanced,
                overrides: Default::default()
            }
            .validate()
            .is_err());
        }
    }
}
