// SPDX-License-Identifier: Apache-2.0
//! Core serialization, SQLite projection and incremental witness must agree.
use super::*;
use nola_agent_engine::agent::recovery::{PendingHostEffect, PendingHostEffectState};

#[test]
fn real_core_serialization_matches_sql_and_incremental_delivery_witnesses() {
    use nola_agent_engine::agent::host::*;
    let model_capability = HostCapabilityDescriptor {
        id: "model.stream".into(), title: "Fixture model".into(), route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available, exposure: HostCapabilityExposure::Internal,
        permission_domain: None, input_schema: serde_json::json!({"type":"object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000, persist_full_output: true, allow_artifacts: true,
        },
        supports_streaming: true, supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    let mut core = AgentCore::try_new(AgentCoreConfig {
        turn_identity: crate::model_fixture::identity(),
        model: Some("fixture".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("host".into()), revision: 1, availability_epoch: 1, lease: None,
            capabilities: [("model.stream".into(), model_capability)].into_iter().collect(),
        }),
        session_id: Some(
            nola_agent_engine::agent::protocol::SessionId::from_string(
                "witness-contract".into(),
            ),
        ),
        ..Default::default()
    })
    .unwrap();
    let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
    assert!(checkpoint
        .state
        .host
        .pending_effects
        .register(PendingHostEffect {
            control: Default::default(),
            effect_id: "effect".into(),
            request_id: "request".into(),
            host_id: "host".into(),
            lease_id: Some("lease".into()),
            session_id: "witness-contract".into(),
            turn_id: Some("turn".into()),
            catalog_revision: u64::MAX,
            availability_epoch: u64::MAX,
            capability_id: "permission".into(),
            call_id: None,
            task_id: None,
            replay_safety: nola_agent_engine::agent::host::HostReplaySafety::Recoverable,
            state: PendingHostEffectState::Dispatched,
            target_effect_id: None,
            cancel_effect_id: None,
            last_response_sequence: Some(u64::MAX),
            request: Some(serde_json::json!({
                "effect_id":"effect","request_id":"request","host_id":"host","lease_id":"lease",
                "session_id":"witness-contract","turn_id":"turn","catalog_revision":u64::MAX,
                "availability_epoch":u64::MAX,"created_at_ms":0,"deadline_ms":null,
                "payload": {
                "type":"request_permission", "allow_conversation":false, "permission_request_id":"permission",
                "capability_id":"permission","resource":"resource","reason":"fixture",
                "input":{"private":"must not enter witness"}
            }})),
        }));
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE core_checkpoints (
        conversation_id TEXT PRIMARY KEY,revision INTEGER NOT NULL,
        event_cursor INTEGER NOT NULL);
        CREATE TABLE core_checkpoint_bodies(conversation_id TEXT PRIMARY KEY,
            checkpoint_json TEXT NOT NULL);",
        )
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../nola-store/src/delivery_checkpoint_witness_projection.sql"
        ))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../nola-store/src/delivery_checkpoint_witness_schema.sql"
        ))
        .unwrap();
    // Exercise insert and the actual UPSERT shape used by idle/admission writes.
    for revision in [1, 2, 3] {
        if revision == 2 {
            core.submit(nola_agent_engine::agent::AgentSubmission::UserInput {
                items: vec![nola_agent_engine::agent::AgentItem::user_message(
                    nola_agent_engine::agent::ItemId::from_string("input".into()), "fixture")],
                turn_context_items: vec![], turn_context_platform: Default::default(),
                turn_context_components: vec![], submission_id: Some("submission".into()),
                submitted_at_epoch_ms: Some(1),
                delivery: nola_agent_engine::agent::AgentInputDelivery::CurrentTurn,
            }).unwrap();
            checkpoint.state.execution.active_turn = core.checkpoint(CheckpointReason::Manual)
                .state.execution.active_turn;
        } else if revision == 3 {
            checkpoint.state.execution.active_turn.as_mut().unwrap().status =
                nola_domain::AgentTurnStatus::Cancelling;
        }
        connection
            .execute(
                "INSERT INTO core_checkpoints VALUES('conversation',?1,0)
            ON CONFLICT(conversation_id) DO UPDATE SET
                revision=excluded.revision",
                [revision],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO core_checkpoint_bodies VALUES('conversation',?1)
            ON CONFLICT(conversation_id) DO UPDATE SET checkpoint_json=excluded.checkpoint_json",
                [serde_json::to_string(&checkpoint).unwrap()],
            )
            .unwrap();
        let encoded: String = connection.query_row(
            "SELECT witness_json FROM core_delivery_witnesses WHERE conversation_id='conversation'",
            [], |row| row.get(0)).unwrap();
        assert!(!encoded.contains("must not enter witness"));
        let sql: Value = serde_json::from_str(&encoded).unwrap();
        let incremental: Value =
            serde_json::from_str(&encode_delivery_witness(&checkpoint).unwrap()).unwrap();
        assert_eq!(sql, incremental);
        if revision == 1 {
            assert!(sql["state"]["execution"]["active_turn"].is_null());
        } else {
            let turn = checkpoint.state.execution.active_turn.as_ref().unwrap();
            assert_eq!(sql["state"]["execution"]["active_turn"]["turn_id"], serde_json::json!(turn.turn_id));
            assert_eq!(sql["state"]["execution"]["active_turn"]["status"], serde_json::json!(turn.status));
            if revision == 3 {
                assert_eq!(sql["state"]["execution"]["active_turn"]["status"], "cancelling");
            }
        }
        let decoded: CommittedDeliveryCheckpoint = serde_json::from_value(sql).unwrap();
        let effect = &decoded.state.host.pending_effects.effects["effect"];
        assert_eq!(effect.request_id, "request");
        assert_eq!(effect.permission_request_id.as_deref(), Some("permission"));
        assert_eq!(effect.last_response_sequence, Some(u64::MAX));
    }
}
