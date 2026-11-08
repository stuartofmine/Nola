// SPDX-License-Identifier: Apache-2.0
//! Real read-only command over isolated persisted facts, without OS inference.
mod support;
use nola_application::ApplicationCommand;
use serde_json::{json, Value};

fn inspect(
    runtime: &mut support::ApplicationRuntime,
    conversation: &str,
    run: &str,
    assistant: &str,
) -> Value {
    let command = ApplicationCommand::InspectRunRecovery {
        conversation_id: conversation.into(),
        expected_run_id: run.into(),
        assistant_message_id: assistant.into(),
    };
    assert!(command.is_read_only_query());
    runtime.execute(command).unwrap()
}

fn seed(path: &std::path::Path) -> rusqlite::Connection {
    drop(nola_store::NolaStore::open(path).unwrap());
    let connection = rusqlite::Connection::open(path).unwrap();
    connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','fixture',1,1,1);
        INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms) VALUES('a','c','r','assistant','running',1,1);
        INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES('r','c','conversation','a','running',1,1,1);").unwrap();
    connection
}

#[test]
fn absence_mismatch_and_unsealed_terminal_claim_are_distinct_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inspection.sqlite");
    let connection = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(
        inspect(&mut runtime, "c", "absent", "a")["observation"],
        json!({"state":"missing"})
    );
    for (c, r, a) in [("other", "r", "a"), ("c", "r", "other")] {
        assert_eq!(
            inspect(&mut runtime, c, r, a)["observation"],
            json!({"state":"identity_mismatch"})
        );
    }
    connection.execute("UPDATE runs SET status='sealed',completed_at_ms=42", []).unwrap();
    let mut reopened = support::open_runtime(&path).unwrap();
    assert!(reopened.execute(ApplicationCommand::InspectRunRecovery {
        conversation_id: "c".into(), expected_run_id: "r".into(), assistant_message_id: "a".into(),
    }).is_err(), "a fabricated lifecycle row is not a source-validated result");
    assert!(runtime
        .execute(ApplicationCommand::InspectRunRecovery {
            conversation_id: "".into(),
            expected_run_id: "r".into(),
            assistant_message_id: "a".into()
        })
        .is_err());
}

#[test]
fn malformed_terminal_evidence_never_becomes_terminal_or_missing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("invalid.sqlite");
    let connection = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    connection
        .execute("UPDATE runs SET status='sealed'", [])
        .unwrap();
    assert!(runtime.execute(ApplicationCommand::InspectRunRecovery {
        conversation_id: "c".into(), expected_run_id: "r".into(), assistant_message_id: "a".into(),
    }).is_err());
    connection
        .execute(
            "UPDATE runs SET status='interrupted',completed_at_ms=42",
            [],
        )
        .unwrap();
    assert_eq!(
        inspect(&mut runtime, "c", "r", "a")["observation"],
        json!({"state":"inconsistent","reason":"nonterminal_has_completion_timestamp"})
    );
    connection
        .execute("UPDATE messages SET role='user'", [])
        .unwrap();
    assert_eq!(
        inspect(&mut runtime, "c", "r", "a")["observation"],
        json!({"state":"inconsistent","reason":"assistant_owner_invalid"})
    );
}

#[test]
fn active_inspection_returns_only_aggregate_durable_facts_without_writes_or_secrets() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("active.sqlite");
    let connection = seed(&path);
    connection.execute_batch("INSERT INTO effect_outbox(effect_id,run_id,request_id,effect_kind,payload_digest,recovery_required,status,created_at_ms) VALUES('e','r','request','tool','retained-history',1,'pending',1);
        INSERT INTO effect_outbox_payloads VALUES('e','{\"secret\":\"do-not-return\"}');
        INSERT INTO effect_delivery_attempts(id,run_id,effect_id,request_id,attempt_number,replay_safety,state,worker_generation,lease_owner,lease_token,lease_expires_at_ms,revision,record_json) VALUES('delivery','r','e','request',1,'requires_recovery_decision','execution_started','worker','owner','secret-token',99,1,'{}');
        INSERT INTO session_run_leases(session_id,owner,pid,process_start,acquired_at_ms) VALUES('c','private-owner',999999,'unknown',1);").unwrap();
    let mut runtime = support::open_runtime(&path).unwrap();
    let observation = inspect(&mut runtime, "c", "r", "a");
    assert_eq!(
        observation["observation"],
        json!({"state":"active","run_status":"running","checkpoint_present":false,"pending_effect_count":1,"source_host_id":null,"has_session_lease":true,"deliveries":[{"state":"execution_started","count":1}]})
    );
    let encoded = observation.to_string();
    for secret in [
        "secret-token",
        "do-not-return",
        "private-owner",
        "999999",
        "lease_token",
    ] {
        assert!(!encoded.contains(secret));
    }
    let persisted:(String,i64,i64)=connection.query_row("SELECT r.status,r.updated_at_ms,c.revision FROM runs r JOIN conversations c ON c.id=r.conversation_id WHERE r.id='r'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(persisted, ("running".into(), 1, 1));
    assert_eq!(
        connection
            .query_row(
                "SELECT state FROM effect_delivery_attempts WHERE id='delivery'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "execution_started"
    );
}
