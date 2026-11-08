// SPDX-License-Identifier: Apache-2.0
//! The public read-only query returns verified binding, not raw Core state.
mod support;
use nola_application::ApplicationCommand;
use nola_agent_engine::agent::protocol::AgentCoreConfig;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn command(conversation: &str, run: &str, assistant: &str) -> ApplicationCommand {
    ApplicationCommand::LoadExactRunInvocation {
        conversation_id: conversation.into(),
        expected_run_id: run.into(),
        assistant_message_id: assistant.into(),
    }
}

fn config() -> AgentCoreConfig {
    let route = json!({"kind":"http","protocol":"chat_completions","output_token_field":"max_tokens","provider":"fixture", "endpoint":"https://fixture.invalid/v1",
        "endpoint_query_sha256":null,"model":"original-model","credential_sha256":"a".repeat(64),
        "credential_source":{"kind":"ephemeral"},"endpoint_query_source":{"kind":"none"}});
    let mut config = support::model_identity::config();
    config.tools = vec![json!({
        "type": "function",
        "function": {"name": "fixture_tool", "parameters": {"type": "object", "properties": {}}},
        "private_context_marker": "must-not-be-disclosed"
    })];
    config.context_window.context_window_tokens = 4096;
    config.context_window.reserved_output_tokens = 64;
    config.context_window.prompt_token_limit = Some(2048);
    config.context_window.auto_compact_tokens = Some(3000);
    config.context_window.compaction_target_tokens = Some(1500);
    config.context_window.auto_compact_percent = 75;
    config.context_window.compaction_target_percent = 40;
    config.turn_identity.remote_tool_plan = Some(serde_json::from_value(json!({
        "schema_version":1,"reference":{"manifest_id":"frozen-plan","host_id":"host","client_id":"client",
            "session_id":"c","catalog_revision":"a".repeat(64)},
        "issued_at_ms":1,"expires_at_ms":300001,"tools":[]
    })).unwrap());
    config.turn_identity.host_invocation = Some(serde_json::from_value(json!({
        "version":nola_host_protocol::HOST_INVOCATION_BINDING_VERSION,
        "primary":route,"fallbacks":[],
        "generation":{"temperature":0,"top_p":0,"max_output_tokens":16384,
            "max_retries":2,"parallel_tool_calls":null,
            "reasoning_selection":null,"sampling_override":false,"generation_options":{},
            "reasoning_tier":"balanced",
            "thinking_enabled":false,"prompt_cache":false,"prompt_cache_key_sha256":null,
            "prompt_cache_key_prefix_sha256":null,"prompt_cache_retention":null},
        "output_admission":{"version":nola_host_protocol::HOST_OUTPUT_ADMISSION_VERSION,
            "routes":[{"route":route,"max_input_tokens":null,"max_output_tokens":null,"max_total_tokens":null}],
            "preference":"default","resolved":{"tokens":16384,"source":"product_default"}},
        "execution_policy_sha256":"b".repeat(64)
    })).unwrap());
    config
}

fn save(connection: &rusqlite::Connection, config: &AgentCoreConfig) {
    let raw = serde_json::to_string(config).unwrap();
    let digest = format!("{:x}", Sha256::digest(raw.as_bytes()));
    connection
        .execute(
            "UPDATE runs SET invocation_config_digest=?1 WHERE id='r'",
            rusqlite::params![digest],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO run_invocation_payloads(run_id,config_json) VALUES('r',?1)
        ON CONFLICT(run_id) DO UPDATE SET config_json=excluded.config_json",
            [raw],
        )
        .unwrap();
}

fn seed(path: &std::path::Path) -> rusqlite::Connection {
    drop(nola_store::NolaStore::open(path).unwrap());
    let connection = rusqlite::Connection::open(path).unwrap();
    connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','fixture',1,1,1);
        INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms) VALUES('a','c','r','assistant','running',1,1);
        INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES('r','c','conversation','a','running',1,1,1);").unwrap();
    save(&connection, &config());
    connection
}

