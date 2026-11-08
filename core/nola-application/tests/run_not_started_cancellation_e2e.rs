// SPDX-License-Identifier: Apache-2.0
//! Transactional non-dispatch proof cannot be inferred from an empty Host registry.
mod support;
use nola_agent_engine::agent::{host::*, protocol::*};
use nola_application::*;
use nola_domain::{EffectDeliveryAttemptState, EffectDeliverySettlement};
use nola_store::NolaStore;
use nola_store_port::assistant_journal::AssistantJournalRepository;

fn admission() -> BeginAssistantTurn {
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "Unsent fixture model".into(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type":"object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000, persist_full_output: true, allow_artifacts: true,
        },
        supports_streaming: true,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    BeginAssistantTurn {
        conversation_id: "conversation".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_title: "Cancel before provider entry".into(),
        user_message_id: "user".into(),
        user_part_id: "user-part".into(),
        user_text: "Work".into(),
        user_content_json: None,
        assistant_message_id: "assistant".into(),
        context_checkpoint_id: None,
        core_config: AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string("conversation".into())),
            model: Some("fixture".into()),
            host_catalog: Some(HostCapabilityCatalog {
                host_id: HostId("fixture".into()), revision: 1, availability_epoch: 1,
                lease: None,
                capabilities: [("model.stream".into(), capability)].into_iter().collect(),
            }),
            ..Default::default()
        },
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string("user".into()), "Work")],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some("run".into()),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        },
        created_at_ms: 1000,
    }
}

fn cancellation(run: &str, at: i64) -> ApplicationCommand {
    ApplicationCommand::RequestRunCancellation {
        input: RequestRunCancellation {
            conversation_id: "conversation".into(), expected_run_id: run.into(),
            assistant_message_id: "assistant".into(), cancellation_id: "cancel-before-entry".into(),
            reason: Some("Escape before provider entry".into()), requested_at_ms: at,
        },
    }
}

fn claim(store: &mut NolaStore, run: &str, at: i64) -> HostRequestEnvelope {
    let mut claimed = claim_committed_host_requests(store, ClaimCommittedHostRequests {
        run_id: run.into(), worker_generation: "generation".into(), lease_owner: "owner".into(),
        claimed_at_ms: at, lease_expires_at_ms: at + 10_000, limit: 64,
    }).unwrap();
    assert_eq!(claimed.requests.len(), 1);
    claimed.requests.remove(0)
}

fn response(begun: &AcceptedAssistantTurn, request: &HostRequestEnvelope,
    sequence: u64, at: i64, payload: HostResponse) -> ApplicationCommand {
    ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "conversation".into(), run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(), received_at_ms: at,
            response: HostResponseEnvelope {
                request_id: request.request_id.clone(), effect_id: request.effect_id.clone(),
                attempt_id: request.attempt_id.clone(), delegated_invocation_id: request.delegated_invocation_id.clone(),
                host_id: request.host_id.clone(), lease_id: request.lease_id.clone(),
                session_id: request.session_id.clone(), turn_id: request.turn_id.clone(),
                catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                delivery: request.delivery.clone(), sequence, payload,
            },
        },
    }
}

fn snapshot(path: &std::path::Path) -> Vec<String> {
    let db = rusqlite::Connection::open(path).unwrap();
    [
        "SELECT record_json FROM effect_delivery_attempts ORDER BY id",
        "SELECT payload_json FROM host_event_inbox ORDER BY event_id",
        "SELECT payload_json FROM run_events ORDER BY run_id,sequence",
        "SELECT checkpoint_json FROM core_checkpoint_bodies ORDER BY conversation_id",
    ].iter().map(|query| {
        let mut statement = db.prepare(query).unwrap();
        let rows = statement.query_map([], |row| row.get::<_, String>(0)).unwrap();
        serde_json::to_string(&rows.collect::<Result<Vec<_>, _>>().unwrap()).unwrap()
    }).collect()
}

