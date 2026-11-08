// SPDX-License-Identifier: Apache-2.0
use nola_domain::generation::{
    resolve_output_limit, resolve_output_token_limit, OutputTokenAdmissionError,
    OutputTokenLimitSource, OutputTokenLimits, OutputTokenPreference, ResolvedOutputLimit,
    DEFAULT_OUTPUT_TOKEN_LIMIT, MAX_OUTPUT_TOKEN_LIMIT, MIN_OUTPUT_TOKEN_LIMIT,
};
use serde_json::json;

#[test]
fn window_admission_adapts_only_defaults_and_keeps_explicit_preferences_exact() {
    use nola_domain::generation::resolve_output_limit_with_window;
    let default = resolve_output_limit_with_window(OutputTokenPreference::Default, limits(None), Some(16_000)).unwrap();
    assert_eq!(default.tokens, 5_600);
    assert_eq!(default.source, OutputTokenLimitSource::ContextWindowDefault);
    let explicit = resolve_output_limit_with_window(OutputTokenPreference::Explicit(8_000), limits(None), Some(16_000)).unwrap();
    assert_eq!(explicit.tokens, 8_000);
    assert_eq!(explicit.source, OutputTokenLimitSource::ExplicitPreference);
    assert!(matches!(resolve_output_limit_with_window(OutputTokenPreference::Explicit(16_384), limits(None), Some(16_000)), Err(OutputTokenAdmissionError::ContextWindowConflict { .. })));
    assert_eq!(resolve_output_limit_with_window(OutputTokenPreference::Default, limits(Some(2_048)), Some(16_000)).unwrap().tokens, 2_048);
    assert_eq!(resolve_output_limit_with_window(OutputTokenPreference::Default, limits(None), Some(262_144)).unwrap().tokens, DEFAULT_OUTPUT_TOKEN_LIMIT);
}

fn limits(cap: Option<u64>) -> OutputTokenLimits {
    OutputTokenLimits {
        known_output_cap: cap,
    }
}

#[test]
fn unknown_capability_does_not_invent_a_cap_or_modify_explicit_preferences() {
    for tokens in [64, 256, 32768, 32769, 64000, MAX_OUTPUT_TOKEN_LIMIT] {
        assert_eq!(
            resolve_output_limit(OutputTokenPreference::Explicit(tokens), limits(None)),
            Ok(ResolvedOutputLimit {
                tokens,
                source: OutputTokenLimitSource::ExplicitPreference
            })
        );
    }
    assert_eq!(
        resolve_output_limit(OutputTokenPreference::Default, limits(None)),
        Ok(ResolvedOutputLimit {
            tokens: DEFAULT_OUTPUT_TOKEN_LIMIT,
            source: OutputTokenLimitSource::ProductDefault
        })
    );
}

#[test]
fn explicit_values_are_exact_at_cap_and_rejected_above_cap() {
    for cap in [64, 8192, 16384, 32768, 64000] {
        let admitted =
            resolve_output_limit(OutputTokenPreference::Explicit(cap), limits(Some(cap))).unwrap();
        assert_eq!(admitted.tokens, cap);
        assert_eq!(admitted.source, OutputTokenLimitSource::ExplicitPreference);
        assert_eq!(
            resolve_output_limit(OutputTokenPreference::Explicit(cap + 1), limits(Some(cap))),
            Err(OutputTokenAdmissionError::ExceedsKnownCapability {
                requested: cap + 1,
                cap
            })
        );
    }
    assert_eq!(
        resolve_output_limit(OutputTokenPreference::Explicit(64000), limits(Some(8192))),
        Err(OutputTokenAdmissionError::ExceedsKnownCapability {
            requested: 64000,
            cap: 8192
        })
    );
}

#[test]
fn only_absent_preference_uses_capability_bound_default_and_records_its_source() {
    for cap in [MIN_OUTPUT_TOKEN_LIMIT, 8192, DEFAULT_OUTPUT_TOKEN_LIMIT - 1] {
        assert_eq!(
            resolve_output_limit(OutputTokenPreference::Default, limits(Some(cap))),
            Ok(ResolvedOutputLimit {
                tokens: cap,
                source: OutputTokenLimitSource::KnownCapabilityDefault
            })
        );
    }
    for cap in [DEFAULT_OUTPUT_TOKEN_LIMIT, 64000, u64::MAX] {
        assert_eq!(
            resolve_output_limit(OutputTokenPreference::Default, limits(Some(cap))),
            Ok(ResolvedOutputLimit {
                tokens: DEFAULT_OUTPUT_TOKEN_LIMIT,
                source: OutputTokenLimitSource::ProductDefault
            })
        );
    }
}

