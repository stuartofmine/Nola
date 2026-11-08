// SPDX-License-Identifier: Apache-2.0
//! Credential-free same-snapshot generation queries. A result is not Run admission.
use nola_domain::{
    generation::{GenerationOptions, ModelProtocol, ReasoningSelection},
    reasoning_tiers::ReasoningTier,
    ConversationSourceOwner,
};
use nola_host_protocol::HostModelProfileReference;
use nola_store_port::{
    consistent_read::ConsistentReadRepository,
    conversation_history::ConversationProjectionRepository,
    model::{ModelResolutionFactsRepository, StoredModelProfile},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveConversationPin {
    pub conversation_id: String,
    pub expected_configuration_revision: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResolveEffectiveModelParameters {
    CurrentConversation {
        conversation_id: String,
        expected_configuration_revision: i64,
        expected_model_profile: HostModelProfileReference,
    },
    CandidatePreview {
        model_profile: HostModelProfileReference,
        conversation: Option<EffectiveConversationPin>,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectiveModelResolution {
    Available {
        protocol: ModelProtocol,
        temperature: Option<f64>,
        top_p: Option<f64>,
        max_output_tokens: Option<u64>,
        context_window_tokens: Option<usize>,
        reasoning_selection: Option<ReasoningSelection>,
        reasoning_tier: ReasoningTier,
        sampling_override: bool,
        generation_options: GenerationOptions,
        chat_output_token_field: crate::ChatOutputTokenField,
    },
    Unavailable {
        reason: EffectiveModelUnavailableReason,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveModelUnavailableReason {
    LocalBaselineUnavailable,
    SourceHostRequired,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveModelParametersSnapshot {
    pub scope: String,
    pub conversation: Option<EffectiveConversationPin>,
    /// Null when this Host cannot verify the source-owned profile; never an
    /// echoed renderer assertion presented as a verified profile.
    pub verified_model_profile: Option<HostModelProfileReference>,
    pub model_key: Option<String>,
    pub sparse_overrides: Value,
    pub client_model_presentation: Value,
    pub resolution: EffectiveModelResolution,
}

pub(crate) fn resolve_profile_parameters(
    profile: &StoredModelProfile,
    settings: &Value,
    model_key: &str,
) -> Result<EffectiveModelResolution, String> {
    if profile.deleted_at_ms.is_some() || !profile.enabled || profile.revision <= 0 {
        return Err("effective_model_profile_unavailable".into());
    }
    if matches!(profile.runtime.as_str(), "mlx" | "llama.cpp") {
        return Ok(EffectiveModelResolution::Unavailable {
            reason: EffectiveModelUnavailableReason::LocalBaselineUnavailable,
        });
    }
    if profile.runtime == "agent-host" {
        return Ok(EffectiveModelResolution::Unavailable {
            reason: EffectiveModelUnavailableReason::SourceHostRequired,
        });
    }
    if profile.runtime != "remote" {
        return Err("effective_model_runtime_unsupported".into());
    }
    let overrides = settings
        .get("modelOverrides")
        .and_then(|v| v.get(model_key))
        .cloned()
        .unwrap_or(Value::Null);
    let profile = crate::model_profile_resolution::profile_with_session_contract(
        &profile.clone().into(),
        &overrides,
    )?;
    let source: Value =
        serde_json::from_str(&profile.source_json).map_err(|_| "effective_model_invalid_source")?;
    let protocol = source["wireProtocol"]
        .as_str()
        .ok_or("effective_model_protocol_missing")?;
    let request = crate::model_profile_resolution::session_override_request(
        &overrides,
        &source["reasoning"],
        protocol,
    )?;
    let resolved = crate::model_profile_resolution::resolve_remote_profile(
        &profile,
        &request,
        ReasoningTier::default(),
    )?;
    let runtime = resolved.runtime;
    Ok(EffectiveModelResolution::Available {
        protocol: ModelProtocol::parse(protocol)?,
        temperature: runtime.temperature,
        top_p: runtime.top_p,
        max_output_tokens: runtime.max_output_tokens,
        context_window_tokens: runtime.context_window_tokens,
        reasoning_selection: runtime.reasoning_selection,
        reasoning_tier: runtime.reasoning_tier,
        sampling_override: runtime.sampling_override,
        generation_options: runtime.generation_options,
        chat_output_token_field: runtime.chat_output_token_field,
    })
}

pub fn resolve<S>(
    store: &mut S,
    input: &ResolveEffectiveModelParameters,
) -> Result<EffectiveModelParametersSnapshot, String>
where
    S: ModelResolutionFactsRepository + ConversationProjectionRepository + ConsistentReadRepository,
{
    store.with_consistent_read(|reader| resolve_snapshot(reader, input))
}
fn resolve_snapshot<S>(
    store: &S,
    input: &ResolveEffectiveModelParameters,
) -> Result<EffectiveModelParametersSnapshot, String>
where
    S: ModelResolutionFactsRepository + ConversationProjectionRepository,
{
    let (scope, pin, expected, current) = match input {
        ResolveEffectiveModelParameters::CurrentConversation {
            conversation_id,
            expected_configuration_revision,
            expected_model_profile,
        } => (
            "current_conversation",
            Some(EffectiveConversationPin {
                conversation_id: conversation_id.clone(),
                expected_configuration_revision: *expected_configuration_revision,
            }),
            expected_model_profile,
            true,
        ),
        ResolveEffectiveModelParameters::CandidatePreview {
            model_profile,
            conversation,
        } => (
            "candidate_preview",
            conversation.clone(),
            model_profile,
            false,
        ),
    };
    if expected.profile_id.trim().is_empty() || expected.admitted_revision <= 0 {
        return Err("effective_model_invalid_profile_pin".into());
    }
    let mut settings = serde_json::json!({});
    if let Some(pin) = &pin {
        if pin.conversation_id.trim().is_empty() || pin.expected_configuration_revision <= 0 {
            return Err("effective_model_invalid_conversation_pin".into());
        }
        let conversation = store
            .load_conversation(&pin.conversation_id)
            .map_err(|e| e.to_string())?
            .ok_or("effective_model_conversation_missing")?;
        if conversation.configuration_revision != pin.expected_configuration_revision {
            return Err("effective_model_configuration_changed".into());
        }
        match store
            .conversation_source_owner(&pin.conversation_id)
            .map_err(|e| e.to_string())?
        {
            ConversationSourceOwner::Local => {}
            ConversationSourceOwner::Remote { .. } => {
                return Ok(EffectiveModelParametersSnapshot {
                    scope: scope.into(),
                    conversation: Some(pin.clone()),
                    verified_model_profile: None,
                    model_key: None,
                    sparse_overrides: Value::Null,
                    client_model_presentation: Value::Null,
                    resolution: EffectiveModelResolution::Unavailable {
                        reason: EffectiveModelUnavailableReason::SourceHostRequired,
                    },
                })
            }
            ConversationSourceOwner::Unavailable => {
                return Err("effective_model_source_unavailable".into())
            }
        }
        settings = serde_json::from_str(conversation.settings_json.as_deref().unwrap_or("{}"))
            .map_err(|_| "effective_model_invalid_configuration")?;
        // Room ownership is metadata, outside the configuration patch schema.
        if let Some(object) = settings.as_object_mut() {
            object.remove(nola_domain::agent_room::CONTEXT_OWNER_FIELD);
        }
        nola_domain::configuration_patch::validate_patch(&settings.to_string())?;
        // A local presentation may delegate execution to a connected Host.
        // Its parameters and model identity belong to that source, not this
        // device's default profile. Candidate preview cannot verify them here.
        if settings
            .get("executionHostRawValue")
            .and_then(Value::as_str)
            == Some("remote")
            || settings
                .get("executionHostBinding")
                .is_some_and(|value| !value.is_null())
            || settings
                .get("modelSelectionID")
                .and_then(Value::as_str)
                .is_some_and(|value| value.starts_with("connectedHost:"))
        {
            return Ok(EffectiveModelParametersSnapshot {
                scope: scope.into(),
                conversation: Some(pin.clone()),
                verified_model_profile: None,
                model_key: None,
                sparse_overrides: Value::Null,
                client_model_presentation: Value::Null,
                resolution: EffectiveModelResolution::Unavailable {
                    reason: EffectiveModelUnavailableReason::SourceHostRequired,
                },
            });
        }
    }
    let facts = store
        .model_resolution_facts(crate::model_selection::DEFAULT_CHAT_MODEL_KEY)
        .map_err(|e| e.to_string())?;
    let profile = facts
        .profiles
        .iter()
        .find(|p| p.id == expected.profile_id)
        .ok_or("effective_model_profile_missing")?;
    if profile.revision != expected.admitted_revision {
        return Err("effective_model_profile_changed".into());
    }
    let model_key = format!(
        "{}:{}",
        if profile.runtime == "remote" {
            "configured"
        } else {
            "local"
        },
        profile.id
    );
    if current {
        let intent = match settings.get("modelSelectionID").and_then(Value::as_str) {
            None => crate::model_selection::ChatModelIntent::Inherit,
            Some(selection) => {
                let selected = selection
                    .strip_prefix("configured:")
                    .or_else(|| selection.strip_prefix("local:"))
                    .filter(|v| !v.is_empty())
                    .ok_or("effective_model_selection_requires_source_host")?;
                crate::model_selection::ChatModelIntent::Explicit {
                    profile_id: selected.into(),
                }
            }
        };
        let candidates = facts
            .profiles
            .iter()
            .filter(|p| p.deleted_at_ms.is_none())
            .map(|p| crate::model_selection::ChatModelCandidate {
                profile_id: p.id.clone(),
                allowed_for_chat: true,
                availability: if p.enabled {
                    crate::model_selection::ModelAvailability::Available
                } else {
                    crate::model_selection::ModelAvailability::Disabled
                },
            })
            .collect::<Vec<_>>();
        match crate::model_selection::resolve_chat_model(
            &intent,
            facts
                .default_chat_preference
                .as_ref()
                .map(|v| v.value_json.as_str()),
            &candidates,
        ) {
            crate::model_selection::ChatModelResolution::Resolved { profile_id, .. }
                if profile_id == profile.id => {}
            _ => return Err("effective_model_selection_changed".into()),
        }
        if profile.runtime != "agent-host"
            && settings
                .get("modelSelectionID")
                .and_then(Value::as_str)
                .is_some_and(|v| v != model_key)
        {
            return Err("effective_model_selection_runtime_mismatch".into());
        }
    }
    if profile.runtime == "agent-host" {
        if profile.deleted_at_ms.is_some() || !profile.enabled {
            return Err("effective_model_profile_unavailable".into());
        }
        return Ok(EffectiveModelParametersSnapshot {
            scope: scope.into(),
            conversation: pin,
            verified_model_profile: None,
            model_key: None,
            sparse_overrides: Value::Null,
            client_model_presentation: Value::Null,
            resolution: EffectiveModelResolution::Unavailable {
                reason: EffectiveModelUnavailableReason::SourceHostRequired,
            },
        });
    }
    Ok(EffectiveModelParametersSnapshot {
        scope: scope.into(),
        conversation: pin,
        verified_model_profile: Some(expected.clone()),
        model_key: Some(model_key.clone()),
        sparse_overrides: settings
            .get("modelOverrides")
            .and_then(|v| v.get(&model_key))
            .cloned()
            .unwrap_or(Value::Null),
        client_model_presentation: settings
            .get("clientModelPresentation")
            .and_then(|v| v.get(&model_key))
            .cloned()
            .unwrap_or(Value::Null),
        resolution: resolve_profile_parameters(profile, &settings, &model_key)?,
    })
}
