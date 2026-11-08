// SPDX-License-Identifier: Apache-2.0
//! Real SQLite admission rollback/replay with a production-created predecessor seal.
use super::*;
use nola_store_port::run_creation::{BeginSuccessorTurnInput, InitialCoreRunEvent};
use serde_json::json;

fn fixture(path: &std::path::Path) -> (NolaStore, BeginSuccessorTurnInput) {
    let mut store = NolaStore::open_archive_mode(path, false).unwrap();
    let identity = begin(&mut store, "successor");
    let admission = json!({"admission_id":"accepted-next","submission_id":"next-run","accepted_sequence":2,
        "origin":"interactive_user","delivery":"next_turn","anchor_turn_id":"successor-actual-turn",
        "payload_digest":"a".repeat(64),"item_ids":["next-user","next-user-two","attachment"]});
    let items = json!([{"type":"message","id":"next-user","role":"user","content":"first next request"},
        {"type":"message","id":"next-user-two","role":"user","content":"second part"},
        {"type":"attachment","id":"attachment","message_id":"next-user","uri":"fixture://diagram","name":"diagram.png","mime_type":"image/png"}]);
    let checkpoint = json!({"session_id":"successor","revision":2,"last_event_cursor":{"sequence":6},
        "state":{"session_id":"successor","task_projection":{"tasks":{}},"execution":{
            "active_turn":{"turn_id":"successor-actual-turn","status":"completed"},
            "input_queue":{"next_turn":[{"admission":admission,"items":items}]}}}});
    let applied=json!([{"admission_id":"accepted-first","turn_id":"successor-actual-turn","applied_sequence":"1"}]);
    let events=vec![
        json!({"type":"input_accepted","admission":{"admission_id":"accepted-first","submission_id":"first-submission",
            "accepted_sequence":1,"origin":"interactive_user","delivery":"current_turn","anchor_turn_id":null,
            "payload_digest":"b".repeat(64),"item_ids":["successor-user"]}}),
        json!({"type":"input_applied","turn_id":"successor-actual-turn","applications":[
            {"admission_id":"accepted-first","destination_turn_id":"successor-actual-turn","applied_sequence":1}]}),
        json!({"type":"requirement_scope_changed","turn_id":"successor-actual-turn","scope":{
            "scope_id":"first-scope","revision":"1","applied_input_refs":applied,"source_input_refs":["successor-user"],
            "requirements":[{"requirement_id":"first-requirement","description":"Continue",
                "source_refs":["successor-user"],"dependency_ids":[],"goal_requirement_ref":null,"resource_scope":{"kind":"undeclared"}}],
            "source":{"kind":"runtime","code":"input_applied","source_refs":["accepted-first"]}}}),
        json!({"type":"runtime_decision_fact_observed","turn_id":"successor-actual-turn","fact":{
            "fact_id":"first-error","kind":{"kind":"error","code":"provider_failed","detail":"Provider failed"}}}),
        json!({"type":"input_accepted","admission":admission}),
        json!({"type":"run_result_decided","turn_id":"successor-actual-turn","decision":{
            "result_id":"first-result","scope_id":"first-scope","requirement_revision":"1","applied_input_refs":applied,
            "evidence_frontier":"4","value":{"kind":"failed","error_ref":"first-error","observed_effect_refs":[],"unknown_effect_refs":[]},
            "source":{"kind":"runtime","code":"provider_failed","source_refs":["first-error"]}}}),
    ].into_iter().enumerate().map(|(index,mut payload)| {
        let cursor=index as i64+1;
        payload["core_envelope"]=json!({"session_id":"successor","turn_id":"successor-actual-turn","revision":2,"cursor":{"sequence":cursor}});
        nola_store_port::run_effect::RunEventInput {
            event_id:format!("core:{}:{cursor}",identity.run_id),sequence:cursor,
            event_kind:payload["type"].as_str().unwrap().into(),payload_json:payload.to_string(),created_at_ms:2,
        }
    }).collect();
    store.begin_write_unit_of_work().unwrap();
    store.commit_transition(&CommitTransitionInput {
        transition_id:"finish-predecessor".into(),run_id:identity.run_id.clone(),expected_checkpoint_revision:1,
        checkpoint_revision:2,checkpoint_event_cursor:6,checkpoint:nola_store_port::run_effect::CheckpointWrite::Baseline {checkpoint_json:checkpoint.to_string()},
        submission_reduction_json:None,host_event:None,events,
        ensured_messages:vec![],assistant_turn_bindings:vec![],part_message_owners:Default::default(),part_mutations:vec![],
        outbox_effects:vec![],model_attempt_mutations:vec![],delegated_invocation_mutations:vec![],delegated_permission_mutations:vec![],
        effect_delivery_attempt_mutations:vec![],acknowledged_effect_ids:vec![],memory_usage_entries:vec![],
        run_status:Some("sealed".into()),assistant_status:Some("failed".into()),assistant_status_message_id:None,committed_at_ms:2,
    }).unwrap();
    store
        .commit_assistant_journal(&CommitAssistantJournal {
            stamp: AssistantSourceCommitStamp::Transition {
                transition_id: "finish-predecessor".into(),
            },
            expected: store.assistant_journal_head(&identity).unwrap(),
            batch: declaration(&identity),
            supplied_chunks: vec![],
        })
        .unwrap();
    store
        .seal_assistant_run_terminal("finish-predecessor")
        .unwrap()
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    let mut users: Vec<crate::TransitionMessageInput> = [
        ("next-user", "first next request"),
        ("next-user-two", "second part"),
    ]
    .into_iter()
    .map(|(id, text)| crate::TransitionMessageInput {
        id: id.into(),
        run_id: Some("next-run".into()),
        role: "user".into(),
        status: "completed".into(),
        created_at_ms: 3,
        completed_at_ms: Some(3),
        initial_parts: vec![crate::MessagePartInput {
            id: format!("{id}:body"),
            kind: "text".into(),
            ordinal: 1,
            state: "completed".into(),
            content_text: text.into(),
            content_json: None,
            content_digest: None,
            created_at_ms: 3,
            updated_at_ms: 3,
        }],
    })
    .collect();
    users[0].initial_parts.push(crate::MessagePartInput {
        id:"attachment".into(),kind:"attachment".into(),ordinal:2,state:"completed".into(),content_text:"diagram.png".into(),
        content_json:Some(json!({"kind":"artifact","id":"attachment","message_id":"next-user","turn_id":"next-run",
            "artifact":{"uri":"fixture://diagram","name":"diagram.png","mime_type":"image/png","operation":"attach"}}).to_string()),
        content_digest:None,created_at_ms:3,updated_at_ms:3,
    });
    let mut next = checkpoint.clone();
    next["revision"] = json!(3);
    next["last_event_cursor"]["sequence"] = json!(8);
    next["state"]["execution"]["active_turn"] =
        json!({"turn_id":"next-run","status":"awaiting_model"});
    next["state"]["execution"]["input_queue"]["next_turn"] = json!([]);
    let event = |cursor, kind: &str, payload: serde_json::Value| InitialCoreRunEvent {
        core_session_id: "successor".into(),
        core_turn_id: Some("next-run".into()),
        core_cursor: cursor,
        core_revision: 3,
        event_kind: kind.into(),
        payload_json: payload.to_string(),
    };
    let turn = BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: vec![
            event(
                7,
                "turn_started",
                json!({"type":"turn_started","session_id":"successor","turn_id":"next-run"}),
            ),
            event(
                8,
                "input_applied",
                json!({"type":"input_applied","turn_id":"next-run",
                "applications":[{"admission_id":"accepted-next","destination_turn_id":"next-run","applied_sequence":1}]}),
            ),
        ],
        conversation_id: "successor".into(),
        conversation_title: "Journal test".into(),
        user_message_id: "next-user".into(),
        user_part_id: "next-user:body".into(),
        user_text: "first next request".into(),
        user_content_json: None,
        additional_user_parts: users[0].initial_parts[1..].to_vec(),
        assistant_message_id: "next-assistant".into(),
        assistant_part_id: "next-part".into(),
        assistant_turn_bindings: vec![nola_domain::AssistantTurnBinding::Admission {
            turn_id: "next-run".into(),
            assistant_message_id: "next-assistant".into(),
        }],
        initial_assistant_part_mutations: vec![],
        run_id: "next-run".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 3,
        checkpoint_event_cursor: 8,
        checkpoint_json: next.to_string(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "config".into(),
        outbox_effects: vec![nola_store_port::run_effect::OutboxEffectInput {
            effect_id: "next-effect".into(),
            request_id: "next-request".into(),
            effect_kind: "fixture".into(),
            model_attempt_id: None,
            delegated_invocation_id: None,
            write: crate::outbox_fixture::tool(
                "next-effect",
                "next-request",
                json!({"scope":"successor"}),
            ),
        }],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        memory_usage_entries: vec![],
        created_at_ms: 3,
    };
    (
        store,
        BeginSuccessorTurnInput {
            previous_run_id: identity.run_id,
            expected_checkpoint_revision: 2,
            admission_id: "accepted-next".into(),
            originating_submission_id: "next-run".into(),
            ensured_user_messages: users,
            turn,
        },
    )
}

