// SPDX-License-Identifier: Apache-2.0
//! Application/SQLite compaction admission, receipt rollback and cold replay.
mod support;
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
