// SPDX-License-Identifier: Apache-2.0
//! Isolated real Store/reducer transactions; no provider/network/UI claims.
mod support;
use nola_agent_engine::agent::{
    host::*, AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId,
    SessionId,
};
use nola_application::*;
use nola_domain::edited_resend::*;
use nola_store::NolaStore;
use nola_store_port::edited_resend::EditedResendRepository;
use serde_json::json;

// Application helpers execute inside the runtime's outer receipt transaction.
// Direct fixture calls preserve that production transaction contract.
fn fixture_command<T>(store: &mut NolaStore, body: impl FnOnce(&mut NolaStore) -> Result<T, String>) -> Result<T, String> {
    store.begin_write_unit_of_work().map_err(|e| e.to_string())?;
    match body(store) {
        Ok(value) => { store.commit_write_unit_of_work().map_err(|e| e.to_string())?; Ok(value) }
        Err(error) => { store.rollback_write_unit_of_work().map_err(|e| format!("{error}; fixture rollback: {e}"))?; Err(error) }
    }
}
fn prepare(store: &mut NolaStore, input: PrepareEditedResend) -> Result<PreparedEditedResend, String> {
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
    begin_using_submission(
        store,
        owner,
        run,
        text,
        config,
        resend_submission(run, text),
    )
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
        .unwrap_or_else(|| panic!("source did not emit model request: requests={:?}; events={:?}", requests.requests, accepted.events));
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
fn preparation_is_empty_and_idempotent_then_fresh_admission_consumes_exact_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resend.db");
    let mut store = NolaStore::open(&path).unwrap();
    source(&mut store);
    let original = store.load_conversation("source").unwrap().unwrap();
    let input = intent(&store);
    let prepared = prepare(&mut store, input.clone()).unwrap();
    assert_eq!(
        prepared,
        prepare(&mut store, input.clone()).unwrap()
    );
    let public = serde_json::to_string(&prepared).unwrap();
    assert!(!public.contains("host_recovery"));
    assert!(!public.contains("model_credential_reference"));
    assert!(!public.contains("opaque-descriptor"));
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
    assert_eq!(
        store.load_conversation("source").unwrap().unwrap(),
        original
    );
    // A valid but different model sampling configuration cannot consume this intent.
    let mut fresh = config(&target.id);
    let mut admission = resend_submission(&prepared.run_id, &input.edited_text);
    if let AgentSubmission::UserInput {
        turn_context_components,
        ..
    } = &mut admission
    {
        turn_context_components.push(serde_json::from_value(json!({"kind":"system","id":"fresh-host-guidance","content":"Current trusted Host guidance"})).unwrap());
    }

    // Product policy is rebuilt, not erased; the new Host policy hash is
    // independent of the original invocation's policy.
    fresh
        .permission_rules
        .rules
        .push(nola_domain::PermissionRule {
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
    // A new catalog may choose its own direct tools; those are not source model facts.
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
    assert_eq!(
        prepared.source.model.model_capability["direct_tool_capability_ids"],
        json!([])
    );
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
    assert!(store
        .load_conversation(&target.id)
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
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
    assert!(store
        .load_conversation(&target.id)
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
    begin_using_submission(
        &mut store,
        &target.id,
        &prepared.run_id,
        &input.edited_text,
        fresh,
        admission,
    )
    .unwrap();
    let target = store.load_conversation(&target.id).unwrap().unwrap();
    assert_eq!(target.messages.len(), 2);
    assert_eq!(target.messages[0].parts[0].content_text, "Edited question");
    assert!(!serde_json::to_string(&target)
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
    assert_eq!(
        store.load_conversation("source").unwrap().unwrap(),
        original
    );
}

#[test]
fn stale_message_snapshot_and_failed_durable_prepare_leave_no_target() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reject.db");
    let mut store = NolaStore::open(&path).unwrap();
    source(&mut store);
    let mut input = intent(&store);
    input.expected_source_message_revision += 1;
    assert!(prepare(&mut store, input.clone()).is_err());
    assert!(store
        .load_conversation(&input.target_conversation_id)
        .unwrap()
        .is_none());
    assert!(store
        .prepared_edited_resend_by_operation(&input.operation_id)
        .unwrap()
        .is_none());
    let input = intent(&store);
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TRIGGER reject_resend BEFORE INSERT ON product_settings WHEN NEW.key LIKE 'edited_resend/prepared/%' BEGIN SELECT RAISE(ABORT,'injected prepare failure'); END;").unwrap();
    assert!(prepare(&mut store, input.clone()).is_err());
    assert!(store
        .load_conversation(&input.target_conversation_id)
        .unwrap()
        .is_none());
    assert!(store
        .prepared_edited_resend_by_operation(&input.operation_id)
        .unwrap()
        .is_none());
}

#[test]
fn generic_settings_cannot_forge_or_delete_send_witnesses() {
    let mut store = NolaStore::open(":memory:").unwrap();
    assert!(store
        .put_product_setting("edited_resend/source/forged", "{}", 1)
        .is_err());
    assert!(store
        .delete_product_setting("edited_resend/source/forged")
        .is_err());
    assert!(store
        .product_setting("edited_resend/model_credential/forged")
        .is_err());
    assert!(store
        .list_product_settings("edited_resend/", 10, 0)
        .is_err());
    assert!(store
        .list_room_submissions("edited_resend/", 10, 0)
        .is_err());
}

#[test]
fn missing_original_send_witness_is_not_rebuilt_from_current_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing-witness.db");
    let mut store = NolaStore::open(&path).unwrap();
    source(&mut store);
    let input = intent(&store);
    // Fault fixture only: production generic settings cannot delete this record.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "DELETE FROM product_settings WHERE key LIKE 'edited_resend/source/%'",
            [],
        )
        .unwrap();
    let error = prepare(&mut store, input.clone()).unwrap_err();
    assert!(error.contains("source_send_snapshot_unavailable"));
    assert!(store
        .load_conversation(&input.target_conversation_id)
        .unwrap()
        .is_none());
    assert!(store
        .prepared_edited_resend_by_operation(&input.operation_id)
        .unwrap()
        .is_none());
}

#[test]
fn received_command_receipts_replay_preparation_and_reject_transaction_failure_without_orphan() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("receipts.db");
    let mut store = NolaStore::open(&path).unwrap();
    source(&mut store);
    let input = intent(&store);
    drop(store);
    let command = ApplicationCommand::PrepareEditedResend {
        input: input.clone(),
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
    let first = runtime
        .execute_received_command(received.clone(), command)
        .unwrap();
    assert!(matches!(
        &first.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    let replay = runtime
        .execute_received_command(
            received.clone(),
            serde_json::from_str(&received.command_json).unwrap(),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(first.into_protocol_receipt()).unwrap(),
        serde_json::to_value(replay.into_protocol_receipt()).unwrap()
    );
    let observed = runtime
        .execute(ApplicationCommand::LoadPreparedEditedResend {
            conversation_id: input.target_conversation_id.clone(),
        })
        .unwrap();
    assert_eq!(observed["admitted_run_id"], serde_json::Value::Null);
    drop(runtime);
    let store = NolaStore::open(&path).unwrap();
    assert!(store
        .load_conversation(&input.target_conversation_id)
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
    let rejected = intent(&store);
    drop(store);
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TRIGGER receipt_prepare_failure BEFORE INSERT ON product_settings WHEN NEW.key LIKE 'edited_resend/prepared/%' BEGIN SELECT RAISE(ABORT,'receipt-path prepare failure'); END;").unwrap();
    let command = ApplicationCommand::PrepareEditedResend {
        input: rejected.clone(),
    };
    let received = ExecuteReceivedCommand {
        client_id: "resend-fixture".into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        command_type: "prepare_edited_resend".into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "2026-10-28T00:00:01Z".into(),
    };
    let mut runtime = support::open_runtime(&path).unwrap();
    let reply = runtime.execute_received_command(received, command).unwrap();
    assert!(!matches!(
        reply.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(runtime);
    let store = NolaStore::open(&path).unwrap();
    assert!(store
        .load_conversation(&rejected.target_conversation_id)
        .unwrap()
        .is_none());
    assert!(store
        .prepared_edited_resend_by_operation(&rejected.operation_id)
        .unwrap()
        .is_none());
}

#[test]
fn host_material_and_policy_cannot_be_requested_through_renderer_application_json() {
    for kind in [
        "load_edited_resend_host_material",
        "attest_edited_resend_host_policy",
    ] {
        assert!(serde_json::from_value::<ApplicationCommand>(json!({
            "type":kind,"conversation_id":"target","core_config":{}
        }))
        .is_err());
    }
}