fn register_successor_source(
    store: &mut NolaStore,
    input: &BeginSuccessorTurnInput,
) -> nola_domain::remote_submission::SourceSubmissionReceipt {
    store.register_source_submission(
        &nola_domain::remote_submission::RemoteSubmissionIdentity {
            source_host_id: "source".into(), client_id: "client".into(),
            conversation_id: input.turn.conversation_id.clone(),
            submission_id: input.turn.run_id.clone(),
            user_message_id: input.turn.user_message_id.clone(),
            assistant_message_id: input.turn.assistant_message_id.clone(),
        },
        &json!({"request": input.turn.user_text}),
    ).unwrap().receipt
}

#[test]
fn successor_admission_reuses_users_seals_predecessor_and_replays_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("successor.sqlite");
    let (mut store, input) = fixture(&path);
    let prepared = register_successor_source(&mut store, &input);
    let mut existing = input.ensured_user_messages[0].clone();
    existing.run_id = Some(input.previous_run_id.clone());
    let mut attachment: serde_json::Value =
        serde_json::from_str(existing.initial_parts[1].content_json.as_deref().unwrap()).unwrap();
    attachment["turn_id"] = input.previous_run_id.clone().into();
    existing.initial_parts[1].content_json = Some(attachment.to_string());
    // Simulate a queued user message already displayed under its original Run.
    crate::ensure_transition_message(&store.connection, &input.turn.conversation_id, &existing)
        .unwrap();
    let result = store.begin_successor_turn(&input).unwrap();
    let admitted = store.source_submission_receipt(
        &prepared.identity, &prepared.request_digest,
    ).unwrap().unwrap();
    assert_eq!(admitted.phase, nola_domain::remote_submission::RemoteSubmissionPhase::Admitted);
    assert_eq!(admitted.admission.as_ref().unwrap().identity.run_id, input.turn.run_id);
    assert_eq!(admitted.admission.as_ref().unwrap().identity.assistant_message_id,
        input.turn.assistant_message_id);
    assert!(result.assistant_ordinal > result.user_ordinal);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT run_id FROM messages WHERE id='next-user'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        input.previous_run_id
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE id IN ('next-user','next-user-two')",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(store.pending_effects("next-run").unwrap().len(), 1);
    drop(store);
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    assert_eq!(store.begin_successor_turn(&input).unwrap(), result);
    assert_eq!(store.source_submission_receipt(
        &prepared.identity, &prepared.request_digest,
    ).unwrap().unwrap(), admitted);
    assert_eq!(store.pending_effects("next-run").unwrap().len(), 1);
    for mutation in ["run", "submission", "time"] {
        let mut changed = input.clone();
        match mutation {
            "run" => changed.turn.run_id = "other".into(),
            "submission" => changed.originating_submission_id = "other".into(),
            _ => changed.turn.created_at_ms += 1,
        };
        assert!(store.begin_successor_turn(&changed).is_err());
    }
}

