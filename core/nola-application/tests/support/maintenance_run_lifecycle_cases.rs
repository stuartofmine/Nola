// SPDX-License-Identifier: Apache-2.0
//! Maintenance has durable operation evidence, not an assistant-task result.
use super::*;

#[test]
fn session_close_is_durable_maintenance_without_model_or_user_result() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("close.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.create_conversation(&nola_store::CreateConversationInput {
        id: "recovery-conversation".into(), title: "Close".into(), revision: 1,
        settings_json: None, created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    let mut close_config = config();
    close_config.host_catalog.as_mut().unwrap().capabilities.insert(
        "lifecycle.session_end".into(), runtime_capability("lifecycle.session_end", HostCapabilityRoute::Tool));
    close_config.host_catalog.as_mut().unwrap().capabilities.remove("model.stream");
    let command = nola_application::BeginMaintenanceTurn {
        conversation_id: "recovery-conversation".into(), context_checkpoint_id: None,
        core_config: close_config, submission: AgentSubmission::CloseSession {
            submission_id: Some("close-operation".into()),
        }, created_at_ms: 1000,
    };
    let accepted = nola_application::begin_maintenance_turn(&mut store, command.clone()).unwrap();
    assert!(nola_application::begin_maintenance_turn(&mut store, command.clone()).unwrap_err().contains("active Run"));
    let requests = load_committed_host_requests(&mut store, &accepted.run_id).unwrap().requests;
    assert_eq!(requests.len(), 1);
    assert!(matches!(&requests[0].payload, HostRequest::RunLifecycleHook { event }
        if event.turn_id.as_deref() == Some(accepted.run_id.as_str())));
    let request = claim_and_start_request(&mut store, &accepted.run_id,
        &accepted.invocation_config_digest, &requests[0], 1100);
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    let settled = reduce_and_accept_host_response(&mut store, ReduceAndAcceptHostResponse {
        conversation_id: "recovery-conversation".into(), run_id: accepted.run_id.clone(),
        invocation_config_digest: accepted.invocation_config_digest,
        response: response_for_request(&request, 1, HostResponse::Completed {
            output: serde_json::json!({"hooks_completed": true}), usage: None,
            full_output_ref: None, artifacts: Vec::new(),
        }), received_at_ms: 1200,
    }).unwrap();
    assert!(settled.events.iter().any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    assert!(!settled.events.iter().any(|event| matches!(event.event, AgentEvent::RunResultDecided { .. })));
    assert!(store.active_run("recovery-conversation").unwrap().is_none());
    assert!(store.load_conversation("recovery-conversation").unwrap().unwrap().messages.is_empty());
    assert!(nola_application::begin_maintenance_turn(&mut store, command).unwrap_err().contains("already closed"));
}

fn config() -> AgentCoreConfig {
    let mut config = runtime_config();
    let catalog = config.host_catalog.as_mut().unwrap();
    for (id, route) in [
        ("context.compact", HostCapabilityRoute::Compaction),
        ("host.cancel", HostCapabilityRoute::Tool),
    ] {
        catalog
            .capabilities
            .insert(id.into(), runtime_capability(id, route));
    }
    config.initial_items = (0..16)
        .map(|index| AgentItem::Message {
            id: ItemId::from_string(format!("history-{index}")),
            role: if index % 2 == 0 {
                nola_agent_engine::agent::AgentRole::User
            } else {
                nola_agent_engine::agent::AgentRole::Assistant
            },
            content: format!(
                "Historical source {index}: {}",
                "retained source evidence ".repeat(200)
            ),
        })
        .collect();
    config
}

#[test]
fn maintenance_empty_archive_records_explicit_noop_without_fabricated_summary() {
    let directory = tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("noop.db")).unwrap();
    store
        .create_conversation(&nola_store::CreateConversationInput {
            id: "recovery-conversation".into(),
            title: "Maintenance".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let mut config = config();
    config.initial_items = vec![AgentItem::user_message(
        ItemId::from_string("small-history".into()),
        "Hello",
    )];
    let accepted = nola_application::begin_maintenance_turn(
        &mut store,
        nola_application::BeginMaintenanceTurn {
            conversation_id: "recovery-conversation".into(),
            context_checkpoint_id: None,
            core_config: config,
            submission: AgentSubmission::Compact {
                operation_id: "noop-compact".into(),
            },
            created_at_ms: 1000,
        },
    )
    .unwrap();
    assert!(accepted.events.iter().any(|event| matches!(&event.event,
        AgentEvent::CompactionCompleted { replaced_item_count: 0, source_item_count: 0,
            compaction_item_id: None, summary, .. } if summary.is_empty())));
    assert!(!accepted
        .events
        .iter()
        .any(|event| matches!(event.event, AgentEvent::RunResultDecided { .. })));
    assert!(store.active_run("recovery-conversation").unwrap().is_none());
}

#[test]
fn maintenance_success_failure_and_cancel_settle_without_user_result_or_messages() {
    for outcome in ["completed", "failed", "cancelled"] {
        let directory = tempdir().unwrap();
        let database = directory.path().join("maintenance.db");
        let mut store = NolaStore::open(&database).unwrap();
        store
            .create_conversation(&nola_store::CreateConversationInput {
                id: "recovery-conversation".into(),
                title: "Maintenance".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let accepted = nola_application::begin_maintenance_turn(
            &mut store,
            nola_application::BeginMaintenanceTurn {
                conversation_id: "recovery-conversation".into(),
                context_checkpoint_id: None,
                core_config: config(),
                submission: AgentSubmission::Compact {
                    operation_id: "compact-operation".into(),
                },
                created_at_ms: 1000,
            },
        )
        .unwrap();
        let mut request = load_committed_host_requests(&mut store, &accepted.run_id)
            .unwrap()
            .requests
            .into_iter()
            .find(|request| matches!(request.payload, HostRequest::Compact { .. }))
            .unwrap();
        support::admit_model_sampling(
            &mut store,
            &accepted.run_id,
            &accepted.invocation_config_digest,
            &mut request,
            1100,
        );
        let response = if outcome == "cancelled" {
            reduce_and_commit_active_submission(
                &mut store,
                ReduceAndCommitActiveSubmission {
                    conversation_id: "recovery-conversation".into(),
                    run_id: accepted.run_id.clone(),
                    transition_id: "cancel-maintenance".into(),
                    invocation_config_digest: accepted.invocation_config_digest.clone(),
                    submission: AgentSubmission::Interrupt {
                        goal_control: None, cancellation_id: "cancel-maintenance-command".into(),
                        reason: Some("stop compaction".into()),
                    },
                    committed_at_ms: 1200,
                },
            )
            .unwrap();
            let cancellation = load_committed_host_requests(&mut store, &accepted.run_id)
                .unwrap()
                .requests
                .into_iter()
                .find(|request| matches!(request.payload, HostRequest::Cancel { .. }))
                .unwrap();
            let cancellation = claim_and_start_request(
                &mut store,
                &accepted.run_id,
                &accepted.invocation_config_digest,
                &cancellation,
                1250,
            );
            response_for_request(
                &cancellation,
                1,
                HostResponse::CancellationConfirmed {
                    target_effect_id: request.effect_id.clone(),
                },
            )
        } else {
            response_for_request(
                &request,
                1,
                if outcome == "completed" {
                    HostResponse::CompactionCompleted {
                        summary: "The preserved historical facts and outstanding work.".into(),
                        usage: serde_json::json!({"input_tokens":100,"output_tokens":20,"total_tokens":120}),
                    }
                } else {
                    HostResponse::Failed {
                        error: nola_agent_engine::agent::host::HostError {
                            code: "provider_failed".into(),
                            message: "provider unavailable".into(),
                            retryable: false,
                            http_status: None,
                        },
                    }
                },
            )
        };
        let settled = reduce_and_accept_host_response(
            &mut store,
            ReduceAndAcceptHostResponse {
                conversation_id: "recovery-conversation".into(),
                run_id: accepted.run_id.clone(),
                invocation_config_digest: accepted.invocation_config_digest,
                response,
                received_at_ms: 1300,
            },
        )
        .unwrap();
        assert!(!settled
            .events
            .iter()
            .any(|event| matches!(event.event, AgentEvent::RunResultDecided { .. })));
        drop(store);
        let store = NolaStore::open(&database).unwrap();
        assert!(store.active_run("recovery-conversation").unwrap().is_none());
        assert!(store
            .load_conversation("recovery-conversation")
            .unwrap()
            .unwrap()
            .messages
            .is_empty());
        let sql = rusqlite::Connection::open(&database).unwrap();
        let (kind, status): (String, String) = sql
            .query_row(
                "SELECT kind,status FROM runs WHERE id=?1",
                [&accepted.run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((kind.as_str(), status.as_str()), ("maintenance", "sealed"));
        let seals: i64 = sql
            .query_row(
                "SELECT COUNT(*) FROM assistant_run_terminal_seals WHERE run_id=?1",
                [&accepted.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(seals, 0);
        let terminal_events: i64 = sql
            .query_row(
                "SELECT COUNT(*) FROM run_events WHERE run_id=?1 AND event_kind=?2",
                rusqlite::params![accepted.run_id, format!("turn_{outcome}")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(terminal_events, 1);
    }
}
