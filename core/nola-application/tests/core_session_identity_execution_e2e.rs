// SPDX-License-Identifier: Apache-2.0
//! Canonical conversation and typed Core session are independent identities.
mod support;
use nola_application::cancellation_delivery::{
    load_run_cancellation_delivery_facts, CancellationRunIdentity,
};
use nola_application::*;
use nola_agent_engine::agent::{host::*, protocol::*};
use nola_store::NolaStore;

const CONVERSATION: &str = "canonical-conversation";
const CORE_SESSION: &str = "independent-core-session";

fn fixture(path: &std::path::Path) -> String {
    let mut store = NolaStore::open(path).unwrap();
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "Isolated model".into(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type":"object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: true,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: CONVERSATION.into(),
            conversation_title: "Identity fixture".into(),
            user_message_id: "user".into(),
            user_part_id: "user-part".into(),
            user_text: "Work".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: AgentCoreConfig {
                turn_identity: support::model_identity::identity(),
                session_id: Some(SessionId::from_string(CORE_SESSION.into())),
                model: Some("fixture".into()),
                host_catalog: Some(HostCapabilityCatalog {
                    host_id: HostId("isolated-host".into()),
                    revision: 1,
                    availability_epoch: 1,
                    lease: None,
                    capabilities: [("model.stream".into(), capability)].into_iter().collect(),
                }),
                ..Default::default()
            },
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("user".into()),
                    "Work",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("submission".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        },
    )
    .unwrap();
    assert_eq!(begun.execution.control.session_id.as_str(), CORE_SESSION);
    claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "fixture-generation".into(),
            lease_owner: "fixture-worker".into(),
            claimed_at_ms: 1100,
            lease_expires_at_ms: 100_000,
            limit: 1,
        },
    )
    .unwrap();
    begun.begin.run_id
}
fn cancel(run: &str) -> RequestRunCancellation {
    RequestRunCancellation {
        conversation_id: CONVERSATION.into(),
        expected_run_id: run.into(),
        assistant_message_id: "assistant".into(),
        cancellation_id: "cancellation".into(),
        reason: None,
        requested_at_ms: 1200,
    }
}
fn recovery() -> ApplicationCommand {
    ApplicationCommand::RecoverOrphanedRuns {
        conversation_id: Some(CONVERSATION.into()),
        reason: "isolated_recovery_check".into(),
        interrupted_at: "before-restart".into(),
        interrupted_at_ms: 1150,
    }
}
fn checkpoint(db: &rusqlite::Connection) -> String {
    db.query_row(
        "SELECT checkpoint_json FROM core_checkpoint_bodies WHERE conversation_id=?1",
        [CONVERSATION],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn independent_core_session_recovers_cancels_and_loads_cold_delivery_facts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("execution.db");
    let run = fixture(&path);
    let db = rusqlite::Connection::open(&path).unwrap();
    let before = checkpoint(&db);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime.execute(recovery()).unwrap();
    assert_eq!(checkpoint(&db), before);
    let receipt: RunCancellationReceipt = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::RequestRunCancellation {
                input: cancel(&run),
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        receipt.disposition,
        RunCancellationDisposition::CancellationCommitted
    );
    assert_eq!(receipt.terminal_result, None);
    drop(runtime);
    let store = NolaStore::open(&path).unwrap();
    let identity = CancellationRunIdentity {
        conversation_id: CONVERSATION.into(),
        expected_run_id: run.clone(),
        assistant_message_id: "assistant".into(),
    };
    let facts = load_run_cancellation_delivery_facts(&store, &identity).unwrap();
    assert!(!facts.targets.is_empty());
    assert!(facts
        .targets
        .iter()
        .all(|fact| fact.cancellation.session_id == CORE_SESSION
            && fact
                .target
                .as_ref()
                .is_none_or(|target| target.session_id == CORE_SESSION)));
    assert!(load_run_cancellation_delivery_facts(
        &store,
        &CancellationRunIdentity {
            conversation_id: "foreign-conversation".into(),
            ..identity.clone()
        }
    )
    .is_err());
    assert!(load_run_cancellation_delivery_facts(
        &store,
        &CancellationRunIdentity {
            expected_run_id: "foreign-run".into(),
            ..identity
        }
    )
    .is_err());
}

#[test]
fn inconsistent_core_session_and_foreign_host_session_fail_without_canonical_writes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rejected-identity.db");
    let run = fixture(&path);
    let db = rusqlite::Connection::open(&path).unwrap();
    let original = checkpoint(&db);
    let mut invalid: serde_json::Value = serde_json::from_str(&original).unwrap();
    invalid["state"]["session_id"] = serde_json::json!("foreign-core-session");
    let invalid = serde_json::to_string(&invalid).unwrap();
    db.execute(
        "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id=?2",
        [&invalid, CONVERSATION],
    )
    .unwrap();
    let mut runtime = support::open_runtime(&path).unwrap();
    assert!(runtime
        .execute(ApplicationCommand::RequestRunCancellation {
            input: cancel(&run)
        })
        .is_err());
    assert!(runtime.execute(recovery()).is_err());
    assert_eq!(checkpoint(&db), invalid);
    db.execute(
        "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id=?2",
        [&original, CONVERSATION],
    )
    .unwrap();
    runtime
        .execute(ApplicationCommand::RequestRunCancellation {
            input: cancel(&run),
        })
        .unwrap();
    drop(runtime);
    let committed = checkpoint(&db);
    let (effect_id, original_request): (String, String) = db.query_row(
        "SELECT effect_id,payload_json FROM effect_outbox JOIN effect_outbox_payloads USING(effect_id) WHERE json_extract(payload_json,'$.payload.type')='cancel' LIMIT 1",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let original_binding: String = db.query_row("SELECT binding_json FROM effect_outbox WHERE effect_id=?1", [&effect_id], |row| row.get(0)).unwrap();
    let store = NolaStore::open(&path).unwrap();
    let identity = CancellationRunIdentity {
        conversation_id: CONVERSATION.into(),
        expected_run_id: run,
        assistant_message_id: "assistant".into(),
    };
    for (field, value) in [
        ("$.session_id", "foreign-core-session"),
        ("$.request_id", "foreign-request"),
        ("$.effect_id", "foreign-effect"),
        ("$.host_id", "foreign-host"),
        ("$.lease_id", "foreign-lease"),
        ("$.turn_id", "foreign-turn"),
        ("$.payload.target_effect_id", "foreign-target"),
    ] {
        let (table,column,original) = if field.starts_with("$.payload.") {
            ("effect_outbox_payloads","payload_json",&original_request)
        } else { ("effect_outbox","binding_json",&original_binding) };
        db.execute(&format!("UPDATE {table} SET {column}=json_set(?1,?2,?3) WHERE effect_id=?4"),
            rusqlite::params![original, field, value, effect_id]).unwrap();
        assert!(load_run_cancellation_delivery_facts(&store, &identity).is_err(), "{field}");
        db.execute(&format!("UPDATE {table} SET {column}=?1 WHERE effect_id=?2"),
            rusqlite::params![original, effect_id]).unwrap();
        assert!(load_run_cancellation_delivery_facts(&store, &identity).is_ok());
    }
    assert_eq!(checkpoint(&db), committed);
    // Well-typed envelope mutations must not be hidden by proof admission.
    for (field, value) in [
        ("$.catalog_revision", serde_json::json!(999)),
        ("$.availability_epoch", serde_json::json!(999)),
        ("$.attempt_id", serde_json::json!("foreign-attempt")),
        ("$.attempt_started", serde_json::json!(true)),
        ("$.created_at_ms", serde_json::json!(999)),
        ("$.deadline_ms", serde_json::json!(999)),
        ("$.batch", serde_json::json!({"batch_id":"foreign-batch","index":0,"size":1})),
    ] {
        db.execute("UPDATE effect_outbox SET binding_json=json_set(?1,?2,json(?3)) WHERE effect_id=?4",
            rusqlite::params![original_binding, field, value.to_string(), effect_id]).unwrap();
        let before_version: i64 = db.query_row("PRAGMA data_version", [], |row| row.get(0)).unwrap();
        assert!(load_run_cancellation_delivery_facts(&store, &identity).is_err(), "{field}");
        assert_eq!(db.query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0)).unwrap(),
            before_version, "rejected read must not commit through the Store connection: {field}");
        assert_eq!(checkpoint(&db), committed, "{field}");
        db.execute("UPDATE effect_outbox SET binding_json=?1 WHERE effect_id=?2",
            rusqlite::params![original_binding, effect_id]).unwrap();
        assert!(load_run_cancellation_delivery_facts(&store, &identity).is_ok());
    }
}
