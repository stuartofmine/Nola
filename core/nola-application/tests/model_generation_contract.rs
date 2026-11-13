// SPDX-License-Identifier: Apache-2.0
//! Public admission boundary: inheritance, explicit clearing and route capability.

use nola_application::resolve_model_reasoning;
use nola_domain::generation::{ModelProtocol, ReasoningSelection};
use serde_json::{json, Value};

fn effort(value: &str) -> Option<ReasoningSelection> {
    Some(ReasoningSelection::Effort {
        value: value.into(),
    })
}

#[test]
fn native_query_resolves_tiers_and_rejects_ambiguous_inputs() {
    use nola_application::resolve_model_reasoning_request;
    use nola_domain::reasoning_tiers::ReasoningTier;
    let declared = json!({"control":"effort","efforts":["low","high"],"defaultPreference":"invalid-but-shadowed"});
    assert_eq!(resolve_model_reasoning_request(&declared, ModelProtocol::Responses,
        16384, true, &Value::Null, Some(ReasoningTier::Balanced)).unwrap(),
        json!({"selection":{"kind":"effort","value":"low"}}));
    assert!(resolve_model_reasoning_request(&declared, ModelProtocol::Responses,
        16384, true, &json!({"kind":"effort","value":"high"}), Some(ReasoningTier::Balanced)).is_err());
    assert!(resolve_model_reasoning_request(&declared, ModelProtocol::Responses,
        16384, false, &Value::Null, Some(ReasoningTier::Balanced)).is_err());
    for limit in [0, 63, 1_000_001] {
        assert!(resolve_model_reasoning_request(&Value::Null, ModelProtocol::Responses,
            limit, false, &Value::Null, None).is_err());
    }
    assert_eq!(resolve_model_reasoning_request(&Value::Null, ModelProtocol::ChatCompletions,
        4096, false, &Value::Null, Some(ReasoningTier::Detailed)).unwrap(), json!({"selection":null}));
}

#[test]
fn missing_inherits_but_null_clears_across_all_protocols() {
    for (protocol, declared, expected) in [
        (
            ModelProtocol::Responses,
            json!({"control":"effort","efforts":["low","high"],"defaultPreference":"high"}),
            effort("high"),
        ),
        (
            ModelProtocol::ChatCompletions,
            json!({"control":"effort","efforts":["low","medium","high"],"defaultPreference":"balanced"}),
            effort("medium"),
        ),
        (
            ModelProtocol::Anthropic,
            json!({"control":"budget","budgets":[1024,4096,8192],"defaultPreference":"detailed"}),
            Some(ReasoningSelection::Budget { tokens: 8192 }),
        ),
    ] {
        assert_eq!(
            resolve_model_reasoning(&declared, protocol, 16384, true, None).unwrap(),
            expected
        );
        assert_eq!(
            resolve_model_reasoning(&declared, protocol, 16384, true, Some(&Value::Null)).unwrap(),
            None
        );
    }
}

#[test]
fn explicit_selection_replaces_invalid_default_without_mutating_profile() {
    for invalid_default in [json!("unsupported"), json!(42), json!({"kind":"effort"})] {
        let declared = json!({"control":"effort","efforts":["low","high"],"defaultPreference":invalid_default});
        let original = declared.clone();
        assert!(
            resolve_model_reasoning(&declared, ModelProtocol::Responses, 16384, true, None)
                .is_err()
        );
        assert_eq!(
            resolve_model_reasoning(
                &declared,
                ModelProtocol::Responses,
                16384,
                true,
                Some(&json!({"kind":"effort","value":"low"}))
            )
            .unwrap(),
            effort("low")
        );
        assert_eq!(
            resolve_model_reasoning(
                &declared,
                ModelProtocol::Responses,
                16384,
                true,
                Some(&Value::Null)
            )
            .unwrap(),
            None
        );
        assert_eq!(declared, original);
    }
}

#[test]
fn malformed_or_unknown_request_fields_never_silently_inherit() {
    let declared = json!({"control":"effort","efforts":["low","high"],"defaultPreference":"high"});
    for request in [
        json!(false),
        json!("low"),
        json!(0),
        json!([]),
        json!({}),
        json!({"kind":"effort","value":5}),
        json!({"kind":"effort","value":"low","tokens":1024}),
        json!({"kind":"effort","value":"unknown"}),
        json!({"kind":"effort","value":"medium"}),
        json!({"kind":"budget","tokens":1024.5}),
        json!({"kind":"budget","tokens":-1}),
        json!({"kind":"budget","tokens":"1024"}),
        json!({"kind":"other","value":"low"}),
    ] {
        assert!(
            resolve_model_reasoning(
                &declared,
                ModelProtocol::Responses,
                16384,
                true,
                Some(&request)
            )
            .is_err(),
            "accepted malformed selection: {request}"
        );
    }
}

