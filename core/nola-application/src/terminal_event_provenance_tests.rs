// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_domain::core_event::EventCursor;

#[test]
fn pending_recovery_accounting_keeps_frontier_without_expanding_ordinary_events() {
    use nola_agent_engine::agent::{AgentEngineeringLoopState, TurnId};
    for pending in [false, true] {
        let mut state = AgentEngineeringLoopState::default();
        if pending {
            state.no_progress_recovery = serde_json::from_value(
                serde_json::json!({"state":"pending","reason":"unchanged observations"}),
            ).unwrap();
            state.strategy_recovery.model_requests.insert(4);
        }
        let original_state = serde_json::to_value(&state).unwrap();
        let envelope = CoreEventEnvelope {
            session_id: "session".into(), turn_id: Some("turn".into()),
            revision: 47, cursor: EventCursor { sequence: 103 },
            event: AgentEvent::EngineeringLoopUpdated {
                turn_id: TurnId::from_string("turn".into()), state,
            },
        };
        let stored = committed_event_payload_for_persistence(&envelope).unwrap();
        assert_eq!(stored["state"], original_state);
        assert_eq!(stored.get("core_envelope").is_some(), pending);
        if pending {
            assert_eq!(stored["core_envelope"], serde_json::to_value(envelope.source()).unwrap());
        }
    }
}

#[test]
fn terminal_event_provenance_keeps_actual_frontier_and_unmodified_outcome() {
    let mut events = vec![
        serde_json::json!({"type":"turn_failed","session_id":"core-session","turn_id":"turn","error":"failure"}),
        serde_json::json!({"type":"turn_cancelled","session_id":"core-session","turn_id":"turn","reason":"user"}),
    ];
    for outcome in ["answered", "action_completed", "needs_input", "blocked", "incomplete"] {
        events.push(serde_json::json!({"type":"turn_completed","session_id":"core-session","turn_id":"turn","outcome":outcome}));
    }
    for value in events {
        let envelope = CoreEventEnvelope {
            session_id: "core-session".into(), turn_id: Some("turn".into()),
            revision: 47, cursor: EventCursor { sequence: 103 },
            event: serde_json::from_value(value.clone()).unwrap(),
        };
        let mut stored = committed_event_payload_for_persistence(&envelope).unwrap();
        assert_eq!(stored.as_object_mut().unwrap().remove("core_envelope").unwrap(),
            serde_json::json!({"session_id":"core-session","turn_id":"turn","revision":47,"cursor":{"sequence":103}}));
        assert_eq!(stored, value, "persisting provenance must not reinterpret the business result");
    }
}

fn input_fixture_config() -> AgentCoreConfig {
    use nola_agent_engine::agent::host::*;
    use nola_agent_engine::agent::SessionId;
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(), title: "fixture".into(), route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available, exposure: HostCapabilityExposure::Internal,
        permission_domain: None, input_schema: serde_json::json!({"type":"object"}), input_normalization: Default::default(),
        output_policy: HostOutputPolicy {preview_char_limit: 4000, persist_full_output: true, allow_artifacts: true},
        supports_streaming: true, supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial, cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    AgentCoreConfig {
        turn_identity: crate::model_fixture::identity(),
        session_id: Some(SessionId::from_string("conversation".into())), model: Some("fixture".into()),
        host_catalog: Some(HostCapabilityCatalog {host_id: HostId("host".into()), revision: 1,
            availability_epoch: 1, lease: None, capabilities: [("model.stream".into(),capability)].into_iter().collect()}),
        ..Default::default()
    }
}

