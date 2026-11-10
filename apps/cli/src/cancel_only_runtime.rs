// SPDX-License-Identifier: Apache-2.0
//! Cold cancellation never constructs a normal Host, tool catalog or provider.
use super::cancel_only_driver::{CancelOnlyDriver, CancellationAuthority};
use crate::application_port::CliApplicationPort;
use nola_application::{
    cancellation_delivery::{
        CancellationRunIdentity, ClaimCancellationHostRequests,
        RunCancellationDeliveryFacts,
    },
    ApplicationCommand,
};
use nola_core::agent::{
    host::{HostRequestEnvelope, HostResponseEnvelope},
};
use nola_runtime_driver::{
    CommittedRuntimeAcceptance as ProtocolAcceptance, CommittedRuntimeApplicationPort, RuntimeEffectDeliveryClaim,
    RuntimeEffectDeliveryRenewal,
};
type CommittedRuntimeAcceptance = ProtocolAcceptance<nola_core::agent::host::AgentRuntimeProtocol>;
use std::sync::{Arc, Mutex};

type SharedPort = Arc<Mutex<CliApplicationPort>>;

fn facts(
    port: &SharedPort,
    identity: &CancellationRunIdentity,
) -> Result<RunCancellationDeliveryFacts, String> {
    port.lock()
        .map_err(|_| "cancellation port lock poisoned")?
        .execute(ApplicationCommand::LoadRunCancellationDeliveryFacts {
            input: identity.clone(),
        })
}

fn observe_dead_attempt(attempt: &nola_domain::EffectDeliveryAttempt) -> Result<(), String> {
    if attempt.worker_generation.is_none()
        && attempt.lease_owner.is_none()
        && attempt.execution_started_at_ms.is_none()
    {
        return Ok(());
    }
    let lease = nola_core::agent::host::EffectDeliveryLease {
        attempt_id: attempt.id.clone(),
        worker_generation: attempt
            .worker_generation
            .clone()
            .ok_or("unknown cancellation target generation")?,
        lease_owner: attempt
            .lease_owner
            .clone()
            .ok_or("unknown cancellation target owner")?,
        lease_token: attempt
            .lease_token
            .clone()
            .ok_or("unknown cancellation target fence")?,
        lease_expires_at_ms: attempt
            .lease_expires_at_ms
            .ok_or("unknown cancellation target expiry")?,
    };
    let pid = super::local_runtime_delivery_pid(&lease)
        .ok_or("unrecognized target owner; cancellation remains pending")?;
    if !super::local_runtime_delivery_is_terminated(&lease)? {
        return Err(format!(
            "Run cancellation remains pending: target owner {pid} is alive"
        ));
    }
    Ok(())
}

fn observe_target_owners(fact: &nola_application::cancellation_delivery::CancellationTargetFact) -> Result<(), String> {
    for owner in nola_application::cancellation_process_observation::cancellation_target_process_owners(fact)? {
        if !owner.is_terminated(&crate::process_observation::observe_process(owner.pid))? {
            return Err("Run cancellation remains pending: target owner is alive".into());
        }
    }
    Ok(())
}

/// A live resumed Host may cancel before it has replayed a crashed worker's
/// target. Its own session lease is not target execution evidence. Require the
/// exact leased control and every persisted target attempt to belong to dead
/// local workers; external operations still require independent stop evidence.
pub(super) fn observe_dispatched_target(
    application: &mut CliApplicationPort,
    conversation_id: &str,
    run_id: &str,
    request: &HostRequestEnvelope,
) -> Result<HostRequestEnvelope, String> {
    let active = application.active_run(conversation_id)?.ok_or("cancellation Run is no longer active")?;
    if active.run_id != run_id { return Err("cancellation Run changed".into()); }
    let identity = CancellationRunIdentity { conversation_id: conversation_id.into(),
        expected_run_id: run_id.into(), assistant_message_id: active.assistant_message_id
            .ok_or("cancellation assistant identity missing")? };
    let plan: nola_application::cancellation_process_observation::CancellationProcessObservationPlan = application.execute(
        ApplicationCommand::InspectCancellationProcessOwners { identity, control: request.clone() })?;
    let observations = plan.target_owners.iter().map(|owner|
        nola_application::cancellation_process_observation::CancellationProcessObservation {
            owner: owner.clone(), observation: crate::process_observation::observe_process(owner.pid),
        }).collect();
    application.execute(ApplicationCommand::ValidateCancellationProcessObservations {
        plan, current_process: crate::process_observation::observe_process(std::process::id()), observations,
    })
}

