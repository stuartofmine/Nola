// SPDX-License-Identifier: Apache-2.0
//! Run invocation recovery, checkpoint identity, recovery inspection, and recorded recovery.

mod run_invocation_recovery_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! The public read-only query returns verified binding, not raw Core state.
    use crate::support;
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
}

mod run_recovery_checkpoint_identity_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Application receipt and cold recovery across queued model-completion boundaries.
    use crate::support;
    use nola_application::{
        begin_assistant_turn, load_committed_host_requests, ApplicationCommand, BeginAssistantTurn,
        ExecuteReceivedCommand, ReduceAndAcceptHostResponse, ReduceAndCommitActiveSubmission,
    };
    use nola_agent_engine::agent::host::*;
    use nola_agent_engine::agent::protocol::*;
    use nola_store::NolaStore;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;

    fn runtime_capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
        HostCapabilityDescriptor {
            id: id.to_string(),
            title: id.to_string(),
            route,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Internal,
            permission_domain: None,
            input_schema: serde_json::json!({"type": "object"}),
            input_normalization: Default::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 4_000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: id == "model.stream",
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        }
    }

    fn runtime_config() -> AgentCoreConfig {
        let mut capabilities = BTreeMap::new();
        capabilities.insert(
            "model.stream".to_string(),
            runtime_capability("model.stream", HostCapabilityRoute::Model),
        );
        support::goal_execution::with_execution_binding(AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string(
                "completion-conversation".to_string(),
            )),
            model: Some("test-model".to_string()),
            host_catalog: Some(HostCapabilityCatalog {
                host_id: HostId("recovery-host".to_string()),
                revision: 1,
                availability_epoch: 1,
                lease: None,
                capabilities,
            }),
            ..AgentCoreConfig::default()
        })
    }

    fn response_for_request(
        request: &HostRequestEnvelope,
        sequence: u64,
        payload: HostResponse,
    ) -> HostResponseEnvelope {
        HostResponseEnvelope {
            attempt_id: request.attempt_id.clone(),
            delegated_invocation_id: request.delegated_invocation_id.clone(),
            request_id: request.request_id.clone(),
            effect_id: request.effect_id.clone(),
            host_id: request.host_id.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            delivery: request.delivery.clone(),
            sequence,
            payload,
        }
    }

    fn response_command(
        request: &HostRequestEnvelope,
        run: &str,
        digest: &str,
        payload: HostResponse,
    ) -> ApplicationCommand {
        ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: "completion-conversation".into(),
                run_id: run.into(),
                invocation_config_digest: digest.into(),
                response: response_for_request(
                    request,
                    request
                        .resume_after_sequence
                        .map_or(1, |sequence| sequence + 1),
                    payload,
                ),
                received_at_ms: 2000,
            },
        }
    }

    fn canonical_rows(db: &rusqlite::Connection) -> Vec<Vec<String>> {
        ["SELECT checkpoint_json FROM core_checkpoint_bodies ORDER BY conversation_id",
         "SELECT id,role,status,coalesce(run_id,''),ordinal FROM messages ORDER BY id",
         "SELECT id,kind,state,coalesce(content_text,''),coalesce(content_json,'') FROM message_parts ORDER BY id",
         "SELECT effect_id,status,json_array(binding_json,payload_json) FROM effect_outbox JOIN effect_outbox_payloads USING(effect_id) ORDER BY effect_id"]
            .into_iter().map(|sql| {
                let mut statement = db.prepare(sql).unwrap();
                let count = statement.column_count();
                statement.query_map([], |row| {
                    let contents = (0..count).map(|index| format!("{:?}", row.get_ref(index).unwrap())).collect::<Vec<_>>().join("|");
                    Ok(format!("{:x}", Sha256::digest(contents.as_bytes())))
                }).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
            }).collect()
    }

    fn user_input(id: &str, delivery: AgentInputDelivery) -> AgentSubmission {
        AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string(id.into()), id)],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some(format!("submit-{id}")),
            submitted_at_epoch_ms: Some(1000),
            delivery,
        }
    }

    fn pending(database: &std::path::Path, run: &str) -> Vec<HostRequestEnvelope> {
        load_committed_host_requests(&mut NolaStore::open(database).unwrap(), run)
            .unwrap()
            .requests
    }

    fn complete_and_replay(
        database: &std::path::Path,
        run: &str,
        digest: &str,
        request: &HostRequestEnvelope,
        answer: Option<&str>,
    ) {
        let mut admitted = request.clone();
        let mut store = NolaStore::open(database).unwrap();
        support::admit_model_sampling(&mut store, run, digest, &mut admitted, 2000);
        drop(store);
        assert!(admitted.attempt_started);
        let request = &admitted;
        let command = response_command(
            request,
            run,
            digest,
            HostResponse::ModelCompleted {
                items: answer
                    .into_iter()
                    .map(|text| {
                        serde_json::to_value(AgentItem::assistant_message(
                            ItemId::from_string(format!("answer-{}", request.request_id)),
                            text,
                        ))
                        .unwrap()
                    })
                    .collect(),
                tool_calls: vec![],
                usage: json!({"input_tokens": 80, "output_tokens": 10}),
            },
        );
        let command_json = serde_json::to_string(&command).unwrap();
        let receipt = || ExecuteReceivedCommand {
            client_id: "completion-client".into(),
            request_id: request.request_id.clone(),
            command_type: "reduce_and_accept_host_response".into(),
            command_json: command_json.clone(),
            expected_revision: None,
            received_at: "2000".into(),
        };
        let mut runtime = support::open_runtime(database).unwrap();
        let accepted = runtime
            .execute_received_command(receipt(), command)
            .unwrap();
        assert!(matches!(
            accepted.outcome,
            nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        drop(runtime);
        let mut runtime = support::open_runtime(database).unwrap();
        let db = rusqlite::Connection::open(database).unwrap();
        let before = canonical_rows(&db);
        runtime
            .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
            .unwrap();
        assert_eq!(
            canonical_rows(&db),
            before,
            "Cold receipt replay cannot duplicate successor models or mutate canonical history"
        );
    }

    fn successor_fixture(database: &std::path::Path) -> (String, String, Vec<HostRequestEnvelope>) {
        let mut store = NolaStore::open(&database).unwrap();
        let begun = begin_assistant_turn(
            &mut store,
            BeginAssistantTurn {
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_id: "completion-conversation".into(),
                conversation_title: "Completion".into(),
                user_message_id: "initial".into(),
                user_part_id: "initial:body".into(),
                user_text: "initial".into(),
                user_content_json: None,
                assistant_message_id: "submit-initial:assistant".into(),
                context_checkpoint_id: None,
                core_config: runtime_config(),
                submission: user_input("initial", AgentInputDelivery::CurrentTurn),
                created_at_ms: 1000,
            },
        )
        .unwrap();
        let run = begun.begin.run_id;
        let digest = begun.invocation_config_digest;
        let first = load_committed_host_requests(&mut store, &run)
            .unwrap()
            .requests
            .into_iter()
            .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
            .unwrap();
        drop(store);
        let mut runtime = support::open_runtime(&database).unwrap();
        for (id, delivery) in [
            ("steering", AgentInputDelivery::CurrentTurn),
            ("next", AgentInputDelivery::NextTurn),
        ] {
            runtime
                .execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
                    input: ReduceAndCommitActiveSubmission {
                        conversation_id: "completion-conversation".into(),
                        run_id: run.clone(),
                        transition_id: format!("queue-{id}"),
                        invocation_config_digest: digest.clone(),
                        submission: user_input(id, delivery),
                        committed_at_ms: 1100,
                    },
                })
                .unwrap();
        }
        drop(runtime);
        complete_and_replay(&database, &run, &digest, &first, None);
        let requests = pending(&database, &run);
        assert_eq!(requests.len(), 1);
        let second = &requests[0];
        assert!(matches!(second.payload, HostRequest::ModelStream { .. }));
        assert_eq!(
            second.turn_id, first.turn_id,
            "Current-turn steering must keep the original Turn alive"
        );
        assert_ne!(second.effect_id, first.effect_id);
        complete_and_replay(&database, &run, &digest, second, Some("Steering handled."));
        assert!(pending(&database, &run).is_empty(), "completion must not dispatch a successor under the predecessor Run");
        let mut store = NolaStore::open(&database).unwrap();
        let successor = nola_application::promote_queued_assistant_turn(
            &mut store,
            nola_application::PromoteQueuedAssistantTurn {
                conversation_id: "completion-conversation".into(),
                created_at_ms: 2000,
            },
        ).unwrap().expect("explicit successor admission");
        assert_ne!(successor.begin.run_id, run);
        let run = successor.begin.run_id;
        let digest = successor.invocation_config_digest;
        drop(store);
        let requests = pending(&database, &run);
        assert_eq!(requests.len(), 1);
        let third = &requests[0];
        assert!(matches!(third.payload, HostRequest::ModelStream { .. }));
        assert_ne!(
            third.turn_id, first.turn_id,
            "Next-turn input begins only after the original Turn completes"
        );
        let db = rusqlite::Connection::open(&database).unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
                .unwrap(),
            2,
            "Each explicitly admitted successor owns an independent canonical Run"
        );
        (run, digest, requests)
    }

    #[test]
    fn startup_uses_current_checkpoint_instead_of_an_earlier_turn_terminal_event() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("completion.db");
        let (run, digest, requests) = successor_fixture(&database);
        let third = &requests[0];
        let db = rusqlite::Connection::open(&database).unwrap();
        let checkpoint_json =
            support::current_checkpoint(&database, "completion-conversation").checkpoint_json;
        let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState> =
            serde_json::from_str(&checkpoint_json).unwrap();
        let cold_core = nola_agent_engine::agent::AgentCore::restore(runtime_config(), checkpoint).unwrap();
        assert_eq!(
            cold_core
                .snapshot()
                .active_turn_id
                .as_ref()
                .map(|id| id.as_str()),
            third.turn_id.as_deref()
        );
        assert_eq!(third.turn_id.as_deref(), Some(run.as_str()));
        assert_eq!(
            cold_core.snapshot().turn_status,
            nola_agent_engine::agent::AgentTurnStatus::AwaitingModel
        );
        let before = canonical_rows(&db);
        let mut runtime = support::open_runtime(&database).unwrap();
        runtime
            .execute(ApplicationCommand::RecoverOrphanedRuns {
                conversation_id: None,
                reason: "server_restarted".into(),
                interrupted_at: "after".into(),
                interrupted_at_ms: 4000,
            })
            .unwrap();
        drop(runtime);
        let store = NolaStore::open(&database).unwrap();
        let active = store.active_run("completion-conversation").unwrap();
        assert_eq!(active.as_ref().map(|record| record.run_id.as_str()), Some(run.as_str()),
            "A pending recoverable successor Model effect cannot be completed by a prior Turn's terminal event");
        assert_eq!(
            canonical_rows(&db),
            before,
            "Startup must preserve canonical items, checkpoint and pending model for explicit recovery"
        );
        let recovered = load_committed_host_requests(&store, &run).unwrap().requests;
        assert_eq!(
            serde_json::to_value(&recovered).unwrap(),
            serde_json::to_value(&requests).unwrap()
        );
        drop(store);
        complete_and_replay(
            &database,
            &run,
            &digest,
            third,
            Some("Recovered successor completed."),
        );
        assert!(
            pending(&database, &run).is_empty(),
            "Recovery must actually accept the pending Host result, not merely relabel the Run"
        );
    }

    fn recovery(scoped: bool) -> ApplicationCommand {
        ApplicationCommand::RecoverOrphanedRuns {
            conversation_id: scoped.then(|| "completion-conversation".into()),
            reason: "server_restarted".into(),
            interrupted_at: "after".into(),
            interrupted_at_ms: 4000,
        }
    }

    fn follow_up_command(frontier: (Option<i64>, Option<String>)) -> BeginAssistantTurn {
        let mut core_config = runtime_config();
        core_config.model = Some("new-build-model".into());
        BeginAssistantTurn {
            expected_configuration_revision: frontier.0,
            expected_parent_message_id: frontier.1,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Completion".into(),
            user_message_id: "follow-up".into(),
            user_part_id: "follow-up:body".into(),
            user_text: "follow-up".into(),
            user_content_json: None,
            assistant_message_id: "submit-follow-up:assistant".into(),
            context_checkpoint_id: None,
            core_config,
            submission: user_input("follow-up", AgentInputDelivery::CurrentTurn),
            created_at_ms: 4000,
        }
    }

    fn obsolete_host_bindings() -> Vec<serde_json::Value> {
        use nola_host_protocol::*;
        let route = HostModelRouteBinding::Native(HostNativeModelRouteBinding {
            adapter: HostNativeModelAdapter::LlamaCpp,
            adapter_contract_revision: 1,
            model_id: "old-build-model".into(),
            profile_ref: None,
        });
        let admission = HostOutputAdmission::admit(
            vec![HostRouteTokenLimits {
                route: route.clone(),
                max_input_tokens: None,
                max_output_tokens: None,
                max_total_tokens: None,
            }],
            nola_domain::generation::OutputTokenPreference::Default,
        )
        .unwrap();
        let binding = HostInvocationBinding {
            mac_launch: None,
            version: HOST_INVOCATION_BINDING_VERSION,
            primary: route,
            fallbacks: vec![],
            generation: HostGenerationBinding {
                max_retries: 2,
                parallel_tool_calls: None,
                reasoning_selection: None,
                reasoning_tier: Default::default(),
                sampling_override: false,
                generation_options: nola_domain::generation::GenerationOptions::default()
                    .try_into()
                    .unwrap(),
                temperature: serde_json::Number::from_f64(0.7).unwrap(),
                top_p: serde_json::Number::from_f64(0.9).unwrap(),
                max_output_tokens: admission.resolved.tokens,
                thinking_enabled: false, reasoning_summary_supported: None,
                prompt_cache: false,
                prompt_cache_key_sha256: None,
                prompt_cache_key_prefix_sha256: None,
                prompt_cache_retention: None,
            },
            output_admission: admission,
            execution_policy_sha256: "a".repeat(64),
        };
        let mut missing_tier = serde_json::to_value(binding).unwrap();
        missing_tier["generation"]
            .as_object_mut()
            .unwrap()
            .remove("reasoning_tier");
        assert!(
            serde_json::from_value::<HostInvocationBinding>(missing_tier.clone())
                .unwrap_err()
                .to_string()
                .contains("reasoning_tier")
        );
        vec![
            missing_tier,
            json!({"version":999,"obsolete_transport":{"unknown":true}}),
        ]
    }

    fn replace_checkpoint_binding(database: &std::path::Path, binding: &serde_json::Value) {
        let db = rusqlite::Connection::open(database).unwrap();
        let raw: String = db
            .query_row(
                "SELECT checkpoint_json FROM core_checkpoint_bodies",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut checkpoint: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for parent in [
            "/state/configuration/turn_identity",
            "/state/execution/active_turn/context/configuration/turn_identity",
        ] {
            if let Some(identity) = checkpoint.pointer_mut(parent) {
                identity["host_invocation"] = binding.clone();
            }
        }
        assert!(serde_json::from_value::<
            nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState>,
        >(checkpoint.clone())
        .is_err());
        db.execute(
            "UPDATE core_checkpoint_bodies SET checkpoint_json=?1",
            [checkpoint.to_string()],
        )
        .unwrap();
    }

    #[test]
    fn cold_new_turn_preserves_completed_history_and_goal_across_obsolete_host_schema() {
        use nola_agent_engine::agent::{
            GoalRecordStatus, GoalRequirement, GoalRuntimeCommand, GoalRuntimeCommandEnvelope,
        };
        for binding in obsolete_host_bindings() {
            let directory = tempfile::tempdir().unwrap();
            let database = directory.path().join("cross-build.db");
            let (old_run, digest, requests) = successor_fixture(&database);
            complete_and_replay(
                &database,
                &old_run,
                &digest,
                &requests[0],
                Some("Previous build completed."),
            );
            let mut store = NolaStore::open(&database).unwrap();
            assert!(store
                .active_run("completion-conversation")
                .unwrap()
                .is_none());
            support::goal_execution::admit_start(
                &mut store,
                nola_application::ReduceAndCommitIdleSubmission {
                    conversation_id: "completion-conversation".into(),
                    core_config: runtime_config(),
                    submission: AgentSubmission::GoalRuntime {
                        envelope: GoalRuntimeCommandEnvelope {
                            operation_id: "durable-goal-start".into(),
                            command: GoalRuntimeCommand::Start {
                                id: "durable-goal".into(),
                                objective: "Preserve work across builds".into(),
                                constraints: vec!["Preserve canonical history".into()],
                                requirements: vec![GoalRequirement {
                                    acceptance_policy: nola_agent_engine::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                                    semantic_revision: 1, id: "cross-build-follow-up".into(),
                                    description:
                                        "Resume the same goal after a build changes Host schema".into(),
                                    dependencies: vec![],
                                    status: GoalRecordStatus::Pending,
                                    evidence_refs: vec![],
                                }],
                                now_epoch_ms: 3000,
                            },
                        },
                    },
                    committed_at_ms: 3000,
                },
            )
            .unwrap();
            drop(store);
            let before: serde_json::Value = serde_json::from_str(
                &support::current_checkpoint(&database, "completion-conversation").checkpoint_json,
            )
            .unwrap();
            assert!(before["state"]["conversation"]["transcript"]
                .to_string()
                .contains("Previous build completed."));
            assert_eq!(
                before["state"]["goal_projection"]["active"]["id"],
                "durable-goal"
            );
            replace_checkpoint_binding(&database, &binding);

            let mut store = NolaStore::open(&database).unwrap();
            // Desktop performs a configuration submission BEFORE beginning a Run.
            // Testing BeginAssistantTurn alone misses the actual bootstrap path.
            nola_application::reduce_and_commit_idle_submission(
                &mut store,
                nola_application::ReduceAndCommitIdleSubmission {
                    conversation_id: "completion-conversation".into(),
                    core_config: runtime_config(),
                    submission: AgentSubmission::UpdateSessionConfiguration {
                        patch: nola_agent_engine::agent::AgentSessionConfigurationPatch {
                            max_model_iterations: Some(17),
                            ..Default::default()
                        },
                    },
                    committed_at_ms: 3100,
                },
            ).unwrap();
            let prepared: serde_json::Value = serde_json::from_str(
                &support::current_checkpoint(&database, "completion-conversation").checkpoint_json,
            ).unwrap();
            assert_eq!(prepared["state"]["conversation"]["transcript"],
                before["state"]["conversation"]["transcript"]);
            assert_eq!(prepared["state"]["goal_projection"], before["state"]["goal_projection"]);
            let frontier = support::admission_frontier::read(&store, "completion-conversation");
            let next = begin_assistant_turn(&mut store, follow_up_command(frontier)).unwrap();
            assert_ne!(next.begin.run_id, old_run);
            assert_eq!(
                store
                    .active_run("completion-conversation")
                    .unwrap()
                    .unwrap()
                    .run_id,
                next.begin.run_id
            );
            drop(store);
            let after: serde_json::Value = serde_json::from_str(
                &support::current_checkpoint(&database, "completion-conversation").checkpoint_json,
            )
            .unwrap();
            assert_eq!(
                after["state"]["goal_projection"],
                before["state"]["goal_projection"]
            );
            let old_items = before["state"]["conversation"]["transcript"]
                .as_array()
                .unwrap();
            let new_items = after["state"]["conversation"]["transcript"]
                .as_array()
                .unwrap();
            assert!(
                new_items.starts_with(old_items),
                "A new build must retain the complete canonical transcript"
            );
            let db = rusqlite::Connection::open(&database).unwrap();
            assert_eq!(
                db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
                    .unwrap(),
                3
            );
        }
    }

    #[test]
    fn obsolete_active_run_cannot_be_replaced_by_new_build_configuration() {
        for binding in obsolete_host_bindings() {
            let directory = tempfile::tempdir().unwrap();
            let database = directory.path().join("active-cross-build.db");
            let (run, _, _) = successor_fixture(&database);
            replace_checkpoint_binding(&database, &binding);
            let db = rusqlite::Connection::open(&database).unwrap();
            let before = canonical_rows(&db);
            let mut store = NolaStore::open(&database).unwrap();
            let idle_error = nola_application::reduce_and_commit_idle_submission(
                &mut store,
                nola_application::ReduceAndCommitIdleSubmission {
                    conversation_id: "completion-conversation".into(),
                    core_config: runtime_config(),
                    submission: AgentSubmission::UpdateSessionConfiguration {
                        patch: Default::default(),
                    },
                    committed_at_ms: 3100,
                },
            ).unwrap_err();
            assert!(idle_error.contains("cannot overlap an active Run"), "{idle_error}");
            assert_eq!(canonical_rows(&db), before);
            let frontier = support::admission_frontier::read(&store, "completion-conversation");
            let error = begin_assistant_turn(&mut store, follow_up_command(frontier)).unwrap_err();
            assert!(error.contains("cannot overlap an active Run"), "{error}");
            assert_eq!(
                store
                    .active_run("completion-conversation")
                    .unwrap()
                    .unwrap()
                    .run_id,
                run
            );
            assert_eq!(
                canonical_rows(&db),
                before,
                "Rejected admission must not rewrite history, output or pending effects"
            );
            assert_eq!(
                db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
                    .unwrap(),
                2
            );
        }
    }

    fn recovery_rows(db: &rusqlite::Connection) -> Vec<Vec<String>> {
        let mut rows = canonical_rows(db);
        for table in ["runs", "trace_journal_runs", "session_run_leases"] {
            let mut query = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let columns = query.column_count();
            rows.push(
                query
                    .query_map([], |row| {
                        Ok((0..columns)
                            .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                            .collect::<Vec<_>>()
                            .join("|"))
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap(),
            );
        }
        rows
    }

    fn traces(db: &rusqlite::Connection, turn: &str) {
        db.execute("INSERT INTO trace_journal_runs(trace_id,run_id,conversation_id,turn_id,status,started_at,updated_at) VALUES('current','diagnostic-only','completion-conversation',?1,'running','before','before')", [turn]).unwrap();
        db.execute("INSERT INTO trace_journal_runs(trace_id,run_id,conversation_id,turn_id,status,started_at,updated_at) VALUES('stale','other-diagnostic','completion-conversation','old-turn','running','before','before')", []).unwrap();
    }

    #[test]
    fn scoped_recovery_respects_a_live_session_lease_without_touching_trace_or_history() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("lease.db");
        let (_, _, requests) = successor_fixture(&database);
        let db = rusqlite::Connection::open(&database).unwrap();
        traces(&db, requests[0].turn_id.as_deref().unwrap());
        db.execute("INSERT INTO session_run_leases(session_id,owner,pid,process_start,acquired_at_ms) VALUES('completion-conversation','live-owner',123,'process-identity',1000)", []).unwrap();
        let before = recovery_rows(&db);
        let mut runtime = support::open_runtime(&database).unwrap();
        runtime.execute(recovery(true)).unwrap();
        drop(runtime);
        assert_eq!(
            recovery_rows(&db),
            before,
            "An explicit conversation scope cannot override a live ownership lease"
        );
    }

    #[test]
    fn corrupt_checkpoint_recovery_fails_without_mutating_canonical_or_trace_records() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("corrupt.db");
        let (_, _, requests) = successor_fixture(&database);
        let db = rusqlite::Connection::open(&database).unwrap();
        traces(&db, requests[0].turn_id.as_deref().unwrap());
        db.execute("UPDATE core_checkpoint_bodies SET checkpoint_json='{}'", [])
            .unwrap();
        let before = recovery_rows(&db);
        let mut runtime = support::open_runtime(&database).unwrap();
        assert!(
            runtime.execute(recovery(false)).is_err(),
            "Unknown checkpoint identity must fail closed"
        );
        drop(runtime);
        assert_eq!(recovery_rows(&db), before);
    }

    #[test]
    fn recovery_trace_and_receipt_faults_roll_back_then_cold_replay_is_exact() {
        for trace_fault in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let database = directory.path().join("rollback.db");
            let (run, _, requests) = successor_fixture(&database);
            let db = rusqlite::Connection::open(&database).unwrap();
            traces(&db, requests[0].turn_id.as_deref().unwrap());
            let before = recovery_rows(&db);
            let command_json = serde_json::to_string(&recovery(false)).unwrap();
            let receipt = |id: &str| ExecuteReceivedCommand {
                client_id: "recovery-client".into(),
                request_id: id.into(),
                command_type: "recover_orphaned_runs".into(),
                command_json: command_json.clone(),
                expected_revision: None,
                received_at: "4000".into(),
            };
            db.execute_batch(if trace_fault {
                "CREATE TRIGGER fail_recovery BEFORE UPDATE ON trace_journal_runs BEGIN SELECT RAISE(ABORT,'trace recovery fault'); END"
            } else {
                "CREATE TRIGGER fail_recovery BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'receipt recovery fault'); END"
            }).unwrap();
            let mut runtime = support::open_runtime(&database).unwrap();
            let failed = runtime.execute_received_command(receipt("fault"), recovery(false));
            if trace_fault {
                let outcome = failed.unwrap().outcome;
                assert!(
                    matches!(outcome, nola_application::ApplicationCommandReceiptOutcome::Rejected { ref message, .. }
                    if message.contains("trace recovery fault")),
                    "Unexpected receipt: {outcome:?}"
                );
            } else {
                let error = failed.unwrap_err();
                assert!(
                    error.message.contains("receipt recovery fault"),
                    "Unexpected rejection: {error}"
                );
            }
            drop(runtime);
            assert_eq!(
                recovery_rows(&db),
                before,
                "Failure cannot publish partial startup changes"
            );
            db.execute_batch("DROP TRIGGER fail_recovery").unwrap();
            let mut runtime = support::open_runtime(&database).unwrap();
            let committed = runtime
                .execute_received_command(receipt("success"), recovery(false))
                .unwrap();
            drop(runtime);
            let mut runtime = support::open_runtime(&database).unwrap();
            let after = recovery_rows(&db);
            let replay = runtime
                .execute_received_command(receipt("success"), recovery(false))
                .unwrap();
            assert!(replay.replayed);
            assert_eq!(replay.outcome, committed.outcome);
            assert_eq!(recovery_rows(&db), after);
            drop(runtime);
            let store = NolaStore::open(&database).unwrap();
            assert_eq!(
                store
                    .active_run("completion-conversation")
                    .unwrap()
                    .unwrap()
                    .run_id,
                run
            );
            assert_eq!(
                serde_json::to_value(load_committed_host_requests(&store, &run).unwrap().requests)
                    .unwrap(),
                serde_json::to_value(&requests).unwrap()
            );
            let state = |trace: &str| {
                db.query_row::<String, _, _>(
                    "SELECT status FROM trace_journal_runs WHERE trace_id=?1",
                    [trace],
                    |row| row.get(0),
                )
                .unwrap()
            };
            assert_eq!(state("current"), "running");
            assert_eq!(state("stale"), "interrupted");
        }
    }
}

