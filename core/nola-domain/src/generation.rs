// SPDX-License-Identifier: Apache-2.0
//! Provider-neutral generation values and validation, with no runtime or IO.
//! Response presentation is a separate product preference.
use serde::{Deserialize, Serialize};

pub const DEFAULT_OUTPUT_TOKEN_LIMIT: u64 = 16_384;
pub const MIN_OUTPUT_TOKEN_LIMIT: u64 = 64;
pub const MAX_OUTPUT_TOKEN_LIMIT: u64 = 1_000_000;

/// Absence is a product default, not an explicit zero or a context reservation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputTokenPreference {
    #[default]
    Default,
    Explicit(u64),
}

impl From<Option<u64>> for OutputTokenPreference {
    fn from(value: Option<u64>) -> Self {
        value.map(Self::Explicit).unwrap_or(Self::Default)
    }
}

/// A known capability is supplied by the exact route's admitted metadata.
/// Unknown is `None`; zero is invalid metadata and must never mean unknown.
/// Context-window estimates and prompt reservation are deliberately absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputTokenLimits {
    pub known_output_cap: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputTokenLimitSource {
    ExplicitPreference,
    ProductDefault,
    KnownCapabilityDefault,
    ContextWindowDefault,
}

/// The value to freeze for execution, with why that value was selected.
/// Route identity, revision, and per-route capability evidence belong to the
/// caller's admission snapshot, not an unqualified capability inferred here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedOutputLimit {
    pub tokens: u64,
    pub source: OutputTokenLimitSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputTokenAdmissionError {
    InvalidPreference { tokens: u64 },
    InvalidKnownCapability { cap: u64 },
    ExceedsKnownCapability { requested: u64, cap: u64 },
    ContextWindowConflict { window: u64, output: u64 },
}

impl std::fmt::Display for OutputTokenAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContextWindowConflict { window, output } => write!(formatter,
                "context_output_budget_conflict: window={window}, output={output}; insufficient input space"),
            Self::InvalidPreference { tokens } => write!(formatter,
                "invalid model output limit {tokens}: expected {MIN_OUTPUT_TOKEN_LIMIT}..={MAX_OUTPUT_TOKEN_LIMIT}"),
            Self::InvalidKnownCapability { cap } => write!(formatter,
                "known model output capability {cap} cannot accommodate the minimum output limit {MIN_OUTPUT_TOKEN_LIMIT}"),
            Self::ExceedsKnownCapability { requested, cap } => write!(formatter,
                "requested model output limit {requested} exceeds the known capability {cap}"),
        }
    }
}

impl std::error::Error for OutputTokenAdmissionError {}

/// Resolve once before freezing the request. Defaults leave the ordinary 60%
/// working set plus safety margin; explicit preferences are never rewritten.
pub fn resolve_output_limit_with_window(
    preference: OutputTokenPreference,
    limits: OutputTokenLimits,
    window: Option<u64>,
) -> Result<ResolvedOutputLimit, OutputTokenAdmissionError> {
    let mut resolved = resolve_output_limit(preference, limits)?;
    if let Some(window) = window {
        let margin = (window / 20).clamp(512, 8_192);
        if preference == OutputTokenPreference::Default {
            let input_target = window.saturating_mul(60) / 100;
            let allowance = window.saturating_sub(margin).saturating_sub(input_target);
            if allowance < resolved.tokens {
                resolved.tokens = allowance;
                resolved.source = OutputTokenLimitSource::ContextWindowDefault;
            }
        }
        if resolved.tokens < MIN_OUTPUT_TOKEN_LIMIT
            || window.saturating_sub(margin).saturating_sub(resolved.tokens) < 512
        {
            return Err(OutputTokenAdmissionError::ContextWindowConflict {
                window, output: resolved.tokens,
            });
        }
    }
    Ok(resolved)
}

