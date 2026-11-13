// SPDX-License-Identifier: Apache-2.0
//! Resolve model reasoning at admission. Missing overrides inherit; explicit
//! null selects provider defaults. No provider IO or process configuration.

use nola_domain::generation::{ModelProtocol, ReasoningSelection};
use nola_domain::reasoning_tiers::{
    resolve_reasoning_tier, ReasoningCapabilityDeclaration, ReasoningTier,
};
use serde_json::Value;

/// Pure service/FFI query. This resolves a supplied frozen contract and never
/// reads a model catalogue, mutates a conversation, or creates command receipts.
pub fn resolve_model_reasoning_request(
    declaration: &Value,
    protocol: ModelProtocol,
    output_limit: u64,
    supports_reasoning: bool,
    requested: &Value,
    tier: Option<ReasoningTier>,
) -> Result<Value, String> {
    nola_domain::generation::resolve_output_token_limit(Some(output_limit))?;
    if tier.is_some() && !requested.is_null() {
        return Err("specify a reasoning tier or selection, not both".into());
    }
    let selection = match tier {
        Some(tier) => resolve_model_reasoning_tier(
            declaration,
            protocol,
            output_limit,
            supports_reasoning,
            tier,
        )?,
        None => resolve_model_reasoning(
            declaration,
            protocol,
            output_limit,
            supports_reasoning,
            Some(requested),
        )?,
    };
    Ok(serde_json::json!({"selection": selection}))
}

/// Admission for clients exposing three product tiers rather than wire values.
/// The effective route and output limit must already be frozen by the caller.
pub fn resolve_model_reasoning_tier(
    declared: &Value,
    protocol: ModelProtocol,
    output_limit: u64,
    supports_reasoning: bool,
    tier: ReasoningTier,
) -> Result<Option<ReasoningSelection>, String> {
    // Validate all persisted metadata, but a stale model default is shadowed
    // by the explicitly chosen tier.
    resolve_model_reasoning(
        declared,
        protocol,
        output_limit,
        supports_reasoning,
        Some(&Value::Null),
    )?;
    let active = active_reasoning_declaration(declared)?;
    let selected = resolve_reasoning_tier(protocol, &active, tier, output_limit)
        .map_err(|error| error.to_string())?;
    let wire = serde_json::to_value(selected).map_err(|error| error.to_string())?;
    resolve_model_reasoning(
        declared,
        protocol,
        output_limit,
        supports_reasoning,
        Some(&wire),
    )
}

fn active_reasoning_declaration(
    declared: &Value,
) -> Result<ReasoningCapabilityDeclaration, String> {
    if declared.is_null() {
        return Ok(ReasoningCapabilityDeclaration::Automatic {});
    }
    match declared["control"].as_str() {
        Some("automatic") => Ok(ReasoningCapabilityDeclaration::Automatic {}),
        Some("effort") => Ok(ReasoningCapabilityDeclaration::Effort {
            values: serde_json::from_value(declared["efforts"].clone())
                .map_err(|_| "invalid declared efforts")?,
        }),
        Some("budget") => Ok(ReasoningCapabilityDeclaration::Budget {
            tokens: serde_json::from_value(declared["budgets"].clone())
                .map_err(|_| "invalid declared budgets")?,
        }),
        _ => Err("invalid reasoning control".into()),
    }
}

pub fn resolve_model_reasoning(
    declared: &Value,
    protocol: ModelProtocol,
    output_limit: u64,
    supports_reasoning: bool,
    requested: Option<&Value>,
) -> Result<Option<ReasoningSelection>, String> {
    let mut effective = declared.clone();
    if requested.is_some() {
        if let Some(object) = effective.as_object_mut() {
            object.remove("defaultPreference");
        }
    }
    // Declared tiers are model capabilities, not budgets all executed this turn.
    // Validate their schema/order here; only the selected budget must fit the
    // effective output limit below. Editors validate the full model defaults.
    crate::validate_model_reasoning(&effective, protocol.as_str(), u64::MAX)?;
    let selection = if let Some(value) = requested {
        serde_json::from_value::<Option<ReasoningSelection>>(value.clone())
            .map_err(|error| format!("invalid reasoning_selection: {error}"))?
    } else {
        model_reasoning_default_selection(declared, protocol)?
    };
    if let Some(selection) = &selection {
        selection.validate(output_limit)?;
        if !supports_reasoning {
            return Err("Selected model does not declare reasoning support".into());
        }
        match selection {
            ReasoningSelection::Effort { value } => {
                if declared["control"] != "effort"
                    || !declared["efforts"]
                        .as_array()
                        .is_some_and(|values| values.iter().any(|v| v.as_str() == Some(value)))
                {
                    return Err("Selected effort is not declared by this model profile".into());
                }
            }
            ReasoningSelection::Budget { tokens } => {
                if protocol != ModelProtocol::Anthropic
                    || declared["control"] != "budget"
                    || !declared["budgets"]
                        .as_array()
                        .is_some_and(|values| values.iter().any(|v| v.as_u64() == Some(*tokens)))
                {
                    return Err(
                        "Selected thinking budget is not declared by this Anthropic model profile"
                            .into(),
                    );
                }
            }
        }
    }
    Ok(selection)
}

