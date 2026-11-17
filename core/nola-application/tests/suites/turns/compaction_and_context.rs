// SPDX-License-Identifier: Apache-2.0
//! Compaction kernel receipts, lineage, context checkpoints, usage reads, and conversation recall.

mod compaction_kernel_receipt_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Application/SQLite compaction admission, receipt rollback and cold replay.
    use crate::support;
    use nola_application::{
        begin_assistant_turn, load_committed_host_requests, ApplicationCommand, BeginAssistantTurn,
        ExecuteReceivedCommand, ReduceAndAcceptHostResponse,
    };
    use nola_agent_engine::agent::host::*;
    use nola_agent_engine::agent::protocol::*;
    use nola_store::NolaStore;
    use serde_json::{json, Value};
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
        AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string(
                "compaction-conversation".to_string(),
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
        }
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
                conversation_id: "compaction-conversation".into(),
                run_id: run.into(),
                invocation_config_digest: digest.into(),
                response: response_for_request(
                    request,
                    request.resume_after_sequence.unwrap_or(0) + 1,
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
         "SELECT effect_id,status,json_array(binding_json,payload_json) FROM effect_outbox JOIN effect_outbox_payloads USING(effect_id) ORDER BY effect_id",
         "SELECT * FROM core_checkpoints ORDER BY conversation_id",
         "SELECT * FROM core_delivery_witnesses ORDER BY conversation_id",
         "SELECT * FROM core_checkpoint_replay_bases ORDER BY conversation_id",
         "SELECT * FROM host_event_inbox ORDER BY event_id",
         "SELECT * FROM host_event_replay_heads ORDER BY run_id",
         "SELECT * FROM host_event_replay_order ORDER BY event_id",
         "SELECT * FROM run_events ORDER BY event_id"]
            .into_iter().map(|sql| {
                let mut statement = db.prepare(sql).unwrap();
                let count = statement.column_count();
                statement.query_map([], |row| {
                    let contents = (0..count).map(|index| format!("{:?}", row.get_ref(index).unwrap())).collect::<Vec<_>>().join("|");
                    Ok(format!("{:x}", Sha256::digest(contents.as_bytes())))
                }).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
            }).collect()
    }

    fn current_checkpoint(runtime: &mut support::ApplicationRuntime) -> Value {
        let current: nola_application::ConversationCheckpoint = serde_json::from_value(
            runtime
                .execute(ApplicationCommand::LoadCheckpoint {
                    conversation_id: "compaction-conversation".into(),
                })
                .unwrap(),
        )
        .unwrap();
        serde_json::from_str(&current.checkpoint_json).unwrap()
    }

    #[test]
    fn committed_compaction_rejects_invalid_summary_then_receipt_and_cold_replay_preserve_history() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("compaction.db");
        let mut store = NolaStore::open(&database).unwrap();
        let mut config = runtime_config();
        config.context_window = nola_agent_engine::agent::ContextWindowConfig {
            context_window_tokens: 128_000,
            reserved_output_tokens: 8_192,
            auto_compact_tokens: Some(120_000),
            compaction_target_tokens: Some(2_000),
            ..Default::default()
        };
        let compact_capability = runtime_capability("context.compact", HostCapabilityRoute::Compaction);
        config
            .host_catalog
            .as_mut()
            .unwrap()
            .capabilities
            .insert("context.compact".into(), compact_capability);
        config.initial_items = (0..24)
            .map(|index| AgentItem::Message {
                id: ItemId::from_string(format!("history-{index}")),
                role: if index % 2 == 0 {
                    AgentRole::User
                } else {
                    AgentRole::Assistant
                },
                content: format!("Original constraint {index}: {}", "x".repeat(4_000)),
            })
            .collect();
        let begun = begin_assistant_turn(
            &mut store,
            BeginAssistantTurn {
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_id: "compaction-conversation".into(),
                conversation_title: "Compaction".into(),
                user_message_id: "current-user".into(),
                user_part_id: "current-user:body".into(),
                user_text: "Keep this original current round".into(),
                user_content_json: None,
                assistant_message_id: "submission:assistant".into(),
                context_checkpoint_id: None,
                core_config: config,
                submission: AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(
                        ItemId::from_string("current-user".into()),
                        "Keep this original current round",
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
        let run = begun.begin.run_id;
        let digest = begun.invocation_config_digest;
        let mut model = load_committed_host_requests(&mut store, &run)
            .unwrap()
            .requests
            .into_iter()
            .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
            .unwrap();
        support::admit_model_sampling(&mut store, &run, &digest, &mut model, 1500);
        drop(store);
        let mut runtime = support::open_runtime(&database).unwrap();
        runtime
            .execute(response_command(
                &model,
                &run,
                &digest,
                HostResponse::Failed {
                    error: HostError {
                        code: "context_length_exceeded".into(),
                        message: "Provider context is full".into(),
                        retryable: false,
                        http_status: None,
                    },
                },
            ))
            .unwrap();
        drop(runtime);
        let mut store = NolaStore::open(&database).unwrap();
        let pending = load_committed_host_requests(&mut store, &run)
            .unwrap()
            .requests;
        let mut compact = pending
            .iter()
            .find(|r| matches!(r.payload, HostRequest::Compact { .. }))
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "Compact Host effect missing; pending count={}, configured target=2000",
                    pending.len()
                )
            });
        support::admit_model_sampling(&mut store, &run, &digest, &mut compact, 2000);
        let HostRequest::Compact { target_tokens, .. } = &compact.payload else {
            unreachable!()
        };
        let target_tokens = usize::try_from(*target_tokens).unwrap();
        drop(store);
        let db = rusqlite::Connection::open(&database).unwrap();
        let mut runtime = support::open_runtime(&database).unwrap();
        let original_checkpoint = current_checkpoint(&mut runtime);
        let baseline = canonical_rows(&db);
        let plan: nola_agent_engine::agent::context_window::PendingCompactionPlan = serde_json::from_value(
            original_checkpoint.pointer("/state/conversation/context_window/pending_compaction")
                .expect("committed pending plan retains the original working-set witness").clone()
        ).unwrap();
        let original_working_set: Vec<AgentItem> = serde_json::from_value(
            original_checkpoint.pointer("/state/conversation/model_context").unwrap().clone()
        ).unwrap();
        let retained = serde_json::to_value(plan.retained_items(&original_working_set)
            .expect("the original working set validates the committed immutable witness"))
            .unwrap().as_array().unwrap().clone();
        let expected_source_ids = original_checkpoint
            .pointer("/state/conversation/context_window/pending_compaction/source_item_ids")
            .and_then(Value::as_array)
            .expect("pending plan records exact canonical source identities")
            .clone();
        let expected_source_fingerprint = original_checkpoint
            .pointer("/state/conversation/context_window/pending_compaction/source_fingerprint")
            .and_then(Value::as_str)
            .expect("pending plan records source fingerprint")
            .to_string();
        let expected_original_tokens = original_checkpoint
            .pointer("/state/conversation/context_window/pending_compaction/original_token_count")
            .and_then(Value::as_u64)
            .expect("pending plan records original token estimate");
        assert!(!expected_source_ids.is_empty());
        assert_eq!(expected_source_fingerprint.len(), 64);
        assert!(expected_original_tokens > 0);
        assert!(!retained.is_empty());
        let original_transcript = original_checkpoint
            .pointer("/state/conversation/transcript")
            .and_then(Value::as_array)
            .unwrap()
            .clone();
        for (summary, usage) in [
            ("".to_string(), json!({})),
            ("too large ".repeat(target_tokens + 1), json!({})),
            (
                "valid-looking but malformed usage".into(),
                json!({"input_tokens":"invalid"}),
            ),
        ] {
            assert!(runtime
                .execute(response_command(
                    &compact,
                    &run,
                    &digest,
                    HostResponse::CompactionCompleted { summary, usage }
                ))
                .is_err());
            assert_eq!(
                canonical_rows(&db),
                baseline,
                "Rejected summary must leave checkpoint, items and effects byte-identical"
            );
            drop(runtime);
            runtime = support::open_runtime(&database).unwrap();
        }
        let command = response_command(
            &compact,
            &run,
            &digest,
            HostResponse::CompactionCompleted {
                summary: "Preserve all original historical constraints and decisions.".into(),
                usage: json!({}),
            },
        );
        let command_json = serde_json::to_string(&command).unwrap();
        let receipt = || ExecuteReceivedCommand {
            client_id: "compaction-client".into(),
            request_id: "summary-result".into(),
            command_type: "reduce_and_accept_host_response".into(),
            command_json: command_json.clone(),
            expected_revision: None,
            received_at: "2000".into(),
        };
        db.execute_batch("CREATE TRIGGER reject_compaction_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'compaction receipt rollback'); END").unwrap();
        assert!(runtime
            .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
            .is_err());
        assert_eq!(canonical_rows(&db), baseline);
        db.execute_batch("DROP TRIGGER reject_compaction_receipt")
            .unwrap();
        let accepted = runtime
            .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
            .unwrap();
        assert!(matches!(
            accepted.outcome,
            nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        let resumed_model_json: String = db
            .query_row(
                "SELECT payload_json FROM effect_outbox JOIN effect_outbox_payloads USING(effect_id) WHERE status='pending' AND payload_json LIKE '%model_stream%' ORDER BY created_at_ms DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("compaction recovery stages one successor model request");
        let resumed_model: HostRequestEnvelope =
            serde_json::from_str(&resumed_model_json).expect("successor model request wire");
        assert_eq!(
            resumed_model.turn_id, model.turn_id,
            "overflow recovery continues the same logical Turn"
        );
        assert_ne!(
            resumed_model.effect_id, model.effect_id,
            "overflow recovery uses a fresh model effect"
        );
        assert_ne!(
            resumed_model.attempt_id, model.attempt_id,
            "overflow recovery uses a fresh model Attempt"
        );
        let committed = canonical_rows(&db);
        let committed_checkpoint = current_checkpoint(&mut runtime);
        assert_ne!(
            committed_checkpoint, original_checkpoint,
            "Successful summary changes the recovered working context, independent of baseline rotation"
        );
        assert_eq!(
            committed[1], baseline[1],
            "Compaction does not replace original canonical messages"
        );
        let compact_ack_count = || {
            db.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM effect_outbox WHERE effect_id=?1 AND status='acknowledged'",
                [&compact.effect_id.0],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(compact_ack_count(), 1);
        drop(runtime);
        assert_eq!(
            canonical_rows(&db),
            committed,
            "Dropping the hot runtime does not rewrite the committed transition"
        );
        let mut runtime = support::open_runtime(&database).unwrap();
        assert_eq!(
            current_checkpoint(&mut runtime),
            committed_checkpoint,
            "cold recovery reproduces the full committed compaction state"
        );
        let reopened = canonical_rows(&db);
        assert_eq!(
            &reopened[..3],
            &committed[..3],
            "Cold open retains exact checkpoint and canonical transcript"
        );
        assert_eq!(
            &reopened[4..],
            &committed[4..],
            "cold open preserves checkpoint frontiers and replay evidence"
        );
        assert_eq!(
            compact_ack_count(),
            0,
            "Store startup prunes the settled Compact acknowledgement"
        );
        assert_eq!(reopened[3].len() + 1, committed[3].len());
        assert!(
            reopened[3].iter().all(|row| committed[3].contains(row)),
            "Cold open only removes the settled acknowledgement"
        );
        runtime
            .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
            .unwrap();
        assert_eq!(
            canonical_rows(&db),
            reopened,
            "Cold receipt replay cannot compact twice or mutate pending effects"
        );
        let checkpoint = current_checkpoint(&mut runtime);
        assert_eq!(
            checkpoint, committed_checkpoint,
            "receipt replay preserves complete recovered state"
        );
        let working_context = checkpoint
            .pointer("/state/conversation/model_context")
            .and_then(Value::as_array)
            .unwrap();
        assert!(
            working_context.ends_with(&retained),
            "Cold receipt replay preserves the exact original kept suffix"
        );
        let transcript = checkpoint
            .pointer("/state/conversation/transcript")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(
            transcript
                .iter()
                .filter(|item| item.get("id").and_then(Value::as_str) == Some("current-user"))
                .count(),
            1,
            "cold overflow recovery never resubmits the user input"
        );
        for item in &original_transcript {
            assert!(
                transcript.contains(item),
                "Every original transcript item survives"
            );
        }
        let compacted = transcript
            .iter()
            .find(|item| item.get("type").and_then(Value::as_str) == Some("compaction"))
            .expect("canonical transcript retains the generated compaction checkpoint");
        assert_eq!(
            compacted.get("source_item_ids").and_then(Value::as_array),
            Some(&expected_source_ids),
            "summary points to the exact canonical items selected before cold recovery"
        );
        assert_eq!(
            compacted.get("source_fingerprint").and_then(Value::as_str),
            Some(expected_source_fingerprint.as_str())
        );
        assert_eq!(
            compacted
                .get("original_token_count")
                .and_then(Value::as_u64),
            Some(expected_original_tokens)
        );
        for source_id in &expected_source_ids {
            assert!(
                original_transcript
                    .iter()
                    .any(|item| item.get("id") == Some(source_id)),
                "every summary source identity locates one original transcript item"
            );
        }
        assert!(working_context.iter().any(|item| item
            .to_string()
            .contains("Keep this original current round")));
        assert!(checkpoint
            .pointer("/state/conversation/context_window/pending_compaction")
            .is_none_or(Value::is_null));
    }
}

mod compaction_lineage_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Bounded canonical compaction lineage remains available outside UI projection.
    use nola_store::NolaStore;
    use serde_json::json;

    #[test]
    fn compaction_lineage_reads_original_source_while_ui_projection_hides_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("lineage.db");
        drop(NolaStore::open(&database).unwrap());

        let source = format!("SOURCE_FACT_7319:{}", "x".repeat(40_000));
        let semantic = json!({
            "retained_item_ids": ["source-message"],
            "source_fingerprint": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        });
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute(
                "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES(?1,?2,1,1,2)",
                ("lineage", "Lineage"),
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages VALUES(?1,?2,NULL,'user','completed',1,1,2)",
                ("source-message", "lineage"),
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
                 VALUES('source-part','source-message','text',1,'completed',?1,NULL,1,2)",
                [&source],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages VALUES(?1,?2,NULL,'assistant','completed',2,1,2)",
                ("compaction-message", "lineage"),
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
                 VALUES('compaction-part','compaction-message','compaction',1,'completed','summary',?1,1,2)",
                [semantic.to_string()],
            )
            .unwrap();
        drop(connection);

        let store = NolaStore::open(&database).unwrap();
        let projection = nola_application::load_conversation_projection_v1(&store, "lineage").unwrap();
        assert!(projection
            .messages
            .iter()
            .flat_map(|message| &message.parts)
            .all(|part| !matches!(part, nola_domain::MessagePartView::Compaction { .. })));

        let lineage = nola_application::load_compaction_lineage(&store, "lineage", None).unwrap();
        assert_eq!(lineage["compaction_part_id"], "compaction-part");
        assert_eq!(lineage["source_item_ids"], json!(["source-message"]));
        assert_eq!(lineage["missing_source_item_ids"], json!([]));
        assert_eq!(lineage["sources"].as_array().unwrap().len(), 1);
        assert_eq!(lineage["source_bodies_are_previews"], true);
        let preview = lineage["sources"][0]["message"]["parts"][0]["content_text"]
            .as_str()
            .unwrap();
        assert!(preview.starts_with("SOURCE_FACT_7319:"));
        assert!(preview.len() < source.len());
        assert_eq!(
            lineage["sources"][0]["message"]["parts"][0]["content_text_complete"],
            false
        );
    }
}