#[cfg(test)]
#[path = "cancel_only_runtime_tests.rs"]
mod tests;

fn observe_session_owner(facts: &RunCancellationDeliveryFacts) -> Result<(), String> {
    if let Some(lease) = &facts.session_lease {
        if lease.pid == 0 || crate::process_observation::process_id_is_alive(lease.pid) {
            return Err(
                "Run cancellation remains pending: session owner is live or unknown".into(),
            );
        }
    }
    Ok(())
}

struct DurableAuthority {
    port: SharedPort,
    identity: CancellationRunIdentity,
}
impl CancellationAuthority for DurableAuthority {
    fn confirm_effect_quiescent(
        &mut self,
        request: &HostRequestEnvelope,
        target: &str,
    ) -> Result<(), String> {
        let facts = facts(&self.port, &self.identity)?;
        observe_session_owner(&facts)?;
        let mut application = self.port.lock().map_err(|_| "cancellation port lock poisoned")?;
        let exact = observe_dispatched_target(&mut application,
            &self.identity.conversation_id, &self.identity.expected_run_id, request)?;
        if exact.effect_id.0 != target { return Err("cancellation target identity changed".into()); }
        Ok(())

    }

    fn request_task_cancellation(
        &mut self,
        request: &HostRequestEnvelope,
        task: &str,
    ) -> Result<bool, String> {
        let facts = facts(&self.port, &self.identity)?;
        if !facts.targets.iter().any(|fact| fact.cancellation.effect_id == request.effect_id
            && matches!(&fact.cancellation.payload, nola_core::agent::host::HostRequest::CancelTask { task_id } if task_id == task)) {
            return Err("task cancellation ownership changed".into());
        }
        self.port
            .lock()
            .map_err(|_| "cancellation port lock poisoned")?
            .execute(ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::RequestCancel {
                    task_id: task.into(),
                    now_ms: chrono::Utc::now().timestamp_millis(),
                },
            })
    }
}

struct CancelPort {
    port: SharedPort,
    identity: CancellationRunIdentity,
    digest: String,
}
impl CancelPort {
    fn terminal(&self) -> Result<bool, String> {
        let inspection: nola_domain::RunRecoveryInspection = self
            .port
            .lock()
            .map_err(|_| "cancellation port lock poisoned")?
            .execute(ApplicationCommand::InspectRunRecovery {
                conversation_id: self.identity.conversation_id.clone(),
                expected_run_id: self.identity.expected_run_id.clone(),
                assistant_message_id: self.identity.assistant_message_id.clone(),
            })?;
        Ok(matches!(
            inspection.observation,
            nola_domain::RunRecoveryObservation::Terminal { .. }
        ))
    }
}

impl CommittedRuntimeApplicationPort for CancelPort {
    type Protocol = nola_core::agent::host::AgentRuntimeProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope>, String> {
        if self.terminal()? {
            return Ok(Vec::new());
        }
        Ok(facts(&self.port, &self.identity)?
            .targets
            .into_iter()
            .map(|fact| fact.cancellation)
            .collect())
    }
    fn claim_committed_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<HostRequestEnvelope>, String> {
        if self.terminal()? {
            return Ok(Vec::new());
        }
        let observed = facts(&self.port, &self.identity)?;
        observe_session_owner(&observed)?;
        for target in &observed.targets {
            observe_target_owners(target)?;
            if let Some(attempt) = &target.cancellation_attempt {
                if !nola_application::cancellation_delivery::cancellation_delivery_reclaimable_by_expiry(
                    attempt, claim.claimed_at_ms,
                ) {
                    observe_dead_attempt(attempt)?;
                }
            }
        }
        let claimed: nola_application::CommittedHostRequests = self
            .port
            .lock()
            .map_err(|_| "cancellation port lock poisoned")?
            .execute(ApplicationCommand::ClaimCancellationHostRequests {
                input: ClaimCancellationHostRequests {
                    identity: self.identity.clone(),
                    observed,
                    worker_generation: claim.worker_generation,
                    lease_owner: claim.lease_owner,
                    claimed_at_ms: claim.claimed_at_ms,
                    lease_expires_at_ms: claim.lease_expires_at_ms,
                    limit: claim.limit,
                },
            })?;
        Ok(claimed.requests)
    }
    fn renew_effect_delivery(
        &mut self,
        renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<nola_core::agent::host::EffectDeliveryLease, String> {
        self.port
            .lock()
            .map_err(|_| "cancellation port lock poisoned")?
            .heartbeat_effect_delivery(nola_application::HeartbeatEffectDelivery {
                run_id: self.identity.expected_run_id.clone(),
                attempt_id: renewal.delivery.attempt_id,
                worker_generation: renewal.delivery.worker_generation,
                lease_owner: renewal.delivery.lease_owner,
                lease_token: renewal.delivery.lease_token,
                current_lease_expires_at_ms: renewal.delivery.lease_expires_at_ms,
                heartbeat_at_ms: renewal.renewed_at_ms,
                requested_lease_expires_at_ms: renewal.lease_expires_at_ms,
            })
    }
    fn accept_raw_host_response(
        &mut self,
        response: HostResponseEnvelope,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        let result = self
            .port
            .lock()
            .map_err(|_| "cancellation port lock poisoned")?
            .reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                conversation_id: self.identity.conversation_id.clone(),
                run_id: self.identity.expected_run_id.clone(),
                invocation_config_digest: self.digest.clone(),
                response,
                received_at_ms: nola_runtime_driver::monotonic_epoch_millis(),
            })?;
        Ok(CommittedRuntimeAcceptance {
            events: result.events,
            traces: result.traces,
            control: result.control,
        })
    }
    fn accept_boundary_input(
        &mut self,
        _: nola_core::agent::host::AgentBoundaryInput,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        Err("cancel-only transport refuses new submissions".into())
    }
}

