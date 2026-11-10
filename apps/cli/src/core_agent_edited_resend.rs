// SPDX-License-Identifier: Apache-2.0
//! Fresh Host assembly from a durable target and model-only private material.
use crate::host_invocation_request::HostInvocationRequest;
use nola_host_protocol::{HostModelRouteBinding, ModelOutputTokenField};

pub(super) fn prepare_request(request: &mut HostInvocationRequest) -> Result<(), String> {
    let Some(operation) = request.session.edited_resend_operation_id.clone() else {
        return Ok(());
    };
    let target = request
        .session
        .session_id
        .clone()
        .ok_or("edited resend requires its fresh target")?;
    let material = super::host_recovery::edited_resend_model_material(&target, request.runtime.expert_template_authority.as_ref())?;
    // The authenticated source read is complete. Neither model/delegation
    // snapshots nor subsequent workers receive this factory-only identity.
    request.runtime.expert_template_authority = None;
    let prepared = &material.prepared;
    if operation != prepared.input.operation_id
        || request.session.host_submission_id.as_deref() != Some(operation.as_str())
        || request.session.host_user_message_id.as_deref()
            != Some(prepared.user_message_id.as_str())
        || request.session.host_assistant_message_id.as_deref()
            != Some(prepared.assistant_message_id.as_str())
        || request.input.message != prepared.input.edited_text
        || request.session.expected_parent_message_id.is_some()
        || request.session.desktop_resume.is_some()
        || request.session.desktop_cancel.is_some()
        || request.session.host_task_execution.is_some()
        || request.session.automation_execution_lease.is_some()
        || !request.input.host_attachments.is_empty()
        || request.input.host_turn_context.is_some()
        || !request.input.host_context_components.is_empty()
        || !request.input.context_files.is_empty()
        || !request.input.expected_artifact_refs.is_empty()
        || request.runtime.core_permission_rules.iter().any(|rule| {
            matches!(
                rule.source,
                nola_domain::PermissionRuleSource::Conversation
                    | nola_domain::PermissionRuleSource::Turn
            )
        })
    {
        return Err("edited_resend_requires_a_fresh_target_request".into());
    }
    let intrinsic: nola_core::agent::ModelCapabilityProfile =
        serde_json::from_value(prepared.source.model.model_capability.clone())
            .map_err(|_| "edited_resend_model_capability_unavailable")?;
    if !intrinsic.direct_tool_capability_ids.is_empty() {
        return Err("edited_resend_model_capability_unavailable".into());
    }
    request.runtime.edited_resend_model_capability = Some(intrinsic);
    request.runtime.edited_resend_prompt_definitions =
        prompt_components(material.child_instructions.as_deref().unwrap_or_default())?;
    request.session.permission_scope_id = Some(target);
    request.session.save_session = true;
    request.session.resume_snapshot = None;
    request.execution.external_execution_session_id = None;
    request.runtime.selected_profile = None;
    request.runtime.siwc = None;
    request.runtime.native_model_route = None;
    request.runtime.llm_fallbacks.clear();
    match &material.primary {
        HostModelRouteBinding::Http(route) => {
            request.execution.execution_backend = "nola".into();
            request.model_settings.endpoint = route.endpoint.clone();
            request.model_settings.model = route.model.clone();
            request.model_settings.api_key = material.credentials[0].clone();
            request.runtime.provider = route.provider.clone();
            request.runtime.chat_output_token_field = match route.output_token_field {
                ModelOutputTokenField::MaxTokens => {
                    nola_application::ChatOutputTokenField::MaxTokens
                }
                ModelOutputTokenField::MaxCompletionTokens => {
                    nola_application::ChatOutputTokenField::MaxCompletionTokens
                }
                ModelOutputTokenField::MaxOutputTokens | ModelOutputTokenField::Omitted => {
                    nola_application::ChatOutputTokenField::Auto
                }
            };
            if let Some(account) = &route.siwc_account {
                let store = crate::store::NolaStore::open_default()?;
                let current = crate::model_siwc::SiwcHost::open(store.path())?.execution_binding(
                    &account.account_id,
                    account.account_revision,
                    &account.credential_ref,
                )?;
                if current != *account {
                    return Err("edited_resend_account_owner_changed".into());
                }
                request.runtime.siwc = Some(crate::model_siwc::SiwcExecutionContext {
                    database: store.path().to_path_buf(),
                    account: account.clone(),
                });
            }
        }
        HostModelRouteBinding::Native(route) => {
            // The trusted native Host factory already admitted a new physical
            // channel. Freeze the original logical selection, never old ports.
            crate::model_route::validate_native_transport(
                route,
                &request.model_settings.endpoint,
                &request.model_settings.model,
                &request.model_settings.api_key,
            )?;
            request.runtime.native_model_route = Some(route.clone());
            request.runtime.selected_profile = route.profile_ref.clone();
        }
        HostModelRouteBinding::External {
            backend,
            model,
            provider,
        } => {
            request.execution.execution_backend = backend.clone();
            request.execution.external_execution_model = model.clone();
            request.execution.external_execution_model_provider = provider.clone();
        }
    }
    for (index, route) in material.fallbacks.iter().enumerate() {
        let HostModelRouteBinding::Http(route) = route else {
            return Err("edited_resend_fallback_adapter_unavailable".into());
        };
        request
            .runtime
            .llm_fallbacks
            .push(crate::runtime_config::RuntimeLlmProvider {
                provider: route.provider.clone(),
                endpoint: route.endpoint.clone(),
                api_key: Some(material.credentials[index + 1].clone()),
                model: Some(route.model.clone()),
            });
    }
    let generation = &material.generation;
    request.runtime.temperature = generation.temperature.as_f64();
    request.runtime.top_p = generation.top_p.as_f64();
    request.runtime.max_output_tokens = match material.output.preference {
        nola_domain::generation::OutputTokenPreference::Default => None,
        nola_domain::generation::OutputTokenPreference::Explicit(tokens) => Some(tokens),
    };
    // This is the original model's declared input capacity, not its consumed
    // context, checkpoint, history, or compaction threshold. Keep it consistent
    // with the frozen output admission; context_budget recomputes the fresh
    // target using that capacity and the current Host's remaining policy.
    request.runtime.compaction.context_window_tokens = material.output.configured_context_window
        .map(usize::try_from).transpose().map_err(|_|"edited_resend_model_context_capacity_unrepresentable")?;
    request.runtime.output_admission = Some(material.output);
    request.runtime.admitted_max_retries = Some(generation.max_retries);
    request.runtime.reasoning_selection = generation.reasoning_selection.clone();
    request.runtime.reasoning_tier = generation.reasoning_tier;
    request.runtime.sampling_override = generation.sampling_override;
    request.runtime.generation_options = generation.generation_options.to_options();
    request.runtime.model_capabilities.reasoning = Some(generation.thinking_enabled);
    request.runtime.model_capabilities.reasoning_summary_supported = generation.reasoning_summary_supported;
    request.runtime.model_capabilities.reasoning_declaration = None;
    request.runtime.openai_prompt_cache = Some(generation.prompt_cache);
    request.runtime.openai_prompt_cache_key = material.prompt_cache_key;
    request.runtime.openai_prompt_cache_key_prefix = material.prompt_cache_key_prefix;
    request.runtime.openai_prompt_cache_retention = generation.prompt_cache_retention.clone();
    Ok(())
}

