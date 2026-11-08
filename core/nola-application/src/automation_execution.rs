// SPDX-License-Identifier: Apache-2.0
//! Scheduled Agent execution uses the ordinary Core admission transaction.
use nola_domain::automation_execution::*;
use nola_store_port::automation::{AutomationRepository, AutomationRepositoryProvider};
use crate::{BeginAssistantTurn, AcceptedAssistantTurn};
use nola_agent_engine::agent::{AgentCore, protocol::{AgentSubmission, AgentItem, AgentRole}};
use sha2::{Digest, Sha256};

pub(crate) fn admit_with_core<R: nola_store_port::run_creation::RunCreationRepository
    + nola_store_port::conversation_history::CanonicalHistoryRepository
    + crate::repositories::AssistantAdmissionRepositories
    + nola_store_port::runtime_task::RuntimeTaskRepository
    + AutomationRepositoryProvider>(
    store: &mut R, lease: AutomationExecutionLease, input: BeginAssistantTurn,
) -> Result<(AcceptedAssistantTurn, Option<AgentCore>), String> {
    let identity = AutomationExecutionIdentity::for_occurrence(&lease.occurrence_id);
    let (submission, items) = match &input.submission {
        AgentSubmission::UserInput { submission_id: Some(id), items, .. }
        | AgentSubmission::AutomatedInput { submission_id: Some(id), items, .. } => (id, items),
        _ => return Err("automation requires stable admitted input".into()),
    };
    if input.conversation_id != identity.conversation_id || *submission != identity.submission_id
        || input.user_message_id != identity.user_message_id || input.assistant_message_id != identity.assistant_message_id {
        return Err("automation input identities differ from exact occurrence".into());
    }
    if !matches!(items.as_slice(), [AgentItem::Message { id, role: AgentRole::User, content }]
        if id.as_str()==identity.user_message_id && content==&input.user_text) {
        return Err("automation submitted items differ from the exact definition prompt".into());
    }
    let now = input.created_at_ms;
    // The lease is a dispatch witness, not part of immutable admission content:
    // after expiry an exact committed receipt can still be read, never dispatched.
    let request_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&(&lease.occurrence_id, lease.definition_revision, &input)).map_err(|e|e.to_string())?));
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        {
            let repository = store.automation_repository();
            if let Some(accepted) = repository.replay_automation_admission(&lease.occurrence_id, &request_digest).map_err(|e|e.to_string())? {
                return Ok((serde_json::from_str(&accepted).map_err(|e|e.to_string())?, None));
            }
            repository.validate_automation_execution(&lease, now).map_err(|e|e.to_string())?;
            let occurrence = repository.automation_run(&lease.occurrence_id).map_err(|e|e.to_string())?.ok_or("automation occurrence missing")?;
            let definition = repository.automation_definition(&occurrence.automation_id).map_err(|e|e.to_string())?.ok_or("automation definition missing")?;
            let spec = definition.execution_spec.agent()?;
            if let AutomationExecutionScope::DeviceLocal { host_id } = &spec.execution_scope {
                if !input.core_config.host_catalog.as_ref().is_some_and(|catalog|catalog.host_id.0.as_str()==host_id.as_str())
                    || input.core_config.turn_identity.workspace.is_some() {
                    return Err("automation device scope differs from actual native Host catalog or carries a workspace".into());
                }
            }
            if input.core_config.automation_execution_spec.as_ref()!=Some(spec)
                || input.user_text!=spec.prompt {
                return Err("automation actual Host execution spec differs from admitted definition".into());
            }
            if repository.observe_automation_execution(&lease.occurrence_id).map_err(|e|e.to_string())?.binding.is_some() {
                return Err("automation occurrence already admitted; recover its exact Run instead of readmitting".into());
            }
        }
        let (accepted, core) = crate::assistant_turn::begin_automation_turn_in_transaction(store, input)?;
        let accepted_json = serde_json::to_string(&accepted).map_err(|e|e.to_string())?;
        store.automation_repository().bind_automation_execution(&lease, &AutomationExecutionBinding {
            occurrence_id: lease.occurrence_id.clone(), definition_revision: lease.definition_revision,
            conversation_id: identity.conversation_id, submission_id: identity.submission_id,
            run_id: accepted.begin.run_id.clone(), invocation_config_digest: accepted.invocation_config_digest.clone(), bound_at_ms: now,
        }, &request_digest, &accepted_json).map_err(|e|e.to_string())?;
        Ok((accepted, Some(core)))
    })
}