#[cfg(test)]
pub(super) fn cancel(
    application: CliApplicationPort,
    identity: CancellationRunIdentity,
    cancellation_id: &str,
    publish: &mut dyn FnMut(nola_runtime_driver::CoreHostOutput<nola_core::agent::host::AgentRuntimeProtocol>),
) -> Result<(), String> {
    cancel_with_reason(application, identity, cancellation_id, "cancelled by user", publish)
}

pub(super) fn cancel_with_reason(
    application: CliApplicationPort,
    identity: CancellationRunIdentity,
    cancellation_id: &str,
    reason: &str,
    publish: &mut dyn FnMut(nola_runtime_driver::CoreHostOutput<nola_core::agent::host::AgentRuntimeProtocol>),
) -> Result<(), String> {
    let shared = Arc::new(Mutex::new(application));
    let receipt: nola_application::RunCancellationReceipt = shared
        .lock()
        .map_err(|_| "cancellation port lock poisoned")?
        .execute(ApplicationCommand::RequestRunCancellation {
            input: nola_application::RequestRunCancellation {
                conversation_id: identity.conversation_id.clone(),
                expected_run_id: identity.expected_run_id.clone(),
                assistant_message_id: identity.assistant_message_id.clone(),
                cancellation_id: cancellation_id.into(),
                reason: Some(reason.into()),
                requested_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })?;
    if receipt.terminal_result.is_some() {
        return Ok(());
    }
    finish_pending(shared, identity, publish)
}

/// Drain an already committed cancellation without issuing another Interrupt.
pub(super) fn resume_pending(
    application: CliApplicationPort,
    identity: CancellationRunIdentity,
    publish: &mut dyn FnMut(nola_runtime_driver::CoreHostOutput<nola_core::agent::host::AgentRuntimeProtocol>),
) -> Result<(), String> {
    finish_pending(Arc::new(Mutex::new(application)), identity, publish)
}

fn finish_pending(
    shared: SharedPort,
    identity: CancellationRunIdentity,
    publish: &mut dyn FnMut(nola_runtime_driver::CoreHostOutput<nola_core::agent::host::AgentRuntimeProtocol>),
) -> Result<(), String> {
    let active = shared
        .lock()
        .map_err(|_| "cancellation port lock poisoned")?
        .active_run(&identity.conversation_id)?
        .ok_or("cancellation Run changed before delivery")?;
    if active.run_id != identity.expected_run_id
        || active.assistant_message_id.as_deref() != Some(identity.assistant_message_id.as_str())
    {
        return Err("cancellation active Run identity changed".into());
    }
    let mut port = CancelPort {
        port: shared.clone(),
        identity: identity.clone(),
        digest: active
            .invocation_config_digest
            .ok_or("cancellation invocation digest missing")?,
    };
    let authority = DurableAuthority {
        port: shared,
        identity,
    };
    let mut runtime =
        nola_runtime_driver::CommittedRuntimeDriver::new(CancelOnlyDriver::new(authority), crate::process_observation::current_worker_identity()?)?;
    runtime
        .execute(&mut port, publish)
        .map_err(super::format_runtime_error)?;
    if !port.terminal()? {
        return Err("Run cancellation is committed but target shutdown remains unconfirmed".into());
    }
    Ok(())
}