/// Recompute a new context budget from source model capacity and current Host
/// compaction policy. No source usage, checkpoint, history, or budget is read.
pub(super) fn context_budget(
    request: &HostInvocationRequest,
    settings: &crate::model_invocation_settings::ModelInvocationSettings,
) -> Result<crate::context_budget::ContextBudget, String> {
    let ordinary = crate::host_output_admission::context_budget(settings)?;
    let Some(profile) = &request.runtime.edited_resend_model_capability else {
        return Ok(ordinary);
    };
    let capacity = match (
        ordinary.context_window_tokens,
        profile.context_window_tokens,
    ) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    if capacity.is_some_and(|window| window < 2_000) {
        return Err("edited_resend_model_context_capacity_below_host_minimum".into());
    }
    let output = crate::host_output_admission::admitted_output_tokens(settings)?;
    if capacity.is_some_and(|window| output >= window as u64) {
        return Err("edited_resend_model_context_capacity_exceeded".into());
    }
    let mut policy = request.runtime.compaction.clone();
    policy.context_window_tokens = capacity;
    Ok(
        crate::context_budget::ContextBudget::from_frozen_provider_limits(
            &policy,
            settings.swe_mode(),
            Some(crate::context_budget::ProviderContextLimits {
                max_input_tokens: capacity,
                max_total_tokens: capacity,
                max_output_tokens: profile.max_output_tokens,
            }),
            output,
        ),
    )
}

pub(super) fn prompt_components(
    definitions: &[nola_domain::expert_message::ExpertPromptDefinition],
) -> Result<Vec<nola_core::context_engine::ContextComponent>, String> {
    use nola_core::context_engine::{
        ContextComponent, ContextComponentKind, ContextComponentSource,
    };
    use sha2::{Digest, Sha256};
    let mut ids = std::collections::BTreeSet::new();
    definitions
        .iter()
        .map(|definition| {
            if definition.component_id.trim().is_empty()
                || !ids.insert(&definition.component_id)
                || format!("{:x}", Sha256::digest(definition.content.as_bytes()))
                    != definition.content_sha256
            {
                return Err("edited_resend_original_prompt_definition_invalid".into());
            }
            let platform = if definition.platform.is_null() {
                None
            } else {
                Some(
                    serde_json::from_value(definition.platform.clone())
                        .map_err(|_| "edited_resend_original_prompt_platform_invalid")?,
                )
            };
            Ok(ContextComponent {
                kind: ContextComponentKind::PromptTemplate,
                id: definition.component_id.clone(),
                content: serde_json::Value::String(definition.content.clone()),
                source: Some(ContextComponentSource {
                    platform,
                    ..Default::default()
                }),
            })
        })
        .collect()
}

#[cfg(test)]
mod frozen_prompt_definition_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    fn definition() -> nola_domain::expert_message::ExpertPromptDefinition {
        nola_domain::expert_message::ExpertPromptDefinition {
            component_id: "prompt_template:original".into(),
            content: "Original expert instruction".into(),
            content_sha256: format!("{:x}", Sha256::digest(b"Original expert instruction")),
            definition_revision: Some(9),
            platform: serde_json::Value::Null,
        }
    }
    #[test]
    fn only_exact_typed_definition_data_enters_new_context() {
        let original = definition();
        let components = prompt_components(std::slice::from_ref(&original)).unwrap();
        let wire = serde_json::to_value(&components).unwrap();
        assert_eq!(wire[0]["kind"], "prompt_template");
        assert_eq!(wire[0]["id"], original.component_id);
        assert_eq!(wire[0]["content"], original.content);
        assert_eq!(wire[0]["source"], serde_json::json!({}));
        assert!(prompt_components(&[original.clone(), original.clone()]).is_err());
        let mut changed = original;
        changed.content.push('!');
        assert!(prompt_components(&[changed]).is_err());
        assert!(prompt_components(&[]).unwrap().is_empty());
    }
}