#[test]
fn persistent_discovery_is_read_only_and_keyset_paginated_without_process_registration() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("discovery.sqlite");
    let connection = seed(&path);
    connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('other','other',1,1,1);
        INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms) VALUES('other-assistant','other','z-run','assistant','running',1,1);
        INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES('z-run','other','conversation','other-assistant','running',1,1,1);").unwrap();
    let mut runtime = support::open_runtime(&path).unwrap();
    let query = ApplicationCommand::ListRecoverableRuns { after_run_id: None, limit: 1 };
    assert!(query.is_read_only_query());
    let first = runtime.execute(query).unwrap();
    assert_eq!(first, json!([{"conversation_id":"c","run_id":"r","assistant_message_id":"a"}]));
    let second = runtime.execute(ApplicationCommand::ListRecoverableRuns { after_run_id: Some("r".into()), limit: 1 }).unwrap();
    assert_eq!(second[0]["run_id"], "z-run");
    assert!(runtime.execute(ApplicationCommand::ListRecoverableRuns { after_run_id: Some("z-run".into()), limit: 1 }).unwrap().as_array().unwrap().is_empty());
    assert!(runtime.execute(ApplicationCommand::ListRecoverableRuns { after_run_id: None, limit: 0 }).is_err());
}

#[test]
fn reopened_read_returns_only_verified_binding_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("invocation.sqlite");
    let connection = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    let before: i64 = connection
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    let query = command("c", "r", "a");
    assert!(query.is_read_only_query());
    let result = runtime.execute(query).unwrap();
    assert_eq!(
        result["host_invocation"]["primary"]["model"],
        "original-model"
    );
    assert_eq!(
        result["host_invocation"]["primary"]["credential_source"],
        json!({"kind":"ephemeral"})
    );
    assert_eq!(
        result["host_invocation"]["primary"]["endpoint_query_source"],
        json!({"kind":"none"})
    );
    assert_eq!(result["host_invocation"]["generation"]["temperature"], 0);
    assert_eq!(result["status"], "running");
    assert_eq!(result["remote_tool_plan"]["reference"]["manifest_id"],"frozen-plan");
    let frozen: nola_domain::host_tool_plan::HostToolPlanSnapshot = serde_json::from_value(result["remote_tool_plan"].clone()).unwrap();
    assert!(frozen.validate("host","client","c",None).is_ok());
    assert!(frozen.validate("host","other-client","c",None).is_err());
    for field in [
        "core_session_id",
        "checkpoint_revision",
        "checkpoint_event_cursor",
        "checkpoint_sha256",
    ] {
        assert_eq!(
            result[field],
            Value::Null,
            "a pre-checkpoint Run has no invented Core witness"
        );
    }
    assert_eq!(
        result["context_budget"],
        json!({
            "context_window_tokens":4096,"reserved_output_tokens":64,"prompt_token_limit":2048,
            "auto_compact_tokens":3000,"compaction_target_tokens":1500,
            "auto_compact_percent":75,"compaction_target_percent":40,
        })
    );
    assert_eq!(
        result["host_invocation"]["generation"]["max_output_tokens"], 16384,
        "output admission is not reconstructed from the context reservation"
    );
    assert!(!result.to_string().contains("must-not-be-disclosed"));
    assert!(result.get("invocation_config_json").is_none());
    assert_eq!(
        runtime.execute(command("c", "missing", "a")).unwrap(),
        Value::Null
    );
    for (c, r, a) in [("other", "r", "a"), ("c", "r", "other"), ("", "r", "a")] {
        assert!(runtime.execute(command(c, r, a)).is_err());
    }
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let after: i64 = connection
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        before, after,
        "successful and rejected queries must not commit writes"
    );
}

#[test]
fn corrupt_or_missing_binding_never_falls_back_to_current_settings() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("corrupt.sqlite");
    let connection = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    connection
        .execute("UPDATE runs SET invocation_config_digest='incorrect'", [])
        .unwrap();
    assert!(runtime
        .execute(command("c", "r", "a"))
        .unwrap_err()
        .contains("integrity"));
    save(&connection, &AgentCoreConfig::default());
    assert!(runtime
        .execute(command("c", "r", "a"))
        .unwrap_err()
        .contains("no frozen"));
    let mut invalid = config();
    let binding = invalid.turn_identity.host_invocation.as_mut().unwrap();
    let nola_host_protocol::HostModelRouteBinding::Http(http) = &mut binding.primary else {
        panic!("HTTP fixture")
    };
    http.endpoint = "https://fixture.invalid/v1?key=private-secret".into();
    binding.output_admission.routes[0].route = binding.primary.clone();
    save(&connection, &invalid);
    let error = runtime.execute(command("c", "r", "a")).unwrap_err();
    assert!(!error.contains("private-secret"));
    assert!(error.contains("non-secret"));
}