#[test]
fn automatic_or_absent_control_and_unsupported_capability_do_not_authorize_reasoning() {
    for declared in [
        Value::Null,
        json!({"control":"automatic","efforts":["low","medium","high"],"budgets":[1024,4096,8192]}),
    ] {
        for protocol in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::Anthropic,
        ] {
            assert_eq!(
                resolve_model_reasoning(&declared, protocol, 16384, false, None).unwrap(),
                None
            );
            for request in [
                json!({"kind":"effort","value":"low"}),
                json!({"kind":"budget","tokens":1024}),
            ] {
                assert!(
                    resolve_model_reasoning(&declared, protocol, 16384, true, Some(&request))
                        .is_err()
                );
            }
        }
    }
    let declared = json!({"control":"effort","efforts":["low","high"],"defaultPreference":"high"});
    assert!(
        resolve_model_reasoning(&declared, ModelProtocol::Responses, 16384, false, None).is_err()
    );
    assert!(resolve_model_reasoning(
        &declared,
        ModelProtocol::Responses,
        16384,
        false,
        Some(&json!({"kind":"effort","value":"low"}))
    )
    .is_err());
    assert_eq!(
        resolve_model_reasoning(
            &declared,
            ModelProtocol::Responses,
            16384,
            false,
            Some(&Value::Null)
        )
        .unwrap(),
        None
    );
}

#[test]
fn inactive_metadata_is_still_schema_checked() {
    for fields in [
        json!({"efforts":"low"}),
        json!({"budgets":["1024"]}),
        json!({"responses":"true"}),
        json!({"outputLimit":"16384"}),
        json!({"outputLimit":-1}),
        json!({"outputLimit":16384.5}),
        json!({"unexpected":true}),
    ] {
        let mut declared = json!({"control":"automatic"});
        declared
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        assert!(
            resolve_model_reasoning(&declared, ModelProtocol::Responses, 16384, true, None)
                .is_err(),
            "accepted malformed declaration: {declared}"
        );
    }
}

#[test]
fn budget_requires_anthropic_declared_integer_and_strict_output_headroom() {
    let declared =
        json!({"control":"budget","budgets":[1024,4096,8192],"defaultPreference":"detailed"});
    assert_eq!(
        resolve_model_reasoning(&declared, ModelProtocol::Anthropic, 8193, true, None).unwrap(),
        Some(ReasoningSelection::Budget { tokens: 8192 })
    );
    assert!(
        resolve_model_reasoning(&declared, ModelProtocol::Anthropic, 8192, true, None).is_err()
    );
    for protocol in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
        assert!(resolve_model_reasoning(&declared, protocol, 16384, true, None).is_err());
    }
    for tokens in [0, 1023, 2048, 16384] {
        assert!(resolve_model_reasoning(
            &declared,
            ModelProtocol::Anthropic,
            16384,
            true,
            Some(&json!({"kind":"budget","tokens":tokens}))
        )
        .is_err());
    }
    let efforts = json!({"control":"effort","efforts":["low","medium","high"]});
    assert_eq!(resolve_model_reasoning(
        &efforts, ModelProtocol::Anthropic, 16384, true,
        Some(&json!({"kind":"effort","value":"low"}))
    ).unwrap(), effort("low"));
}

#[test]
fn turn_output_limit_constrains_selected_budget_not_unselected_tiers() {
    let declared =
        json!({"control":"budget","budgets":[1024,4096,8192],"defaultPreference":"detailed"});
    assert!(
        resolve_model_reasoning(&declared, ModelProtocol::Anthropic, 4096, true, None).is_err()
    );
    assert_eq!(
        resolve_model_reasoning(
            &declared,
            ModelProtocol::Anthropic,
            4096,
            true,
            Some(&json!({"kind":"budget","tokens":1024}))
        )
        .unwrap(),
        Some(ReasoningSelection::Budget { tokens: 1024 })
    );
    assert_eq!(
        resolve_model_reasoning(
            &declared,
            ModelProtocol::Anthropic,
            4096,
            true,
            Some(&Value::Null)
        )
        .unwrap(),
        None
    );
    assert!(resolve_model_reasoning(
        &declared,
        ModelProtocol::Anthropic,
        4096,
        true,
        Some(&json!({"kind":"budget","tokens":4096}))
    )
    .is_err());
    assert!(resolve_model_reasoning(
        &declared,
        ModelProtocol::Anthropic,
        4096,
        true,
        Some(&json!({"kind":"budget","tokens":8192}))
    )
    .is_err());
}

#[test]
fn native_effort_has_exact_defaults_and_keeps_budget_controls_distinct() {
    let declaration = json!({"control":"effort","efforts":["none","low","high","max"],"defaultPreference":"high"});
    assert_eq!(resolve_model_reasoning(&declaration, ModelProtocol::Anthropic, 8192, true, None).unwrap(), effort("high"));
    assert_eq!(resolve_model_reasoning(&declaration, ModelProtocol::Anthropic, 8192, true, Some(&Value::Null)).unwrap(), None);
    assert_eq!(resolve_model_reasoning(&declaration, ModelProtocol::Anthropic, 8192, true, Some(&json!({"kind":"effort","value":"none"}))).unwrap(), effort("none"));
    assert!(resolve_model_reasoning(&declaration, ModelProtocol::Anthropic, 8192, true, Some(&json!({"kind":"effort","value":"medium"}))).is_err());
    assert!(resolve_model_reasoning(&declaration, ModelProtocol::Anthropic, 8192, true, Some(&json!({"kind":"budget","tokens":1024}))).is_err());
}
