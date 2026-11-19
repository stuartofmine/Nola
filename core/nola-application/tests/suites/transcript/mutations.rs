// SPDX-License-Identifier: Apache-2.0
//! Consolidated Transcript Mutation suite:
//! edited resend preparation/policy attestation/receipts, same-conversation
//! in-place rewind, canonical conversation branching (including 10MB bounded
//! rebranching), and bounded idle revert outside the tail window.

mod edited_resend_transactions {
    use crate::support;
    use nola_agent_engine::agent::{
        host::*, AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId,
        SessionId,
    };
    use nola_application::*;
    use nola_domain::edited_resend::*;
    use nola_store::NolaStore;
    use nola_store_port::edited_resend::EditedResendRepository;
    use serde_json::json;

    fn fixture_command<T>(
        store: &mut NolaStore,
        body: impl FnOnce(&mut NolaStore) -> Result<T, String>,
    ) -> Result<T, String> {
        store
            .begin_write_unit_of_work()
            .map_err(|e| e.to_string())?;
        match body(store) {
            Ok(value) => {
                store
                    .commit_write_unit_of_work()
                    .map_err(|e| e.to_string())?;
                Ok(value)
            }
            Err(error) => {
                store
                    .rollback_write_unit_of_work()
                    .map_err(|e| format!("{error}; fixture rollback: {e}"))?;
                Err(error)
            }
        }
    }
    fn prepare(
        store: &mut NolaStore,
        input: PrepareEditedResend,
    ) -> Result<PreparedEditedResend, String> {
        fixture_command(store, |store| edited_resend::prepare(store, input))
    }