fn persisted_input_prefix(path: &std::path::Path, run: &str) -> Vec<(i64, String, String)> {
    let connection = rusqlite::Connection::open_with_flags(
        path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    let mut query = connection.prepare(
        "SELECT p.run_sequence,a.submission_id,p.chain_digest FROM run_input_applications p
         JOIN input_admissions a USING(core_session_id,admission_id)
         WHERE p.run_id=?1 ORDER BY p.run_sequence",
    ).unwrap();
    query.query_map([run], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap().collect::<Result<Vec<_>, _>>().unwrap()
}

#[test]
fn queued_successor_owns_new_run_and_original_admission_across_reopen() {
    use nola_agent_engine::agent::{AgentInputDelivery, AgentItem, ItemId};
    use nola_agent_engine::agent::host::{HostResponse, HostResponseEnvelope};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("successor.sqlite");
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let input = |id: &str, delivery| AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string(id.into()), "Work")],
        turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
        submission_id: Some(id.into()), submitted_at_epoch_ms: Some(1000), delivery,
    };
    let accepted = crate::begin_assistant_turn(&mut store, crate::BeginAssistantTurn {
        expected_configuration_revision: None, expected_parent_message_id: None,
        conversation_id: "conversation".into(), conversation_title: "Successor".into(),
        user_message_id: "origin".into(), user_part_id: "user-body".into(), user_text: "Work".into(),
        user_content_json: None, assistant_message_id: "assistant".into(), context_checkpoint_id: None,
        core_config: input_fixture_config(), submission: input("origin", AgentInputDelivery::CurrentTurn), created_at_ms: 1000,
    }).unwrap();
    let queued = crate::reduce_and_commit_active_submission(&mut store, crate::ReduceAndCommitActiveSubmission {
        conversation_id: "conversation".into(), run_id: accepted.begin.run_id.clone(), transition_id: "queued-next".into(),
        invocation_config_digest: accepted.invocation_config_digest.clone(),
        submission: input("next", AgentInputDelivery::NextTurn), committed_at_ms: 1001,
    }).unwrap();
    let admission_id = queued.events.iter().find_map(|event| match &event.event {
        AgentEvent::InputAccepted { admission } => Some(admission.admission_id.clone()), _ => None,
    }).unwrap();
    let original_prefix = persisted_input_prefix(&path, &accepted.begin.run_id);
    assert_eq!(original_prefix.len(), 1, "queued successor is not applied to its parent's scope");
    assert_eq!((original_prefix[0].0, original_prefix[0].1.as_str()), (1, "origin"));
    assert_eq!(original_prefix[0].2.len(), 64);
    assert!(persisted_input_prefix(&path, "next").is_empty());
    let request = crate::claim_committed_host_requests(&mut store, crate::ClaimCommittedHostRequests {
        run_id: accepted.begin.run_id.clone(), worker_generation: "fixture".into(), lease_owner: "fixture".into(),
        claimed_at_ms: 1001, lease_expires_at_ms: 9000, limit: 8,
    }).unwrap().requests[0].clone();
    for (sequence, payload) in [
        (1, HostResponse::ExecutionStarted),
        (2, HostResponse::ModelAttemptStarted {provider:"fixture".into(), model:"fixture".into(), reason:"initial".into()}),
        (3, HostResponse::ModelCompleted {
            items: vec![serde_json::to_value(AgentItem::assistant_message(ItemId::from_string("answer".into()), "Done")).unwrap()],
            tool_calls: vec![], usage: serde_json::json!({"input_tokens":10,"output_tokens":2}),
        }),
    ] {
        crate::reduce_and_accept_host_response(&mut store, crate::ReduceAndAcceptHostResponse {
            conversation_id: "conversation".into(), run_id: accepted.begin.run_id.clone(),
            invocation_config_digest: accepted.invocation_config_digest.clone(), received_at_ms: 1001 + sequence as i64,
            response: HostResponseEnvelope {
                request_id: request.request_id.clone(), attempt_id: request.attempt_id.clone(),
                effect_id: request.effect_id.clone(), delegated_invocation_id: request.delegated_invocation_id.clone(),
                host_id: request.host_id.clone(), lease_id: request.lease_id.clone(), session_id: request.session_id.clone(),
                turn_id: request.turn_id.clone(), catalog_revision: request.catalog_revision,
                availability_epoch: request.availability_epoch, delivery: request.delivery.clone(), sequence, payload,
            },
        }).unwrap();
    }
    assert!(store.active_run("conversation").unwrap().is_none());
    assert!(crate::load_committed_host_requests(&store, &accepted.begin.run_id).unwrap().requests.is_empty());
    drop(store);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let promoted = crate::promote_queued_assistant_turn(&mut store, crate::PromoteQueuedAssistantTurn {
        conversation_id: "conversation".into(), created_at_ms: 2000,
    }).unwrap().unwrap();
    assert_eq!(promoted.begin.run_id, "next");
    assert_eq!(promoted.invocation_config_digest, accepted.invocation_config_digest);
    assert!(promoted.events.iter().all(|event| !matches!(event.event, AgentEvent::InputAccepted { .. })));
    assert!(promoted.events.iter().any(|event| matches!(&event.event,
        AgentEvent::InputApplied { applications, .. } if applications.len() == 1 && applications[0].admission_id == admission_id)));
    assert!(crate::promote_queued_assistant_turn(&mut store, crate::PromoteQueuedAssistantTurn {
        conversation_id: "conversation".into(), created_at_ms: 2001,
    }).unwrap().is_none());
    drop(store);
    assert_eq!(persisted_input_prefix(&path, &accepted.begin.run_id), original_prefix,
        "promotion and its repeat cannot extend or rewrite the parent input frontier");
    let successor_prefix = persisted_input_prefix(&path, "next");
    assert_eq!(successor_prefix.len(), 1);
    assert_eq!((successor_prefix[0].0, successor_prefix[0].1.as_str()), (1, "next"));
    assert_ne!(successor_prefix[0].2, original_prefix[0].2,
        "equal-length scopes with different inputs must have different digests");
    let db = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let owners: (String,String) = db.query_row(
        "SELECT a.source_run_id,p.source_run_id FROM input_admissions a JOIN input_turn_applications p USING(core_session_id,admission_id) WHERE a.admission_id=?1",
        [&admission_id], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(owners, ("origin".into(), "next".into()));
}