#[test]
fn stored_secret_provenance_cannot_be_omitted_or_repurposed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("provenance.sqlite");
    let connection = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    // Recompute the outer integrity digest deliberately: semantic admission
    // must reject bad provenance even when storage bytes are self-consistent.
    for replacement in [
        None,
        Some(json!({"kind":"none"})),
        Some(json!({
            "kind":"managed", "authority":"ios_app",
            "owner_id":"0123456789abcdef0123456789abcdef",
            "handle_id":"fedcba9876543210fedcba9876543210",
            "revision":1, "purpose":"endpoint_query"
        })),
    ] {
        let mut value = serde_json::to_value(config()).unwrap();
        let binding = &mut value["turn_identity"]["host_invocation"];
        let route = binding["primary"].as_object_mut().unwrap();
        match replacement {
            Some(source) => {
                route.insert("credential_source".into(), source);
            }
            None => {
                route.remove("credential_source");
            }
        }
        binding["output_admission"]["routes"][0]["route"] = binding["primary"].clone();
        let raw = serde_json::to_string(&value).unwrap();
        let digest = format!("{:x}", Sha256::digest(raw.as_bytes()));
        connection
            .execute(
                "UPDATE runs SET invocation_config_digest=?1 WHERE id='r'",
                [digest],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE run_invocation_payloads SET config_json=?1 WHERE run_id='r'",
                [raw],
            )
            .unwrap();
        assert!(runtime.execute(command("c", "r", "a")).is_err());
    }
}

#[test]
fn sensitive_binding_is_rejected_before_run_or_message_persistence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("admission.sqlite");
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let mut invalid = config();
    let binding = invalid.turn_identity.host_invocation.as_mut().unwrap();
    let nola_host_protocol::HostModelRouteBinding::Http(http) = &mut binding.primary else {
        panic!("HTTP fixture")
    };
    http.endpoint = "https://fixture.invalid/v1?key=private-secret".into();
    binding.output_admission.routes[0].route = binding.primary.clone();
    let error = nola_application::begin_assistant_turn(
        &mut store,
        nola_application::BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "new-conversation".into(),
            conversation_title: "fixture".into(),
            user_message_id: "user".into(),
            user_part_id: "user:text".into(),
            user_text: "hello".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: invalid,
            submission: serde_json::from_value(json!({"type":"user_input","items":[],
                "submission_id":"run","submitted_at_epoch_ms":1}))
            .unwrap(),
            created_at_ms: 1,
        },
    )
    .unwrap_err();
    assert!(error.contains("non-secret") && !error.contains("private-secret"));
    let connection = rusqlite::Connection::open(&path).unwrap();
    for table in ["conversations", "messages", "runs", "effect_outbox"] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "admission must fail before writing {table}");
    }
}

fn begin_distinct(path: &std::path::Path) -> nola_application::AcceptedAssistantTurn {
    begin_distinct_turn(path, "owned-run", "owned-assistant", "user", 1)
}