/// Resolve a single admitted output limit. Explicit preferences are exact or
/// rejected; only an absent preference may choose a lower capability-bound
/// default. A provider capability above the product input schema is valid
/// metadata, but does not expand the product's accepted explicit-value range.
pub fn resolve_output_limit(
    preference: OutputTokenPreference,
    limits: OutputTokenLimits,
) -> Result<ResolvedOutputLimit, OutputTokenAdmissionError> {
    if let OutputTokenPreference::Explicit(tokens) = preference {
        if !(MIN_OUTPUT_TOKEN_LIMIT..=MAX_OUTPUT_TOKEN_LIMIT).contains(&tokens) {
            return Err(OutputTokenAdmissionError::InvalidPreference { tokens });
        }
    }
    if let Some(cap) = limits.known_output_cap {
        if cap < MIN_OUTPUT_TOKEN_LIMIT {
            return Err(OutputTokenAdmissionError::InvalidKnownCapability { cap });
        }
    }
    match preference {
        OutputTokenPreference::Explicit(tokens) => {
            if let Some(cap) = limits.known_output_cap {
                if tokens > cap {
                    return Err(OutputTokenAdmissionError::ExceedsKnownCapability {
                        requested: tokens, cap,
                    });
                }
            }
            Ok(ResolvedOutputLimit { tokens, source: OutputTokenLimitSource::ExplicitPreference })
        }
        OutputTokenPreference::Default => {
            if let Some(cap) = limits.known_output_cap.filter(|cap| *cap < DEFAULT_OUTPUT_TOKEN_LIMIT) {
                Ok(ResolvedOutputLimit { tokens: cap, source: OutputTokenLimitSource::KnownCapabilityDefault })
            } else {
                Ok(ResolvedOutputLimit { tokens: DEFAULT_OUTPUT_TOKEN_LIMIT, source: OutputTokenLimitSource::ProductDefault })
            }
        }
    }
}

/// Resolve an absent preference once at admission. Explicit invalid values are
/// rejected, never replaced by defaults or context-reservation heuristics.
pub fn resolve_output_token_limit(requested: Option<u64>) -> Result<u64, String> {
    resolve_output_limit(requested.into(), OutputTokenLimits::default())
        .map(|resolved| resolved.tokens)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod output_limit_tests {
    use super::*;

    #[test]
    fn admission_defaults_only_absent_limits_and_preserves_explicit_values() {
        assert_eq!(resolve_output_token_limit(None).unwrap(), 16_384);
        for value in [64, 256, 32_768, 64_000, 1_000_000] {
            assert_eq!(resolve_output_token_limit(Some(value)).unwrap(), value);
        }
        for value in [0, 63, 1_000_001, u64::MAX] {
            assert!(resolve_output_token_limit(Some(value)).is_err());
        }
    }
}

/// Wire identity of the resolved route, separate from generation preferences.
/// Provider names and transport construction remain runtime responsibilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelProtocol {
    ChatCompletions,
    Responses,
    SiwcResponses,
    Anthropic,
}

impl ModelProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
            Self::SiwcResponses => "siwc_responses",
            Self::Anthropic => "anthropic",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "chat_completions" => Ok(Self::ChatCompletions),
            "responses" => Ok(Self::Responses),
            "siwc_responses" => Ok(Self::SiwcResponses),
            "anthropic" => Ok(Self::Anthropic),
            _ => Err("unsupported model_protocol".into()),
        }
    }
}

/// Known wire values are a schema vocabulary, not a claim that a model supports
/// every value. Route-specific capability validation is still required.
pub const REASONING_EFFORT_VALUES: &[&str] =
    &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

pub fn validate_thinking_budget(tokens: u64, max_output_tokens: u64) -> Result<(), String> {
    if tokens >= 1024 && tokens < max_output_tokens {
        Ok(())
    } else {
        Err("thinking budget must be at least 1024 and below the output limit".into())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repetition_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

impl GenerationOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.top_k.is_some_and(|v| !(1..=1_000_000).contains(&v))
            || [self.presence_penalty, self.frequency_penalty]
                .into_iter()
                .flatten()
                .any(|v| !(-2.0..=2.0).contains(&v))
            || self
                .repetition_penalty
                .is_some_and(|v| !(0.8..=1.4).contains(&v))
            || self
                .stop
                .as_ref()
                .is_some_and(|v| v.len() > 16 || v.iter().any(|s| s.is_empty() || s.len() > 2048))
        {
            return Err("invalid generation parameters".into());
        }
        Ok(())
    }
}

/// Validated canonical representation of the existing generation contract.
/// The private representation prevents unvalidated floating-point values from
/// entering an Eq execution identity; it is not a second options schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ValidatedGenerationOptions(serde_json::Value);

impl TryFrom<GenerationOptions> for ValidatedGenerationOptions {
    type Error = String;

    fn try_from(options: GenerationOptions) -> Result<Self, Self::Error> {
        options.validate()?;
        serde_json::to_value(options).map(Self).map_err(|error| error.to_string())
    }
}

