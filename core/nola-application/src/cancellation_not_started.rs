// SPDX-License-Identifier: Apache-2.0
//! Compose original target revocations into the existing cancellation commit.
use crate::*;

pub(crate) fn prepare<R: RunEffectRepository + ?Sized>(store: &R, run: &str,
    revision: i64, at: i64, state: &AgentThreadState, requests: &mut [HostRequestEnvelope])
    -> Result<Vec<nola_domain::EffectDeliveryAttemptMutation>, String> {
    let mut mutations = Vec::new();
    for request in requests {
        let HostRequest::Cancel {target_effect_id,target_not_started} = &request.payload else {continue};
        if target_not_started.is_some() || store.outbox_effect_identity(&request.effect_id.0)
            .map_err(|error| error.to_string())?.is_some() {continue;}
        // A buffered reduction can create the Cancel and consume the target's
        // terminal response before this transition is flushed. Absence here
        // supplies no NotStarted fact, but must not reject that valid flush.
        let Some(target) = state.host.pending_effects.get(&target_effect_id.0) else {continue};
        let target = target.validated_control()?.clone();
        let control = nola_domain::host_control::HostEffectControl::from_request(request)?;
        if let Some(prepared) = store.prepare_unstarted_cancellation_target(run,&control,&target,revision,at)
            .map_err(|error| format!("prepare cancellation NotStarted admission: {error}"))? {
            let HostRequest::Cancel {target_not_started,..} = &mut request.payload else {unreachable!()};
            *target_not_started = Some(prepared.proof);
            mutations.extend(prepared.mutations);
        }
    }
    Ok(mutations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_agent_engine::agent::{AgentCore, host::*, protocol::*};

    #[test]
    fn missing_current_target_preserves_control_without_asking_store_for_a_no_start_proof() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("missing-current-target.db");
        let store = nola_store::NolaStore::open(&path).unwrap();
        let core = AgentCore::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string("session".into())),
            turn_identity: crate::model_fixture::identity(),
            ..Default::default()
        }).unwrap();
        let checkpoint = core.checkpoint(CheckpointReason::Manual);
        let state_before = serde_json::to_value(&checkpoint.state).unwrap();
        let mut requests = vec![HostRequestEnvelope {
            attempt_started: false,
            attempt_id: None,
            delegated_invocation_id: None,
            request_id: "cancel-request".into(),
            effect_id: HostEffectId("cancel-effect".into()),
            host_id: HostId("host".into()),
            lease_id: None,
            session_id: "session".into(),
            turn_id: Some("run".into()),
            catalog_revision: 1,
            availability_epoch: 1,
            created_at_ms: 1000,
            deadline_ms: None,
            batch: None,
            resume_after_sequence: None,
            delivery: None,
            payload: HostRequest::Cancel {
                target_effect_id: HostEffectId("already-consumed-target".into()),
                target_not_started: None,
            },
        }];
        let controls_before = requests.clone();
        // No Application writer transaction exists here. A mistaken call to
        // Store's proof preparation would fail its is_autocommit guard.
        let mutations = prepare(&store, "run", 2, 1100, &checkpoint.state, &mut requests).unwrap();
        assert!(mutations.is_empty());
        assert_eq!(requests, controls_before);
        assert_eq!(serde_json::to_value(&checkpoint.state).unwrap(), state_before);
        let db = rusqlite::Connection::open(&path).unwrap();
        for table in ["effect_delivery_attempts", "effect_outbox", "run_events", "core_checkpoints"] {
            let count: i64 = db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0)).unwrap();
            assert_eq!(count, 0, "a missing current target cannot create {table} facts");
        }
        // This is the optional preparation guard only, not a fabricated
        // accepted buffered Host-response integration or an OS stop proof.
    }
}