mod context_checkpoint_ownership_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Canonical context ownership and rollback through the Application command boundary.
    use crate::support;
    use nola_application::{ApplicationCommand, ContextCommand, CreateConversation};
    use serde_json::{json, Value};
    use support::ApplicationRuntime;

    fn commit(
        conversation: &str,
        checkpoint: &str,
        component: &str,
        title: &str,
    ) -> ApplicationCommand {
        ApplicationCommand::Context { command: serde_json::from_value(json!({
            "type": "commit_checkpoint", "input": {
                "checkpoint": {
                    "id": checkpoint, "conversation_id": conversation, "run_id": null,
                    "model_profile_id": "fixture-model", "policy_version": "v2",
                    "covered_through_message_id": null, "covered_through_sequence": null,
                    "summary_part_id": null, "retained_tail_start_message_id": null,
                    "token_usage": {"estimated": 100, "measured": null, "window": 64000, "prompt_limit": 60000},
                    "payload_digest": "fixture", "created_at_ms": 2
                },
                "components": [{
                    "id": component, "checkpoint_id": checkpoint, "kind": "data_source",
                    "source_id": "fixture-source", "source_uri": null, "title": title, "mime_type": "text/plain",
                    "authorization": "authorized", "observed_at_ms": 1, "maximum_age_minutes": 60,
                    "freshness": "fresh", "content_reference": {"type": "inline_preview", "text": title, "truncated": false},
                    "content_digest": "fixture", "estimated_tokens": 100,
                    "provenance": {"provider_id": "fixture", "collection_method": "test", "source_revision": null, "disclosed_to_model": true}
                }]
            }
        })).unwrap() }
    }

    fn load(runtime: &mut ApplicationRuntime, id: &str) -> Value {
        runtime
            .execute(ApplicationCommand::Context {
                command: ContextCommand::LoadCheckpoint {
                    checkpoint_id: id.into(),
                },
            })
            .unwrap()
    }

    fn runtime(path: &std::path::Path) -> ApplicationRuntime {
        let mut runtime = support::open_runtime(path).unwrap();
        for id in ["a", "b"] {
            runtime
                .execute(ApplicationCommand::CreateConversation {
                    input: CreateConversation {
                        id: id.into(),
                        title: id.into(),
                        settings_json: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    },
                })
                .unwrap();
        }
        runtime
    }

    #[test]
    fn foreign_checkpoint_identity_cannot_replace_components_even_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("context.db");
        let mut runtime = runtime(&path);
        let original = runtime
            .execute(commit("a", "checkpoint-a", "component-a", "original"))
            .unwrap();
        let error = runtime
            .execute(commit("b", "checkpoint-a", "component-b", "foreign"))
            .unwrap_err();
        assert!(error.contains("another conversation"), "{error}");
        assert_eq!(load(&mut runtime, "checkpoint-a"), original);
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        assert_eq!(load(&mut runtime, "checkpoint-a"), original);
        let own = runtime
            .execute(commit("b", "checkpoint-b", "component-b", "own"))
            .unwrap();
        assert_eq!(own["checkpoint"]["conversation_id"], "b");
        assert_eq!(load(&mut runtime, "checkpoint-a"), original);
    }

    #[test]
    fn conflicting_component_rolls_back_checkpoint_and_component_deletion() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("context.db");
        let mut runtime = runtime(&path);
        let a = runtime
            .execute(commit("a", "checkpoint-a", "component-a", "original-a"))
            .unwrap();
        let b = runtime
            .execute(commit("b", "checkpoint-b", "component-b", "original-b"))
            .unwrap();
        assert!(runtime
            .execute(commit("a", "checkpoint-a", "component-b", "replacement"))
            .is_err());
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        assert_eq!(load(&mut runtime, "checkpoint-a"), a);
        assert_eq!(load(&mut runtime, "checkpoint-b"), b);
        let replacement = runtime
            .execute(commit(
                "a",
                "checkpoint-a",
                "component-a",
                "valid replacement",
            ))
            .unwrap();
        assert_eq!(replacement["components"][0]["title"], "valid replacement");
        assert_eq!(load(&mut runtime, "checkpoint-b"), b);
    }
}