fn begin_distinct_turn(
    path: &std::path::Path,
    run_id: &str,
    assistant_id: &str,
    user_id: &str,
    at_ms: i64,
) -> nola_application::AcceptedAssistantTurn {
    use nola_agent_engine::agent::{host::*, protocol::*};
    let mut config = config();
    // The small persisted-budget fixture above tests configuration reads.
    // Actual model/recovery turns need room for the current protocol catalog;
    // otherwise they enter compaction instead of the model step tested here.
    config.context_window = Default::default();
    config.session_id = Some(SessionId::from_string("actual-core-session".into()));
    config.model = Some("original-model".into());
    config.host_catalog = Some(HostCapabilityCatalog {
        host_id: HostId("fixture-host".into()),
        revision: 1,
        availability_epoch: 1,
        lease: None,
        capabilities: [(
            "model.stream".into(),
            HostCapabilityDescriptor {
                id: "model.stream".into(),
                title: "Fixture model".into(),
                route: HostCapabilityRoute::Model,
                availability: CapabilityAvailability::Available,
                exposure: HostCapabilityExposure::Internal,
                permission_domain: None,
                input_schema: json!({"type":"object"}),
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
            },
        )]
        .into_iter()
        .collect(),
    });
    let mut store = nola_store::NolaStore::open(path).unwrap();
    let frontier = support::admission_frontier::read(&store, "canonical-conversation");
    nola_application::begin_assistant_turn(
        &mut store,
        nola_application::BeginAssistantTurn {
            expected_configuration_revision: frontier.0,
            expected_parent_message_id: frontier.1,
            conversation_id: "canonical-conversation".into(),
            conversation_title: "Distinct identity".into(),
            user_message_id: user_id.into(),
            user_part_id: format!("{user_id}:text"),
            user_text: "hello".into(),
            user_content_json: None,
            assistant_message_id: assistant_id.into(),
            context_checkpoint_id: None,
            core_config: config,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string(user_id.into()),
                    "hello",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some(run_id.into()),
                submitted_at_epoch_ms: Some(at_ms),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: at_ms,
        },
    )
    .unwrap()
}