mod run_recovery_inspection_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Real read-only command over isolated persisted facts, without OS inference.
    use crate::support;
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
}

mod recorded_recovery_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Opt-in regression against a private recorded failure. Never opens the source
    //! with application adapters and never executes tools or calls a model.
    use crate::support;

    #[test]
    #[ignore = "requires NOLA_RECOVERY_SOURCE_DB and NOLA_RECOVERY_CONVERSATION"]
    fn recorded_pending_model_can_be_materialized_and_cancelled_on_an_isolated_copy() {
        use nola_application::{ApplicationCommand, RequestRunCancellation, RunCancellationReceipt};
        let source = std::env::var("NOLA_RECOVERY_SOURCE_DB").unwrap();
        let conversation = std::env::var("NOLA_RECOVERY_CONVERSATION").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let copy = directory.path().join("recorded.db");
        let status = std::process::Command::new("sqlite3")
            .args(["-readonly", &source, &format!(".backup '{}'", copy.display())])
            .status().unwrap();
        assert!(status.success());
        let db = rusqlite::Connection::open(&copy).unwrap();
        let (run, assistant): (String, String) = db.query_row(
            "SELECT id,assistant_message_id FROM runs WHERE conversation_id=?1 AND status='running' ORDER BY created_at_ms DESC LIMIT 1",
            [&conversation], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        drop(db);
        let mut runtime = support::open_runtime(&copy).unwrap();
        runtime.execute(ApplicationCommand::LoadCheckpoint { conversation_id: conversation.clone() }).unwrap();
        let exact: nola_application::ExactRunInvocation = serde_json::from_value(runtime.execute(
            ApplicationCommand::LoadExactRunInvocation { conversation_id: conversation.clone(),
                expected_run_id: run.clone(), assistant_message_id: assistant.clone() }).unwrap()).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        let original: nola_application::CommittedHostRequests = serde_json::from_value(runtime.execute(
            ApplicationCommand::LoadCommittedHostRequests { run_id: run.clone() }).unwrap()).unwrap();
        runtime.execute(ApplicationCommand::RecoverExpiredRunDeliveries {
            conversation_id: conversation.clone(), expected_run_id: run.clone(),
            host_invocation: exact.host_invocation, interrupted_at: now.to_string(), interrupted_at_ms: now,
        }).unwrap();
        // The shared Application command selects only authorized interrupted tails.
        let prepare = || ApplicationCommand::PrepareRunEffectRecovery {
            input: nola_application::PrepareRunEffectRecovery {
                conversation_id: conversation.clone(), run_id: run.clone(),
                invocation_config_digest: exact.invocation_config_digest.clone(), now_ms: now,
            },
        };
        for (owner, digest) in [("another-run".to_owned(), exact.invocation_config_digest.clone()),
            (run.clone(), "wrong-frozen-digest".to_owned())] {
            assert!(runtime.execute(ApplicationCommand::PrepareRunEffectRecovery {
                input: nola_application::PrepareRunEffectRecovery {
                    conversation_id: conversation.clone(), run_id: owner,
                    invocation_config_digest: digest, now_ms: now,
                },
            }).is_err());
            let unchanged: nola_application::CommittedHostRequests = serde_json::from_value(runtime.execute(
                ApplicationCommand::LoadCommittedHostRequests { run_id: run.clone() }).unwrap()).unwrap();
            assert_eq!(unchanged.requests.iter().map(|r| &r.request_id).collect::<Vec<_>>(),
                original.requests.iter().map(|r| &r.request_id).collect::<Vec<_>>(),
                "rejected recovery must not rebind another owner's requests");
        }
        let rebound = runtime.execute(prepare()).unwrap()["rebound_count"].as_u64().unwrap();
        assert!(rebound > 0, "recorded interrupted stream must be rebound");
        assert_eq!(runtime.execute(prepare()).unwrap()["rebound_count"].as_u64(), Some(0), "replay must not allocate another request");
        let claimed: nola_application::CommittedHostRequests = serde_json::from_value(runtime.execute(
            ApplicationCommand::ClaimCommittedHostRequests { input: nola_application::ClaimCommittedHostRequests {
                run_id: run.clone(), worker_generation: "isolated-generation".into(), lease_owner: "isolated-owner".into(),
                claimed_at_ms: now + 1, lease_expires_at_ms: now + 60_000, limit: 1,
            } }).unwrap()).unwrap();
        assert_eq!(claimed.requests.len(), 1, "the original pending request must be reclaimable");
        let resumed = &claimed.requests[0];
        let frozen = original.requests.iter().find(|r| r.effect_id == resumed.effect_id).unwrap();
        assert_eq!(serde_json::to_value(&resumed.payload).unwrap(), serde_json::to_value(&frozen.payload).unwrap(),
            "recovery must preserve the exact original model input");
        assert_eq!(resumed.turn_id.as_deref(), Some(run.as_str()));
        // Reclaim only: no Host executor runs and no model request is sent.
        let make_command = || ApplicationCommand::RequestRunCancellation { input: RequestRunCancellation {
            conversation_id: conversation.clone(), expected_run_id: run.clone(), assistant_message_id: assistant.clone(),
            cancellation_id: "isolated-recorded-cancel".into(), reason: None,
            requested_at_ms: 1 + std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64,
        } };
        let first: RunCancellationReceipt = serde_json::from_value(runtime.execute(make_command()).unwrap()).unwrap();
        assert_eq!(first.run_id, run);
        assert_eq!(first.assistant_message_id, assistant);
        assert_eq!(first.disposition, nola_application::RunCancellationDisposition::CancellationCommitted);
        assert!(first.terminal_result.is_none(), "intent is not cancellation proof");
        drop(runtime);
        let mut runtime = support::open_runtime(&copy).unwrap();
        let second: RunCancellationReceipt = serde_json::from_value(runtime.execute(make_command()).unwrap()).unwrap();
        assert_eq!(second.disposition, nola_application::RunCancellationDisposition::AlreadyCancelling);
        assert!(second.terminal_result.is_none());
    }
}