mod context_usage_read_e2e {
    // SPDX-License-Identifier: Apache-2.0
    use crate::committed_usage;
    use crate::support;
    use nola_application::{execute_application_command, ApplicationCommand};
    use nola_domain::committed_run_context_usage::CommittedRunContextUsage;
    use nola_domain::run_message_admission::{RunMessageAdmissionCursor, RunMessageAdmissionPage};
    use nola_store::NolaStore;
    use serde_json::{json, Value};
    use tempfile::tempdir;
    fn query(store: &mut NolaStore) -> Result<Value, String> {
        execute_application_command(
            store,
            ApplicationCommand::LoadConversationContextUsageV1 {
                conversation_id: "usage".into(),
            },
        )
    }
    fn accept(store: &mut NolaStore, record: CommittedRunContextUsage) -> Result<Value, String> {
        execute_application_command(
            store,
            ApplicationCommand::AcceptRemoteRunContextUsage {
                source_host_id: "usage-host".into(),
                record,
            },
        )
    }

    fn admit_usage_run(source: &mut NolaStore, receiver: &mut NolaStore, receipt: nola_domain::remote_submission::SourceSubmissionReceipt) {
        execute_application_command(receiver, ApplicationCommand::AdoptObservedRemoteRun { receipt: receipt.clone() }).unwrap();
        let through = receipt.admission.unwrap();
        let mut after = RunMessageAdmissionCursor::initial(through.identity.clone()).unwrap();
        loop {
            let page: RunMessageAdmissionPage = serde_json::from_value(execute_application_command(source,
                ApplicationCommand::ReadRunMessageAdmissions { after, through: through.clone(), max_records: 2, max_encoded_bytes: 65536 }).unwrap()).unwrap();
            after = page.next.clone();
            let complete = page.complete;
            execute_application_command(receiver, ApplicationCommand::ReceiveRunMessageAdmissions { source_host_id: "usage-host".into(), page }).unwrap();
            if complete { break; }
        }
    }