#[test]
fn successor_admission_cas_and_atomic_failure_leave_original_queue_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollback.sqlite");
    let (mut store, input) = fixture(&path);
    let prepared = register_successor_source(&mut store, &input);
    let original = store.load_checkpoint("successor").unwrap().unwrap();
    let mut stale = input.clone();
    stale.expected_checkpoint_revision = 1;
    assert!(store.begin_successor_turn(&stale).is_err());
    let mut substituted = input.clone();
    substituted.ensured_user_messages[0].initial_parts[0].content_text = "substituted".into();
    assert!(store.begin_successor_turn(&substituted).is_err());
    store.connection.execute_batch("CREATE TRIGGER fail_successor BEFORE INSERT ON run_successor_admissions BEGIN SELECT RAISE(ABORT,'successor receipt fault'); END;").unwrap();
    assert!(store
        .begin_successor_turn(&input)
        .unwrap_err()
        .to_string()
        .contains("successor receipt fault"));
    assert_eq!(store.source_submission_receipt(
        &prepared.identity, &prepared.request_digest,
    ).unwrap().unwrap(), prepared, "failed successor admission cannot settle the source receipt");
    assert_eq!(
        store.load_checkpoint("successor").unwrap().unwrap(),
        original
    );
    for table in [
        "runs",
        "messages",
        "effect_outbox",
        "input_turn_applications",
    ] {
        let column = match table {
            "runs" => "id",
            "messages" => "run_id",
            "effect_outbox" => "run_id",
            _ => "source_run_id",
        };
        assert_eq!(
            store
                .connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {column}='next-run'"),
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0,
            "{table}"
        );
    }
    store
        .connection
        .execute_batch("DROP TRIGGER fail_successor")
        .unwrap();
    store.begin_successor_turn(&input).unwrap();
}