/// Translate stored preference syntax while assembling sparse overrides.
/// This does not authorize execution: callers must validate the final merged
/// configuration with `resolve_model_reasoning`, including its output limit.
pub fn model_reasoning_default_selection(
    declared: &Value,
    protocol: ModelProtocol,
) -> Result<Option<ReasoningSelection>, String> {
    let Some(preference) = declared
        .get("defaultPreference")
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    let preference = preference
        .as_str()
        .ok_or("defaultPreference must be a string or null")?;
    if preference == "automatic" {
        return Ok(None);
    }
    if (protocol == ModelProtocol::Anthropic && declared["control"] == "budget")
        || (protocol == ModelProtocol::ChatCompletions && ["concise", "balanced", "detailed"].contains(&preference)) {
        let tier: ReasoningTier = serde_json::from_value(Value::String(preference.into()))
            .map_err(|_| "invalid default thinking tier")?;
        let declaration = active_reasoning_declaration(declared)?;
        if matches!(declaration, ReasoningCapabilityDeclaration::Automatic {}) {
            return Err("default thinking selection is incompatible with the control".into());
        }
        return resolve_reasoning_tier(protocol, &declaration, tier, u64::MAX)
            .map_err(|error| error.to_string());
    }
    match declared["control"].as_str() {
        Some("effort") => {
            let value = preference;
            if !declared["efforts"]
                .as_array()
                .is_some_and(|values| values.iter().any(|item| item.as_str() == Some(value)))
            {
                return Err("default thinking effort is not supported by this model".into());
            }
            Ok(Some(ReasoningSelection::Effort {
                value: value.into(),
            }))
        }
        _ => Err("default thinking selection is incompatible with the control".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sparse_chat_defaults_accept_native_efforts_and_keep_published_tiers_readable() {
        for (preference, expected) in [("none", "none"), ("high", "high"), ("max", "max"), ("balanced", "low")] {
            let declared = serde_json::json!({"control":"effort","efforts":["none","low","high","max"],"defaultPreference":preference});
            assert_eq!(resolve_model_reasoning(&declared, ModelProtocol::ChatCompletions, 8192, true, None).unwrap(),
                Some(ReasoningSelection::Effort { value: expected.into() }));
        }
        let declared = serde_json::json!({"control":"effort","efforts":["low","high"],"defaultPreference":"medium"});
        assert!(resolve_model_reasoning(&declared, ModelProtocol::ChatCompletions, 8192, true, None).is_err());
        let automatic = serde_json::json!({"control":"effort","efforts":["none","low","high"],"defaultPreference":"automatic"});
        assert_eq!(resolve_model_reasoning(&automatic, ModelProtocol::ChatCompletions, 8192, true, None).unwrap(), None);
    }

    #[test]
    fn inheritance_clear_and_explicit_selection_are_distinct() {
        for protocol in [ModelProtocol::Responses, ModelProtocol::SiwcResponses] {
            let mut declared =
                json!({"control":"effort","efforts":["low","high"],"defaultPreference":"high"});
            let resolve = |value: &Value, request: Option<&Value>| {
                resolve_model_reasoning(value, protocol, 16384, true, request)
            };
            assert_eq!(
                resolve(&declared, None).unwrap(),
                Some(ReasoningSelection::Effort {
                    value: "high".into()
                })
            );
            assert_eq!(resolve(&declared, Some(&Value::Null)).unwrap(), None);
            declared["defaultPreference"] = json!("unsupported");
            assert!(resolve(&declared, None).is_err());
            assert!(resolve(&declared, Some(&json!({"kind":"effort","value":"low"}))).is_ok());
            assert_eq!(resolve(&declared, Some(&Value::Null)).unwrap(), None);
        }
    }

    #[test]
    fn inactive_metadata_does_not_authorize_reasoning() {
        let declared = json!({"control":"automatic","efforts":["low","medium","high"]});
        assert!(resolve_model_reasoning(
            &declared,
            ModelProtocol::Responses,
            16384,
            true,
            Some(&json!({"kind":"effort","value":"low"}))
        )
        .is_err());
    }

    #[test]
    fn tier_mapping_and_budget_limits_share_one_boundary() {
        let declared =
            json!({"control":"budget","budgets":[1024,4096,8192],"defaultPreference":"detailed"});
        assert_eq!(
            resolve_model_reasoning(&declared, ModelProtocol::Anthropic, 16384, true, None)
                .unwrap(),
            Some(ReasoningSelection::Budget { tokens: 8192 })
        );
        assert!(
            resolve_model_reasoning(&declared, ModelProtocol::Anthropic, 4096, true, None).is_err()
        );
        assert!(
            resolve_model_reasoning(&declared, ModelProtocol::Responses, 16384, true, None)
                .is_err()
        );
        assert!(
            resolve_model_reasoning(&declared, ModelProtocol::Anthropic, 16384, false, None)
                .is_err()
        );
        let declared = json!({"control":"effort","efforts":["low","medium","high"],"defaultPreference":"balanced"});
        assert_eq!(
            resolve_model_reasoning(&declared, ModelProtocol::ChatCompletions, 16384, true, None)
                .unwrap(),
            Some(ReasoningSelection::Effort {
                value: "medium".into()
            })
        );
    }
}