    /// Exercise the same immutable terminal/admission/journal protocol as a real
    /// receiver. A usage observation must never itself finish the active Run.
    fn receive_usage_run_terminal(source: &mut NolaStore, receiver: &mut NolaStore, run: &str) {
        use nola_application::{AssistantJournalChunk, AssistantJournalReadResult, AssistantJournalSnapshotHandle, StageAssistantJournalContent};
        use nola_domain::assistant_journal::AssistantJournalCursor;
        use nola_domain::assistant_journal_operations::JournalNumber;
        use nola_domain::assistant_run_terminal::{AssistantRunTerminalManifestEntry, AssistantRunTerminalManifestPage, AssistantRunTerminalSeal};
        let seal: AssistantRunTerminalSeal = serde_json::from_value(execute_application_command(source,
            ApplicationCommand::LocalAssistantRunTerminalSeal { conversation_id: "usage".into(), run_id: run.into() }).unwrap()).unwrap();
        execute_application_command(receiver, ApplicationCommand::BeginReceiveAssistantRunTerminal {
            source_host_id: "usage-host".into(), seal: seal.clone(),
        }).unwrap();
        let mut entry = JournalNumber(0);
        loop {
            let page: AssistantRunTerminalManifestPage = serde_json::from_value(execute_application_command(source,
                ApplicationCommand::ReadLocalAssistantRunTerminalManifest { seal: seal.clone(), after_entry: entry, max_records: 2, max_encoded_bytes: 65536 }).unwrap()).unwrap();
            execute_application_command(receiver, ApplicationCommand::StageAssistantRunTerminalManifest {
                source_host_id: "usage-host".into(), page: page.clone(),
            }).unwrap();
            for item in &page.entries {
                let AssistantRunTerminalManifestEntry::Assistant { through, .. } = item else { continue };
                let mut after = AssistantJournalCursor::initial(through.identity.clone()).unwrap();
                loop {
                    let records: AssistantJournalReadResult = serde_json::from_value(execute_application_command(source,
                        ApplicationCommand::ReadAssistantJournal { after, through: through.clone(), max_records: 2, max_encoded_bytes: 65536 }).unwrap()).unwrap();
                    let AssistantJournalReadResult::Records { records, next, complete, .. } = records else { panic!("fresh fixture journal must be replayable") };
                    for record in records {
                        let at = AssistantJournalCursor { identity: record.header.identity.clone(), sequence: record.header.sequence, record_digest: record.header.record_digest.clone() };
                        let snapshot: AssistantJournalSnapshotHandle = serde_json::from_value(execute_application_command(source,
                            ApplicationCommand::OpenAssistantJournalSnapshot { at }).unwrap()).unwrap();
                        loop {
                            let need: Option<nola_store_port::assistant_journal::AssistantJournalContentNeed> = serde_json::from_value(execute_application_command(receiver,
                                ApplicationCommand::NextAssistantJournalContent { source_host_id: "usage-host".into(), record: record.clone() }).unwrap()).unwrap();
                            let Some(need) = need else { break };
                            let chunk: AssistantJournalChunk = serde_json::from_value(execute_application_command(source,
                                ApplicationCommand::ReadAssistantJournalRecordChunk { snapshot: snapshot.clone(), part_id: need.part_id.clone(), content: need.content.clone(), chunk_index: need.expected_prefix.chunk_count }).unwrap()).unwrap();
                            execute_application_command(receiver, ApplicationCommand::StageAssistantJournalContent {
                                input: StageAssistantJournalContent { source_host_id: "usage-host".into(), record: record.clone(), part_id: need.part_id,
                                    content: need.content, expected_prefix: need.expected_prefix, chunks: vec![chunk], restart: false },
                            }).unwrap();
                        }
                        execute_application_command(receiver, ApplicationCommand::ReceiveAssistantJournalRecord {
                            source_host_id: "usage-host".into(), record, chunks: vec![],
                        }).unwrap();
                        execute_application_command(source, ApplicationCommand::ReleaseAssistantJournalSnapshot { snapshot }).unwrap();
                    }
                    after = next;
                    if complete { break; }
                }
            }
            entry = page.next_entry;
            if page.complete { break; }
        }
        execute_application_command(receiver, ApplicationCommand::CommitReceivedAssistantRunTerminal {
            source_host_id: "usage-host".into(), seal,
        }).unwrap();
        assert!(receiver.active_run("usage").unwrap().is_none());
    }

