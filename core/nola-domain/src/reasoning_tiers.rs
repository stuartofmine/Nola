// SPDX-License-Identifier: Apache-2.0
//! Pure three-tier reasoning resolution from explicit route capabilities.
//! Presentation instructions and provider/model-name inference do not belong here.
use crate::generation::{
    validate_thinking_budget, ModelProtocol, ReasoningSelection, REASONING_EFFORT_VALUES,
};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningTier {
    Concise,
    #[default]
    Balanced,
    Detailed,
}

impl ReasoningTier {
    fn index(self) -> usize {
        match self {
            Self::Concise => 0,
            Self::Balanced => 1,
            Self::Detailed => 2,
        }
    }
}

/// Active capability declaration. Inactive editor drafts must be validated by
/// their configuration schema before constructing this domain value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReasoningCapabilityDeclaration {
    Automatic {},
    Effort { values: Vec<String> },
    Budget { tokens: Vec<u64> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningTierError {
    IncompatibleProtocol,
    EmptyEfforts,
    UnknownEffort(String),
    MissingChatTiers,
    InvalidBudgets,
    BudgetExceedsOutputLimit,
}

impl fmt::Display for ReasoningTierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompatibleProtocol => {
                formatter.write_str("reasoning control is incompatible with the effective protocol")
            }
            Self::EmptyEfforts => formatter.write_str("reasoning efforts must not be empty"),
            Self::UnknownEffort(value) => write!(formatter, "unknown reasoning effort: {value}"),
            Self::MissingChatTiers => formatter.write_str(
                "Chat Completions reasoning requires declared low, medium and high efforts",
            ),
            Self::InvalidBudgets => formatter
                .write_str("reasoning requires three increasing budgets, each at least 1024"),
            Self::BudgetExceedsOutputLimit => formatter
                .write_str("selected reasoning budget must be below the effective output limit"),
        }
    }
}

impl std::error::Error for ReasoningTierError {}

impl ReasoningCapabilityDeclaration {
    pub fn validate(&self, protocol: ModelProtocol) -> Result<(), ReasoningTierError> {
        match self {
            Self::Automatic {} => Ok(()),
            Self::Effort { values } => {
                if protocol == ModelProtocol::Anthropic {
                    return Err(ReasoningTierError::IncompatibleProtocol);
                }
                if values.is_empty() {
                    return Err(ReasoningTierError::EmptyEfforts);
                }
                for value in values {
                    if !REASONING_EFFORT_VALUES.contains(&value.as_str()) {
                        return Err(ReasoningTierError::UnknownEffort(value.clone()));
                    }
                }
                if protocol == ModelProtocol::ChatCompletions
                    && !["low", "medium", "high"]
                        .iter()
                        .all(|required| values.iter().any(|value| value == required))
                {
                    return Err(ReasoningTierError::MissingChatTiers);
                }
                Ok(())
            }
            Self::Budget { tokens } => {
                if protocol != ModelProtocol::Anthropic {
                    return Err(ReasoningTierError::IncompatibleProtocol);
                }
                if tokens.len() != 3
                    || tokens.windows(2).any(|pair| pair[0] >= pair[1])
                    || tokens.iter().any(|tokens| *tokens < 1024)
                {
                    return Err(ReasoningTierError::InvalidBudgets);
                }
                Ok(())
            }
        }
    }
}

