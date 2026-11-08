// SPDX-License-Identifier: Apache-2.0
//! User recovery is a canonical decision transaction, not a process-status shortcut.
use nola_domain::unknown_effect_recovery::*;
use nola_domain::{EffectDeliveryAttemptState, EffectDeliverySettlement};
use nola_store_port::{effect_delivery::EffectDeliveryRepository, run_effect::RunEffectRepository};
use nola_agent_engine::agent::{protocol::AgentSubmission, host::HostResponseEnvelope};
use sha2::{Digest, Sha256};

pub use nola_protocol::{ResolveUnknownEffectRecovery, UnknownEffectRecoveryChoice, UnknownEffectRecoveryReply};

pub(crate) fn resolve<R: RunEffectRepository + EffectDeliveryRepository + crate::repositories::AssistantPublicationRepositories>(store:&mut R,input:ResolveUnknownEffectRecovery)
    -> Result<UnknownEffectRecoveryReply,String> {
    resolve_with_core(store,input,None,Vec::new(),Vec::new(),Vec::new()).map(|resolved|resolved.reply)
}

pub(crate) struct ResolvedRecovery {
    pub reply: UnknownEffectRecoveryReply,
    pub core: Option<nola_agent_engine::agent::AgentCore>,
    pub accepted: Option<crate::AcceptedActiveSubmission>,
    pub deferred_events: Vec<nola_domain::core_event::CoreEventEnvelope<nola_agent_engine::agent::protocol::AgentEvent>>,
    pub deferred_host_events: Vec<nola_store_port::run_effect::HostEventInput>,
    pub deferred_traces: Vec<nola_agent_engine::agent_trace::AgentTraceEvent>,
}