    #[test]
    fn cumulative_remote_usage_is_per_run_frontier_guarded_and_replay_safe() {
        let root = tempdir().unwrap();
        let source_path = root.path().join("source.db");
        let receiver_path = root.path().join("receiver.db");
        let mut source = NolaStore::open(&source_path).unwrap();
        let mut receiver = NolaStore::open(&receiver_path).unwrap();
        let (first, receipt) = committed_usage::seed(&mut source, 1000);
        admit_usage_run(&mut source, &mut receiver, receipt);
        accept(&mut receiver, first.clone()).unwrap();
        let sql = rusqlite::Connection::open(&source_path).unwrap();
        // Two further physical attempts commit without a context_updated event.
        // One reports telemetry, one has actually started but reports none.
        for (id, usage) in [("second", json!({"cache_usage": nola_domain::PromptCacheUsage::measured(Some(3000),Some(300),Some(0),false)})),
            ("unreported", Value::Null)] {
            let seq: i64 = sql.query_row("SELECT MAX(sequence)+1 FROM run_events WHERE run_id=?1", [&first.identity.run_id], |r|r.get(0)).unwrap();
            sql.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES(?1,?2,?3,'model_attempt_changed',?4,2000)",
                rusqlite::params![format!("extra:{id}"),first.identity.run_id,seq,json!({"id":id,"state":"succeeded","started_at_ms":1000,"usage":usage}).to_string()]).unwrap();
        }
        let next = source.local_run_context_usage(&first.identity).unwrap().unwrap();
        assert_eq!(next.event_sequence, first.event_sequence);
        assert!(next.attempt_sequence > first.attempt_sequence);
        assert_eq!(next.run_cache_usage.unwrap().reported_requests, 2);
        assert_eq!(next.run_cache_usage.unwrap().unreported_requests, 1);
        for _ in 0..2 { accept(&mut receiver, next.clone()).unwrap(); }
        assert_eq!(serde_json::from_value::<CommittedRunContextUsage>(accept(&mut receiver, first.clone()).unwrap()).unwrap(), next);
        let mut fork = next.clone();
        fork.run_cache_usage.as_mut().unwrap().unreported_requests += 1;
        fork.record_digest = fork.computed_digest().unwrap();
        assert!(accept(&mut receiver, fork).is_err());
        let mut crossed = next.clone();
        crossed.event_sequence.0 += 1;
        crossed.attempt_sequence = first.attempt_sequence;
        crossed.record_digest = crossed.computed_digest().unwrap();
        assert!(accept(&mut receiver, crossed).is_err());
        assert_eq!(query(&mut receiver).unwrap(), query(&mut source).unwrap());