/// Resolve only after the route and effective output limit have been frozen.
/// Responses uses the nearest declared effort to low/medium/high; a tie chooses
/// the lower effort, independently of declaration order. Automatic is omission,
/// not an explicit `none` effort. Invalid declarations never become Automatic.
pub fn resolve_reasoning_tier(
    protocol: ModelProtocol,
    declaration: &ReasoningCapabilityDeclaration,
    tier: ReasoningTier,
    max_output_tokens: u64,
) -> Result<Option<ReasoningSelection>, ReasoningTierError> {
    declaration.validate(protocol)?;
    Ok(match declaration {
        ReasoningCapabilityDeclaration::Automatic {} => None,
        ReasoningCapabilityDeclaration::Effort { values } => {
            let target = tier.index() + 2;
            let value = REASONING_EFFORT_VALUES
                .iter()
                .enumerate()
                .filter(|(_, effort)| values.iter().any(|value| value == *effort))
                .min_by_key(|(index, _)| (index.abs_diff(target), *index))
                .expect("validated nonempty recognized efforts")
                .1;
            Some(ReasoningSelection::Effort {
                value: (*value).to_owned(),
            })
        }
        ReasoningCapabilityDeclaration::Budget { tokens } => {
            let tokens = tokens[tier.index()];
            validate_thinking_budget(tokens, max_output_tokens)
                .map_err(|_| ReasoningTierError::BudgetExceedsOutputLimit)?;
            Some(ReasoningSelection::Budget { tokens })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIERS: [ReasoningTier; 3] = [
        ReasoningTier::Concise,
        ReasoningTier::Balanced,
        ReasoningTier::Detailed,
    ];

    fn efforts(values: &[&str]) -> ReasoningCapabilityDeclaration {
        ReasoningCapabilityDeclaration::Effort {
            values: values.iter().map(|value| (*value).into()).collect(),
        }
    }

    #[test]
    fn automatic_omits_reasoning_on_every_protocol() {
        for protocol in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::Anthropic,
        ] {
            for tier in TIERS {
                assert_eq!(
                    resolve_reasoning_tier(
                        protocol,
                        &ReasoningCapabilityDeclaration::Automatic {},
                        tier,
                        8192
                    ),
                    Ok(None)
                );
            }
        }
        assert_eq!(
            resolve_reasoning_tier(
                ModelProtocol::Responses,
                &efforts(&["none"]),
                ReasoningTier::Detailed,
                8192
            ),
            Ok(Some(ReasoningSelection::Effort {
                value: "none".into()
            }))
        );
    }

    #[test]
    fn sparse_responses_preserves_mac_lower_effort_tie_rule() {
        for values in [
            vec!["low", "high"],
            vec!["high", "low"],
            vec!["high", "low", "low"],
        ] {
            for (tier, expected) in TIERS.into_iter().zip(["low", "low", "high"]) {
                assert_eq!(
                    resolve_reasoning_tier(ModelProtocol::Responses, &efforts(&values), tier, 8192),
                    Ok(Some(ReasoningSelection::Effort {
                        value: expected.into()
                    }))
                );
            }
        }
    }

    #[test]
    fn every_responses_capability_subset_resolves_monotonically_without_invention() {
        for mask in 1usize..(1 << REASONING_EFFORT_VALUES.len()) {
            let values: Vec<&str> = REASONING_EFFORT_VALUES
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, value)| *value)
                .collect();
            let mut previous = 0;
            for tier in TIERS {
                let Some(ReasoningSelection::Effort { value }) =
                    resolve_reasoning_tier(ModelProtocol::Responses, &efforts(&values), tier, 8192)
                        .unwrap()
                else {
                    panic!("expected effort")
                };
                assert!(values.contains(&value.as_str()));
                let rank = REASONING_EFFORT_VALUES
                    .iter()
                    .position(|known| *known == value)
                    .unwrap();
                assert!(rank >= previous);
                previous = rank;
            }
        }
    }

    #[test]
    fn chat_requires_and_maps_all_three_exact_efforts() {
        for (tier, expected) in TIERS.into_iter().zip(["low", "medium", "high"]) {
            assert_eq!(
                resolve_reasoning_tier(
                    ModelProtocol::ChatCompletions,
                    &efforts(&["xhigh", "high", "none", "medium", "low"]),
                    tier,
                    8192
                ),
                Ok(Some(ReasoningSelection::Effort {
                    value: expected.into()
                }))
            );
        }
        assert_eq!(
            resolve_reasoning_tier(
                ModelProtocol::ChatCompletions,
                &efforts(&["low", "high"]),
                ReasoningTier::Balanced,
                8192
            ),
            Err(ReasoningTierError::MissingChatTiers)
        );
    }

    #[test]
    fn budget_uses_all_declared_tiers_and_effective_output_limit() {
        let declaration = ReasoningCapabilityDeclaration::Budget {
            tokens: vec![1024, 4096, 8192],
        };
        for (tier, tokens) in TIERS.into_iter().zip([1024, 4096, 8192]) {
            assert_eq!(
                resolve_reasoning_tier(ModelProtocol::Anthropic, &declaration, tier, 8193),
                Ok(Some(ReasoningSelection::Budget { tokens }))
            );
        }
        assert_eq!(
            resolve_reasoning_tier(
                ModelProtocol::Anthropic,
                &declaration,
                ReasoningTier::Concise,
                4096
            ),
            Ok(Some(ReasoningSelection::Budget { tokens: 1024 }))
        );
        for limit in [4096, 8192] {
            assert_eq!(
                resolve_reasoning_tier(
                    ModelProtocol::Anthropic,
                    &declaration,
                    ReasoningTier::Detailed,
                    limit
                ),
                Err(ReasoningTierError::BudgetExceedsOutputLimit)
            );
        }
    }

    #[test]
    fn invalid_declarations_do_not_fall_back_to_automatic() {
        for tokens in [
            vec![],
            vec![1024],
            vec![1024, 2048, 4096, 8192],
            vec![0, 2048, 4096],
            vec![1024, 1024, 4096],
            vec![4096, 2048, 1024],
        ] {
            assert_eq!(
                resolve_reasoning_tier(
                    ModelProtocol::Anthropic,
                    &ReasoningCapabilityDeclaration::Budget { tokens },
                    ReasoningTier::Concise,
                    8192
                ),
                Err(ReasoningTierError::InvalidBudgets)
            );
        }
        assert_eq!(
            resolve_reasoning_tier(
                ModelProtocol::Responses,
                &efforts(&[]),
                ReasoningTier::Balanced,
                8192
            ),
            Err(ReasoningTierError::EmptyEfforts)
        );
        assert_eq!(
            resolve_reasoning_tier(
                ModelProtocol::Responses,
                &efforts(&["low", "future"]),
                ReasoningTier::Balanced,
                8192
            ),
            Err(ReasoningTierError::UnknownEffort("future".into()))
        );
    }

    #[test]
    fn protocol_switch_requires_matching_capability() {
        assert_eq!(
            resolve_reasoning_tier(
                ModelProtocol::Anthropic,
                &efforts(&["low", "medium", "high"]),
                ReasoningTier::Balanced,
                16384
            ),
            Err(ReasoningTierError::IncompatibleProtocol)
        );
        for protocol in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
            assert_eq!(
                resolve_reasoning_tier(
                    protocol,
                    &ReasoningCapabilityDeclaration::Budget {
                        tokens: vec![1024, 4096, 8192]
                    },
                    ReasoningTier::Balanced,
                    16384
                ),
                Err(ReasoningTierError::IncompatibleProtocol)
            );
        }
    }

    #[test]
    fn typed_contract_rejects_unrecognized_fields_and_tiers() {
        assert_eq!(
            serde_json::to_value(ReasoningTier::Balanced).unwrap(),
            "balanced"
        );
        assert!(serde_json::from_str::<ReasoningTier>("\"high\"").is_err());
        for invalid in [
            r#"{"control":"automatic","values":["low"]}"#,
            r#"{"control":"budget","tokens":[-1,2048,4096]}"#,
            r#"{"control":"effort","values":["low"],"model":"guess"}"#,
        ] {
            assert!(serde_json::from_str::<ReasoningCapabilityDeclaration>(invalid).is_err());
        }
    }
}
