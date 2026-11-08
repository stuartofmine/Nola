// SPDX-License-Identifier: Apache-2.0
//! Configuration CAS and active-policy notification have one transaction owner.
use crate::*;
use nola_domain::{ActivePermissionPolicy, PermissionProfile};
use nola_domain::conversation_configuration::ConfigurationCommitResult;
use nola_store_port::{agent_queue::AgentQueueRepository, conversation_profile::ConversationProfileRepository,
    run_effect::RunEffectRepository};

pub(crate) fn patch_configuration<R: repositories::ConversationCommandRepositories>(
    store: &mut R,
    input: &nola_store_port::conversation_profile::PatchConversationConfigurationInput,
) -> Result<ConfigurationCommitResult, String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        let result = store.conversation_write_repository().patch_conversation_configuration(input)
            .map_err(|error| error.to_string())?;
        if input.patch_json.fields().contains_key("permissionProfile") {
            if let ConfigurationCommitResult::Committed { snapshot } = &result {
                if let Some(active) = store.active_run(&input.conversation_id).map_err(|error| error.to_string())? {
                    // A replica configuration is a local preference mirror.
                    // Its revision must never be delivered as a source-Host
                    // policy notification on the remote Run.
                    let run = store.inspect_exact_run(&active.run_id).map_err(|error| error.to_string())?
                        .ok_or("active permission Run disappeared")?;
                    if run.source_host_id.is_some() { return Ok(result); }
                    let profile = match snapshot.permission_profile.as_str() {
                        Some(value) => PermissionProfile::from_canonical_name(value).ok_or("invalid committed permission policy")?,
                        None if snapshot.permission_profile.is_null() => PermissionProfile::Cautious,
                        _ => return Err("invalid committed permission policy".into()),
                    };
                    let policy = ActivePermissionPolicy { run_id: active.run_id,
                        configuration_revision: snapshot.configuration_revision, profile };
                    policy.validate()?;
                    store.enqueue_agent_command(&nola_store_port::agent_queue::EnqueueAgentCommandInput {
                        session_id: input.conversation_id.clone(), command_id: policy.command_id(),
                        payload_json: serde_json::to_string(&nola_agent_engine::agent::host::AgentBoundaryInput::PermissionPolicyAvailable { policy })
                            .map_err(|error| error.to_string())?, created_at_ms: input.updated_at_ms,
                    }).map_err(|error| error.to_string())?;
                }
            }
        }
        Ok(result)
    })
}

pub(crate) fn prepare_apply<R: AgentQueueRepository + RunEffectRepository>(
    store: &mut R, conversation_id: &str, policy: &ActivePermissionPolicy, now_ms: i64,
) -> Result<Option<ReduceAndCommitActiveSubmission>, String> {
    policy.validate()?;
    let Some(active) = store.active_run(conversation_id).map_err(|error| error.to_string())?
        .filter(|active| active.run_id == policy.run_id) else { return Ok(None); };
    let source = store.leased_agent_commands(conversation_id).map_err(|error| error.to_string())?
        .into_iter().find(|command| command.command_id == policy.command_id())
        .ok_or("active policy has no claimed configuration notification")?;
    let expected = nola_agent_engine::agent::host::AgentBoundaryInput::PermissionPolicyAvailable { policy: policy.clone() };
    if serde_json::from_str::<serde_json::Value>(&source.payload_json).map_err(|error| error.to_string())?
        != serde_json::to_value(expected).map_err(|error| error.to_string())? {
        return Err("active policy differs from its committed notification".into());
    }
    Ok(Some(ReduceAndCommitActiveSubmission {
        conversation_id: conversation_id.into(), run_id: active.run_id,
        transition_id: policy.command_id(),
        invocation_config_digest: active.invocation_config_digest.ok_or("active policy Run has no invocation identity")?,
        submission: AgentSubmission::ApplyPermissionPolicy { policy: policy.clone() },
        committed_at_ms: now_ms.max(source.created_at_ms),
    }))
}