#[test]
fn cold_nonexecution_proof_revokes_prepared_or_leased_target_and_rejects_late_start() {
    for stage in ["unclaimed", "prepared", "leased"] {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("not-started.db");
        let mut store = NolaStore::open(&path).unwrap();
        let begun = begin_assistant_turn(&mut store, admission()).unwrap();
        let run = &begun.begin.run_id;
        let target = if stage == "leased" {
            claim(&mut store, run, 1100)
        } else {
            load_committed_host_requests(&store, run).unwrap().requests.remove(0)
        };
        let original_attempt = if stage == "prepared" {
            let mutation = nola_kernel::prepare_effect_delivery_attempt(
                "prepared-target", run, &target.effect_id.0, &target.request_id, 1,
                nola_domain::EffectDeliveryReplaySafety::Recoverable, None, 1100,
            ).unwrap();
            store.commit_effect_delivery_attempt_mutations(&[mutation]).unwrap();
            Some("prepared-target".to_owned())
        } else {
            target.delivery.as_ref().map(|lease| lease.attempt_id.clone())
        };
        let original_record = original_attempt.as_ref()
            .map(|id| store.effect_delivery_attempt(id).unwrap().unwrap());
        drop(store);
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime.execute(cancellation(run, 1200)).unwrap();
        drop(runtime);

        // Reopen, rather than trusting a hot Host's absent local registry.
        let mut store = NolaStore::open(&path).unwrap();
        let control = claim(&mut store, run, 1300);
        let HostRequest::Cancel { target_effect_id, target_not_started: Some(proof) } = &control.payload else {
            panic!("{stage} must produce exact durable non-execution proof")
        };
        proof.validate().unwrap();
        assert!(proof.matches_control(&control.host_id.0, &control.session_id,
            control.turn_id.as_deref(), &control.effect_id.0, &control.request_id, &target.effect_id.0));
        assert_eq!(target_effect_id, &target.effect_id);
        assert_eq!(proof.conversation_id, "conversation");
        assert_eq!(proof.target_request_id, target.request_id);
        assert_eq!(proof.revoked_attempts.len(), usize::from(stage != "unclaimed"));
        if let Some(id) = &original_attempt {
            let revoked = store.effect_delivery_attempt(id).unwrap().unwrap();
            assert_eq!(revoked.state, EffectDeliveryAttemptState::Settled);
            assert_eq!(revoked.settlement, Some(EffectDeliverySettlement::RejectedBeforeExecution));
            assert_eq!(revoked.execution_started_at_ms, None);
            assert_eq!(proof.revoked_attempts[0].attempt_id, *id);
            assert_eq!(proof.revoked_attempts[0].revision, revoked.revision);
        }
        if stage == "prepared" {
            let stale_claim = nola_kernel::claim_effect_delivery_attempt(
                original_record.as_ref().unwrap(), "late-generation", "late-owner", "late-token", 10_000, 1301,
            ).unwrap();
            let before = snapshot(&path);
            assert!(store.commit_effect_delivery_attempt_mutations(&[stale_claim]).is_err());
            assert_eq!(snapshot(&path), before, "a pre-revocation Prepared snapshot cannot be claimed late");
        }
        drop(store);
        let mut runtime = support::open_runtime(&path).unwrap();
        if stage == "leased" {
            let before = snapshot(&path);
            assert!(runtime.execute(response(&begun, &target, 1, 1301, HostResponse::ExecutionStarted)).is_err());
            assert_eq!(snapshot(&path), before, "the revoked old lease cannot start or leave an inbox fact");
        }
        runtime.execute(response(&begun, &control, 1, 1302, HostResponse::ExecutionStarted)).unwrap();
        runtime.execute(response(&begun, &control, 2, 1303, HostResponse::CancellationConfirmed {
            target_effect_id: target.effect_id.clone(),
        })).unwrap();
        drop(runtime);
        let store = NolaStore::open(&path).unwrap();
        let seal = store.local_assistant_run_terminal_seal("conversation", run).unwrap().unwrap();
        assert!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Cancelled { .. }));
        assert!(load_committed_host_requests(&store, run).unwrap().requests.is_empty());
        let db = rusqlite::Connection::open(&path).unwrap();
        let starts: i64 = db.query_row(
            "SELECT count(*) FROM host_event_inbox WHERE json_extract(payload_json,'$.effect_id')=?1
             AND json_extract(payload_json,'$.payload.type') IN ('execution_started','model_attempt_started','model_completed')",
            [&target.effect_id.0], |row| row.get(0),
        ).unwrap();
        assert_eq!(starts, 0, "cancelling unsent work must not fabricate provider execution");
    }
}

