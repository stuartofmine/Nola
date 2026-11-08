// SPDX-License-Identifier: Apache-2.0
//! Core selects exact cancellation owners; trusted Hosts only observe the OS.
use crate::cancellation_delivery::{
    supports_dead_owner_confirmation,
    CancellationRunIdentity, CancellationTargetFact, RunCancellationDeliveryFacts,
};
use nola_agent_engine::agent::host::HostRequestEnvelope;
use nola_host_protocol::{HostProcessObservation, HostWorkerProcessIdentity};
use nola_store_port::cancellation_delivery::CancellationDeliveryRepository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancellationProcessObservationPlan {
    pub observed: RunCancellationDeliveryFacts,
    pub control: HostRequestEnvelope,
    pub control_owner: HostWorkerProcessIdentity,
    pub target_owners: Vec<HostWorkerProcessIdentity>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancellationProcessObservation {
    pub owner: HostWorkerProcessIdentity,
    pub observation: HostProcessObservation,
}

pub fn cancellation_target_process_owners(
    fact: &CancellationTargetFact,
) -> Result<Vec<HostWorkerProcessIdentity>, String> {
    if !supports_dead_owner_confirmation(fact) {
        return Err("External target requires independent cancellation reconciliation".into());
    }
    let mut owners = Vec::new();
    for attempt in fact.latest_attempt.iter().chain(fact.prior_attempts.iter()) {
        if attempt.worker_generation.is_none()
            && attempt.lease_owner.is_none()
            && attempt.execution_started_at_ms.is_none()
        {
            continue;
        }
        let owner = HostWorkerProcessIdentity::from_lease(
            attempt
                .worker_generation
                .as_deref()
                .ok_or("Cancellation target generation is missing")?,
            attempt
                .lease_owner
                .as_deref()
                .ok_or("Cancellation target process owner is missing")?,
        )?;
        if !owners.contains(&owner) {
            owners.push(owner);
        }
    }
    Ok(owners)
}

pub fn inspect_cancellation_process_owners<R: CancellationDeliveryRepository + ?Sized>(
    store: &R,
    identity: CancellationRunIdentity,
    control: HostRequestEnvelope,
) -> Result<CancellationProcessObservationPlan, String> {
    inspect_with_checkpoint(store, identity, control, None)
}

/// Validate the exact pending control and its committed physical delivery fence.
/// This establishes control identity only: it neither proves target quiescence
/// nor assumes that process death can reconcile an external operation.
pub fn validate_cancellation_control_binding(
    fact: &CancellationTargetFact,
    control: &HostRequestEnvelope,
) -> Result<HostWorkerProcessIdentity, String> {
    let mut expected = fact.cancellation.clone();
    expected.delivery = None;
    let mut supplied = control.clone();
    supplied.delivery = None;
    if expected != supplied {
        return Err("Cancellation control identity changed".into());
    }
    let attempt = fact
        .cancellation_attempt
        .as_ref()
        .ok_or("Cancellation control has no delivery owner")?;
    let delivery = control
        .delivery
        .as_ref()
        .ok_or("Cancellation control delivery is missing")?;
    if attempt.id != delivery.attempt_id
        || attempt.worker_generation.as_ref() != Some(&delivery.worker_generation)
        || attempt.lease_owner.as_ref() != Some(&delivery.lease_owner)
        || attempt.lease_token.as_ref() != Some(&delivery.lease_token)
        || attempt.lease_expires_at_ms != Some(delivery.lease_expires_at_ms)
    {
        return Err("Cancellation control delivery fence changed".into());
    }
    HostWorkerProcessIdentity::from_lease(&delivery.worker_generation, &delivery.lease_owner)
}

pub(crate) fn inspect_with_checkpoint<R: CancellationDeliveryRepository + ?Sized>(
    store: &R, identity: CancellationRunIdentity, control: HostRequestEnvelope,
    checkpoint: Option<&crate::cancellation_delivery::CancellationCheckpoint>,
) -> Result<CancellationProcessObservationPlan, String> {
    let observed = crate::cancellation_delivery::load_run_cancellation_delivery_facts_with_checkpoint(store, &identity, checkpoint)?;
    let fact = observed
        .targets
        .iter()
        .find(|fact| {
            fact.cancellation.effect_id == control.effect_id
                && fact.cancellation.request_id == control.request_id
        })
        .ok_or("Cancellation control is no longer pending")?;
    let control_owner = validate_cancellation_control_binding(fact, &control)?;
    let target_owners = cancellation_target_process_owners(fact)?;
    Ok(CancellationProcessObservationPlan {
        observed,
        control,
        control_owner,
        target_owners,
    })
}

pub fn validate_cancellation_process_observations<R: CancellationDeliveryRepository + ?Sized>(
    store: &R,
    plan: CancellationProcessObservationPlan,
    current_process: HostProcessObservation,
    observations: Vec<CancellationProcessObservation>,
) -> Result<HostRequestEnvelope, String> {
    validate_with_checkpoint(store, plan, current_process, observations, None)
}

pub(crate) fn validate_with_checkpoint<R: CancellationDeliveryRepository + ?Sized>(
    store: &R, plan: CancellationProcessObservationPlan, current_process: HostProcessObservation,
    observations: Vec<CancellationProcessObservation>,
    checkpoint: Option<&crate::cancellation_delivery::CancellationCheckpoint>,
) -> Result<HostRequestEnvelope, String> {
    let current = inspect_with_checkpoint(store, plan.observed.identity.clone(), plan.control.clone(), checkpoint)?;
    if current != plan {
        return Err("Cancellation owners changed after OS observation".into());
    }
    if !matches!(current_process, HostProcessObservation::Running { .. })
        || plan.control_owner.is_terminated(&current_process)?
    {
        return Err("Cancellation control is not owned by the observing process".into());
    }
    if observations.len() != plan.target_owners.len() {
        return Err("Incomplete cancellation process observations".into());
    }
    for owner in &plan.target_owners {
        let values: Vec<_> = observations
            .iter()
            .filter(|value| &value.owner == owner)
            .collect();
        if values.len() != 1 || !owner.is_terminated(&values[0].observation)? {
            return Err("Cancellation target owner remains alive or unobserved".into());
        }
    }
    plan.observed
        .targets
        .into_iter()
        .find(|fact| fact.cancellation.effect_id == plan.control.effect_id)
        .and_then(|fact| fact.target)
        .ok_or("Cancellation target is missing".into())
}