#[test]
fn successor_admission_rejects_unsettled_effects_and_unrelated_historical_user_owner() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gates.sqlite");
    let (mut store, input) = fixture(&path);
    store.begin_write_unit_of_work().unwrap();
    crate::insert_outbox_effects(
        &store.connection,
        &input.previous_run_id,
        &input.turn.outbox_effects,
        3,
    )
    .unwrap();
    assert!(store
        .begin_successor_turn(&input)
        .unwrap_err()
        .to_string()
        .contains("settled effects"));
    store.rollback_write_unit_of_work().unwrap();
    // Fault injection: unrelated owner has no admitted source or seal and must
    // never authorize the queued input, even if it claims a sealed lifecycle.
    store.connection.execute("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms) VALUES('unrelated-old-run','successor','conversation','sealed',1,0,0,0)",[]).unwrap();
    let mut historical = input.ensured_user_messages[0].clone();
    historical.run_id = Some("unrelated-old-run".into());
    crate::ensure_transition_message(&store.connection, &input.turn.conversation_id, &historical)
        .unwrap();
    assert!(store
        .begin_successor_turn(&input)
        .unwrap_err()
        .to_string()
        .contains("accepted source Run"));
    assert_eq!(
        store
            .load_checkpoint("successor")
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM runs WHERE id='next-run'", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}

#[test]
fn successor_admission_rejects_foreign_or_settled_source_receipts_atomically() {
    for case in ["conversation", "user", "assistant", "cancelled", "rejected"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source-gates.sqlite");
        let (mut store, input) = fixture(&path);
        let mut source_input = input.clone();
        match case {
            "conversation" => source_input.turn.conversation_id = "foreign".into(),
            "user" => source_input.turn.user_message_id = "foreign-user".into(),
            "assistant" => source_input.turn.assistant_message_id = "foreign-assistant".into(),
            _ => {}
        }
        let mut receipt = register_successor_source(&mut store, &source_input);
        if matches!(case, "cancelled" | "rejected") {
            receipt = store.settle_source_submission(
                &receipt.identity, &receipt.request_digest,
                (case == "rejected").then_some("test_rejection"),
            ).unwrap();
        }
        let original = store.load_checkpoint("successor").unwrap().unwrap();
        let error = store.begin_successor_turn(&input).unwrap_err().to_string();
        assert!(error.contains("source submission is not pending for this exact Run owner"), "{case}: {error}");
        assert_eq!(store.load_checkpoint("successor").unwrap().unwrap(), original);
        assert_eq!(store.source_submission_receipt(
            &receipt.identity, &receipt.request_digest,
        ).unwrap().unwrap(), receipt);
        assert_eq!(store.connection.query_row(
            "SELECT COUNT(*) FROM runs WHERE id='next-run'", [], |row| row.get::<_, i64>(0),
        ).unwrap(), 0, "{case}");
        assert!(store.pending_effects("next-run").unwrap().is_empty(), "{case}");
    }
}

#[test]
fn successor_admission_independent_connections_race_without_duplicate_facts() {
    use std::sync::{Arc, Barrier};

    for changed_request in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("successor-race.sqlite");
        let (store, input) = fixture(&path);
        drop(store);
        // Open independently before spawning so a failed setup cannot leave a
        // peer parked at the barrier. No shared Store or user-space writer lock.
        let connections = [
            NolaStore::open_archive_mode(&path, false).unwrap(),
            NolaStore::open_archive_mode(&path, false).unwrap(),
        ];
        let barrier = Arc::new(Barrier::new(3));
        let workers: Vec<_> = connections
            .into_iter()
            .enumerate()
            .map(|(index, mut store)| {
                let barrier = Arc::clone(&barrier);
                let mut request = input.clone();
                if changed_request && index == 1 {
                    // Same admission and Run, different request: never accepted as
                    // an exact replay, regardless of which connection wins SQLite.
                    request.turn.created_at_ms += 1;
                }
                std::thread::spawn(move || {
                    barrier.wait();
                    store.begin_successor_turn(&request).map_err(|error| {
                        (
                            matches!(&error, crate::StoreError::Conflict(_)),
                            error.to_string(),
                        )
                    })
                })
            })
            .collect();
        barrier.wait();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        if changed_request {
            assert_eq!(
                results.iter().filter(|result| result.is_ok()).count(),
                1,
                "{results:?}"
            );
            let error = results
                .iter()
                .find_map(|result| result.as_ref().err())
                .unwrap();
            assert!(
                error.0 && error.1.contains("replay changed its request"),
                "{error:?}"
            );
        } else {
            assert!(results.iter().all(Result::is_ok), "{results:?}");
            assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
        }
        // Reopen a third connection to inspect only committed durable facts.
        let store = NolaStore::open_archive_mode(&path, false).unwrap();
        for (table, predicate, expected) in [
            ("runs", "id='next-run'", 1_i64),
            (
                "run_successor_admissions",
                "admission_id='accepted-next'",
                1,
            ),
            (
                "input_turn_applications",
                "admission_id='accepted-next' AND source_run_id='next-run'",
                1,
            ),
            ("input_admissions", "admission_id='accepted-next'", 1),
            ("effect_outbox", "run_id='next-run'", 1),
            ("effect_outbox_payloads", "effect_id='next-effect'", 1),
            ("messages", "run_id='next-run'", 3),
            (
                "run_events",
                "run_id='next-run' AND event_kind='input_applied'",
                1,
            ),
            (
                "run_events",
                "run_id='next-run' AND event_kind='turn_started'",
                1,
            ),
        ] {
            let actual: i64 = store
                .connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                actual, expected,
                "{table}: changed_request={changed_request}"
            );
        }
        assert_eq!(
            store
                .load_checkpoint("successor")
                .unwrap()
                .unwrap()
                .revision,
            3
        );
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert!(store
            .connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none());
    }
}