    fn config(owner: &str) -> AgentCoreConfig {
        let mut config = support::goal_execution::config();
        config.session_id = Some(SessionId::from_string(owner.into()));
        config.model = Some("original-model".into());
        config.turn_identity.model_capability = nola_agent_engine::agent::ModelCapabilityProfile {
            profile_id: "original-intrinsic".into(),
            revision: 7,
            native_tool_calls: true,
            context_window_tokens: Some(64_000),
            max_output_tokens: Some(16_384),
            stream_text: true,
            ..Default::default()
        };
        config.turn_identity.model_capability_id = Some("original-intrinsic".into());
        config.turn_identity.model_capability_revision = 7;
        config.context_window.context_window_tokens = 64_000;
        config.context_window.reserved_output_tokens = 16_384;
        config.context_window.prompt_token_limit = Some(48_000);
        config.context_window.auto_compact_tokens = Some(48_000);
        config.context_window.compaction_target_tokens = Some(24_000);
        config.tools.clear();
        config.turn_identity.remote_tool_plan = None;
        config.turn_identity.host_recovery = None;
        config.host_catalog = Some(HostCapabilityCatalog {
            host_id: HostId("host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: std::collections::BTreeMap::from([(
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
                        allow_artifacts: false,
                    },
                    supports_streaming: true,
                    supports_argument_streaming: false,
                    parallelism: HostCapabilityParallelism::Serial,
                    cancellation: HostCancellationMode::Cooperative,
                    replay_safety: HostReplaySafety::Recoverable,
                },
            )]),
        });
        config
    }
    fn begin(
        store: &mut NolaStore,
        owner: &str,
        run: &str,
        text: &str,
        config: AgentCoreConfig,
    ) -> Result<AcceptedAssistantTurn, String> {
        begin_using_submission(store, owner, run, text, config, resend_submission(run, text))
    }
    fn begin_using_submission(
        store: &mut NolaStore,
        owner: &str,
        run: &str,
        text: &str,
        config: AgentCoreConfig,
        submission: AgentSubmission,
    ) -> Result<AcceptedAssistantTurn, String> {
        let frontier = support::admission_frontier::read(store, owner);
        begin_assistant_turn(
            store,
            BeginAssistantTurn {
                conversation_id: owner.into(),
                expected_configuration_revision: frontier.0,
                expected_parent_message_id: frontier.1,
                conversation_title: "Resend fixture".into(),
                user_message_id: format!("{run}:user"),
                user_part_id: format!("{run}:user:text"),
                user_text: text.into(),
                user_content_json: None,
                assistant_message_id: format!("{run}:assistant"),
                context_checkpoint_id: None,
                core_config: config,
                created_at_ms: 10,
                submission,
            },
        )
    }
    fn resend_submission(run: &str, text: &str) -> AgentSubmission {
        AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string(format!("{run}:user")),
                text,
            )],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some(run.into()),
            submitted_at_epoch_ms: Some(10),
            delivery: AgentInputDelivery::CurrentTurn,
        }
    }
    fn source(store: &mut NolaStore) {
        let accepted = begin(
            store,
            "source",
            "original-run",
            "Original question",
            config("source"),
        )
        .unwrap();
        let requests = claim_committed_host_requests(
            store,
            ClaimCommittedHostRequests {
                run_id: accepted.begin.run_id.clone(),
                worker_generation: "fixture-generation".into(),
                lease_owner: "fixture-owner".into(),
                claimed_at_ms: 11,
                lease_expires_at_ms: 1000,
                limit: 64,
            },
        )
        .unwrap();
        let request = requests
            .requests
            .iter()
            .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
            .unwrap();
        for (i, payload) in [
            HostResponse::ExecutionStarted,
            HostResponse::ModelAttemptStarted {
                provider: "fixture".into(),
                model: "original-model".into(),
                reason: "initial".into(),
            },
            HostResponse::ModelCompleted {
                items: vec![serde_json::to_value(AgentItem::assistant_message(
                    ItemId::from_string("answer".into()),
                    "Original answer",
                ))
                .unwrap()],
                tool_calls: vec![],
                usage: json!({}),
            },
        ]
        .into_iter()
        .enumerate()
        {
            reduce_and_accept_host_response(
                store,
                ReduceAndAcceptHostResponse {
                    conversation_id: "source".into(),
                    run_id: accepted.begin.run_id.clone(),
                    invocation_config_digest: accepted.invocation_config_digest.clone(),
                    received_at_ms: 12 + i as i64,
                    response: HostResponseEnvelope {
                        request_id: request.request_id.clone(),
                        attempt_id: request.attempt_id.clone(),
                        delegated_invocation_id: request.delegated_invocation_id.clone(),
                        effect_id: request.effect_id.clone(),
                        host_id: request.host_id.clone(),
                        lease_id: request.lease_id.clone(),
                        session_id: request.session_id.clone(),
                        turn_id: request.turn_id.clone(),
                        catalog_revision: request.catalog_revision,
                        availability_epoch: request.availability_epoch,
                        delivery: request.delivery.clone(),
                        sequence: i as u64 + 1,
                        payload,
                    },
                },
            )
            .unwrap();
        }
        assert!(store.active_run("source").unwrap().is_none());
    }
    fn intent(store: &NolaStore) -> PrepareEditedResend {
        let conversation = store.load_conversation("source").unwrap().unwrap();
        let projection = load_conversation_projection_v1(store, "source").unwrap();
        let message = projection
            .messages
            .iter()
            .find(|m| m.message_id.0 == "original-run:user")
            .unwrap();
        PrepareEditedResend {
            operation_id: uuid::Uuid::new_v4().to_string(),
            source_conversation_id: "source".into(),
            source_message_id: "original-run:user".into(),
            expected_source_revision: conversation.revision,
            expected_source_message_revision: message.revision,
            edited_text: "Edited question".into(),
            target_conversation_id: uuid::Uuid::new_v4().to_string(),
            title: "重发分支".into(),
            created_at_ms: 20,
        }
    }

    #[test]
    fn edited_resend_settings_wire_and_full_prepare_attest_receipt_lifecycle() {
        // 1. Generic settings cannot forge/delete send witnesses & renderer wire cannot request host material.
        let mut mem = NolaStore::open(":memory:").unwrap();
        assert!(mem.put_product_setting("edited_resend/source/forged", "{}", 1).is_err());
        assert!(mem.delete_product_setting("edited_resend/source/forged").is_err());
        assert!(mem.product_setting("edited_resend/model_credential/forged").is_err());
        assert!(mem.list_product_settings("edited_resend/", 10, 0).is_err());
        assert!(mem.list_room_submissions("edited_resend/", 10, 0).is_err());
        for kind in [
            "load_edited_resend_host_material",
            "attest_edited_resend_host_policy",
        ] {
            assert!(serde_json::from_value::<ApplicationCommand>(json!({
                "type": kind, "conversation_id": "target", "core_config": {}
            }))
            .is_err());
        }

        // 2. Single source turn exercising stale message snapshot, trigger rollback, receipted prepare & replay,
        //    fresh policy attestation, fresh admission, and missing source witness rejection.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resend.db");
        let mut store = NolaStore::open(&path).unwrap();
        source(&mut store);
        let original = store.load_conversation("source").unwrap().unwrap();

        // Stale message snapshot & failed durable prepare leave no target.
        let mut stale_input = intent(&store);
        stale_input.expected_source_message_revision += 1;
        assert!(prepare(&mut store, stale_input.clone()).is_err());
        assert!(store
            .load_conversation(&stale_input.target_conversation_id)
            .unwrap()
            .is_none());
        assert!(store
            .prepared_edited_resend_by_operation(&stale_input.operation_id)
            .unwrap()
            .is_none());

        let fault_input = intent(&store);
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch(
            "CREATE TRIGGER reject_resend BEFORE INSERT ON product_settings
             WHEN NEW.key LIKE 'edited_resend/prepared/%'
             BEGIN SELECT RAISE(ABORT,'injected prepare failure'); END;",
        )
        .unwrap();
        assert!(prepare(&mut store, fault_input.clone()).is_err());
        assert!(store
            .load_conversation(&fault_input.target_conversation_id)
            .unwrap()
            .is_none());
        sql.execute_batch("DROP TRIGGER reject_resend;").unwrap();

        // Preparation idempotency, conflict, fresh policy attestation, and fresh admission.
        let input = intent(&store);
        let prepared = prepare(&mut store, input.clone()).unwrap();
        assert_eq!(prepared, prepare(&mut store, input.clone()).unwrap());
        let public = serde_json::to_string(&prepared).unwrap();
        for secret in ["host_recovery", "model_credential_reference", "opaque-descriptor"] {
            assert!(!public.contains(secret));
        }
        let mut changed = input.clone();
        changed.edited_text = "Different transport retry".into();
        assert!(prepare(&mut store, changed)
            .unwrap_err()
            .contains("operation_conflict"));
        let target = store
            .load_conversation(&input.target_conversation_id)
            .unwrap()
            .unwrap();
        assert!(target.messages.is_empty());
        assert!(store.active_run(&target.id).unwrap().is_none());
        assert!(store.load_checkpoint(&target.id).unwrap().is_none());
        assert_eq!(prepared.admitted_run_id, None);
        assert_eq!(store.load_conversation("source").unwrap().unwrap(), original);

        let mut fresh = config(&target.id);
        let mut admission = resend_submission(&prepared.run_id, &input.edited_text);
        if let AgentSubmission::UserInput {
            turn_context_components,
            ..
        } = &mut admission
        {
            turn_context_components.push(
                serde_json::from_value(
                    json!({"kind":"system","id":"fresh-host-guidance","content":"Current trusted Host guidance"}),
                )
                .unwrap(),
            );
        }
        fresh.permission_rules.rules.push(nola_domain::PermissionRule {
            capability_pattern: "*".into(),
            resource_pattern: "*".into(),
            decision: nola_domain::PermissionPolicyDecision::Allow,
            source: nola_domain::PermissionRuleSource::ProductDefault,
        });
        fresh
            .turn_identity
            .host_invocation
            .as_mut()
            .unwrap()
            .execution_policy_sha256 = "e".repeat(64);

        let mut borrowed_grant = fresh.clone();
        borrowed_grant
            .permission_rules
            .rules
            .push(nola_domain::PermissionRule {
                capability_pattern: "read_file".into(),
                resource_pattern: "/old-owner/private/*".into(),
                decision: nola_domain::PermissionPolicyDecision::Allow,
                source: nola_domain::PermissionRuleSource::Conversation,
            });
        assert!(edited_resend::attest_fresh_policy(
            &store,
            &target.id,
            &borrowed_grant,
            &resend_submission(&prepared.run_id, &input.edited_text)
        )
        .is_err());

        let mut changed_capacity = fresh.clone();
        changed_capacity
            .turn_identity
            .model_capability
            .context_window_tokens = Some(128_000);
        assert!(edited_resend::attest_fresh_policy(
            &store,
            &target.id,
            &changed_capacity,
            &resend_submission(&prepared.run_id, &input.edited_text)
        )
        .is_err());

        let mut changed_tools = fresh.clone();
        changed_tools
            .turn_identity
            .model_capability
            .native_tool_calls = false;
        assert!(edited_resend::attest_fresh_policy(
            &store,
            &target.id,
            &changed_tools,
            &resend_submission(&prepared.run_id, &input.edited_text)
        )
        .is_err());

        let mut new_selection = fresh.clone();
        new_selection
            .turn_identity
            .model_capability
            .direct_tool_capability_ids = vec!["new-host-tool".into()];
        assert!(edited_resend::attest_fresh_policy(
            &store,
            &target.id,
            &new_selection,
            &resend_submission(&prepared.run_id, &input.edited_text)
        )
        .is_ok());

        fresh.turn_identity.fresh_resend_policy =
            Some(edited_resend::attest_fresh_policy(&store, &target.id, &fresh, &admission).unwrap());
        let mut altered = serde_json::to_value(fresh.clone()).unwrap();
        altered["turn_identity"]["host_invocation"]["generation"]["temperature"] = json!(0.5);
        assert!(begin(
            &mut store,
            &target.id,
            &prepared.run_id,
            &input.edited_text,
            serde_json::from_value(altered).unwrap()
        )
        .is_err());

        let mut swapped = admission.clone();
        if let AgentSubmission::UserInput {
            turn_context_components,
            ..
        } = &mut swapped
        {
            turn_context_components[0].content = json!("Different guidance after attestation");
        }
        assert!(begin_using_submission(
            &mut store,
            &target.id,
            &prepared.run_id,
            &input.edited_text,
            fresh.clone(),
            swapped
        )
        .is_err());

        let mut extra_context = admission.clone();
        if let AgentSubmission::UserInput {
            turn_context_items, ..
        } = &mut extra_context
        {
            turn_context_items.push(AgentItem::user_message(
                ItemId::from_string("old-hidden-context".into()),
                "Old history",
            ));
        }
        assert!(begin_using_submission(
            &mut store,
            &target.id,
            &prepared.run_id,
            &input.edited_text,
            fresh.clone(),
            extra_context
        )
        .is_err());

        begin_using_submission(
            &mut store,
            &target.id,
            &prepared.run_id,
            &input.edited_text,
            fresh,
            admission,
        )
        .unwrap();
        let admitted_target = store.load_conversation(&target.id).unwrap().unwrap();
        assert_eq!(admitted_target.messages.len(), 2);
        assert_eq!(admitted_target.messages[0].parts[0].content_text, "Edited question");
        assert!(!serde_json::to_string(&admitted_target)
            .unwrap()
            .contains("Original question"));
        assert_eq!(
            store
                .prepared_edited_resend(&target.id)
                .unwrap()
                .unwrap()
                .admitted_run_id,
            Some(prepared.run_id)
        );
        assert_eq!(store.load_conversation("source").unwrap().unwrap(), original);

        // Receipted PrepareEditedResend replay & trigger failure on the same source.
        let receipt_input = intent(&store);
        let rejected_input = intent(&store);
        drop(store);
        let command = ApplicationCommand::PrepareEditedResend {
            input: receipt_input.clone(),
        };
        let received = ExecuteReceivedCommand {
            client_id: "resend-fixture".into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            command_type: "prepare_edited_resend".into(),
            command_json: serde_json::to_string(&command).unwrap(),
            expected_revision: None,
            received_at: "2026-10-28T00:00:00Z".into(),
        };
        let mut runtime = support::open_runtime(&path).unwrap();
        let first_rcpt = runtime
            .execute_received_command(received.clone(), command)
            .unwrap();
        assert!(matches!(
            &first_rcpt.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        let replay_rcpt = runtime
            .execute_received_command(
                received.clone(),
                serde_json::from_str(&received.command_json).unwrap(),
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(first_rcpt.into_protocol_receipt()).unwrap(),
            serde_json::to_value(replay_rcpt.into_protocol_receipt()).unwrap()
        );
        let observed = runtime
            .execute(ApplicationCommand::LoadPreparedEditedResend {
                conversation_id: receipt_input.target_conversation_id.clone(),
            })
            .unwrap();
        assert_eq!(observed["admitted_run_id"], serde_json::Value::Null);

        sql.execute_batch(
            "CREATE TRIGGER receipt_prepare_failure BEFORE INSERT ON product_settings
             WHEN NEW.key LIKE 'edited_resend/prepared/%'
             BEGIN SELECT RAISE(ABORT,'receipt-path prepare failure'); END;",
        )
        .unwrap();
        let rej_cmd = ApplicationCommand::PrepareEditedResend {
            input: rejected_input.clone(),
        };
        let rej_reply = runtime
            .execute_received_command(
                ExecuteReceivedCommand {
                    client_id: "resend-fixture".into(),
                    request_id: uuid::Uuid::new_v4().to_string(),
                    command_type: "prepare_edited_resend".into(),
                    command_json: serde_json::to_string(&rej_cmd).unwrap(),
                    expected_revision: None,
                    received_at: "2026-10-28T00:00:01Z".into(),
                },
                rej_cmd,
            )
            .unwrap();
        assert!(!matches!(
            rej_reply.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        sql.execute_batch("DROP TRIGGER receipt_prepare_failure;")
            .unwrap();
        drop(runtime);

        // Missing original send witness is not rebuilt from current configuration.
        let mut store = NolaStore::open(&path).unwrap();
        assert!(store
            .load_conversation(&rejected_input.target_conversation_id)
            .unwrap()
            .is_none());
        let missing_witness_input = intent(&store);
        sql.execute(
            "DELETE FROM product_settings WHERE key LIKE 'edited_resend/source/%'",
            [],
        )
        .unwrap();
        let err = prepare(&mut store, missing_witness_input.clone()).unwrap_err();
        assert!(err.contains("source_send_snapshot_unavailable"));
        assert!(store
            .load_conversation(&missing_witness_input.target_conversation_id)
            .unwrap()
            .is_none());
    }
}

mod in_place_resend {
    use crate::support;
    use nola_agent_engine::agent::recovery::CheckpointReason;
    use nola_agent_engine::agent::{
        AgentCore, AgentCoreConfig, AgentItem, AgentRole, AgentSubmission, ItemId, SessionId,
    };
    use nola_application::{execute_application_command, ApplicationCommand};
    use nola_domain::edited_resend::{PrepareInPlaceResend, PreparedInPlaceResend};
    use nola_store::NolaStore;

    fn prepare(
        store: &mut NolaStore,
        input: &PrepareInPlaceResend,
    ) -> Result<PreparedInPlaceResend, String> {
        execute_application_command(
            store,
            ApplicationCommand::PrepareInPlaceResend {
                input: input.clone(),
            },
        )
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
    }

    #[test]
    fn in_place_resend_rewind_rollback_raw_frontier_and_opaque_metadata_contracts() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("in-place.db");
        drop(NolaStore::open(&path).unwrap());
        let db = rusqlite::Connection::open(&path).unwrap();

        // 1. Seed "history" with 5 messages; test stale revision & trigger rollback BEFORE saving checkpoint.
        db.execute(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('history','History',1,1,1)",
            [],
        )
        .unwrap();
        for (ordinal, role, text) in [
            (1, "user", "retained question"),
            (2, "assistant", "retained answer"),
            (3, "user", "abandoned question"),
            (4, "assistant", "abandoned answer"),
            (5, "user", "abandoned followup"),
        ] {
            let id = format!("m{ordinal}");
            db.execute(
                "INSERT INTO messages VALUES(?1,'history',NULL,?2,'completed',?3,1,2)",
                rusqlite::params![id, role, ordinal],
            )
            .unwrap();
            db.execute(
                "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
                 VALUES(?1,?2,'text',1,'completed',?3,1,2)",
                rusqlite::params![format!("{id}:text"), id, text],
            )
            .unwrap();
        }
        let mut store = NolaStore::open(&path).unwrap();
        let proj = nola_application::load_conversation_projection_v1(&store, "history").unwrap();
        let mut input = PrepareInPlaceResend {
            operation_id: uuid::Uuid::new_v4().to_string(),
            conversation_id: "history".into(),
            message_id: "m3".into(),
            expected_revision: 1,
            expected_message_revision: proj.messages[2].revision,
            edited_text: "edited question".into(),
            created_at_ms: 10,
        };
        let mut stale = input.clone();
        stale.expected_revision += 1;
        assert!(prepare(&mut store, &stale).is_err());

        db.execute_batch(
            "CREATE TRIGGER fail_edit BEFORE INSERT ON product_settings
             WHEN NEW.key LIKE 'edited_resend/in_place/%'
             BEGIN SELECT RAISE(ABORT,'late fault'); END",
        )
        .unwrap();
        assert!(prepare(&mut store, &input).is_err());
        assert!(store.load_checkpoint("history").unwrap().is_none());
        assert_eq!(
            store
                .load_conversation_message_window("history", None, 20, 65536)
                .unwrap()
                .unwrap()
                .total_message_count,
            5
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM message_projection_events", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        db.execute_batch("DROP TRIGGER fail_edit").unwrap();

        // 2. Save compacted checkpoint, rewind same owner, append new output, and verify cold retry idempotency.
        let mut core = AgentCore::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string("history".into())),
            initial_items: vec![AgentItem::Message {
                id: ItemId::from_string("old-summary".into()),
                role: AgentRole::Assistant,
                content: "abandoned compacted summary".into(),
            }],
            ..support::model_identity::config()
        })
        .unwrap();
        core.submit(AgentSubmission::RebaseConversation {
            items: vec![AgentItem::Message {
                id: ItemId::from_string("old-summary".into()),
                role: AgentRole::Assistant,
                content: "abandoned compacted summary".into(),
            }],
            item_ownership: Default::default(),
        })
        .unwrap();
        let ckpt = core.into_checkpoint(CheckpointReason::Manual);
        store
            .save_checkpoint(&nola_store::SaveCheckpointInput {
                conversation_id: "history".into(),
                expected_revision: 0,
                revision: 1,
                event_cursor: ckpt.last_event_cursor.sequence as i64,
                checkpoint_json: serde_json::to_string(&ckpt).unwrap(),
                updated_at_ms: 2,
            })
            .unwrap();
        input.expected_message_revision =
            nola_application::load_conversation_projection_v1(&store, "history")
                .unwrap()
                .messages
                .iter()
                .find(|m| m.message_id.0 == "m3")
                .unwrap()
                .revision;
        let result = prepare(&mut store, &input).unwrap();
        let window = store
            .load_conversation_message_window("history", None, 20, 65536)
            .unwrap()
            .unwrap();
        assert_eq!(
            window
                .conversation
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["m1", "m2"]
        );
        let rewound_ckpt = store
            .load_checkpoint("history")
            .unwrap()
            .unwrap()
            .checkpoint_json;
        assert!(rewound_ckpt.contains("retained question"));
        assert!(rewound_ckpt.contains("retained answer"));
        assert!(!rewound_ckpt.contains("abandoned"));
        assert!(!rewound_ckpt.contains("edited question"));
        drop(store);

        db.execute(
            "INSERT INTO messages VALUES('new-answer','history',NULL,'assistant','completed',6,11,12)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
             VALUES('new-answer:text','new-answer','text',1,'completed','new answer',11,12)",
            [],
        )
        .unwrap();
        let mut store = NolaStore::open(&path).unwrap();
        assert_eq!(prepare(&mut store, &input).unwrap(), result);
        let window = store
            .load_conversation_message_window("history", None, 20, 65536)
            .unwrap()
            .unwrap();
        assert_eq!(window.total_message_count, 3);
        assert_eq!(window.conversation.messages.last().unwrap().id, "new-answer");
        let mut conflict = input;
        conflict.edited_text = "different edit".into();
        assert!(prepare(&mut store, &conflict).is_err());

        // 3. Canonical input after rewind uses raw ledger frontier ("frontier-history").
        store
            .create_conversation(
                &nola_store_port::conversation_profile::CreateConversationInput {
                    id: "frontier-history".into(),
                    title: "History".into(),
                    revision: 1,
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            )
            .unwrap();
        for i in 0..3 {
            execute_application_command(
                &mut store,
                ApplicationCommand::AdmitConversationUserMessage {
                    input: nola_protocol::AdmitConversationUserMessage {
                        conversation_id: "frontier-history".into(),
                        message_id: format!("u{i}"),
                        text: format!("original-{i}"),
                        created_at_ms: i + 2,
                    },
                },
            )
            .unwrap();
        }
        let f_proj =
            nola_application::load_conversation_projection_v1(&store, "frontier-history").unwrap();
        let f_input = PrepareInPlaceResend {
            operation_id: uuid::Uuid::new_v4().to_string(),
            conversation_id: "frontier-history".into(),
            message_id: "u1".into(),
            expected_revision: f_proj.revision as i64,
            expected_message_revision: f_proj.messages[1].revision,
            edited_text: "replacement".into(),
            created_at_ms: 10,
        };
        let f_prepared = prepare(&mut store, &f_input).unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::AdmitConversationUserMessage {
                input: nola_protocol::AdmitConversationUserMessage {
                    conversation_id: "frontier-history".into(),
                    message_id: f_prepared.user_message_id.clone(),
                    text: f_input.edited_text.clone(),
                    created_at_ms: f_input.created_at_ms,
                },
            },
        )
        .unwrap();
        let visible = store
            .load_conversation_message_window("frontier-history", None, 20, 65536)
            .unwrap()
            .unwrap();
        assert_eq!(visible.total_message_count, 2);
        assert_eq!(visible.conversation.messages.last().unwrap().ordinal, 4);
        assert_eq!(prepare(&mut store, &f_input).unwrap(), f_prepared);

        // 4. Opaque message metadata does not make disclosed user text uneditable ("ios").
        store
            .create_conversation(
                &nola_store_port::conversation_profile::CreateConversationInput {
                    id: "ios".into(),
                    title: "Metadata".into(),
                    revision: 1,
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            )
            .unwrap();
        store
            .append_messages(&nola_store_port::transcript_write::AppendMessagesInput {
                conversation_id: "ios".into(),
                expected_message_count: 0,
                updated_at_ms: 2,
                messages: vec![nola_store_port::transcript_write::MessageInput {
                    id: "user".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 2,
                    completed_at_ms: Some(2),
                    parts: vec![nola_domain::MessagePartInput {
                        id: "user:text".into(),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: "original text".into(),
                        content_json: Some(r#"{"nola.canonicalMessageID":"user"}"#.into()),
                        content_digest: None,
                        created_at_ms: 2,
                        updated_at_ms: 2,
                    }],
                }],
            })
            .unwrap();
        let ios_proj = nola_application::load_conversation_projection_v1(&store, "ios").unwrap();
        assert!(matches!(
            &ios_proj.messages[0].parts[0],
            nola_domain::MessagePartView::Historical { .. }
        ));
        let ios_input = PrepareInPlaceResend {
            operation_id: uuid::Uuid::new_v4().to_string(),
            conversation_id: "ios".into(),
            message_id: "user".into(),
            expected_revision: ios_proj.revision as i64,
            expected_message_revision: ios_proj.messages[0].revision,
            edited_text: "replacement".into(),
            created_at_ms: 3,
        };
        prepare(&mut store, &ios_input).unwrap();
        assert_eq!(
            store
                .load_conversation_message_window("ios", None, 10, 65536)
                .unwrap()
                .unwrap()
                .total_message_count,
            0
        );
    }
}

mod branch_transactions {
    use crate::support;
    use nola_agent_engine::agent::{AgentCoreConfig, AgentItem, AgentSubmission, ItemId, SessionId};
    use nola_application::{
        begin_assistant_turn, create_conversation_branch, load_conversation_projection_v1,
        BeginAssistantTurn, CreateConversationBranch,
    };
    use nola_domain::{CanonicalBranchTarget, MessagePartView, MessageRole};
    use nola_store::{
        AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput, NolaStore,
    };
    use nola_store_port::canonical_branch::{CanonicalBranchRepository, CanonicalBranchResult};
    use tempfile::tempdir;

    fn message(id: &str, role: &str, ordinal: i64, text: &str) -> MessageInput {
        MessageInput {
            id: id.into(),
            run_id: None,
            role: role.into(),
            status: "completed".into(),
            ordinal,
            created_at_ms: ordinal,
            completed_at_ms: Some(ordinal),
            parts: vec![MessagePartInput {
                id: format!("{id}:body"),
                kind: "text".into(),
                ordinal: 1,
                state: "completed".into(),
                content_text: text.into(),
                content_json: None,
                content_digest: None,
                created_at_ms: ordinal,
                updated_at_ms: ordinal,
            }],
        }
    }
    fn intent(
        source: &str,
        target: &str,
        boundary: &str,
        revision: i64,
    ) -> CreateConversationBranch {
        CreateConversationBranch {
            operation_id: format!("branch-{target}"),
            source_conversation_id: source.into(),
            expected_source_revision: revision,
            through_message_id: boundary.into(),
            conversation: CanonicalBranchTarget {
                id: target.into(),
                title: "Branch".into(),
                settings_json: Some(r#"{"owner":"test"}"#.into()),
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        }
    }
    fn seed(store: &mut NolaStore, text: &str) {
        store
            .create_conversation(&CreateConversationInput {
                id: "source".into(),
                title: "Source".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: "source".into(),
                expected_message_count: 0,
                messages: vec![
                    message("question", "user", 1, text),
                    message("answer", "assistant", 2, "answer"),
                ],
                updated_at_ms: 2,
            })
            .unwrap();
    }
    fn config(conversation: &str) -> AgentCoreConfig {
        use nola_agent_engine::agent::host::*;
        let descriptor = HostCapabilityDescriptor {
            id: "model.stream".into(),
            title: "Model".into(),
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
        AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string(conversation.into())),
            model: Some("test-model".into()),
            host_catalog: Some(HostCapabilityCatalog {
                host_id: HostId("test-host".into()),
                revision: 1,
                availability_epoch: 1,
                lease: None,
                capabilities: std::collections::BTreeMap::from([(
                    "model.stream".into(),
                    descriptor,
                )]),
            }),
            ..Default::default()
        }
    }
    fn begin_input(store: &NolaStore, conversation: &str) -> BeginAssistantTurn {
        let frontier = support::admission_frontier::read(store, conversation);
        BeginAssistantTurn {
            expected_configuration_revision: frontier.0,
            expected_parent_message_id: frontier.1,
            conversation_id: conversation.into(),
            conversation_title: "Branch".into(),
            user_message_id: "next-user".into(),
            user_part_id: "next-user:body".into(),
            user_text: "continue".into(),
            user_content_json: None,
            assistant_message_id: "next-assistant".into(),
            context_checkpoint_id: None,
            core_config: config(conversation),
            submission: AgentSubmission::UserInput {
                delivery: nola_agent_engine::agent::AgentInputDelivery::CurrentTurn,
                items: vec![AgentItem::user_message(
                    ItemId::from_string("next-user".into()),
                    "continue",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("next-turn".into()),
                submitted_at_epoch_ms: Some(4),
            },
            created_at_ms: 4,
        }
    }
    fn begin(
        store: &mut NolaStore,
        conversation: &str,
    ) -> nola_application::AcceptedAssistantTurn {
        let command = begin_input(store, conversation);
        begin_assistant_turn(store, command).unwrap()
    }

    #[test]
    fn empty_context_after_rewind_excludes_hidden_originals_and_rejects_overlapping_recovery() {
        let directory = tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("rewind.db")).unwrap();
        let hidden = "ORIGINAL_REMOVED_BEFORE_FIRST_MESSAGE";
        seed(&mut store, hidden);
        nola_application::revert_conversation(&mut store, nola_application::RevertConversation {
            conversation_id: "source".into(), operation_id: "rewind-to-start".into(),
            through_message_id: "question".into(), keep_message: false,
            committed_at_ms: 3, core_config: config("source"),
        }).unwrap();
        let accepted = begin(&mut store, "source");
        let effects = nola_application::load_committed_host_requests(&store, &accepted.begin.run_id).unwrap();
        let request = serde_json::to_string(&effects).unwrap();
        assert!(request.contains("continue"));
        assert!(!request.contains(hidden));
        let before = store.load_checkpoint("source").unwrap().unwrap();
        let mut overlap = begin_input(&store, "source");
        overlap.user_message_id = "overlap-user".into();
        overlap.assistant_message_id = "overlap-assistant".into();
        overlap.user_part_id = "overlap-part".into();
        if let AgentSubmission::UserInput { submission_id, .. } = &mut overlap.submission {
            *submission_id = Some("overlap-turn".into());
        }
        let error = begin_assistant_turn(&mut store, overlap).unwrap_err();
        assert!(error.contains("active Run"), "{error}");
        assert_eq!(before.checkpoint_json, store.load_checkpoint("source").unwrap().unwrap().checkpoint_json);
        assert_eq!(store.active_run("source").unwrap().unwrap().run_id, accepted.begin.run_id);
    }

    #[test]
    fn completed_history_can_be_copied_during_compaction_but_not_an_active_reply() {
        let directory = tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("branch.db")).unwrap();
        seed(&mut store, "Keep this history");
        let mut runtime = config("source");
        let catalog = runtime.host_catalog.as_mut().unwrap();
        let mut compact = catalog.capabilities["model.stream"].clone();
        compact.id = "context.compact".into();
        compact.route = nola_agent_engine::agent::host::HostCapabilityRoute::Compaction;
        catalog.capabilities.insert(compact.id.clone(), compact);
        runtime.initial_items = (0..16).map(|index| {
            let text = format!("Turn {index} {}", "archived evidence ".repeat(200));
            let id = ItemId::from_string(format!("history-{index}"));
            if index % 2 == 0 { AgentItem::user_message(id, text) }
            else { AgentItem::assistant_message(id, text) }
        }).collect();
        let compaction = nola_application::begin_maintenance_turn(&mut store,
            nola_application::BeginMaintenanceTurn {
                conversation_id: "source".into(), context_checkpoint_id: None,
                core_config: runtime, created_at_ms: 3,
                submission: AgentSubmission::Compact { operation_id: "compact".into() },
            }).unwrap();
        let revision = store.load_conversation("source").unwrap().unwrap().revision;
        let copy = create_conversation_branch(&mut store.conversation_write_repository(),
            intent("source", "copied", "answer", revision)).unwrap();
        assert_eq!(copy.message_count, 2);
        assert_eq!(store.active_run("source").unwrap().unwrap().run_id, compaction.run_id);
        assert!(store.active_run("copied").unwrap().is_none());
        begin(&mut store, "copied");
        let revision = store.load_conversation("copied").unwrap().unwrap().revision;
        let error = create_conversation_branch(&mut store.conversation_write_repository(),
            intent("copied", "denied", "next-user", revision)).unwrap_err();
        assert!(error.contains("cannot branch a conversation with an active Run"), "{error}");
    }

    #[test]
    fn branch_port_preconditions_lineage_and_first_turn_context_after_restart() {
        // 1. Non-SQLite CanonicalBranchRepository contract.
        #[derive(Default)]
        struct MockBranchRepo {
            seen: Option<CreateConversationBranch>,
            fail: bool,
        }
        impl CanonicalBranchRepository for MockBranchRepo {
            type Error = String;
            fn create_conversation_branch(
                &mut self,
                intent: &CreateConversationBranch,
            ) -> Result<CanonicalBranchResult, String> {
                self.seen = Some(intent.clone());
                if self.fail {
                    return Err("branch dependency publication failed".into());
                }
                Ok(CanonicalBranchResult {
                    created: true,
                    message_count: 2,
                    conversation_revision: 1,
                    conversation_id: intent.conversation.id.clone(),
                    source_conversation_id: intent.source_conversation_id.clone(),
                    through_message_id: intent.through_message_id.clone(),
                })
            }
        }
        let mock_intent = intent("source", "target", "boundary", 7);
        let mut repo = MockBranchRepo::default();
        assert_eq!(
            create_conversation_branch(&mut repo, mock_intent.clone())
                .unwrap()
                .conversation_id,
            "target"
        );
        assert_eq!(repo.seen, Some(mock_intent.clone()));
        repo.fail = true;
        assert!(create_conversation_branch(&mut repo, mock_intent)
            .unwrap_err()
            .contains("dependency publication failed"));

        let mut wire = serde_json::to_value(intent("source", "bad", "question", 2)).unwrap();
        wire["messages"] = serde_json::json!([]);
        assert!(serde_json::from_value::<CreateConversationBranch>(wire).is_err());

        // 2. Preconditions (stale revision, foreign boundary, existing target) + lineage + first turn.
        let dir = tempdir().unwrap();
        let path = dir.path().join("branch.db");
        let mut store = NolaStore::open(&path).unwrap();
        seed(&mut store, "question evidence");
        for bad in [
            intent("source", "bad", "question", 1),
            intent("source", "bad", "foreign", 2),
            intent("source", "source", "question", 2),
        ] {
            assert!(
                create_conversation_branch(&mut store.conversation_write_repository(), bad)
                    .is_err()
            );
            assert!(store.load_conversation("bad").unwrap().is_none());
        }

        let input = intent("source", "target", "answer", 2);
        let result =
            create_conversation_branch(&mut store.conversation_write_repository(), input.clone())
                .unwrap();
        assert_eq!((result.message_count, result.created), (2, true));
        assert!(store.load_checkpoint("target").unwrap().is_none());
        assert!(
            !create_conversation_branch(&mut store.conversation_write_repository(), input.clone())
                .unwrap()
                .created
        );
        let mut changed = input;
        changed.conversation.title = "changed".into();
        assert!(
            create_conversation_branch(&mut store.conversation_write_repository(), changed)
                .is_err()
        );
        drop(store);

        let mut store = NolaStore::open(&path).unwrap();
        let projection = load_conversation_projection_v1(&store, "target").unwrap();
        assert_eq!(projection.messages.len(), 2);
        assert_eq!(projection.messages[0].role, MessageRole::User);
        assert_eq!(projection.messages[1].role, MessageRole::Assistant);
        assert!(
            matches!(projection.messages[0].parts.as_slice(), [MessagePartView::Text { markdown, .. }] if markdown == "question evidence")
        );
        let connection = rusqlite::Connection::open(&path).unwrap();
        let lineage: String = connection
            .query_row(
                "SELECT intent_json FROM conversation_branch_operations WHERE operation_id='branch-target'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let lineage: CreateConversationBranch = serde_json::from_str(&lineage).unwrap();
        assert_eq!(
            (
                lineage.source_conversation_id.as_str(),
                lineage.through_message_id.as_str()
            ),
            ("source", "answer")
        );
        // The real Web/CLI Host commits its selected configuration before
        // admission, creating an idle checkpoint in the freshly copied chat.
        nola_application::reduce_and_commit_idle_submission(&mut store,
            nola_application::ReduceAndCommitIdleSubmission {
                conversation_id: "target".into(), core_config: config("target"), committed_at_ms: 4,
                submission: AgentSubmission::UpdateSessionConfiguration {
                    patch: nola_agent_engine::agent::AgentSessionConfigurationPatch {
                        max_model_iterations: Some(12), ..Default::default()
                    },
                },
            }).unwrap();
        let begun = begin(&mut store, "target");
        let checkpoint = store.load_checkpoint("target").unwrap().unwrap().checkpoint_json;
        assert!(checkpoint.contains("question evidence") && checkpoint.contains("answer"));
        let requests = serde_json::to_string(
            &nola_application::load_committed_host_requests(&store, &begun.begin.run_id).unwrap(),
        )
        .unwrap();
        assert!(requests.contains("question evidence") && requests.contains("answer"));
        assert_eq!(
            store
                .load_conversation("target")
                .unwrap()
                .unwrap()
                .messages
                .len(),
            4
        );
    }

    #[test]
    fn ten_million_byte_branch_and_rebranch_use_small_intents_and_bounded_first_turn_context() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("large-branch.db");
        let mut store = NolaStore::open(&path).unwrap();
        let original = "x".repeat(9_999_991) + "TAIL-DONE";
        seed(&mut store, &original);
        let input = intent("source", "large", "question", 2);
        assert!(serde_json::to_vec(&input).unwrap().len() < 1024);
        drop(store);
        let mut runtime = support::open_runtime(&path).unwrap();
        let command = nola_application::ApplicationCommand::CreateConversationBranch { input };
        let received = nola_application::ExecuteReceivedCommand {
            client_id: "branch-client".into(),
            request_id: "large-branch".into(),
            command_type: "create_conversation_branch".into(),
            command_json: serde_json::to_string(&command).unwrap(),
            expected_revision: None,
            received_at: "2026-09-08T00:00:00Z".into(),
        };
        let receipt = runtime
            .execute_received_command(
                received.clone(),
                serde_json::from_str(&received.command_json).unwrap(),
            )
            .unwrap();
        assert!(matches!(
            &receipt.outcome,
            nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        assert!(serde_json::to_vec(&receipt.into_protocol_receipt()).unwrap().len() < 2048);
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        assert!(runtime
            .execute_received_command(received, command)
            .unwrap()
            .replayed);
        drop(runtime);
        let connection = rusqlite::Connection::open(&path).unwrap();
        let event_bytes: i64 = connection
            .query_row(
                "SELECT MAX(length(CAST(payload_json AS BLOB))) FROM projection_events",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(event_bytes < 4096);

        let mut store = NolaStore::open(&path).unwrap();
        let window = store
            .load_conversation_message_window("large", None, 10, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(window.deferred_text_parts[0].total_utf8_bytes, 10_000_000);
        let msg = &window.conversation.messages[0];
        let part = &msg.parts[0];
        assert_eq!(
            store
                .read_conversation_text_range("large", &msg.id, &part.id, 9_999_991, 32)
                .unwrap()
                .text,
            "TAIL-DONE"
        );
        create_conversation_branch(
            &mut store.conversation_write_repository(),
            intent("large", "rebranch", &msg.id, window.conversation.revision),
        )
        .unwrap();
        let again = store
            .load_conversation_message_window("rebranch", None, 10, 512)
            .unwrap()
            .unwrap();
        assert_ne!(again.conversation.messages[0].id, msg.id);
        assert_ne!(again.conversation.messages[0].parts[0].id, part.id);
        assert_eq!(
            store.load_conversation("rebranch").unwrap().unwrap().messages[0].parts[0]
                .content_text,
            original
        );
        let begun = begin(&mut store, "rebranch");
        let encoded = serde_json::to_string(&begun).unwrap();
        assert!(encoded.len() < 512 * 1024);
        let reply: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert!(reply.get("checkpoint").is_none() && reply.get("snapshot").is_none());
        assert!(!encoded.contains("bounded context excerpt"));
        assert!(begun.execution.configuration.initial_items.is_empty());
        assert!(begun.execution.control.context_usage.used_tokens > 0);
        let requests = serde_json::to_string(
            &nola_application::load_committed_host_requests(&store, &begun.begin.run_id).unwrap(),
        )
        .unwrap();
        assert!(requests.len() < 256 * 1024);
        assert!(requests.contains("bounded context excerpt") && requests.contains("10000000"));
    }
}

mod revert_bounded_receipt {
    use crate::support;
    use nola_agent_engine::agent::{AgentCoreConfig, SessionId};
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
        RevertConversation,
    };
    use nola_protocol::CommandMetadata;
    use nola_store::NolaStore;
    use serde_json::Value;

    fn command(keep_message: bool) -> ApplicationCommand {
        ApplicationCommand::RevertConversation {
            input: RevertConversation {
                conversation_id: "history".into(),
                operation_id: "revert-old-boundary".into(),
                through_message_id: "m40".into(),
                keep_message,
                committed_at_ms: 100,
                core_config: AgentCoreConfig {
                    turn_identity: support::model_identity::identity(),
                    session_id: Some(SessionId::from_string("history".into())),
                    ..Default::default()
                },
            },
        }
    }
    fn received(id: &str, command: &ApplicationCommand) -> ExecuteReceivedCommand {
        ExecuteReceivedCommand {
            client_id: "bounded-revert".into(),
            request_id: id.into(),
            command_type: command.wire_command_name().into(),
            command_json: serde_json::to_string(command).unwrap(),
            expected_revision: None,
            received_at: "2026-09-08T00:00:00Z".into(),
        }
    }
    fn count(db: &rusqlite::Connection, table: &str) -> i64 {
        db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn cold_idle_revert_outside_tail_window_keeps_originals_and_receipt_transaction_atomic() {
        for keep in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("history.db");
            drop(NolaStore::open(&path).unwrap());
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute(
                "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('history','History',1,1,1)",
                [],
            )
            .unwrap();
            for ordinal in 1..=80 {
                let id = format!("m{ordinal}");
                db.execute(
                    "INSERT INTO messages VALUES(?1,'history',NULL,'user','completed',?2,1,2)",
                    rusqlite::params![id, ordinal],
                )
                .unwrap();
                db.execute(
                    "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
                     VALUES(?1,?2,'text',1,'completed',?3,1,2)",
                    rusqlite::params![
                        format!("p{ordinal}"),
                        id,
                        if ordinal == 1 {
                            "x".repeat(10_000_000)
                        } else {
                            format!("evidence-{ordinal}")
                        }
                    ],
                )
                .unwrap();
            }
            let store = NolaStore::open(&path).unwrap();
            let tail = store
                .load_conversation_message_window("history", None, 32, 32768)
                .unwrap()
                .unwrap();
            assert!(tail.conversation.messages.iter().all(|m| m.id != "m40"));
            assert!(store.load_checkpoint("history").unwrap().is_none());
            drop(store);

            let mut runtime = support::open_runtime(&path).unwrap();
            let before_outbox = count(&db, "sync_outbox");
            let before_events = count(&db, "projection_events");
            db.execute_batch(
                "CREATE TRIGGER fail_revert_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed'
                 BEGIN SELECT RAISE(ABORT,'receipt fault'); END",
            )
            .unwrap();
            let input = command(keep);
            if let Ok(receipt) =
                runtime.execute_received_command(received("failed-revert", &input), input)
            {
                assert!(matches!(
                    receipt.outcome,
                    ApplicationCommandReceiptOutcome::Rejected { .. }
                ));
            }
            assert_eq!(count(&db, "message_projection_events"), 0);
            assert_eq!(count(&db, "sync_outbox"), before_outbox);
            assert_eq!(count(&db, "projection_events"), before_events);
            assert_eq!(count(&db, "core_checkpoints"), 0);
            db.execute_batch("DROP TRIGGER fail_revert_receipt")
                .unwrap();
            drop(runtime);

            let mut runtime = support::open_runtime(&path).unwrap();
            let input = command(keep);
            let admission = received("successful-revert", &input);
            let receipt = runtime
                .execute_received_command(admission.clone(), input)
                .unwrap();
            let result: Value = match &receipt.outcome {
                ApplicationCommandReceiptOutcome::Committed { result, .. } => result.clone(),
                other => panic!("{other:?}"),
            };
            assert!(result["execution"]["control"]["history_item_count"].as_u64().unwrap() > 20);
            assert!(result.get("checkpoint").is_none() && result.get("snapshot").is_none());
            let store = NolaStore::open(&path).unwrap();
            let ckpt = store.load_checkpoint("history").unwrap().unwrap().checkpoint_json;
            assert!(ckpt.len() < 100_000);
            assert!(!ckpt.contains("evidence-41"));
            assert_eq!(ckpt.contains("evidence-40"), keep);
            let window = store
                .load_conversation_message_window("history", None, 80, 1024)
                .unwrap()
                .unwrap();
            assert_eq!(window.total_message_count, if keep { 40 } else { 39 });
            assert_eq!(
                window.conversation.messages.last().unwrap().id,
                if keep { "m40" } else { "m39" }
            );
            assert_eq!(
                db.query_row(
                    "SELECT length(CAST(content_text AS BLOB)) FROM message_parts WHERE id='p1'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                10_000_000
            );
            let after_events = count(&db, "projection_events");
            let after_hides = count(&db, "message_projection_events");
            let after_outbox = count(&db, "sync_outbox");
            let expected = receipt.into_protocol_receipt();
            drop(store);
            drop(runtime);

            let mut runtime = support::open_runtime(&path).unwrap();
            let replay = runtime
                .execute_received_command(admission, command(keep))
                .unwrap();
            assert!(replay.replayed);
            assert_eq!(replay.into_protocol_receipt(), expected);
            assert_eq!(count(&db, "projection_events"), after_events);
            assert_eq!(count(&db, "message_projection_events"), after_hides);
            assert_eq!(count(&db, "sync_outbox"), after_outbox);

            let mut hidden = match command(keep) {
                ApplicationCommand::RevertConversation { input } => input,
                _ => unreachable!(),
            };
            hidden.operation_id = "hidden-boundary".into();
            hidden.through_message_id = "m70".into();
            let rejected_cmd = ApplicationCommand::RevertConversation { input: hidden };
            let rejected = runtime
                .execute_received_command(
                    received("hidden-boundary", &rejected_cmd),
                    rejected_cmd,
                )
                .unwrap();
            assert!(matches!(
                rejected.outcome,
                ApplicationCommandReceiptOutcome::Rejected { .. }
            ));

            let mut second = match command(true) {
                ApplicationCommand::RevertConversation { input } => input,
                _ => unreachable!(),
            };
            second.operation_id = "checkpoint-revert".into();
            second.through_message_id = "m20".into();
            let second_cmd = ApplicationCommand::RevertConversation { input: second };
            let second_rcpt = runtime
                .execute_received_command(
                    received("checkpoint-revert", &second_cmd),
                    second_cmd,
                )
                .unwrap();
            assert!(matches!(
                second_rcpt.outcome,
                ApplicationCommandReceiptOutcome::Committed { .. }
            ));
            let store = NolaStore::open(&path).unwrap();
            assert_eq!(
                store
                    .load_conversation_message_window("history", None, 10, 1024)
                    .unwrap()
                    .unwrap()
                    .total_message_count,
                20
            );
        }
    }
}
