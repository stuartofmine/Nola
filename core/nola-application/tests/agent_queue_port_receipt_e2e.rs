// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    AgentCommand, ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
};
use serde_json::json;

#[test]
fn empty_queue_observation_has_no_lease_or_mutation_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("queue-observation.sqlite");
    let mut runtime = support::open_runtime(&database).unwrap();
    let db = rusqlite::Connection::open(&database).unwrap();
    let before: i64 = db.query_row("SELECT count(*) FROM command_receipts", [], |row| row.get(0)).unwrap();
    for _ in 0..100 {
        let command = ApplicationCommand::PeekAgentCommand { session_id: "owner".into() };
        assert!(command.is_read_only_query());
        assert_eq!(runtime.execute(command).unwrap(), json!(null));
    }
    let after: i64 = db.query_row("SELECT count(*) FROM command_receipts", [], |row| row.get(0)).unwrap();
    assert_eq!(after, before);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_commands", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn public_queue_rejects_invalid_or_internal_payloads_without_inserting_rows() {
    let directory=tempfile::tempdir().unwrap();let database=directory.path().join("invalid.sqlite");
    let mut runtime=support::open_runtime(&database).unwrap();
    for payload in ["{", "{}", "null", r#"{"type":"future_unknown"}"#,
        r#"{"type":"user_input"}"#,
        r#"{"type":"task_terminal_available","task_id":"t","receipt_id":"r"}"#,
        r#"{"type":"task_guidance_available","task_id":"t","message_id":"m","settlement":true}"#,
        r#"{"type":"permission_policy_available","policy":{"run_id":"r","configuration_revision":2,"profile":"allow_all"}}"#,
        r#"{"type":"apply_permission_policy","policy":{"run_id":"r","configuration_revision":2,"profile":"allow_all"}}"#,
        r#"{"type":"submission","submission":{"type":"interrupt","reason":null}}"#] {
        assert!(runtime.execute(ApplicationCommand::EnqueueAgentCommand {input:AgentCommand {
            session_id:"s".into(),command_id:"same-untrusted-id".into(),payload_json:payload.into(),created_at_ms:1,
        }}).is_err(),"accepted invalid queue input {payload}");
    }
    let db=rusqlite::Connection::open(&database).unwrap();
    assert_eq!(db.query_row("SELECT COUNT(*) FROM agent_commands",[],|row|row.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn queue_claim_port_rolls_back_receipt_failure_and_cold_replays_lease() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("queue-port.db");
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(ApplicationCommand::EnqueueAgentCommand {
            input: AgentCommand {
                session_id: "s".into(),
                command_id: "c".into(),
                payload_json: serde_json::to_string(&nola_agent_engine::agent::AgentSubmission::UserInput {
                    items:vec![nola_agent_engine::agent::AgentItem::user_message(nola_agent_engine::agent::ItemId::from_string("input".into()),"继续")],
                    turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
                    submission_id:Some("submission".into()),submitted_at_epoch_ms:Some(1),
                    delivery:nola_agent_engine::agent::AgentInputDelivery::NextTurn,
                }).unwrap(),
                created_at_ms: 1,
            },
        })
        .unwrap();
    let command = ApplicationCommand::ClaimNextAgentCommand {
        session_id: "s".into(),
        lease_owner: "host".into(),
        now_ms: 2,
        lease_until_ms: 20,
    };
    let head = runtime.execute(ApplicationCommand::PeekAgentCommand { session_id: "s".into() }).unwrap();
    assert_eq!(head["command_id"], "c");
    assert_eq!(head["lease_owner"], json!(null));
    assert!(head.get("payload_json").is_none(), "observation is not command delivery");
    let encoded = serde_json::to_string(&command).unwrap();
    let envelope = || ExecuteReceivedCommand {
        client_id: "queue-test".into(),
        request_id: "claim".into(),
        command_type: "claim_next_agent_command".into(),
        command_json: encoded.clone(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    let fault = rusqlite::Connection::open(&database).unwrap();
    fault.execute_batch("CREATE TRIGGER fail_queue_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'queue receipt fault'); END;").unwrap();
    assert!(runtime
        .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
        .unwrap_err()
        .to_string()
        .contains("queue receipt fault"));
    assert_eq!(
        runtime
            .execute(ApplicationCommand::LeasedAgentCommands {
                session_id: "s".into()
            })
            .unwrap(),
        json!([])
    );
    fault
        .execute_batch("DROP TRIGGER fail_queue_receipt")
        .unwrap();
    let committed = runtime
        .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
        .unwrap();
    let ApplicationCommandReceiptOutcome::Committed { ref result, .. } = committed.outcome else {
        panic!("claim not committed")
    };
    assert_eq!(result["lease_owner"], "host");
    assert_eq!(result["delivery_attempt"], 1);
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    let leases = runtime
        .execute(ApplicationCommand::LeasedAgentCommands {
            session_id: "s".into(),
        })
        .unwrap();
    assert_eq!(leases[0]["delivery_attempt"], 1);
    assert_eq!(
        runtime
            .execute(ApplicationCommand::AcknowledgeAgentCommand {
                session_id: "s".into(),
                command_id: "c".into(),
                lease_owner: "wrong".into()
            })
            .unwrap(),
        json!(false)
    );
    assert_eq!(
        runtime
            .execute(ApplicationCommand::AcknowledgeAgentCommand {
                session_id: "s".into(),
                command_id: "c".into(),
                lease_owner: "host".into()
            })
            .unwrap(),
        json!(true)
    );
}

#[test]
fn queued_turn_rejects_a_previous_claim_from_the_same_owner() {
    use nola_agent_engine::agent::{AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId};
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("queued-admission-fence.sqlite");
    let mut runtime = support::open_runtime(&database).unwrap();
    let submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string("ui-user".into()), "card action")],
        turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
        submission_id: Some("ui-command".into()), submitted_at_epoch_ms: Some(1),
        delivery: AgentInputDelivery::NextTurn,
    };
    runtime.execute(ApplicationCommand::EnqueueAgentCommand { input: AgentCommand {
        session_id: "chat".into(), command_id: "ui-command".into(),
        payload_json: serde_json::to_string(&submission).unwrap(), created_at_ms: 1,
    }}).unwrap();
    let claim = |now_ms| ApplicationCommand::ClaimNextAgentCommand {
        session_id: "chat".into(), lease_owner: "same-host".into(), now_ms, lease_until_ms: 100,
    };
    let first = runtime.execute(claim(2)).unwrap();
    runtime.execute(ApplicationCommand::ReleaseAgentCommandLease {
        session_id: "chat".into(), command_id: "ui-command".into(), lease_owner: "same-host".into(),
    }).unwrap();
    let second = runtime.execute(claim(3)).unwrap();
    assert_ne!(first["delivery_attempt"], second["delivery_attempt"]);
    let error = runtime.execute(ApplicationCommand::BeginQueuedUserTurn {
        input: nola_application::BeginQueuedUserTurn {
            conversation_id: "chat".into(), command_id: "ui-command".into(), lease_owner: "same-host".into(),
            delivery_attempt: first["delivery_attempt"].as_i64().unwrap(),
            expected_configuration_revision: 1, expected_parent_message_id: None,
            core_config: AgentCoreConfig::default(), turn_context_items: vec![],
            turn_context_platform: Default::default(), turn_context_components: vec![], now_ms: 4,
        },
    }).unwrap_err();
    assert!(error.contains("lease is stale"), "{error}");
    let head = runtime.execute(ApplicationCommand::PeekAgentCommand { session_id: "chat".into() }).unwrap();
    assert_eq!(head["command_id"], "ui-command");
    let db = rusqlite::Connection::open(database).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM runs", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn rejected_ui_dispatch_can_be_cancelled_without_replaying_or_erasing_receipts() {
    use nola_domain::remote_submission::RemoteSubmissionIdentity;
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("cancel-ui.sqlite");
    let mut store = nola_store::NolaStore::open(&database).unwrap();
    let identity = RemoteSubmissionIdentity { source_host_id:"host".into(), client_id:"client".into(),
        conversation_id:"conversation".into(), submission_id:"action".into(),
        user_message_id:"ui:action:user".into(), assistant_message_id:"ui:action:assistant".into() };
    let surface = json!({"owner":{"authority_id":"host","conversation_id":"conversation","participant_id":"assistant",
        "context_id":"conversation","originating_run_id":"source","message_id":"source:assistant","part_id":"tool"},"surface_id":"form","generation":1});
    let action = json!({"surface":{"identity":surface,"revision":1},"summary":"Submitted form · submit",
        "envelope":{"type":"ui_action","surface":surface,"message":{"version":"v0.9.1","action":{"surfaceId":"form","name":"submit","sourceComponentId":"button","timestamp":"2026-10-21T00:00:00Z"}}}});
    let item: nola_agent_engine::agent::AgentItem = serde_json::from_value(json!({"type":"ui_action","id":"ui:action:user","action":action})).unwrap();
    let payload = nola_agent_engine::agent::host::AgentBoundaryInput::Submission {
        submission:nola_agent_engine::agent::AgentSubmission::UserInput {
            items:vec![item],turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
            submission_id:Some("action".into()),submitted_at_epoch_ms:Some(1),delivery:nola_agent_engine::agent::AgentInputDelivery::NextTurn }};
    store.enqueue_agent_command(&nola_store_port::agent_queue::EnqueueAgentCommandInput {
        session_id:"conversation".into(),command_id:"action".into(),payload_json:serde_json::to_string(&payload).unwrap(),created_at_ms:1 }).unwrap();
    let registered = store.register_source_submission(&identity, &json!({"original":"frozen"})).unwrap();
    let digest = registered.receipt.request_digest;
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let cancel = |identity: RemoteSubmissionIdentity, now_ms| ApplicationCommand::CancelRejectedQueuedUiAction { identity,request_digest:digest.clone(),now_ms };
    assert!(runtime.execute(cancel(identity.clone(),10)).is_err(), "prepared is uncertain, never cancellable as rejected");
    runtime.execute(ApplicationCommand::RejectSourceSubmission { identity:identity.clone(),request_digest:digest.clone(),rejection_code:"dispatch_not_admitted".into() }).unwrap();
    let mut foreign = identity.clone(); foreign.client_id="other".into();
    assert!(runtime.execute(cancel(foreign,10)).is_err());
    runtime.execute(ApplicationCommand::ClaimNextAgentCommand {session_id:"conversation".into(),lease_owner:"worker".into(),now_ms:10,lease_until_ms:20}).unwrap();
    assert!(runtime.execute(cancel(identity.clone(),15)).is_err(), "live dispatch lease blocks cancellation");
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute_batch("CREATE TRIGGER reject_ui_cancel BEFORE DELETE ON agent_commands BEGIN SELECT RAISE(ABORT, 'injected cancellation failure'); END;").unwrap();
    assert!(runtime.execute(cancel(identity.clone(),21)).is_err());
    let retained = runtime.execute(ApplicationCommand::PeekAgentCommand {session_id:"conversation".into()}).unwrap();
    assert_eq!(retained["lease_owner"], "worker", "failed cancellation must roll back its temporary lease");
    db.execute_batch("DROP TRIGGER reject_ui_cancel;").unwrap();
    assert_eq!(runtime.execute(cancel(identity.clone(),21)).unwrap()["cancelled"], true);
    assert_eq!(runtime.execute(cancel(identity.clone(),22)).unwrap()["cancelled"], true);
    assert_eq!(runtime.execute(ApplicationCommand::PeekAgentCommand {session_id:"conversation".into()}).unwrap(),json!(null));
    let receipt = runtime.execute(ApplicationCommand::ReadSourceSubmissionReceipt {identity,request_digest:digest}).unwrap();
    assert_eq!(receipt["phase"],"rejected");
    assert_eq!(receipt["rejection_code"],"dispatch_not_admitted");
    assert_eq!(receipt["admission"],json!(null));
}
