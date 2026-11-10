// SPDX-License-Identifier: Apache-2.0
// SessionStore -> real in-process Application commit -> source query -> emitter.
// This exercises the same port as --no-save, not a handwritten projection.
#[cfg(test)]
mod assistant_journal_notifications_tests {
    use super::*;
    use nola_core::agent::host::*;
    use nola_runtime_driver::{CommittedRuntimeApplicationPort, RuntimeEffectDeliveryClaim};

    #[test]
    fn execution_views_publish_only_independent_control_state() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| sink.lock().unwrap().push(event));
        let core = AgentCore::try_new(AgentCoreConfig {
            turn_identity: crate::test_model_identity::identity(),
            session_id: Some(SessionId::from_string("control-source".into())),
            ..Default::default()
        })
        .unwrap();
        let mut execution = core.host_execution_state();
        emitter.begin_recovery_transport_isolation();
        CoreEventPresenter::new(&mut emitter, 1).publish_execution(&execution);
        assert!(
            captured.lock().unwrap().is_empty(),
            "idle preparation must not publish control state into the next Run transport"
        );
        emitter.end_recovery_transport_isolation();
        let mut presenter = CoreEventPresenter::new(&mut emitter, 1);
        for cursor in [1, 128, 256, 1024] {
            execution.control.revision = cursor;
            execution.control.last_event_cursor.sequence = cursor;
            presenter.publish_execution(&execution);
        }
        execution.control.turn_status = nola_core::agent::AgentTurnStatus::Completed;
        presenter.publish_execution(&execution);
        let frames = captured.lock().unwrap();
        assert!(!frames.is_empty());
        assert!(frames
            .iter()
            .all(|frame| frame["type"] == "core_permission_projection_updated"));
        assert!(frames.iter().all(|frame| frame.get("snapshot").is_none()));
    }

    fn response(
        request: &HostRequestEnvelope,
        sequence: u64,
        payload: HostResponse,
    ) -> HostResponseEnvelope {
        HostResponseEnvelope {
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
            sequence,
            payload,
        }
    }

    fn begin() -> (SessionStore, HostRequestEnvelope) {
        begin_with_store(SessionStore::open_ephemeral(None, "/fixture").unwrap())
    }

    fn begin_with_store(mut store: SessionStore) -> (SessionStore, HostRequestEnvelope) {
        let capability = HostCapabilityDescriptor {
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
        };
        let config = AgentCoreConfig {
            turn_identity: crate::test_model_identity::identity(),
            session_id: Some(SessionId::from_string(store.id.clone())),
            model: Some("fixture-model".into()),
            host_catalog: Some(HostCapabilityCatalog {
                host_id: HostId("fixture-host".into()),
                revision: 1,
                availability_epoch: 1,
                lease: None,
                capabilities: [("model.stream".into(), capability)].into_iter().collect(),
            }),
            ..Default::default()
        };
        let (configuration_revision, parent_message_id) = store.read_submission_boundary().unwrap();
        store
            .begin_core_turn(BeginAssistantTurn {
                expected_configuration_revision: Some(configuration_revision),
                expected_parent_message_id: parent_message_id,
                conversation_id: store.id.clone(),
                conversation_title: "Committed source".into(),
                user_message_id: "source-user".into(),
                user_part_id: "source-user:text".into(),
                user_text: "SOURCE_USER_MUST_NOT_BE_RETRANSMITTED".into(),
                user_content_json: None,
                assistant_message_id: "source-run:assistant".into(),
                context_checkpoint_id: None,
                core_config: config,
                submission: AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(
                        ItemId::from_string("source-user".into()),
                        "SOURCE_USER_MUST_NOT_BE_RETRANSMITTED",
                    )],
                    turn_context_items: vec![],
                    turn_context_platform: Default::default(),
                    turn_context_components: vec![],
                    submission_id: Some("source-run".into()),
                    submitted_at_epoch_ms: Some(1000),
                    delivery: AgentInputDelivery::CurrentTurn,
                },
                created_at_ms: 1000,
            })
            .unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let mut request = store
            .claim_committed_requests(RuntimeEffectDeliveryClaim {
                worker_generation: "fixture-generation".into(),
                lease_owner: "fixture-owner".into(),
                claimed_at_ms: now,
                lease_expires_at_ms: now + 60_000,
                limit: 32,
            })
            .unwrap()
            .into_iter()
            .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
            .unwrap();
        store
            .accept_raw_host_response(response(&request, 1, HostResponse::ExecutionStarted))
            .unwrap();
        store
            .accept_raw_host_response(response(
                &request,
                2,
                HostResponse::ModelAttemptStarted {
                    provider: "fixture".into(),
                    model: "fixture-model".into(),
                    reason: "initial".into(),
                },
            ))
            .unwrap();
        let delivery = request.delivery.clone();
        request = store
            .load_committed_requests()
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.effect_id == request.effect_id)
            .unwrap();
        request.delivery = delivery;
        (store, request)
    }

    #[test]
    fn committed_assistant_source_query_emits_only_owned_message_after_real_commit() {
        let (mut store, request) = begin();
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| sink.lock().unwrap().push(event));
        let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
            .with_assistant_journal_notifications(&mut store, &request.session_id);
        presenter.publish_assistant_journal_notifications();
        captured.lock().unwrap().clear();
        let text_part = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        };
        // Process-retained CLI responses have the same received transaction
        // boundary as the persistent Service. A short prefix is a real journal
        // commit, not a reconstructed PartUpdated or provider-trace preview.
        let accepted_prefix = store
            .accept_raw_host_response(response(
                &request,
                3,
                HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id: text_part.clone(),
                    delta: "Committed prefix. ".into(),
                },
            ))
            .unwrap();
        for event in accepted_prefix.events {
            presenter.publish(CoreHostOutput::Event(event));
        }
        for trace in accepted_prefix.traces {
            presenter.publish(CoreHostOutput::Trace(trace));
        }
        assert!(!captured
            .lock()
            .unwrap()
            .iter()
            .any(|event| event["type"] == "remote_assistant_projection_committed_v1"));
        presenter.publish(CoreHostOutput::Committed);
        assert!(
            captured
                .lock()
                .unwrap()
                .iter()
                .any(|event| event["type"] == "assistant_journal_committed_v1"),
            "a received short response must publish its actual committed journal target"
        );
        let mut previous = std::collections::HashMap::new();
        // Unique records exercise multi-range persistence without
        // legitimately triggering Core's repetitive-model-output guard.
        let first = (0..3_000)
            .map(|index| format!("First committed record {index:04}. "))
            .collect::<String>();
        let second = (0..3_000)
            .map(|index| format!("Second committed record {index:04}. "))
            .collect::<String>();
        let completed_body = format!("Committed prefix. {first}{second}");
        let batches = [(4, first), (5, second)];
        for (sequence, delta) in batches {
            let accepted = store
                .accept_raw_host_response(response(
                    &request,
                    sequence,
                    HostResponse::ModelDelta {
                        channel: ModelDeltaChannel::Text,
                        part_id: text_part.clone(),
                        delta: delta.clone(),
                    },
                ))
                .unwrap();
            let committed_cursors = store
                .assistant_journal_notifications_queue()
                .lock()
                .unwrap()
                .journal_publications
                .clone();
            assert!(!committed_cursors.is_empty());
            // The real driver publishes the commit independently of body/control
            // events. A text-only commit must not depend on the removed trace.
            presenter.publish(CoreHostOutput::Committed);
            for event in accepted.events {
                presenter.publish(CoreHostOutput::Event(event));
            }
            for trace in accepted.traces {
                presenter.publish(CoreHostOutput::Trace(trace));
            }
            if let Some(control) = accepted.control.as_ref() {
                presenter.publish_control(control);
            }
            let frames = captured.lock().unwrap();
            for cursor in committed_cursors {
                let notice = frames
                    .iter()
                    .rev()
                    .find(|event| {
                        event["type"] == "assistant_journal_committed_v1"
                            && event["through"] == serde_json::to_value(&cursor).unwrap()
                    })
                    .expect("Commit watermark must reach the actual transport");
                assert_eq!(notice["session_id"], store.id);
                assert_eq!(notice["source_host_id"], "local");
                assert_eq!(notice["through"]["identity"]["run_id"], "source-run");
                assert!(
                    notice.to_string().len() < 2048,
                    "Notification size must not grow with the assistant body"
                );
                assert!(!notice.to_string().contains("committed record"));
                let owner = (
                    &cursor.identity.source_store_epoch,
                    &cursor.identity.conversation_id,
                    &cursor.identity.run_id,
                    &cursor.identity.assistant_message_id,
                );
                let owner = (
                    owner.0.clone(),
                    owner.1.clone(),
                    owner.2.clone(),
                    owner.3.clone(),
                );
                if let Some(prior) = previous.insert(owner, cursor.sequence) {
                    assert!(
                        cursor.sequence > prior,
                        "committed journal cursor must advance"
                    );
                }
            }
            assert!(
                !frames
                    .iter()
                    .any(|event| event["type"] == "remote_assistant_projection_committed_v1"),
                "durable streaming batches must not retransmit a cumulative assistant body"
            );
            let count = frames
                .iter()
                .filter(|event| event["type"] == "assistant_journal_committed_v1")
                .count();
            drop(frames);
            presenter.publish_assistant_journal_notifications();
            assert_eq!(
                captured
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|event| event["type"] == "assistant_journal_committed_v1")
                    .count(),
                count
            );
        }
        let mut foreign = response(
            &request,
            6,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: text_part.clone(),
                delta: "UNCOMMITTED_FORGED_TEXT".into(),
            },
        );
        foreign.session_id = "foreign-session".into();
        assert!(store.accept_raw_host_response(foreign).is_err());
        presenter.publish_assistant_journal_notifications();
        assert!(!captured
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.to_string().contains("UNCOMMITTED_FORGED_TEXT")));
        store
            .accept_raw_host_response(response(
                &request,
                6,
                HostResponse::ModelCompleted {
                    items: vec![serde_json::to_value(AgentItem::assistant_message(
                        ItemId::from_string(text_part),
                        completed_body,
                    ))
                    .unwrap()],
                    tool_calls: vec![],
                    usage: json!({}),
                },
            ))
            .unwrap();
        // Model completion is not Run completion: the durable lifecycle hook
        // must settle first. Deliver its last valid partial before finalizing.
        presenter.publish_assistant_journal_notifications();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let remaining = store
            .claim_committed_requests(RuntimeEffectDeliveryClaim {
                worker_generation: "fixture-generation".into(),
                lease_owner: "fixture-owner".into(),
                claimed_at_ms: now,
                lease_expires_at_ms: now + 60_000,
                limit: 32,
            })
            .unwrap();
        for hook in remaining {
            assert!(
                matches!(hook.payload, HostRequest::RunLifecycleHook { .. }),
                "unexpected remaining effect: {:?}",
                hook.payload
            );
            store
                .accept_raw_host_response(response(&hook, 1, HostResponse::ExecutionStarted))
                .unwrap();
            presenter.publish_assistant_journal_notifications();
            store
                .accept_raw_host_response(response(
                    &hook,
                    2,
                    HostResponse::Completed {
                        output: json!({}),
                        usage: None,
                        full_output_ref: None,
                        artifacts: vec![],
                    },
                ))
                .unwrap();
        }
        assert!(store.load_committed_requests().unwrap().is_empty());
        assert!(
            store.active_core_run_id().is_none(),
            "canonical Run must actually be terminal"
        );
    }

    #[test]
    fn lost_notification_queue_is_seeded_from_committed_heads_once_per_run() {
        let store = SessionStore::open_ephemeral(None, "/fixture").unwrap();
        let subscriber = store.assistant_journal_notifications_queue();
        let (mut store, request) = begin_with_store(store);
        let queue = store.assistant_journal_notifications_queue();
        // Admission exposes an initial sequence-zero read cursor, not a
        // committed content head. Lose a real publication, not that sentinel.
        queue.lock().unwrap().journal_publications.clear();
        let part_id = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        };
        store.accept_raw_host_response(response(&request, 3, HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text, part_id,
            delta: "committed before subscriber loss".into(),
        })).unwrap();
        let expected = std::mem::take(&mut queue.lock().unwrap().journal_publications);
        assert!(!expected.is_empty());
        assert!(expected.iter().all(|cursor| cursor.sequence > 0));
        drop(subscriber);
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| sink.lock().unwrap().push(event));
        {
            let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
                .with_assistant_journal_notifications(&mut store, &request.session_id);
            presenter.publish_assistant_journal_notifications();
        }
        for cursor in &expected {
            assert!(captured
                .lock()
                .unwrap()
                .iter()
                .any(|event| event["type"] == "assistant_journal_committed_v1"
                    && event["through"] == serde_json::to_value(cursor).unwrap()));
        }
        let count = captured
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "assistant_journal_committed_v1")
            .count();
        {
            let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
                .with_assistant_journal_notifications(&mut store, &request.session_id);
            presenter.publish_assistant_journal_notifications();
        }
        assert_eq!(
            captured
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event["type"] == "assistant_journal_committed_v1")
                .count(),
            count,
            "Recreating a presenter must not re-enumerate and replay an unchanged Run"
        );
    }

    #[test]
    fn human_transport_consumes_committed_journal_without_retaining_notifications() {
        let (mut store, request) = begin();
        let mut emitter = Emitter::new(false);
        let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
            .with_assistant_journal_notifications(&mut store, &request.session_id);
        presenter.publish(CoreHostOutput::Committed);
        assert!(presenter.emitter.take_markdown_text().is_empty());
        let part_id = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        };
        let text = (0..4_000)
            .map(|index| format!("committed record {index:04}. "))
            .collect::<String>();
        store
            .accept_raw_host_response(response(
                &request,
                3,
                HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id,
                    delta: text.clone(),
                },
            ))
            .unwrap();
        let queue = store.assistant_journal_notifications_queue();
        {
            let delivery = queue.lock().unwrap();
            assert!(
                !delivery.journal_publications.is_empty(),
                "an attached human presenter owns pending committed journal delivery"
            );
            assert!(delivery.observer_error.is_none());
        }
        presenter.publish(CoreHostOutput::Committed);
        assert_eq!(presenter.emitter.take_markdown_text(), text);
        presenter.publish(CoreHostOutput::Committed);
        assert!(presenter.emitter.take_markdown_text().is_empty());
        let delivery = queue.lock().unwrap();
        assert!(
            delivery.journal_publications.is_empty(),
            "plain CLI must not accumulate undeliverable journal notifications"
        );
        assert!(delivery.observer_error.is_none());
    }

    #[test]
    fn same_run_subscriber_return_seeds_commits_made_without_a_presenter() {
        let (mut store, request) = begin();
        let part_id = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        };
        store.accept_raw_host_response(response(&request, 3, HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text, part_id: part_id.clone(),
            delta: "before subscriber return".into(),
        })).unwrap();
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| sink.lock().unwrap().push(event));
        {
            let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
                .with_assistant_journal_notifications(&mut store, &request.session_id);
            presenter.publish_assistant_journal_notifications();
        }
        let before = captured
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "assistant_journal_committed_v1")
            .map(|event| event["through"].clone())
            .collect::<Vec<_>>();
        assert!(!before.is_empty());
        store
            .accept_raw_host_response(response(
                &request,
                4,
                HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id,
                    delta: "while detached ".repeat(8_000),
                },
            ))
            .unwrap();
        assert!(store
            .assistant_journal_notifications_queue()
            .lock()
            .unwrap()
            .journal_publications
            .is_empty());
        captured.lock().unwrap().clear();
        {
            let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
                .with_assistant_journal_notifications(&mut store, &request.session_id);
            presenter.publish_assistant_journal_notifications();
        }
        let frames = captured.lock().unwrap();
        let after = frames
            .iter()
            .filter(|event| event["type"] == "assistant_journal_committed_v1")
            .map(|event| event["through"].clone())
            .collect::<Vec<_>>();
        assert!(!after.is_empty());
        assert_ne!(
            before, after,
            "rejoining the same Run must discover its newer committed head"
        );
    }

    #[test]
    fn streaming_publishes_journal_without_reading_cumulative_projection() {
        let (mut store, request) = begin();
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| sink.lock().unwrap().push(event));
        let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
            .with_assistant_journal_notifications(&mut store, &request.session_id);
        presenter.publish_assistant_journal_notifications();
        let queue = store.assistant_journal_notifications_queue();
        let part_id = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        };
        // Exceeds the legacy whole-message query budget. Streaming must not
        // query that representation at all, regardless of elapsed wall time.
        store
            .accept_raw_host_response(response(
                &request,
                3,
                HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id,
                    delta: "x".repeat(9 * 1024 * 1024),
                },
            ))
            .expect("an optional presentation read cannot undo the accepted model response");
        let delivery = queue.lock().unwrap();
        assert!(
            !delivery.journal_publications.is_empty(),
            "committed content still needs a journal target"
        );
        drop(delivery);
        // Next response is still accepted against the same live committed Run.
        store
            .accept_raw_host_response(response(
                &request,
                4,
                HostResponse::ModelCompleted {
                    items: vec![],
                    tool_calls: vec![],
                    usage: json!({}),
                },
            ))
            .unwrap();
        presenter.publish_assistant_journal_notifications();
        let frames = captured.lock().unwrap();
        assert!(frames
            .iter()
            .any(|frame| frame["type"] == "assistant_journal_committed_v1"));
        assert!(frames.iter().all(|frame| frame["type"] != "remote_assistant_projection_committed_v1"),
            "stream and terminal boundaries must not emit the retired cumulative assistant protocol");
    }

    #[test]
    fn ephemeral_short_delta_commits_public_journal_before_human_delivery() {
        let (mut store, request) = begin();
        let mut emitter = Emitter::new(false);
        let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
            .with_assistant_journal_notifications(&mut store, &request.session_id);
        presenter.publish(CoreHostOutput::Committed);
        assert!(presenter.emitter.take_markdown_text().is_empty());
        let part_id = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        };
        let delta = "短🙂";
        let accepted = store
            .accept_raw_host_response(response(
                &request,
                3,
                HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id,
                    delta: delta.into(),
                },
            ))
            .unwrap();
        assert!(accepted
            .events
            .iter()
            .all(|event| !matches!(event.event, AgentEvent::PartUpdated { .. })));
        // No terminal response, sleep or subsequent provider callback supplies
        // a flush. Read the source's actual public facts before rendering them.
        let source = store
            .read_source_run_journal("source-run", "source-run:assistant")
            .unwrap();
        assert_eq!(source.public_text(), delta);
        assert!(
            source.seal.is_none(),
            "a visible prefix cannot finish the Run"
        );
        presenter.publish(CoreHostOutput::Committed);
        assert_eq!(presenter.emitter.take_markdown_text(), delta);
        presenter.publish(CoreHostOutput::Committed);
        assert!(presenter.emitter.take_markdown_text().is_empty());
        let queue = store.assistant_journal_notifications_queue();
        let delivery = queue.lock().unwrap();
        assert!(delivery.journal_publications.is_empty());
        assert!(delivery.observer_error.is_none());
    }

    #[test]
    fn terminal_reads_only_committed_journal_suffixes_without_part_events() {
        let (mut store, request) = begin();
        let mut emitter = Emitter::new(false);
        let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
            .with_assistant_journal_notifications(&mut store, &request.session_id);
        presenter.publish(CoreHostOutput::Committed);
        let text_part_id = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        };
        for (index, delta) in ["你好", " journal", " suffix"].iter().enumerate() {
            let accepted = store
                .accept_raw_host_response(response(
                    &request,
                    index as u64 + 3,
                    HostResponse::ModelDelta {
                        channel: ModelDeltaChannel::Text,
                        part_id: text_part_id.clone(),
                        delta: (*delta).into(),
                    },
                ))
                .unwrap();
            assert!(accepted
                .events
                .iter()
                .all(|event| !matches!(event.event, AgentEvent::PartUpdated { .. })));
            presenter.publish(CoreHostOutput::Committed);
            assert_eq!(presenter.emitter.take_markdown_text(), *delta);
            presenter.publish(CoreHostOutput::Committed);
            assert!(presenter.emitter.take_markdown_text().is_empty());
            assert!(store
                .assistant_journal_notifications_queue()
                .lock()
                .unwrap()
                .observer_error
                .is_none());
        }
        store
            .accept_raw_host_response(response(
                &request,
                6,
                HostResponse::ModelCompleted {
                    items: vec![],
                    tool_calls: vec![],
                    usage: json!({}),
                },
            ))
            .unwrap();
        presenter.publish(CoreHostOutput::Committed);
        assert!(!presenter.emitter.has_text_preview());
    }

    #[test]
    fn service_receipt_transaction_emits_short_committed_delta_without_ipc() {
        let store = SessionStore::open_ephemeral(None, "/fixture")
            .unwrap()
            .with_service_receipts();
        let (mut store, request) = begin_with_store(store);
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| sink.lock().unwrap().push(event));
        let mut presenter = CoreEventPresenter::new(&mut emitter, 1)
            .with_assistant_journal_notifications(&mut store, &request.session_id);
        presenter.publish_assistant_journal_notifications();
        captured.lock().unwrap().clear();
        std::thread::sleep(std::time::Duration::from_millis(55));
        let part_id = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        };
        let accepted = store
            .accept_raw_host_response(response(
                &request,
                3,
                HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id,
                    delta: "Short durable answer.".into(),
                },
            ))
            .unwrap();
        for event in accepted.events {
            presenter.publish(CoreHostOutput::Event(event));
        }
        for trace in accepted.traces {
            presenter.publish(CoreHostOutput::Trace(trace));
        }
        presenter.publish_assistant_journal_notifications();
        let events = captured.lock().unwrap();
        let frame = events
            .iter()
            .find(|event| event["type"] == "assistant_journal_committed_v1")
            .expect("received command unit of work durably commits even a short model delta");
        assert_eq!(frame["through"]["identity"]["run_id"], "source-run");
        assert!(
            serde_json::from_value::<nola_domain::assistant_journal::AssistantJournalCursor>(
                frame["through"].clone()
            )
            .unwrap()
            .sequence
                > 0
        );
        assert!(
            !frame.to_string().contains("Short durable answer."),
            "committed notifications carry cursors, never cumulative bodies"
        );
        assert!(events
            .iter()
            .all(|event| event["type"] != "remote_assistant_projection_committed_v1"));
    }
}
