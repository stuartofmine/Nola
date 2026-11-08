// SPDX-License-Identifier: Apache-2.0
//! Host-owned chat preference and pure model identity resolution.
use serde::{Deserialize, Serialize};

pub const DEFAULT_CHAT_MODEL_KEY: &str = "chat.default_model";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DefaultChatSelection {
    Automatic {},
    Profile { profile_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DefaultChatConfiguration {
    pub version: u32,
    pub selection: DefaultChatSelection,
}

pub fn parse_default_chat_configuration(
    value: Option<&str>,
) -> Result<DefaultChatConfiguration, String> {
    let configuration = match value {
        None => DefaultChatConfiguration {
            version: 1,
            selection: DefaultChatSelection::Automatic {},
        },
        Some(value) => {
            serde_json::from_str::<DefaultChatConfiguration>(value).map_err(|e| e.to_string())?
        }
    };
    if configuration.version != 1 {
        return Err("unsupported default chat model version".into());
    }
    if matches!(&configuration.selection, DefaultChatSelection::Profile { profile_id } if profile_id.trim().is_empty())
    {
        return Err("default chat model requires a profile identity".into());
    }
    Ok(configuration)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatModelIntent {
    Inherit,
    Explicit { profile_id: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelAvailability {
    Available,
    Disabled,
    NotInstalled,
    UnsupportedRuntime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatModelCandidate {
    pub profile_id: String,
    pub allowed_for_chat: bool,
    pub availability: ModelAvailability,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatModelOrigin {
    Conversation,
    Default,
    Automatic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelUnavailableReason {
    Missing,
    Disabled,
    NotInstalled,
    UnsupportedRuntime,
    NotAllowedForChat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatModelResolution {
    Resolved {
        profile_id: String,
        origin: ChatModelOrigin,
    },
    Unavailable {
        profile_id: String,
        origin: ChatModelOrigin,
        reason: ModelUnavailableReason,
    },
    NoAutomaticCandidate,
    InvalidConfiguration,
}

fn unavailable_reason(candidate: &ChatModelCandidate) -> Option<ModelUnavailableReason> {
    match candidate.availability {
        ModelAvailability::Disabled => Some(ModelUnavailableReason::Disabled),
        ModelAvailability::NotInstalled => Some(ModelUnavailableReason::NotInstalled),
        ModelAvailability::UnsupportedRuntime => Some(ModelUnavailableReason::UnsupportedRuntime),
        ModelAvailability::Available if !candidate.allowed_for_chat => {
            Some(ModelUnavailableReason::NotAllowedForChat)
        }
        ModelAvailability::Available => None,
    }
}

/// Candidate order describes automatic preference only. Explicit identity never falls back.
pub fn resolve_chat_model(
    intent: &ChatModelIntent,
    default_value_json: Option<&str>,
    candidates: &[ChatModelCandidate],
) -> ChatModelResolution {
    let configuration = match parse_default_chat_configuration(match intent {
        ChatModelIntent::Explicit { .. } => None,
        ChatModelIntent::Inherit => default_value_json,
    }) {
        Ok(value) => value,
        Err(_) => return ChatModelResolution::InvalidConfiguration,
    };
    let target = match intent {
        ChatModelIntent::Explicit { profile_id } => {
            Some((profile_id.as_str(), ChatModelOrigin::Conversation))
        }
        ChatModelIntent::Inherit => match &configuration.selection {
            DefaultChatSelection::Profile { profile_id } => {
                Some((profile_id.as_str(), ChatModelOrigin::Default))
            }
            DefaultChatSelection::Automatic {} => None,
        },
    };
    if let Some((profile_id, origin)) = target {
        let reason = candidates
            .iter()
            .find(|c| c.profile_id == profile_id)
            .map_or(Some(ModelUnavailableReason::Missing), unavailable_reason);
        return match reason {
            Some(reason) => ChatModelResolution::Unavailable {
                profile_id: profile_id.into(),
                origin,
                reason,
            },
            None => ChatModelResolution::Resolved {
                profile_id: profile_id.into(),
                origin,
            },
        };
    }
    candidates
        .iter()
        .find(|candidate| unavailable_reason(candidate).is_none())
        .map_or(ChatModelResolution::NoAutomaticCandidate, |candidate| {
            ChatModelResolution::Resolved {
                profile_id: candidate.profile_id.clone(),
                origin: ChatModelOrigin::Automatic,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absence_is_automatic_but_invalid_values_are_errors() {
        assert_eq!(
            resolve_chat_model(&ChatModelIntent::Inherit, None, &[]),
            ChatModelResolution::NoAutomaticCandidate
        );
        for value in [
            "{}",
            "bad",
            r#"{"version":2,"selection":{"kind":"automatic"}}"#,
            r#"{"version":1,"selection":{"kind":"profile","profile_id":""}}"#,
            r#"{"version":1,"selection":{"kind":"automatic","extra":true}}"#,
        ] {
            assert_eq!(
                resolve_chat_model(&ChatModelIntent::Inherit, Some(value), &[]),
                ChatModelResolution::InvalidConfiguration,
                "accepted malformed default configuration: {value}"
            );
        }
    }

    #[test]
    fn explicit_conversation_does_not_consume_invalid_global_preference() {
        let candidate = ChatModelCandidate {
            profile_id: "api:valid".into(),
            allowed_for_chat: true,
            availability: ModelAvailability::Available,
        };
        assert_eq!(
            resolve_chat_model(
                &ChatModelIntent::Explicit {
                    profile_id: candidate.profile_id.clone()
                },
                Some("invalid JSON"),
                &[candidate]
            ),
            ChatModelResolution::Resolved {
                profile_id: "api:valid".into(),
                origin: ChatModelOrigin::Conversation
            }
        );
        assert_eq!(
            resolve_chat_model(
                &ChatModelIntent::Explicit {
                    profile_id: "missing".into()
                },
                Some("invalid JSON"),
                &[]
            ),
            ChatModelResolution::Unavailable {
                profile_id: "missing".into(),
                origin: ChatModelOrigin::Conversation,
                reason: ModelUnavailableReason::Missing
            }
        );
    }
    #[test]
    fn explicit_identity_is_exact_and_unavailable_never_falls_back() {
        let candidates = vec![ChatModelCandidate {
            profile_id: "api:a:same".into(),
            allowed_for_chat: true,
            availability: ModelAvailability::Available,
        }];
        let setting = r#"{"version":1,"selection":{"kind":"profile","profile_id":"api:b:same"}}"#;
        assert_eq!(
            resolve_chat_model(&ChatModelIntent::Inherit, Some(setting), &candidates),
            ChatModelResolution::Unavailable {
                profile_id: "api:b:same".into(),
                origin: ChatModelOrigin::Default,
                reason: ModelUnavailableReason::Missing
            }
        );
        assert_eq!(
            resolve_chat_model(
                &ChatModelIntent::Explicit {
                    profile_id: "api:a:same".into()
                },
                Some(setting),
                &candidates
            ),
            ChatModelResolution::Resolved {
                profile_id: "api:a:same".into(),
                origin: ChatModelOrigin::Conversation
            }
        );
        for availability in [
            ModelAvailability::Disabled,
            ModelAvailability::NotInstalled,
            ModelAvailability::UnsupportedRuntime,
        ] {
            let mut facts = candidates.clone();
            facts[0].availability = availability;
            assert!(matches!(
                resolve_chat_model(
                    &ChatModelIntent::Explicit {
                        profile_id: facts[0].profile_id.clone()
                    },
                    None,
                    &facts
                ),
                ChatModelResolution::Unavailable { .. }
            ));
            assert_eq!(
                resolve_chat_model(&ChatModelIntent::Inherit, None, &facts),
                ChatModelResolution::NoAutomaticCandidate
            );
        }
    }
}