pub(crate) fn resolve_with_core<R: RunEffectRepository + EffectDeliveryRepository + crate::repositories::AssistantPublicationRepositories>(store:&mut R,input:ResolveUnknownEffectRecovery,
    mut core:Option<nola_agent_engine::agent::AgentCore>,
    mut deferred_events:Vec<nola_domain::core_event::CoreEventEnvelope<nola_agent_engine::agent::protocol::AgentEvent>>,
    mut deferred_host_events:Vec<nola_store_port::run_effect::HostEventInput>,
    mut deferred_traces:Vec<nola_agent_engine::agent_trace::AgentTraceEvent>) -> Result<ResolvedRecovery,String> {
    if input.validate().is_err() {
        return Ok(ResolvedRecovery{reply:UnknownEffectRecoveryReply::Rejected{code:"invalid_recovery_identity".into()},core,accepted:None,deferred_events,deferred_host_events,deferred_traces});
    }
    let digest=format!("{:x}",Sha256::digest(serde_json::to_vec(&input).map_err(|e|e.to_string())?));
    let mut accepted=None;
    let reply=crate::assistant_publication_transaction::with_publication_transaction(store,|store| {
        if let Some((old,receipt))=store.unknown_effect_recovery_receipt(&input.resolution_id).map_err(|e|e.to_string())? {
            return if old==digest { serde_json::from_str(&receipt).map_err(|e|e.to_string()) }
                else { Ok(UnknownEffectRecoveryReply::Conflict{code:"resolution_identity_reused".into()}) };
        }
        crate::validate_run_invocation_config_digest(store,&input.run_id,&input.invocation_config_digest)?;
        let verify_applied=matches!(&input.choice,UnknownEffectRecoveryChoice::VerifyAppliedReceipt{..});
        let preserve=matches!(&input.choice,UnknownEffectRecoveryChoice::KeepUnknownBlocked{..});
        let mut ids=std::collections::BTreeSet::new();
        let mut attempts=Vec::new();
        for target in &input.targets {
            if !ids.insert(target.effect_id.clone()) { return Ok(UnknownEffectRecoveryReply::Rejected{code:"duplicate_effect_target".into()}); }
            let Some(attempt)=store.effect_delivery_attempt(&target.attempt_id).map_err(|e|e.to_string())? else {
                return Ok(UnknownEffectRecoveryReply::Conflict{code:"attempt_missing".into()});
            };
            if attempt.run_id!=input.run_id || attempt.effect_id!=target.effect_id || attempt.request_id!=target.request_id
                || attempt.revision!=target.expected_attempt_revision
                || attempt.worker_generation.as_deref()!=Some(target.worker_generation.as_str())
                || attempt.lease_owner.as_deref()!=Some(target.lease_owner.as_str()) || attempt.lease_token.as_deref()!=Some(target.lease_token.as_str())
                || attempt.finished_at_ms.is_some_and(|time|time>input.resolved_at_ms)
                || (!verify_applied && !preserve && attempt.state!=EffectDeliveryAttemptState::OutcomeUnknown) {
                return Ok(UnknownEffectRecoveryReply::Conflict{code:"attempt_identity_or_revision_changed".into()});
            }
            attempts.push(attempt);
        }
        let reply=match &input.choice {
            UnknownEffectRecoveryChoice::VerifyAppliedReceipt { host_event_id,response_json } => {
                if attempts.len()!=1 || attempts[0].state!=EffectDeliveryAttemptState::Settled
                    || attempts[0].settlement!=Some(EffectDeliverySettlement::Succeeded) {
                    return Ok(UnknownEffectRecoveryReply::Rejected{code:"original_successful_host_receipt_required".into()});
                }
                let response:HostResponseEnvelope=serde_json::from_str(response_json).map_err(|e|e.to_string())?;
                if response.attempt_id.as_deref()!=Some(attempts[0].id.as_str()) || response.effect_id.0!=attempts[0].effect_id
                    || response.request_id.as_str()!=attempts[0].request_id
                    || crate::effect_delivery::terminal_settlement(&response.payload)?!=Some(EffectDeliverySettlement::Succeeded)
                    || store.committed_terminal_host_response(&input.conversation_id,&input.run_id,host_event_id,response_json).map_err(|e|e.to_string())?.is_none() {
                    return Ok(UnknownEffectRecoveryReply::Rejected{code:"host_receipt_not_exact_original".into()});
                }
                UnknownEffectRecoveryReply::AppliedReceiptVerified{resolution_id:input.resolution_id.clone(),run_id:input.run_id.clone()}
            }
            choice => {
                let (disposition,reason)=match choice {
                    UnknownEffectRecoveryChoice::AuthorizeRiskRetry{reason} => (UnknownEffectRecoveryDisposition::AuthorizeRiskRetry,reason),
                    UnknownEffectRecoveryChoice::KeepUnknownBlocked{reason} => (UnknownEffectRecoveryDisposition::KeepUnknownBlocked,reason),
                    _=>unreachable!(),
                };
                // Text/identity bounds belong to the protocol validation above.
                // This layer checks the choice against canonical execution facts;
                // a second, different text limit would be a competing contract.
                if disposition==UnknownEffectRecoveryDisposition::AuthorizeRiskRetry && attempts.len()!=1 {
                    return Ok(UnknownEffectRecoveryReply::Rejected{code:"explicit_user_recovery_choice_required".into()});
                }
                let pending=store.pending_outbox_effect_ids(&input.run_id).map_err(|e|e.to_string())?;
                let controls = crate::load_committed_host_requests(store,&input.run_id)?.requests.into_iter()
                    .filter_map(|request| match request.payload {
                        nola_agent_engine::agent::host::HostRequest::Cancel {target_effect_id,..} if ids.contains(&target_effect_id.0) => Some(request.effect_id.0),
                        _=>None,
                    }).collect::<std::collections::BTreeSet<_>>();
                if !preserve && !controls.is_empty() {
                    return Ok(UnknownEffectRecoveryReply::Rejected { code: "cancellation_must_retain_unknown".into() });
                }
                if (!preserve && input.targets.iter().any(|target|!pending.contains(&target.effect_id)))
                    || (preserve && pending.iter().any(|id|!ids.contains(id) && !controls.contains(id))) {
                    return Ok(UnknownEffectRecoveryReply::Rejected{code:"independent_work_must_settle".into()});
                }
                if preserve && attempts.iter().any(|attempt|attempt.state!=EffectDeliveryAttemptState::OutcomeUnknown) {
                    let targeted:std::collections::BTreeSet<_>=crate::load_committed_host_requests(store,&input.run_id)?.requests.into_iter()
                        .filter_map(|request|match request.payload { nola_agent_engine::agent::host::HostRequest::Cancel {target_effect_id,..}=>Some(target_effect_id.0), _=>None }).collect();
                    if input.targets.iter().any(|target|!targeted.contains(&target.effect_id)) {
                        return Ok(UnknownEffectRecoveryReply::Rejected{code:"committed_cancellation_required".into()});
                    }
                }
                let decision=UnknownEffectRecoveryDecision{resolution_id:input.resolution_id.clone(),targets:input.targets.clone(),disposition,reason:reason.clone()};
                // Bypass only the generic-route guard, not the ordinary Core commit/seal pipeline.
                store.append_host_event_history(&input.run_id,&deferred_host_events).map_err(|e|e.to_string())?;
                crate::execution_trace_journal::record_accepted(store,&input.conversation_id,&deferred_traces)?;
                let (committed,next_core)=crate::reduce_and_commit_active_submission_in_transaction(store,crate::ReduceAndCommitActiveSubmission{
                    conversation_id:input.conversation_id.clone(),run_id:input.run_id.clone(),
                    transition_id:input.resolution_id.clone(),invocation_config_digest:input.invocation_config_digest.clone(),
                    submission:AgentSubmission::ResolveUnknownEffects{decision:decision.clone()},committed_at_ms:input.resolved_at_ms,
                },core.take(),std::mem::take(&mut deferred_events))?;
                deferred_host_events.clear();
                deferred_traces.clear();
                core=Some(next_core);
                accepted=Some(committed);
                if disposition==UnknownEffectRecoveryDisposition::AuthorizeRiskRetry {
                    let target=&input.targets[0];
                    let mutation=nola_kernel::authorize_unknown_effect_retry(&attempts[0],&input.run_id,&target.effect_id,&target.request_id,
                        &target.attempt_id,&target.worker_generation,&target.lease_owner,&target.lease_token,target.expected_attempt_revision,
                        &serde_json::to_string(&decision).map_err(|e|e.to_string())?,input.resolved_at_ms)?;
                    store.authorize_unknown_effect_retry(&mutation).map_err(|e|e.to_string())?;
                    UnknownEffectRecoveryReply::RetryAuthorized{resolution_id:input.resolution_id.clone(),run_id:input.run_id.clone()}
                } else {
                    UnknownEffectRecoveryReply::UnknownPreservedBlocked{resolution_id:input.resolution_id.clone(),run_id:input.run_id.clone()}
                }
            }
        };
        store.record_unknown_effect_recovery_receipt(&input.resolution_id,&digest,&serde_json::to_string(&reply).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        Ok(reply)
    })?;
    Ok(ResolvedRecovery{reply,core,accepted,deferred_events,deferred_host_events,deferred_traces})
}