#[test]
fn a_started_predecessor_prevents_proof_even_when_current_successor_has_not_started() {
    for successor in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("started-history.db");
        let mut store = NolaStore::open(&path).unwrap();
        let begun = begin_assistant_turn(&mut store, admission()).unwrap();
        let run = &begun.begin.run_id;
        let target = claim(&mut store, run, 1100);
        drop(store);
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime.execute(response(&begun, &target, 1, 1101, HostResponse::ExecutionStarted)).unwrap();
        drop(runtime);
        if successor {
            let mut store = NolaStore::open(&path).unwrap();
            let current = store.effect_delivery_attempt(&target.delivery.as_ref().unwrap().attempt_id).unwrap().unwrap();
            let interrupted = nola_kernel::interrupt_effect_delivery_attempt(
                &current, current.worker_generation.as_deref().unwrap(), current.lease_owner.as_deref().unwrap(),
                current.lease_token.as_deref().unwrap(), nola_domain::EffectDeliveryRecoveryPlan::Resume,
                "original worker lost after entry", 1150,
            ).unwrap();
            let next = nola_kernel::claim_interrupted_effect_delivery_successor(
                &interrupted.record, "unstarted-successor", &target.request_id,
                "successor-generation", "successor-owner", "successor-token", 10_000, 1160,
            ).unwrap();
            store.commit_effect_delivery_attempt_mutations(&[interrupted, next]).unwrap();
        }
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime.execute(cancellation(run, 1200)).unwrap();
        drop(runtime);
        let mut store = NolaStore::open(&path).unwrap();
        let control = claim(&mut store, run, 1300);
        assert!(matches!(&control.payload, HostRequest::Cancel { target_not_started: None, .. }),
            "one unstarted successor cannot erase its started predecessor");
        assert!(store.local_assistant_run_terminal_seal("conversation", run).unwrap().is_none());
        assert_eq!(store.effect_delivery_attempt(&target.delivery.as_ref().unwrap().attempt_id)
            .unwrap().unwrap().execution_started_at_ms, Some(1101));
    }
}

#[test]
fn failed_revocation_rolls_back_cancel_control_and_checkpoint_without_revoking_old_lease() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("revocation-rollback.db");
    let mut store = NolaStore::open(&path).unwrap();
    let begun = begin_assistant_turn(&mut store, admission()).unwrap();
    let target = claim(&mut store, &begun.begin.run_id, 1100);
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_nonexecution_revocation BEFORE UPDATE OF state ON effect_delivery_attempts
        WHEN NEW.state='settled' AND json_extract(NEW.record_json,'$.settlement')='rejected_before_execution'
        BEGIN SELECT RAISE(ABORT,'nonexecution revocation fault'); END;").unwrap();
    let before = snapshot(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert!(runtime.execute(cancellation(&begun.begin.run_id, 1200)).is_err());
    assert_eq!(snapshot(&path), before, "failed proof construction is not a partial revocation");
    let store = NolaStore::open(&path).unwrap();
    let current = store.effect_delivery_attempt(&target.delivery.as_ref().unwrap().attempt_id).unwrap().unwrap();
    assert_eq!(current.state, EffectDeliveryAttemptState::Leased);
    assert!(load_committed_host_requests(&store, &begun.begin.run_id).unwrap().requests
        .iter().all(|request| !matches!(request.payload, HostRequest::Cancel { .. })));
}

#[test]
fn hot_cancel_claim_preserves_atomically_admitted_proof_after_late_start_rejection() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("hot-not-started.db");
    let mut store = NolaStore::open(&path).unwrap();
    let begun = begin_assistant_turn(&mut store, admission()).unwrap();
    let target = claim(&mut store, &begun.begin.run_id, 1100);
    drop(store);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime.execute(cancellation(&begun.begin.run_id, 1200)).unwrap();
    assert!(runtime.execute(response(&begun, &target, 1, 1201, HostResponse::ExecutionStarted)).is_err());
    let controls: CommittedHostRequests = serde_json::from_value(runtime.execute(
        ApplicationCommand::ClaimCommittedHostRequests { input: ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(), worker_generation: "control-worker".into(),
            lease_owner: "control-owner".into(), claimed_at_ms: 1300,
            lease_expires_at_ms: 10_000, limit: 64,
        }}).unwrap()).unwrap();
    assert_eq!(controls.requests.len(), 1);
    let control = &controls.requests[0];
    let HostRequest::Cancel { target_effect_id, target_not_started: Some(proof) } = &control.payload else {
        panic!("same hot runtime must materialize durable cancellation proof")
    };
    assert_eq!(target_effect_id, &target.effect_id);
    assert_eq!(proof.revoked_attempts[0].attempt_id, target.delivery.as_ref().unwrap().attempt_id);
    runtime.execute(response(&begun, control, 1, 1301, HostResponse::ExecutionStarted)).unwrap();
    runtime.execute(response(&begun, control, 2, 1302, HostResponse::CancellationConfirmed {
        target_effect_id: target.effect_id,
    })).unwrap();
    drop(runtime);
    let store = NolaStore::open(&path).unwrap();
    let seal = store.local_assistant_run_terminal_seal("conversation", &begun.begin.run_id).unwrap().unwrap();
    assert!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Cancelled { .. }));
}
