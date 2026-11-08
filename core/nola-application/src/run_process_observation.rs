// SPDX-License-Identifier: Apache-2.0
//! Core selects exact delivery owners. Trusted Hosts supply only OS observations.
use nola_agent_engine::agent::host::EffectDeliveryLease;
use nola_host_protocol::{
    HostInvocationBinding, HostProcessObservation, HostWorkerIdentity, HostWorkerProcessIdentity,
};
use nola_store_port::{
    effect_delivery::EffectDeliveryRepository, run_effect::RunEffectRepository,
    run_recovery_inspection::RunRecoveryInspectionRepository,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunProcessObservationPlan {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub invocation_config_digest: String,
    pub host_invocation: HostInvocationBinding,
    pub has_session_lease: bool,
    pub deliveries: Vec<EffectDeliveryLease>,
    pub process_owners: Vec<HostWorkerProcessIdentity>,
    pub has_unobservable_owner: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunProcessObservation {
    pub owner: HostWorkerProcessIdentity,
    pub observation: HostProcessObservation,
}

pub fn inspect<R>(
    store: &R,
    conversation_id: String,
    expected_run_id: String,
) -> Result<RunProcessObservationPlan, String>
where
    R: RunRecoveryInspectionRepository + EffectDeliveryRepository + RunEffectRepository + ?Sized,
{
    let run = store
        .inspect_exact_run(&expected_run_id)
        .map_err(|e| e.to_string())?
        .ok_or("Run process observation has no Run")?;
    if run.conversation_id != conversation_id
        || run.kind != "conversation"
        || run.source_host_id.is_some()
        || run.terminal.is_some()
        || run.completed_at_ms.is_some()
        || !run.assistant_owner_valid
    {
        return Err(
            "Run process observation requires an exact unfinished local conversation Run".into(),
        );
    }
    let assistant = run
        .assistant_message_id
        .ok_or("Run process observation has no assistant owner")?;
    let exact = crate::run_invocation_recovery::load_exact_run_invocation(
        store,
        conversation_id.clone(),
        expected_run_id.clone(),
        assistant.clone(),
    )?
    .ok_or("Run process observation has no frozen invocation")?;
    let deliveries = crate::effect_delivery::load_run_delivery_ownership(store, &expected_run_id)?;
    if deliveries.len() > 256 {
        return Err("Too many Run delivery owners".into());
    }
    let mut process_owners = Vec::new();
    let mut has_unobservable_owner = false;
    for lease in &deliveries {
        match HostWorkerIdentity::from_lease(&lease.worker_generation, &lease.lease_owner)? {
            HostWorkerIdentity::Process(owner) => {
                if !process_owners.contains(&owner) {
                    process_owners.push(owner);
                }
            }
            HostWorkerIdentity::Unobservable { .. } => has_unobservable_owner = true,
        }
    }
    Ok(RunProcessObservationPlan {
        conversation_id,
        run_id: expected_run_id,
        assistant_message_id: assistant,
        invocation_config_digest: exact.invocation_config_digest,
        host_invocation: exact.host_invocation,
        has_session_lease: run.has_session_lease,
        deliveries,
        process_owners,
        has_unobservable_owner,
    })
}

/// False means no death proof yet, not a failed Run. The ordinary expiry path
/// remains available. An identity/fence change is an error, never a new claim.
pub fn validate<R>(
    store: &R,
    plan: RunProcessObservationPlan,
    observations: Vec<RunProcessObservation>,
) -> Result<bool, String>
where
    R: RunRecoveryInspectionRepository + EffectDeliveryRepository + RunEffectRepository + ?Sized,
{
    let current = inspect(store, plan.conversation_id.clone(), plan.run_id.clone())?;
    if current != plan {
        return Err("Run delivery owners changed after OS observation".into());
    }
    validate_owner_observations(&plan, &observations)
}

pub fn validate_owner_observations(
    plan: &RunProcessObservationPlan,
    observations: &[RunProcessObservation],
) -> Result<bool, String> {
    if observations.len() != plan.process_owners.len() {
        return Err("Incomplete Run process observations".into());
    }
    let mut terminated = !plan.has_unobservable_owner && !plan.deliveries.is_empty();
    for owner in &plan.process_owners {
        let values: Vec<_> = observations.iter().filter(|v| &v.owner == owner).collect();
        if values.len() != 1 {
            return Err("Duplicate or missing Run process observation".into());
        }
        if matches!(values[0].observation, HostProcessObservation::Unknown { pid } if pid == owner.pid)
        {
            terminated = false;
        } else if !owner.is_terminated(&values[0].observation)? {
            terminated = false;
        }
    }
    Ok(terminated)
}