#[test]
fn impossible_capabilities_are_errors_not_unknown_or_a_raised_limit() {
    for cap in [0, 1, MIN_OUTPUT_TOKEN_LIMIT - 1] {
        for preference in [
            OutputTokenPreference::Default,
            OutputTokenPreference::Explicit(64000),
        ] {
            assert_eq!(
                resolve_output_limit(preference, limits(Some(cap))),
                Err(OutputTokenAdmissionError::InvalidKnownCapability { cap })
            );
        }
    }
}

#[test]
fn invalid_explicit_values_do_not_become_defaults_even_with_capability_metadata() {
    for tokens in [0, 1, 63, MAX_OUTPUT_TOKEN_LIMIT + 1, u64::MAX] {
        for cap in [None, Some(64000), Some(u64::MAX)] {
            assert_eq!(
                resolve_output_limit(OutputTokenPreference::Explicit(tokens), limits(cap)),
                Err(OutputTokenAdmissionError::InvalidPreference { tokens })
            );
        }
    }
    assert_eq!(
        OutputTokenPreference::from(None),
        OutputTokenPreference::Default
    );
    assert_eq!(
        OutputTokenPreference::from(Some(0)),
        OutputTokenPreference::Explicit(0)
    );
}

#[test]
fn schema_only_entry_point_delegates_to_unknown_capability_admission() {
    for preference in [
        None,
        Some(0),
        Some(64),
        Some(64000),
        Some(MAX_OUTPUT_TOKEN_LIMIT + 1),
    ] {
        assert_eq!(
            resolve_output_token_limit(preference),
            resolve_output_limit(preference.into(), limits(None))
                .map(|value| value.tokens)
                .map_err(|error| error.to_string())
        );
    }
}

#[test]
fn admission_values_and_provenance_have_a_typed_snapshot_contract() {
    assert_eq!(
        serde_json::to_value(OutputTokenPreference::Default).unwrap(),
        json!("default")
    );
    assert_eq!(
        serde_json::to_value(OutputTokenPreference::Explicit(64000)).unwrap(),
        json!({"explicit":64000})
    );
    for preference in [
        OutputTokenPreference::Default,
        OutputTokenPreference::Explicit(0),
        OutputTokenPreference::Explicit(64000),
    ] {
        assert_eq!(
            serde_json::from_value::<OutputTokenPreference>(
                serde_json::to_value(preference).unwrap()
            )
            .unwrap(),
            preference
        );
    }
    for cap in [None, Some(8192), Some(u64::MAX)] {
        let facts = limits(cap);
        assert_eq!(
            serde_json::from_value::<OutputTokenLimits>(serde_json::to_value(facts).unwrap())
                .unwrap(),
            facts
        );
    }
    let resolved =
        resolve_output_limit(OutputTokenPreference::Default, limits(Some(8192))).unwrap();
    assert_eq!(
        serde_json::to_value(resolved).unwrap(),
        json!({"tokens":8192,"source":"known_capability_default"})
    );
    assert_eq!(
        serde_json::from_value::<ResolvedOutputLimit>(serde_json::to_value(resolved).unwrap())
            .unwrap(),
        resolved
    );
    assert!(serde_json::from_value::<OutputTokenLimits>(
        json!({"known_output_cap":64000,"context_window_tokens":128000})
    )
    .is_err());
    assert!(serde_json::from_value::<OutputTokenLimits>(json!({"known_output_cap":-1})).is_err());
    assert!(serde_json::from_value::<OutputTokenPreference>(json!({"explicit":1.5})).is_err());
    assert!(serde_json::from_value::<OutputTokenPreference>(
        json!({"default":null,"unexpected":true})
    )
    .is_err());
    assert!(serde_json::from_value::<ResolvedOutputLimit>(
        json!({"tokens":8192,"source":"guessed_from_model_name"})
    )
    .is_err());
}

#[test]
fn rejection_preserves_the_failed_preference_and_actual_capability() {
    let error = resolve_output_limit(OutputTokenPreference::Explicit(64000), limits(Some(8192)))
        .unwrap_err();
    let encoded = serde_json::to_value(error).unwrap();
    assert_eq!(
        encoded,
        json!({"type":"exceeds_known_capability","requested":64000,"cap":8192})
    );
    assert_eq!(
        serde_json::from_value::<OutputTokenAdmissionError>(encoded).unwrap(),
        error
    );
    assert!(error.to_string().contains("64000"));
    assert!(error.to_string().contains("8192"));
    assert!(serde_json::from_value::<OutputTokenAdmissionError>(json!({
        "type":"exceeds_known_capability","requested":64000,"cap":8192,"override":true
    }))
    .is_err());
}