#[test]
fn exact_read_proves_distinct_core_session_and_rejects_forged_checkpoint_witnesses() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("owned-checkpoint.sqlite");
    let begun = begin_distinct(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    let query = || {
        command(
            "canonical-conversation",
            &begun.begin.run_id,
            "owned-assistant",
        )
    };
    let result = runtime.execute(query()).unwrap();
    assert_eq!(result["core_session_id"], "actual-core-session");
    assert_eq!(result["checkpoint_revision"], begun.execution.control.revision);
    assert_eq!(
        result["checkpoint_event_cursor"],
        begun.execution.control.last_event_cursor.sequence
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    let raw: String = db
        .query_row(
            "SELECT checkpoint_json FROM core_checkpoint_bodies",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        result["checkpoint_sha256"],
        format!("{:x}", Sha256::digest(raw.as_bytes()))
    );
    for field in ["root", "state", "revision", "cursor", "turn", "binding"] {
        let mut invalid: Value = serde_json::from_str(&raw).unwrap();
        match field {
            "root" => invalid["session_id"] = json!("foreign-core"),
            "state" => invalid["state"]["session_id"] = json!("foreign-core"),
            "revision" => invalid["revision"] = json!(999),
            "cursor" => invalid["last_event_cursor"]["sequence"] = json!(999),
            "turn" => {
                invalid["state"]["execution"]["active_turn"]["turn_id"] = json!("foreign-run")
            }
            "binding" => {
                invalid["state"]["configuration"]["turn_identity"]["host_invocation"]
                    ["generation"]["temperature"] = json!(100)
            }
            _ => unreachable!(),
        }
        db.execute(
            "UPDATE core_checkpoint_bodies SET checkpoint_json=?1",
            [invalid.to_string()],
        )
        .unwrap();
        assert!(
            runtime.execute(query()).is_err(),
            "present invalid {field} is never an absent witness"
        );
        db.execute(
            "UPDATE core_checkpoint_bodies SET checkpoint_json=?1",
            [&raw],
        )
        .unwrap();
    }
    let admission: String = db.query_row(
        "SELECT payload_json FROM run_events WHERE run_id=?1 AND event_kind='turn_started' ORDER BY sequence LIMIT 1",
        [&begun.begin.run_id], |row| row.get(0),
    ).unwrap();
    for field in ["missing_marker", "foreign_session", "mismatched_revision"] {
        let mut invalid: Value = serde_json::from_str(&admission).unwrap();
        match field {
            "missing_marker" => {
                invalid.as_object_mut().unwrap().remove("core_envelope");
            }
            "foreign_session" => {
                invalid["session_id"] = json!("foreign-core");
                invalid["core_envelope"]["session_id"] = json!("foreign-core");
            }
            "mismatched_revision" => invalid["core_envelope"]["revision"] = json!(999),
            _ => unreachable!(),
        }
        db.execute(
            "UPDATE run_events SET payload_json=?1 WHERE run_id=?2 AND event_kind='turn_started'",
            rusqlite::params![invalid.to_string(), begun.begin.run_id],
        )
        .unwrap();
        assert!(runtime.execute(query()).is_err(),
            "initial admission requires independent exact {field} proof, not a self-consistent checkpoint");
        db.execute(
            "UPDATE run_events SET payload_json=?1 WHERE run_id=?2 AND event_kind='turn_started'",
            rusqlite::params![admission, begun.begin.run_id],
        )
        .unwrap();
        assert!(
            runtime.execute(query()).is_ok(),
            "restored exact initial proof is valid"
        );
    }
    db.execute(
        "DELETE FROM assistant_turn_message_bindings WHERE admission=1",
        [],
    )
    .unwrap();
    assert!(
        runtime.execute(query()).is_err(),
        "checkpoint needs exact Run admission ownership"
    );
    db.execute("DELETE FROM core_checkpoints", []).unwrap();
    assert!(runtime.execute(query()).is_err(),
        "a Run with committed Host effects cannot become a pre-checkpoint Run after losing its checkpoint");
}

fn cancel_distinct_turn(
    runtime: &mut support::ApplicationRuntime,
    begun: &nola_application::AcceptedAssistantTurn,
) {
    use nola_application::*;
    use nola_agent_engine::agent::host::*;
    runtime
        .execute(ApplicationCommand::RequestRunCancellation {
            input: RequestRunCancellation {
                conversation_id: "canonical-conversation".into(),
                expected_run_id: begun.begin.run_id.clone(),
                assistant_message_id: "owned-assistant".into(),
                cancellation_id: "cancel".into(),
                reason: None,
                requested_at_ms: 2,
            },
        })
        .unwrap();
    let requests: CommittedHostRequests = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ClaimCommittedHostRequests {
                input: ClaimCommittedHostRequests {
                    run_id: begun.begin.run_id.clone(),
                    worker_generation: "fixture-worker".into(),
                    lease_owner: "fixture-worker".into(),
                    claimed_at_ms: 3,
                    lease_expires_at_ms: 1000,
                    limit: 64,
                },
            })
            .unwrap(),
    )
    .unwrap();
    let cancel = requests
        .requests
        .iter()
        .find(|request| matches!(request.payload, HostRequest::Cancel { .. }))
        .unwrap();
    let HostRequest::Cancel { target_effect_id, .. } = &cancel.payload else {
        unreachable!()
    };
    for (sequence, payload) in [
        (1, HostResponse::ExecutionStarted),
        (
            2,
            HostResponse::CancellationConfirmed {
                target_effect_id: target_effect_id.clone(),
            },
        ),
    ] {
        runtime
            .execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                input: ReduceAndAcceptHostResponse {
                    conversation_id: "canonical-conversation".into(),
                    run_id: begun.begin.run_id.clone(),
                    invocation_config_digest: begun.invocation_config_digest.clone(),
                    received_at_ms: 3 + sequence as i64,
                    response: HostResponseEnvelope {
                        request_id: cancel.request_id.clone(),
                        attempt_id: cancel.attempt_id.clone(),
                        delegated_invocation_id: cancel.delegated_invocation_id.clone(),
                        effect_id: cancel.effect_id.clone(),
                        host_id: cancel.host_id.clone(),
                        lease_id: cancel.lease_id.clone(),
                        session_id: cancel.session_id.clone(),
                        turn_id: cancel.turn_id.clone(),
                        catalog_revision: cancel.catalog_revision,
                        availability_epoch: cancel.availability_epoch,
                        delivery: cancel.delivery.clone(),
                        sequence,
                        payload,
                    },
                },
            })
            .unwrap();
    }
}