impl ValidatedGenerationOptions {
    pub fn to_options(&self) -> GenerationOptions {
        // Every constructor validates and serializes this exact Domain type.
        serde_json::from_value(self.0.clone()).expect("validated generation options invariant")
    }
}

impl<'de> Deserialize<'de> for ValidatedGenerationOptions {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let options = GenerationOptions::deserialize(deserializer)?;
        Self::try_from(options).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReasoningSelection {
    Effort { value: String },
    Budget { tokens: u64 },
}

impl ReasoningSelection {
    pub fn enables_thinking(&self) -> bool {
        !matches!(self, Self::Effort { value } if value == "none")
    }

    pub fn validate(&self, max_output_tokens: u64) -> Result<(), String> {
        match self {
            Self::Effort { value } if REASONING_EFFORT_VALUES.contains(&value.as_str()) => Ok(()),
            Self::Budget { tokens } => validate_thinking_budget(*tokens, max_output_tokens),
            _ => Err("invalid reasoning selection: effort must be recognized".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validated_options_are_canonical_and_lossless() {
        let a: ValidatedGenerationOptions = serde_json::from_str(
            r#"{"seed":0,"presence_penalty":0,"top_k":1,"stop":[]}"#).unwrap();
        let b: ValidatedGenerationOptions = serde_json::from_str(
            r#"{"stop":[],"top_k":1,"presence_penalty":0.0,"seed":0}"#).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.to_options().seed, Some(0));
        assert_eq!(a.to_options().stop, Some(vec![]));
        assert_ne!(a, GenerationOptions::default().try_into().unwrap());
        let encoded = serde_json::to_vec(&a).unwrap();
        assert_eq!(a, serde_json::from_slice(&encoded).unwrap());
        for invalid in [r#"{"unknown":1}"#, r#"{"top_k":0}"#, r#"{"seed":1.5}"#, "null"] {
            assert!(serde_json::from_str::<ValidatedGenerationOptions>(invalid).is_err());
        }
        assert!(ValidatedGenerationOptions::try_from(GenerationOptions {
            presence_penalty: Some(f64::NAN), ..Default::default()
        }).is_err());
    }
    #[test]
    fn protocol_identity_has_one_closed_wire_vocabulary() {
        for protocol in [
            ModelProtocol::ChatCompletions,
            ModelProtocol::Responses,
            ModelProtocol::Anthropic,
        ] {
            let encoded = serde_json::to_value(protocol).unwrap();
            assert_eq!(
                ModelProtocol::parse(encoded.as_str().unwrap()).unwrap(),
                protocol
            );
            for invalid in [
                serde_json::Value::Null,
                serde_json::json!(1),
                serde_json::json!("unknown"),
            ] {
                assert!(serde_json::from_value::<ModelProtocol>(invalid).is_err());
            }
        }
    }
    #[test]
    fn budget_is_not_an_output_limit() {
        assert!(ReasoningSelection::Budget { tokens: 1024 }
            .validate(4096)
            .is_ok());
        assert!(ReasoningSelection::Budget { tokens: 4096 }
            .validate(4096)
            .is_err());
        assert!(ReasoningSelection::Budget { tokens: 0 }
            .validate(4096)
            .is_err());
    }
    #[test]
    fn rejects_unrecognized_effort_and_fields() {
        assert!(ReasoningSelection::Effort {
            value: "detailed".into()
        }
        .validate(4096)
        .is_err());
        assert!(serde_json::from_str::<ReasoningSelection>(
            r#"{"kind":"effort","value":"high","tokens":2048}"#
        )
        .is_err());
    }

    #[test]
    fn generation_rejects_non_finite_values_and_invalid_limits() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 2.01] {
            assert!(GenerationOptions {
                presence_penalty: Some(value),
                ..Default::default()
            }
            .validate()
            .is_err());
        }
        assert!(GenerationOptions {
            top_k: Some(0),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(GenerationOptions {
            stop: Some(vec![String::new()]),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(GenerationOptions {
            seed: Some(0),
            top_k: Some(1),
            presence_penalty: Some(0.0),
            ..Default::default()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn reasoning_contract_preserves_wire_identity() {
        let input = r#"{"kind":"effort","value":"high"}"#;
        let selection: ReasoningSelection = serde_json::from_str(input).unwrap();
        assert_eq!(
            serde_json::to_value(selection).unwrap(),
            serde_json::from_str::<serde_json::Value>(input).unwrap()
        );
    }
}