        assert!(receiver.active_run("usage").unwrap().is_some(), "usage alone is not terminal authority");
        receive_usage_run_terminal(&mut source, &mut receiver, &first.identity.run_id);
        let (second_run, receipt) = committed_usage::seed_another_run(&mut source, 2000);
        assert_ne!(first.identity.run_id, second_run.identity.run_id);
        admit_usage_run(&mut source, &mut receiver, receipt);
        accept(&mut receiver, second_run.clone()).unwrap();
        accept(&mut receiver, second_run).unwrap();
        // A local projection copy is not a second physical execution ledger.
        let replica_sql = rusqlite::Connection::open(&receiver_path).unwrap();
        replica_sql.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES('copy',?1,(SELECT COALESCE(MAX(sequence),0)+1 FROM run_events WHERE run_id=?1),'model_attempt_changed',?2,2000)",
            rusqlite::params![first.identity.run_id,json!({"id":"copy","state":"succeeded","usage":{"cache_usage":nola_domain::PromptCacheUsage::measured(Some(9999),Some(9999),Some(0),false)}}).to_string()]).unwrap();
        drop(receiver);
        let mut receiver = NolaStore::open(&receiver_path).unwrap();
        assert_eq!(query(&mut receiver).unwrap(), query(&mut source).unwrap());
        let totals = query(&mut receiver).unwrap()["conversation_cache_usage"].clone();
        assert_eq!(totals["input_tokens"], 6000);
        assert_eq!(totals["read_tokens"], 1800);
        assert_eq!(totals["hit_rate_bps"], 3000);
        assert_eq!(totals["reported_requests"], 3);
        assert_eq!(totals["unreported_requests"], 1);
    }
    #[test]
    fn cold_usage_reads_committed_event_without_reading_or_rewriting_large_checkpoint() {
        let root = tempdir().unwrap();
        let path = root.path().join("nola.db");
        let mut store = NolaStore::open(&path).unwrap();
        let (record, _) = committed_usage::seed(&mut store, 5432);
        assert_eq!(record.usage.used_tokens.0, 5432);
        assert!(!record.usage.estimated);
        // Corrupt execution state must not poison an independently committed control fact.
        let sql = rusqlite::Connection::open(&path).unwrap();
        let large =
            json!({"unrelated_history":"x".repeat(10_000_000),"session_id":"foreign"}).to_string();
        sql.execute(
            "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id='usage'",
            [&large],
        )
        .unwrap();
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        let usage = query(&mut store).unwrap();
        assert_eq!(usage["used_tokens"], 5432);
        assert_eq!(usage["context_window_tokens"], 65536);
        assert_eq!(usage["prompt_token_limit"], 57344);
        assert_eq!(usage["estimated"], false);
        assert_eq!(usage["cache_usage"]["hit_rate_bps"], 5000);
        assert_eq!(usage["cache_usage"]["input_tokens"], 5432);
        assert_eq!(usage["cache_usage"]["read_tokens"], 2716);
        assert!(usage.to_string().len() < 1024);
        assert_eq!(
            store
                .load_checkpoint("usage")
                .unwrap()
                .unwrap()
                .checkpoint_json,
            large
        );
        let plan:String=sql.query_row("EXPLAIN QUERY PLAN SELECT sequence,payload_json FROM run_events WHERE run_id='usage-run' AND event_kind='context_updated' ORDER BY sequence DESC LIMIT 1",[],|r|r.get(3)).unwrap();
        assert!(plan.contains("run_context_usage_by_sequence"), "{plan}");
    }
    #[test]
    fn usage_wire_requires_decimal_integers_and_exact_identity() {
        let root = tempdir().unwrap();
        let mut store = NolaStore::open(root.path().join("nola.db")).unwrap();
        let (record, _) = committed_usage::seed(&mut store, 5432);
        for bad in [
            json!(true),
            json!(false),
            json!(5432),
            json!("-1"),
            json!("5.5"),
            Value::Null,
        ] {
            let mut value = serde_json::to_value(&record).unwrap();
            value["usage"]["used_tokens"] = bad;
            assert!(serde_json::from_value::<CommittedRunContextUsage>(value).is_err());
        }
        let mut foreign = record.identity.clone();
        foreign.source_store_epoch.push_str("-foreign");
        assert!(store.local_run_context_usage(&foreign).is_err());
        let mut foreign = record.identity.clone();
        foreign.assistant_message_id = "foreign".into();
        assert!(store.local_run_context_usage(&foreign).is_err());
        let mut invalid = record.clone();
        invalid.usage.used_tokens.0 += 1;
        assert!(invalid.validate().is_err());
        let mut obsolete = record.clone();
        obsolete.protocol_version = 0;
        obsolete.record_digest = obsolete.computed_digest().unwrap();
        assert!(obsolete.validate().is_err());
        let mut missing_frontier = serde_json::to_value(record).unwrap();
        missing_frontier.as_object_mut().unwrap().remove("attempt_sequence");
        assert!(serde_json::from_value::<CommittedRunContextUsage>(missing_frontier).is_err());
    }
    #[test]
    fn usage_receiver_requires_real_admission_and_survives_cold_reopen() {
        let root = tempdir().unwrap();
        let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
        let (record, receipt) = committed_usage::seed(&mut source, 5432);
        let path = root.path().join("receiver.db");
        let mut receiver = NolaStore::open(&path).unwrap();
        assert!(accept(&mut receiver, record.clone()).is_err());
        execute_application_command(
            &mut receiver,
            ApplicationCommand::AdoptObservedRemoteRun {
                receipt: receipt.clone(),
            },
        )
        .unwrap();
        assert!(
            accept(&mut receiver, record.clone()).is_err(),
            "shell alone is not received admission"
        );
        let through = receipt.admission.unwrap();
        let mut after = RunMessageAdmissionCursor::initial(through.identity.clone()).unwrap();
        loop {
            let page: RunMessageAdmissionPage = serde_json::from_value(
                execute_application_command(
                    &mut source,
                    ApplicationCommand::ReadRunMessageAdmissions {
                        after,
                        through: through.clone(),
                        max_records: 2,
                        max_encoded_bytes: 65536,
                    },
                )
                .unwrap(),
            )
            .unwrap();
            after = page.next.clone();
            let complete = page.complete;
            execute_application_command(
                &mut receiver,
                ApplicationCommand::ReceiveRunMessageAdmissions {
                    source_host_id: "usage-host".into(),
                    page,
                },
            )
            .unwrap();
            if complete {
                break;
            }
        }
        accept(&mut receiver, record.clone()).unwrap();
        assert_eq!(query(&mut receiver).unwrap()["cache_usage"]["hit_rate_bps"], 5000);
        let mut fork = record.clone();
        fork.usage.used_tokens.0 += 1;
        fork.record_digest = fork.computed_digest().unwrap();
        assert!(accept(&mut receiver, fork).is_err());
        let mut stale = record.clone();
        stale.event_sequence.0 -= 1;
        stale.record_digest = stale.computed_digest().unwrap();
        assert_eq!(
            serde_json::from_value::<CommittedRunContextUsage>(accept(&mut receiver, stale).unwrap())
                .unwrap(),
            record
        );
        assert!(execute_application_command(
            &mut receiver,
            ApplicationCommand::AcceptRemoteRunContextUsage {
                source_host_id: "foreign-host".into(),
                record: record.clone()
            }
        )
        .is_err());
        let mut epoch = record.clone();
        epoch.identity.source_store_epoch.push_str("-foreign");
        epoch.admission.identity = epoch.identity.clone();
        epoch.record_digest = epoch.computed_digest().unwrap();
        assert!(accept(&mut receiver, epoch).is_err());
        drop(receiver);
        let mut receiver = NolaStore::open(&path).unwrap();
        assert_eq!(query(&mut receiver).unwrap(), query(&mut source).unwrap());
        let full = nola_application::load_conversation_projection_v1(&receiver, "usage").unwrap();
        assert_eq!(
            serde_json::to_value(full.context_usage).unwrap(),
            query(&mut receiver).unwrap()
        );
        let window = nola_application::load_conversation_projection_window_v1(
            &receiver, "usage", None, 2, 32768,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(window.projection.context_usage).unwrap(),
            query(&mut receiver).unwrap()
        );
        let sql = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            sql.query_row("SELECT count(*) FROM remote_run_context_usage", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }
    #[test]
    fn absent_committed_usage_is_unknown_not_summary_tokens() {
        let root = tempdir().unwrap();
        let mut store = NolaStore::open(root.path().join("nola.db")).unwrap();
        let usage = query(&mut store).unwrap();
        assert_eq!(usage["used_tokens"], 0);
        assert_eq!(usage["estimated"], true);
    }

    #[test]
    fn missing_current_run_fact_does_not_reuse_previous_run_or_checkpoint_usage() {
        let root = tempdir().unwrap();
        let path = root.path().join("nola.db");
        let mut store = NolaStore::open(&path).unwrap();
        committed_usage::seed(&mut store, 5432);
        let sql = rusqlite::Connection::open(&path).unwrap();
        // A distinct canonical Run with no committed usage must not borrow its
        // predecessor's measured tokens. This fixture adds only metadata, no event.
        sql.execute("INSERT INTO runs(id,conversation_id,kind,status,assistant_message_id,next_event_sequence,created_at_ms,updated_at_ms) VALUES('new-run','usage','conversation','running',NULL,1,2000,2000)",[]).unwrap();
        let usage = query(&mut store).unwrap();
        assert_eq!(usage["used_tokens"], 0);
        assert_eq!(usage["estimated"], true);
    }
    #[test]
    fn source_estimate_remains_explicit_until_provider_usage_is_committed() {
        let root = tempdir().unwrap();
        let database = root.path().join("nola.db");
        let mut store = NolaStore::open(&database).unwrap();
        let (record, _) = committed_usage::seed_with_usage(&mut store, None);
        assert!(record.usage.estimated);
        assert_eq!(query(&mut store).unwrap()["estimated"], true);
        drop(store);
        let mut reopened = NolaStore::open(&database).unwrap();
        assert_eq!(reopened.local_run_context_usage(&record.identity).unwrap(), Some(record.clone()));
        assert_eq!(query(&mut reopened).unwrap()["used_tokens"], record.usage.used_tokens.0);
        let sql = rusqlite::Connection::open(&database).unwrap();
        let (event_id, payload): (String,String) = sql.query_row(
            "SELECT event_id,payload_json FROM run_events WHERE event_kind='turn_started' ORDER BY sequence LIMIT 1",
            [], |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        let payload: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["session_id"], payload["core_envelope"]["session_id"]);
        assert_eq!(payload["turn_id"], payload["core_envelope"]["turn_id"]);
        assert_eq!(event_id, format!("core:{}:{}", record.identity.run_id, payload["core_envelope"]["cursor"]["sequence"].as_u64().unwrap()));
    }

    #[test]
    fn initial_context_fact_failure_rolls_back_the_entire_admission() {
        let root = tempdir().unwrap();
        let database = root.path().join("initial-fact-rollback.db");
        let mut store = NolaStore::open(&database).unwrap();
        let sql = rusqlite::Connection::open(&database).unwrap();
        sql.execute_batch("CREATE TRIGGER reject_initial_context BEFORE INSERT ON run_events WHEN NEW.event_kind='context_updated' BEGIN SELECT RAISE(ABORT,'initial fact fault'); END").unwrap();
        // The shared real-model-delivery fixture unwraps begin's Result; catching
        // that expected fixture panic lets us inspect the same failed transaction.
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            committed_usage::seed_with_usage(&mut store, None)
        }));
        assert!(failed.is_err());
        drop(store);
        let reopened = NolaStore::open(&database).unwrap();
        assert!(reopened.active_run("usage").unwrap().is_none());
        for table in ["runs", "run_events", "messages", "effect_outbox", "assistant_journal_heads", "run_invocation_payloads"] {
            let count: i64 = sql.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0)).unwrap();
            assert_eq!(count, 0, "initial fact failure leaked {table}");
        }
    }

    #[test]
    fn imported_history_without_usage_never_requires_phantom_admission() {
        use nola_application::CreateConversation;
        use nola_protocol::{
            ConversationTranscriptImportSource, ImportConversationTranscript, TranscriptMessageInput,
            TranscriptPartInput,
        };
        for persisted_run in [false, true] {
            let root = tempdir().unwrap();
            let path = root.path().join("history.db");
            let mut store = NolaStore::open(&path).unwrap();
            execute_application_command(
                &mut store,
                ApplicationCommand::CreateConversation {
                    input: CreateConversation {
                        id: "usage".into(),
                        title: "Imported".into(),
                        settings_json: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    },
                },
            )
            .unwrap();
            let sql = rusqlite::Connection::open(&path).unwrap();
            if persisted_run {
                // Fault injection: claimed sealed metadata has no source admission
                // or result seal and must not authorize the usage read below.
                sql.execute("INSERT INTO runs(id,conversation_id,kind,status,assistant_message_id,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms) VALUES('archived-run','usage','conversation','sealed',NULL,1,1,2,2)",[]).unwrap();
            }
            execute_application_command(
                &mut store,
                ApplicationCommand::ImportConversationTranscript {
                    input: ImportConversationTranscript {
                        import_id: "archive".into(),
                        source: ConversationTranscriptImportSource::ExternalArchive,
                        conversation_id: "usage".into(),
                        expected_message_count: 0,
                        imported_at_ms: 3,
                        messages: vec![TranscriptMessageInput {
                            id: "archived-root".into(),
                            run_id: Some("archived-run".into()),
                            role: "assistant".into(),
                            status: "completed".into(),
                            ordinal: 1,
                            created_at_ms: 1,
                            completed_at_ms: Some(2),
                            parts: vec![TranscriptPartInput {
                                id: "archived-text".into(),
                                kind: "text".into(),
                                ordinal: 1,
                                state: "completed".into(),
                                content_text: "Historical answer".into(),
                                content_json: None,
                                content_digest: None,
                                created_at_ms: 1,
                                updated_at_ms: 2,
                            }],
                        }],
                    },
                },
            )
            .unwrap();
            if persisted_run {
                sql.execute("UPDATE runs SET assistant_message_id='archived-root' WHERE id='archived-run'",[]).unwrap();
            }
            // Missing facts are unknown, not a synthesized local journal owner.
            assert!(store
                .local_run_context_usage(&nola_domain::assistant_journal::AssistantJournalIdentity {
                    source_store_epoch: "archive-is-not-a-source-epoch".into(),
                    conversation_id: "usage".into(),
                    run_id: "archived-run".into(),
                    assistant_message_id: "archived-root".into(),
                })
                .unwrap()
                .is_none());
            let full = nola_application::load_conversation_projection_v1(&store, "usage").unwrap();
            let page =
                nola_application::load_conversation_projection_page_v1(&store, "usage", 0, 10).unwrap();
            let window = nola_application::load_conversation_projection_window_v1(
                &store, "usage", None, 10, 32768,
            )
            .unwrap();
            for projection in [full, page, window.projection] {
                assert_eq!(projection.messages.len(), 1);
                assert_eq!(projection.messages[0].message_id.0, "archived-root");
                assert_eq!(projection.context_usage.used_tokens, 0);
                assert!(projection.context_usage.estimated);
            }
            assert!(window.assistant_journal_heads.is_empty());
            assert_eq!(
                sql.query_row(
                    "SELECT count(*) FROM run_message_admission_records",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                sql.query_row(
                    "SELECT count(*) FROM run_events WHERE event_kind='context_updated'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
    }
    #[test]
    fn receipted_usage_query_replays_without_mutating_conversation_facts() {
        let root = tempdir().unwrap();
        let database = root.path().join("nola.db");
        let mut store = NolaStore::open(&database).unwrap();
        committed_usage::seed(&mut store, 5432);
        let before = store.load_checkpoint("usage").unwrap();
        let scope = nola_store::ProjectionScopeKey {
            scope_type: "conversation".into(),
            scope_id: "usage".into(),
        };
        let before_cursor = store.projection_cursor(&scope).unwrap();
        drop(store);

        let command_json = serde_json::to_string(&ApplicationCommand::LoadConversationContextUsageV1 {
            conversation_id: "usage".into(),
        })
        .unwrap();
        let receipt = || nola_application::ExecuteReceivedCommand {
            client_id: "usage-client".into(),
            request_id: "usage-request".into(),
            command_type: "load_conversation_context_usage_v1".into(),
            command_json: command_json.clone(),
            expected_revision: None,
            received_at: "2026-09-05T00:00:00Z".into(),
        };
        let mut runtime = support::open_runtime(&database).unwrap();
        let first = runtime
            .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
            .unwrap();
        assert!(!first.replayed);
        assert!(matches!(
            first.outcome,
            nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        let replay = runtime
            .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(first.outcome, replay.outcome);
        drop(runtime);
        let store = NolaStore::open(&database).unwrap();
        assert_eq!(store.load_checkpoint("usage").unwrap(), before);
        assert_eq!(store.projection_cursor(&scope).unwrap(), before_cursor);
    }
}

mod conversation_recall_e2e {
    // SPDX-License-Identifier: Apache-2.0
    use crate::support;
    use nola_application::{
        ApplicationCommand, CreateConversation, DeleteConversationMessage, TranscriptImportResult,
    };
    use nola_protocol::ConversationRecallHit;
    use nola_protocol::{ConversationTranscriptImportSource, ImportConversationTranscript};
    use nola_protocol::{
        TranscriptMessageInput as MessageInput, TranscriptPartInput as MessagePartInput,
    };
    use support::ApplicationRuntime;

    fn import(runtime: &mut ApplicationRuntime, id: &str, text: String) -> i64 {
        runtime
            .execute(ApplicationCommand::CreateConversation {
                input: CreateConversation {
                    id: id.into(),
                    title: id.into(),
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            })
            .unwrap();
        let result = runtime
            .execute(ApplicationCommand::ImportConversationTranscript {
                input: ImportConversationTranscript {
                    import_id: format!("import-{id}"),
                    source: ConversationTranscriptImportSource::ExternalArchive,
                    conversation_id: id.into(),
                    expected_message_count: 0,
                    imported_at_ms: 2,
                    messages: vec![MessageInput {
                        id: format!("message-{id}"),
                        run_id: None,
                        role: "user".into(),
                        status: "completed".into(),
                        ordinal: 1,
                        created_at_ms: 1,
                        completed_at_ms: Some(1),
                        parts: vec![MessagePartInput {
                            id: format!("part-{id}"),
                            kind: "text".into(),
                            ordinal: 1,
                            state: "completed".into(),
                            content_text: text,
                            content_json: None,
                            content_digest: None,
                            created_at_ms: 1,
                            updated_at_ms: 1,
                        }],
                    }],
                },
            })
            .unwrap();
        serde_json::from_value::<TranscriptImportResult>(result)
            .unwrap()
            .conversation_revision
    }

    fn query(
        runtime: &mut ApplicationRuntime,
        id: &str,
        query: &str,
        limit: usize,
    ) -> Result<serde_json::Value, String> {
        runtime.execute(ApplicationCommand::SearchConversationText {
            conversation_id: id.into(),
            query: query.into(),
            limit,
        })
    }

    #[test]
    fn local_user_history_returns_exact_cross_conversation_sources_and_rejects_wrong_owners() {
        use nola_domain::canonical_history::*;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("cross-history.db");
        let mut runtime = support::open_runtime(&path).unwrap();
        import(&mut runtime, "current", "Follow up on my earlier decision".into());
        let target_revision = import(&mut runtime, "earlier", "之前约定 search current：Tokyo launch is October 14.".into());
        import(&mut runtime, "unrelated", "A different source".into());
        let input = CanonicalHistoryQuery::Search { requester_conversation_id: "current".into(),
            scope: CanonicalHistoryScope::LocalUserHistory, query: "之前约定".into(), limit: 8, after_conversation_id: None };
        let response: CanonicalHistoryResponse = serde_json::from_value(runtime.execute(
            ApplicationCommand::QueryCanonicalHistory { input: input.clone() }).unwrap()).unwrap();
        response.validate_for(&input).unwrap();
        let CanonicalHistoryResponse::Search { hits, .. } = response else { panic!("search response") };
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source.conversation_id, "earlier");
        assert_eq!(hits[0].source.message_id, "message-earlier");
        assert_eq!(hits[0].source.part_id, "part-earlier");
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        let read = |conversation: &str| CanonicalHistoryQuery::Read { requester_conversation_id: "current".into(),
            conversation_id: conversation.into(), message_id: "message-earlier".into(), part_id: "part-earlier".into(),
            expected_conversation_revision: Some(target_revision), offset_bytes: 0, limit_bytes: 1_024 };
        let response: CanonicalHistoryResponse = serde_json::from_value(runtime.execute(
            ApplicationCommand::QueryCanonicalHistory { input: read("earlier") }).unwrap()).unwrap();
        response.validate_for(&read("earlier")).unwrap();
        assert!(matches!(response, CanonicalHistoryResponse::Read { source, text, .. }
            if source.conversation_id == "earlier" && text.contains("October 14")));
        assert!(runtime.execute(ApplicationCommand::QueryCanonicalHistory { input: read("unrelated") }).is_err());
        let mut no_owner = input.clone();
        if let CanonicalHistoryQuery::Search { requester_conversation_id, .. } = &mut no_owner { *requester_conversation_id = "other-store-owner".into(); }
        assert!(runtime.execute(ApplicationCommand::QueryCanonicalHistory { input: no_owner }).is_err());
        runtime.execute(ApplicationCommand::DeleteConversationMessage { input: DeleteConversationMessage {
            operation_id: "hide-cross-source".into(), conversation_id: "earlier".into(), expected_revision: target_revision,
            message_id: "message-earlier".into(), committed_at_ms: 3 } }).unwrap();
        let response: CanonicalHistoryResponse = serde_json::from_value(runtime.execute(
            ApplicationCommand::QueryCanonicalHistory { input }).unwrap()).unwrap();
        assert!(matches!(response, CanonicalHistoryResponse::Search { hits, .. } if hits.is_empty()));
        assert!(runtime.execute(ApplicationCommand::QueryCanonicalHistory { input: read("earlier") }).is_err());
    }

    #[test]
    fn cross_history_search_reports_unsearched_conversations_instead_of_false_empty_completion() {
        use nola_domain::canonical_history::*;
        let mut runtime = support::open_runtime(":memory:").unwrap();
        for index in 0..18 {
            import(&mut runtime, &format!("history-{index:02}"), if index == 17 { "pagination-marker".into() } else { "unrelated".into() });
        }
        let query = |cursor| CanonicalHistoryQuery::Search { requester_conversation_id: "history-00".into(),
            scope: CanonicalHistoryScope::LocalUserHistory, query: "pagination-marker".into(), limit: 8, after_conversation_id: cursor };
        let first: CanonicalHistoryResponse = serde_json::from_value(runtime.execute(
            ApplicationCommand::QueryCanonicalHistory { input: query(None) }).unwrap()).unwrap();
        let CanonicalHistoryResponse::Search { hits, searched_conversation_ids, next_conversation_cursor, .. } = first else { panic!("search response") };
        assert!(hits.is_empty());
        assert_eq!(searched_conversation_ids.len(), 16);
        assert_eq!(next_conversation_cursor.as_deref(), Some("history-15"));
        let second: CanonicalHistoryResponse = serde_json::from_value(runtime.execute(
            ApplicationCommand::QueryCanonicalHistory { input: query(next_conversation_cursor) }).unwrap()).unwrap();
        assert!(matches!(second, CanonicalHistoryResponse::Search { hits, next_conversation_cursor: None, .. }
            if hits.len() == 1 && hits[0].source.conversation_id == "history-17"));
    }

    #[test]
    fn cold_application_recall_is_bounded_and_respects_conversation_deletion() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recall.db");
        let marker = "预算约束 100%_literal CODE-7319";
        let mut text = "x".repeat(10_000_000 - marker.chars().count());
        text.push_str(marker);
        let mut runtime = support::open_runtime(&path).unwrap();
        let revision = import(&mut runtime, "a", text);
        import(&mut runtime, "b", "private-other-conversation".into());
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        let started = std::time::Instant::now();
        let response = query(&mut runtime, "a", "预算约束", 20).unwrap();
        let response_size = serde_json::to_vec(&response).unwrap().len();
        eprintln!(
            "Application 10M recall: {:?}; response bytes={response_size}",
            started.elapsed()
        );
        assert!(response_size < 8_000);
        let hits: Vec<ConversationRecallHit> = serde_json::from_value(response).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "message-a");
        assert_eq!(hits[0].part_id, "part-a");
        assert_eq!(hits[0].role, "user");
        assert_eq!(hits[0].message_ordinal, 1);
        assert_eq!(
            hits[0].start_character,
            (10_000_000 - marker.chars().count() - 240) as i64
        );
        assert!(hits[0].snippet.ends_with(marker));
        assert!(query(&mut runtime, "a", "private-other-conversation", 20)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            query(&mut runtime, "a", "%_", 20)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        runtime
            .execute(ApplicationCommand::DeleteConversationMessage {
                input: DeleteConversationMessage {
                    operation_id: "hide-a".into(),
                    conversation_id: "a".into(),
                    expected_revision: revision,
                    message_id: "message-a".into(),
                    committed_at_ms: 3,
                },
            })
            .unwrap();
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        assert!(query(&mut runtime, "a", "预算约束", 20)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            query(&mut runtime, "b", "private-other-conversation", 20)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn application_recall_rejects_unbounded_requests() {
        let mut runtime = support::open_runtime(":memory:").unwrap();
        for (text, limit) in [
            (" ".to_string(), 1),
            ("x".repeat(501), 1),
            ("valid".to_string(), 0),
            ("valid".to_string(), 21),
        ] {
            assert!(query(&mut runtime, "a", &text, limit).is_err());
        }
    }
}