#[test]
fn exact_read_accepts_successor_initial_admission_at_a_later_store_revision() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("successor-checkpoint.sqlite");
    let first = begin_distinct(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    cancel_distinct_turn(&mut runtime, &first);
    assert_eq!(
        runtime
            .execute(command(
                "canonical-conversation",
                &first.begin.run_id,
                "owned-assistant"
            ))
            .unwrap()["status"],
        "sealed"
    );
    drop(runtime);

    let next = begin_distinct_turn(
        &path,
        "successor-run",
        "successor-assistant",
        "successor-user",
        100,
    );
    assert_ne!(next.begin.run_id, first.begin.run_id);
    let current = support::current_checkpoint(&path, "canonical-conversation");
    assert!(
        current.revision > 1,
        "a new Run may inherit the conversation's later Store CAS"
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    let commits: i64 = db
        .query_row(
            "SELECT count(*) FROM committed_transitions WHERE run_id=?1",
            [&next.begin.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        commits, 0,
        "the successor is proved by its actual initial admission, not a Host commit"
    );
    let mut reopened = support::open_runtime(&path).unwrap();
    let exact = reopened
        .execute(command(
            "canonical-conversation",
            &next.begin.run_id,
            "successor-assistant",
        ))
        .unwrap();
    assert_eq!(exact["core_session_id"], "actual-core-session");
    assert_eq!(exact["checkpoint_revision"], current.revision);
    assert_eq!(exact["checkpoint_event_cursor"], current.event_cursor);
    assert_eq!(
        exact["checkpoint_sha256"],
        format!("{:x}", Sha256::digest(current.checkpoint_json.as_bytes()))
    );
}

#[test]
fn exact_read_accepts_owned_terminal_checkpoint_with_retained_or_removed_active_turn() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("terminal-checkpoint.sqlite");
    let begun = begin_distinct(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    cancel_distinct_turn(&mut runtime, &begun);
    let query = || {
        command(
            "canonical-conversation",
            &begun.begin.run_id,
            "owned-assistant",
        )
    };
    let terminal = runtime.execute(query()).unwrap();
    assert_eq!(terminal["status"], "sealed");
    assert_eq!(terminal["core_session_id"], "actual-core-session");
    drop(runtime);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE core_checkpoint_bodies SET checkpoint_json=json_set(checkpoint_json,'$.state.execution.active_turn',NULL)", []).unwrap();
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(
        runtime.execute(query()).unwrap()["core_session_id"],
        "actual-core-session"
    );
    assert!(db.execute(
        "DELETE FROM committed_transitions WHERE run_id=?1",
        [&begun.begin.run_id],
    )
    .is_err(), "immutable requirement evidence must protect the committed transition");
    // Fault-inject only the repository witness, using the actual sealed record.
    // Do not disable SQLite protections or damage unrelated replay revisions.
    use nola_store_port::run_recovery_inspection::*;
    struct MissingCommitWitness(StoredExactRunInvocation);
    impl RunRecoveryInspectionRepository for MissingCommitWitness {
        type Error = String;
        fn list_recoverable_runs(&self, _: Option<&str>, _: usize) -> Result<Vec<RecoverableRunRef>, String> {
            panic!("exact invocation query must not discover unrelated Runs")
        }
        fn inspect_unknown_effect_recovery(&self, _: &str, _: &str, _: &str)
            -> Result<Option<StoredUnknownEffectRecoveryInspection>, String> {
            panic!("exact invocation query must not query another snapshot")
        }
        fn inspect_exact_run(&self, _: &str) -> Result<Option<StoredRunRecoveryInspection>, String> {
            panic!("exact invocation query must not query another snapshot")
        }
        fn load_exact_run_invocation(&self, conversation: &str, run: &str, assistant: &str)
            -> Result<Option<StoredExactRunInvocation>, String> {
            assert_eq!(conversation, self.0.conversation_id);
            assert_eq!(run, self.0.run_id);
            assert_eq!(assistant, self.0.assistant_message_id);
            Ok(Some(self.0.clone()))
        }
    }
    let store = nola_store::NolaStore::open(&path).unwrap();
    let mut record = store.load_exact_run_invocation("canonical-conversation", &begun.begin.run_id, "owned-assistant")
        .unwrap().unwrap();
    assert!(record.terminal.is_some());
    assert!(record.core_checkpoint.as_ref().unwrap().committed_by_run);
    assert!(nola_application::load_exact_run_invocation(&MissingCommitWitness(record.clone()),
        "canonical-conversation".into(), begun.begin.run_id.clone(), "owned-assistant".into()).unwrap().is_some());
    record.core_checkpoint.as_mut().unwrap().committed_by_run = false;
    assert!(
        nola_application::load_exact_run_invocation(&MissingCommitWitness(record),
            "canonical-conversation".into(), begun.begin.run_id.clone(), "owned-assistant".into()).is_err(),
        "absent terminal Turn still requires this Run's commit witness"
    );
}

#[test]
fn exact_invocation_reads_real_answered_failed_and_cancelled_seals_after_reopen() {
    use nola_application::*;
    use nola_agent_engine::agent::{host::*, protocol::*};
    for outcome in ["answered", "failed", "cancelled"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("sealed-invocation.sqlite");
        let begun = begin_distinct(&path);
        let mut runtime = support::open_runtime(&path).unwrap();
        if outcome == "cancelled" {
            cancel_distinct_turn(&mut runtime, &begun);
        } else {
            let claimed: CommittedHostRequests = serde_json::from_value(runtime.execute(
                ApplicationCommand::ClaimCommittedHostRequests { input: ClaimCommittedHostRequests {
                    run_id: begun.begin.run_id.clone(), worker_generation: "reader-worker".into(),
                    lease_owner: "reader-worker".into(), claimed_at_ms: 2, lease_expires_at_ms: 1000, limit: 64,
                }}).unwrap()).unwrap();
            let request = claimed.requests.iter().find(|request|
                matches!(request.payload, HostRequest::ModelStream { .. })).unwrap();
            let finish = if outcome == "failed" {
                HostResponse::Failed { error: HostError { code: "reader_fixture_failure".into(),
                    message: "Observed model failure".into(), retryable: false, http_status: None }}
            } else {
                HostResponse::ModelCompleted { items: vec![serde_json::to_value(
                    AgentItem::assistant_message(ItemId::from_string("owned-answer".into()), "Hello.")
                ).unwrap()], tool_calls: vec![], usage: json!({}) }
            };
            for (index, payload) in [HostResponse::ExecutionStarted,
                HostResponse::ModelAttemptStarted { provider: "fixture".into(), model: "original-model".into(), reason: "initial".into() },
                finish].into_iter().enumerate() {
                runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                    input: ReduceAndAcceptHostResponse {
                        conversation_id: "canonical-conversation".into(), run_id: begun.begin.run_id.clone(),
                        invocation_config_digest: begun.invocation_config_digest.clone(), received_at_ms: 3 + index as i64,
                        response: HostResponseEnvelope {
                            request_id: request.request_id.clone(), attempt_id: request.attempt_id.clone(),
                            delegated_invocation_id: request.delegated_invocation_id.clone(), effect_id: request.effect_id.clone(),
                            host_id: request.host_id.clone(), lease_id: request.lease_id.clone(), session_id: request.session_id.clone(),
                            turn_id: request.turn_id.clone(), catalog_revision: request.catalog_revision,
                            availability_epoch: request.availability_epoch, delivery: request.delivery.clone(),
                            sequence: index as u64 + 1, payload,
                        },
                    },
                }).unwrap();
            }
        }
        drop(runtime);
        let store = nola_store::NolaStore::open(&path).unwrap();
        let seal = store.canonical_run_terminal("canonical-conversation", &begun.begin.run_id).unwrap().unwrap();
        assert_eq!(serde_json::to_value(&seal.result.value).unwrap()["kind"], outcome);
        drop(store);
        let mut runtime = support::open_runtime(&path).unwrap();
        let exact = runtime.execute(command("canonical-conversation", &begun.begin.run_id, "owned-assistant")).unwrap();
        assert_eq!(exact["status"], "sealed");
        assert_eq!(exact["invocation_config_digest"], begun.invocation_config_digest);
        assert_eq!(exact["core_session_id"], "actual-core-session");
        assert!(runtime.execute(command("canonical-conversation", &begun.begin.run_id, "foreign-assistant")).is_err());
        drop(runtime);
        // A sealed header alone is not completion evidence. Keep the original
        // identity and checkpoint; remove only the immutable result witness.
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("DELETE FROM assistant_run_terminal_seals WHERE run_id=?1", [&begun.begin.run_id]).unwrap();
        let mut runtime = support::open_runtime(&path).unwrap();
        assert!(runtime.execute(command("canonical-conversation", &begun.begin.run_id, "owned-assistant")).is_err(),
            "{outcome}: sealed header without its result witness cannot be read as terminal");
    }
}