#[test]
fn input_admission_is_persisted_by_real_application_before_and_after_restart() {
    use nola_agent_engine::agent::{AgentInputDelivery, AgentItem, ItemId};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input-source.sqlite");
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let config = input_fixture_config();
    let input = |id: &str, delivery| AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string(id.into()), "Work")],
        turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
        submission_id: Some(id.into()), submitted_at_epoch_ms: Some(1000), delivery,
    };
    let accepted = crate::begin_assistant_turn(&mut store, crate::BeginAssistantTurn {
        expected_configuration_revision: None, expected_parent_message_id: None,
        conversation_id: "conversation".into(), conversation_title: "Input sources".into(),
        user_message_id: "origin".into(), user_part_id: "user-body".into(), user_text: "Work".into(),
        user_content_json: None, assistant_message_id: "assistant".into(), context_checkpoint_id: None,
        core_config: config, submission: input("origin", AgentInputDelivery::CurrentTurn), created_at_ms: 1000,
    }).unwrap();
    crate::reduce_and_commit_active_submission(&mut store, crate::ReduceAndCommitActiveSubmission {
        conversation_id: "conversation".into(), run_id: accepted.begin.run_id.clone(), transition_id: "queued-next".into(),
        invocation_config_digest: accepted.invocation_config_digest.clone(),
        submission: input("next", AgentInputDelivery::NextTurn), committed_at_ms: 1001,
    }).unwrap();
    drop(store);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    crate::reduce_and_commit_active_submission(&mut store, crate::ReduceAndCommitActiveSubmission {
        conversation_id: "conversation".into(), run_id: accepted.begin.run_id.clone(), transition_id: "queued-current".into(),
        invocation_config_digest: accepted.invocation_config_digest,
        submission: input("current", AgentInputDelivery::CurrentTurn), committed_at_ms: 1002,
    }).unwrap();
    drop(store);
    let connection = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut statement = connection.prepare("SELECT payload_json FROM run_events WHERE run_id=?1 AND event_kind IN ('input_accepted','input_applied') ORDER BY sequence").unwrap();
    let rows = statement.query_map([accepted.begin.run_id], |row| row.get::<_,String>(0)).unwrap()
        .map(|row| serde_json::from_str::<Value>(&row.unwrap()).unwrap()).collect::<Vec<_>>();
    let admissions = rows.iter().filter(|row| row["type"] == "input_accepted").collect::<Vec<_>>();
    assert_eq!(admissions.len(), 3);
    for (index, id) in ["origin", "next", "current"].into_iter().enumerate() {
        assert_eq!(admissions[index]["admission"]["submission_id"], id);
        assert_eq!(admissions[index]["admission"]["accepted_sequence"], (index + 1) as u64);
    }
    for row in &rows {
        assert!(row["core_envelope"]["cursor"]["sequence"].as_u64().unwrap() > 0);
    }
    let applied = rows.iter().filter(|row| row["type"] == "input_applied").collect::<Vec<_>>();
    assert_eq!(applied.len(), 1, "queued inputs must not be credited as applied");
    assert_eq!(applied[0]["applications"][0]["admission_id"], admissions[0]["admission"]["admission_id"]);
    assert_eq!(applied[0]["applications"][0]["applied_sequence"], 1);
    assert!(applied[0].get("items").is_none(), "initial event must not duplicate canonical input body");
    let indexed: (i64,i64,i64) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM input_admissions),
                (SELECT COUNT(*) FROM input_turn_applications),
                (SELECT COUNT(*) FROM input_admissions WHERE provenance='current_frontier')",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(indexed,(3,1,3),"both real admission paths commit exact indices across reopen");
    let prefix = persisted_input_prefix(&path, "origin");
    assert_eq!(prefix.len(), 1, "neither queued delivery mode is applied yet");
    assert_eq!((prefix[0].0, prefix[0].1.as_str()), (1, "origin"));
}
