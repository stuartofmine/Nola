// SPDX-License-Identifier: Apache-2.0
//! Public part semantics, stored public parts, and kernel projection journal tests.

mod public_part_semantics_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Cold SQLite checkpoint -> real Application public projection. No model fixture
    //! can conceal dropped metadata, private output paths or null-result absence.

    use crate::support::model_identity as fixture_model_identity;
    use nola_application::{load_conversation_projection_v1, load_conversation_projection_window_v1};
    use nola_agent_engine::agent::parts::{AgentPart, PartUpdate};
    use nola_agent_engine::agent::recovery::{CheckpointReason, CoreCheckpoint};
    use nola_domain::core_event::{EventCursor};
    use nola_agent_engine::agent::{AgentSessionConfiguration, AgentThreadState, SessionId};
    use nola_domain::{
        AssistantTurnBinding, MessagePartView, PartMutation, SourceLocation, StructuredValue,
        TextReviewState,
    };
    use nola_store::{BeginTurnInput, CommitTransitionInput, NolaStore};
    use serde_json::{json, Value};

    const CONVERSATION: &str = "public-semantic-cold";
    const RUN: &str = "source-run";
    const MESSAGE: &str = "canonical-assistant";

    fn core_part(mut value: Value) -> AgentPart {
        if value["kind"] == "tool" {
            value["execution_source"] = json!({"kind":"model"});
        }
        value["message_id"] = json!(format!("{RUN}:assistant"));
        value["turn_id"] = json!(RUN);
        serde_json::from_value(value).unwrap()
    }

    fn persist_checkpoint(database: &std::path::Path, parts: &[AgentPart]) {
        let mut state = AgentThreadState::new(
            SessionId::from_string(CONVERSATION.into()),
            AgentSessionConfiguration {
                model: None,
                max_model_iterations: 0,
                max_strategy_recovery_attempts: 3,
                tools: vec![],
                context_window: Default::default(),
                memory_policy: Default::default(),
                turn_identity: fixture_model_identity::identity(),
            },
        );
        for part in parts {
            state
                .conversation
                .projection
                .apply(PartUpdate::Created { part: part.clone() })
                .expect("fixture part must satisfy the canonical projection contract");
        }
        let checkpoint = CoreCheckpoint {
            schema_version: nola_agent_engine::agent::recovery::AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: CONVERSATION.into(),
            revision: 1,
            last_event_cursor: EventCursor { sequence: 1 },
            capability_catalog_revision: 0,
            created_at_ms: 3,
            reason: CheckpointReason::Manual,
            state,
        };
        let mut store = NolaStore::open(database).unwrap();
        store
            .begin_turn(&BeginTurnInput {
                user_part_kind: "text".into(),
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                initial_events: Vec::new(),
                conversation_id: CONVERSATION.into(),
                conversation_title: "Public semantic facts".into(),
                user_message_id: "semantic-user".into(),
                user_part_id: "semantic-user-part".into(),
                user_text: "Persist public semantic facts".into(),
                user_content_json: None,
                additional_user_parts: vec![],
                assistant_message_id: MESSAGE.into(),
                assistant_part_id: "admission-placeholder".into(),
                assistant_turn_bindings: vec![AssistantTurnBinding::Admission {
                    turn_id: RUN.into(),
                    assistant_message_id: MESSAGE.into(),
                }],
                initial_assistant_part_mutations: vec![],
                run_id: RUN.into(),
                context_checkpoint_id: None,
                checkpoint_revision: 0,
                checkpoint_event_cursor: 0,
                checkpoint_json: "{}".into(),
                invocation_config_json: "{}".into(),
                invocation_config_digest: "fixture".into(),
                outbox_effects: vec![],
                model_attempt_mutations: vec![],
                delegated_invocation_mutations: vec![],
                delegated_permission_mutations: vec![],
                memory_usage_entries: vec![],
                created_at_ms: 1,
            })
            .unwrap();
        store
            .commit_transition(&CommitTransitionInput {
                transition_id: "public-semantic-materialization".into(),
                run_id: RUN.into(),
                expected_checkpoint_revision: 0,
                checkpoint_revision: 1,
                checkpoint_event_cursor: 1,
                checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: serde_json::to_string(&checkpoint).unwrap() },
                submission_reduction_json: None, host_event: None,
                events: vec![],
                ensured_messages: vec![],
                part_message_owners: Default::default(),
                assistant_turn_bindings: parts
                    .iter()
                    .map(|part| AssistantTurnBinding::Part {
                        turn_id: RUN.into(),
                        assistant_message_id: MESSAGE.into(),
                        part_id: part.id().into(),
                    })
                    .collect(),
                part_mutations: parts
                    .iter()
                    .enumerate()
                    .map(|(index, part)| PartMutation::Upsert {
                        part_id: part.id().into(),
                        kind: match part {
                            AgentPart::Text(_) => "text",
                            AgentPart::Reasoning(_) => "reasoning",
                            AgentPart::Tool(_) => "tool",
                            AgentPart::Source(_) => "source",
                            AgentPart::Citation(_) => "citation",
                            AgentPart::Artifact(_) => "artifact",
                            AgentPart::Step(_) => "step",
                            AgentPart::Compaction(_) => "compaction",
                        }
                        .into(),
                        ordinal: index as i64 + 1,
                        state: "completed".into(),
                        content_text: String::new(),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    })
                    .collect(),
                outbox_effects: vec![],
                model_attempt_mutations: vec![],
                delegated_invocation_mutations: vec![],
                delegated_permission_mutations: vec![],
                effect_delivery_attempt_mutations: vec![],
                acknowledged_effect_ids: vec![],
                memory_usage_entries: vec![],
                // This fixture checks cold public Part materialization, not a
                // business result. Closing a message must not fabricate a Run seal.
                run_status: Some("running".into()),
                assistant_status: Some("completed".into()),
                assistant_status_message_id: None,
                committed_at_ms: 3,
            })
            .unwrap();
    }

    #[test]
    fn cold_public_projection_preserves_independent_tool_and_reasoning_fields() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("facts.db");
        let long_title = "独立长标题🙂".repeat(700);
        let previews = "独立技术预览🙂".repeat(500);
        let results = [
            None,
            Some(Value::Null),
            Some(json!("")),
            Some(json!([])),
            Some(json!({})),
            Some(json!({"content": "唯一结构化结果🙂".repeat(1200)})),
        ];
        let mut parts = vec![core_part(
            json!({"kind":"reasoning", "id":"reasoning", "title":long_title, "detail":"", "is_complete":true}),
        )];
        for (index, result) in results.iter().enumerate() {
            let mut part = json!({"kind":"tool", "id":format!("tool-{index}"), "call_id":format!("call-{index}"), "capability_id":"files.read",
                "title":long_title, "summary":"", "origin":{"kind":"mcp", "provider_id":"provider", "provider_title":long_title, "namespace":"files"},
                "state":"completed", "input":{"path":"actual input"}, "raw_input":"PRIVATE_RAW_INPUT_MARKER", "permission":{"request_id":"approval", "permission_domain":"files", "rationale":long_title, "decision":"allow_once"},
                "output_preview":previews, "full_output_ref":if index == 0 { Some("/PRIVATE_OUTPUT_PATH_MARKER") } else { None },
                "artifacts":[{"uri":"file:///output.pdf", "name":long_title, "mime_type":"application/pdf", "operation":null}], "started_at_ms":0, "finished_at_ms":u64::MAX});
            if let Some(result) = result {
                part["structured_output"] = result.clone();
            }
            parts.push(core_part(part));
        }
        persist_checkpoint(&database, &parts);
        let store = NolaStore::open(&database).unwrap();
        let full = load_conversation_projection_v1(&store, CONVERSATION).unwrap();
        let window =
            load_conversation_projection_window_v1(&store, CONVERSATION, None, 1, 32768).unwrap();
        let public = full
            .messages
            .iter()
            .find(|message| message.message_id.0 == MESSAGE)
            .unwrap();
        assert_eq!(&window.projection.messages[0], public);
        assert_eq!(public.message_id.0, MESSAGE);
        let committed_parts = &public.parts[1..];
        assert!(
            matches!(&committed_parts[0], MessagePartView::Reasoning { title, summary: Some(summary), .. } if title == &long_title && summary.is_empty())
        );
        for (index, expected) in results.iter().enumerate() {
            let MessagePartView::Tool {
                title,
                summary,
                origin,
                permission,
                input,
                output,
                output_preview,
                full_output,
                artifacts,
                started_at_ms,
                finished_at_ms,
                ..
            } = &committed_parts[index + 1]
            else {
                panic!("Tool missing")
            };
            assert_eq!(title, &long_title);
            assert_eq!(summary.as_deref(), Some(""));
            assert_eq!(origin.provider_title.as_ref(), Some(&long_title));
            assert_eq!(permission.as_ref().unwrap().rationale, long_title);
            assert_eq!(
                serde_json::to_value(input).unwrap(),
                json!({"path":"actual input"})
            );
            assert_eq!(
                output
                    .as_ref()
                    .map(|value| serde_json::to_value(value).unwrap()),
                *expected
            );
            assert_eq!(output_preview.as_ref(), Some(&previews));
            assert_eq!(artifacts[0].name, long_title);
            assert_eq!(artifacts[0].operation, None);
            assert_eq!(*started_at_ms, Some(0));
            assert_eq!(*finished_at_ms, Some(u64::MAX));
            if index == 0 {
                let owner = full_output.as_ref().unwrap();
                assert_eq!(owner.message_id.0, MESSAGE);
                assert_eq!(owner.part_id.0, "tool-0");
                assert_eq!(owner.tool_call_id.0, "call-0");
            } else {
                assert!(full_output.is_none());
            }
            committed_parts[index + 1]
                .validate_public_semantics(&public.message_id)
                .unwrap();
        }
        let wire = serde_json::to_string(&full).unwrap();
        assert!(!wire.contains("PRIVATE_RAW_INPUT_MARKER"));
        assert!(!wire.contains("PRIVATE_OUTPUT_PATH_MARKER"));
        let roundtrip: nola_domain::ConversationProjection = serde_json::from_str(&wire).unwrap();
        assert_eq!(roundtrip, full);
        assert!(matches!(
            &roundtrip
                .messages
                .iter()
                .find(|message| message.message_id.0 == MESSAGE)
                .unwrap()
                .parts[3],
            MessagePartView::Tool {
                output: Some(StructuredValue::Null),
                ..
            }
        ));
    }

    #[test]
    fn cold_public_projection_keeps_review_evidence_citation_and_artifact_identity() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("sources.db");
        let section = "章节与行号🙂".repeat(700);
        let evidence = json!({"kind":"memory", "usage":{"record_id":"record", "component_id":"component", "scope":"conversation", "namespace":{"kind":"custom","name":"资料🙂"},
            "provenance_kind":"tool_result", "source":"source", "title":"Memory evidence", "observed_at":"observed", "source_role":"tool", "event_time":"event", "range_start":"start", "range_end":"end",
            "requires_current_state_verification":true, "truncated":true, "supersedes":["prior"], "usage_count":u64::MAX, "last_used_at_epoch_ms":i64::MIN}});
        let mut parts = vec![core_part(
            json!({"kind":"source", "id":"source-part", "source_id":"source", "title":"Evidence", "uri":"memory://record", "mime_type":null, "evidence":evidence}),
        )];
        for (index, location) in [
            Value::Null,
            json!({}),
            json!({"page":u64::MAX, "line_start":0,"line_end":u64::MAX,"section":section}),
        ]
        .into_iter()
        .enumerate()
        {
            parts.push(core_part(json!({"kind":"citation", "id":format!("citation-part-{index}"), "citation_id":format!("citation-identity-{index}"), "source_id":"source", "title":"Quote", "uri":"file:///output.pdf", "mime_type":"application/pdf",
                "location":location, "range":{"start":0,"end":u64::MAX}, "origin_tool_call_id":"call"})));
        }
        parts.push(core_part(json!({"kind":"artifact","id":"artifact","artifact":{"uri":"file:///output.pdf", "name":"Output", "mime_type":"application/pdf","operation":"write"}})));
        for (index, review) in [
            None,
            Some(TextReviewState::Pending),
            Some(TextReviewState::Rejected),
            Some(TextReviewState::Interrupted),
            Some(TextReviewState::Approved),
        ]
        .into_iter()
        .enumerate()
        {
            parts.push(core_part(json!({"kind":"text", "id":format!("review-{index}"), "text":if matches!(review, Some(state) if state != TextReviewState::Approved) { "PRIVATE_CANDIDATE_BODY_MARKER" } else { "approved" }, "is_complete":true, "review":review})));
        }
        persist_checkpoint(&database, &parts);
        let store = NolaStore::open(&database).unwrap();
        let projection = load_conversation_projection_v1(&store, CONVERSATION).unwrap();
        let public = projection
            .messages
            .iter()
            .find(|message| message.message_id.0 == MESSAGE)
            .unwrap();
        let committed_parts = &public.parts[1..];
        let MessagePartView::Source {
            evidence: Some(actual),
            ..
        } = &committed_parts[0]
        else {
            panic!("source evidence missing")
        };
        assert_eq!(serde_json::to_value(actual).unwrap(), evidence);
        for (index, expected_location) in [
            None,
            Some(SourceLocation::default()),
            Some(SourceLocation {
                page: Some(u64::MAX),
                line_start: Some(0),
                line_end: Some(u64::MAX),
                section: Some(section.clone()),
            }),
        ]
        .into_iter()
        .enumerate()
        {
            let MessagePartView::Citation {
                citation_id,
                mime_type,
                location,
                section: actual_section,
                range_start,
                range_end,
                ..
            } = &committed_parts[index + 1]
            else {
                panic!("citation missing")
            };
            assert_eq!(citation_id, &format!("citation-identity-{index}"));
            assert_eq!(mime_type.as_deref(), Some("application/pdf"));
            assert_eq!(location, &expected_location);
            assert_eq!(
                actual_section.as_ref(),
                expected_location
                    .as_ref()
                    .and_then(|value| value.section.as_ref())
            );
            assert_eq!(*range_start, Some(0));
            assert_eq!(*range_end, Some(u64::MAX));
        }
        assert!(
            matches!(&committed_parts[4], MessagePartView::Attachment { operation: Some(operation), .. } if operation == "write")
        );
        for part in &public.parts {
            part.validate_public_semantics(&public.message_id).unwrap();
        }
        let wire = serde_json::to_string(&projection).unwrap();
        assert!(!wire.contains("PRIVATE_CANDIDATE_BODY_MARKER"));
        assert_eq!(
            serde_json::from_str::<nola_domain::ConversationProjection>(&wire).unwrap(),
            projection
        );
        for (index, expected_review) in [
            None,
            Some(TextReviewState::Pending),
            Some(TextReviewState::Rejected),
            Some(TextReviewState::Interrupted),
            Some(TextReviewState::Approved),
        ]
        .into_iter()
        .enumerate()
        {
            let MessagePartView::Text {
                review, markdown, ..
            } = &committed_parts[index + 5]
            else {
                panic!("review missing")
            };
            assert_eq!(*review, expected_review);
            assert_eq!(
                markdown.is_empty(),
                matches!(expected_review, Some(state) if state != TextReviewState::Approved)
            );
        }
    }
}

mod stored_public_part_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Canonical transcript admission/query/cold reopen retains bounded public facts.
    use nola_application::{
        load_conversation_projection_page_v1, load_conversation_projection_v1,
        load_conversation_projection_window_v1,
    };
    use nola_domain::{HistoricalProjectionStatus, MessagePartView, MessageView, SemanticValue};
    use nola_store::NolaStore;
    use serde_json::{json, Value};

    const CONVERSATION: &str = "stored-public-conversation";
    const MESSAGE: &str = "stored-public-assistant";

    fn open(store: &mut NolaStore) {
        store
            .create_conversation(
                &nola_store_port::conversation_profile::CreateConversationInput {
                    id: CONVERSATION.into(),
                    title: "Public facts".into(),
                    revision: 1,
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            )
            .unwrap();
    }

    fn tool(index: usize, output: Option<Value>) -> Value {
        let mut value = json!({
            "kind":"tool", "part_id":format!("tool-{index}"), "revision":2,
            "execution_source":{"kind":"model"},
            "tool_call_id":format!("call-{index}"), "capability_id":"files.write",
            "title":"真实工具标题 α", "summary":"", "state":"completed",
            "input":{"path":"actual input"},
            "origin":{"kind":"mcp", "provider_title":"提供方"},
            "permission":{"request_id":"approval", "permission_domain":"files", "rationale":"仅展示", "decision":"allow_once"},
            "output_preview":"独立预览", "full_output":{"message_id":MESSAGE, "part_id":format!("tool-{index}"), "tool_call_id":format!("call-{index}")},
            "artifacts":[{"uri":"file:///output.pdf", "name":"PDF", "mime_type":"application/pdf", "operation":"create"}],
            "started_at_ms":0, "finished_at_ms":u64::MAX, "affected_resources":[]
        });
        if let Some(output) = output {
            value["output"] = output;
        }
        value
    }

    fn commit(store: &mut NolaStore, parts: Vec<Value>) -> MessageView {
        // Canonical transcript fixtures exercise persisted public semantics without
        // reviving the removed full-aggregate remote write protocol.
        let message: MessageView = serde_json::from_value(json!({
            "message_id":MESSAGE,"role":"assistant","state":"completed","revision":2,"created_at_ms":1,"parts":parts
        })).unwrap();
        for part in &message.parts {
            part.validate_public_semantics(&nola_domain::MessageId(MESSAGE.into()))
                .unwrap();
        }
        let canonical_parts = parts
            .into_iter()
            .enumerate()
            .map(|(index, part)| nola_domain::MessagePartInput {
                id: part["part_id"].as_str().unwrap().into(),
                kind: part["kind"].as_str().unwrap().into(),
                ordinal: index as i64 + 1,
                state: part["state"].as_str().unwrap_or("completed").into(),
                content_text: part["markdown"]
                    .as_str()
                    .or_else(|| part["summary"].as_str())
                    .unwrap_or("")
                    .into(),
                content_json: Some(part.to_string()),
                content_digest: None,
                created_at_ms: 1,
                updated_at_ms: 3,
            })
            .collect();
        use nola_store_port::transcript_write::{AppendMessagesInput, MessageInput};
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: CONVERSATION.into(),
                expected_message_count: 0,
                updated_at_ms: 3,
                messages: vec![
                    MessageInput {
                        id: "user".into(),
                        run_id: None,
                        role: "user".into(),
                        status: "completed".into(),
                        ordinal: 1,
                        created_at_ms: 1,
                        completed_at_ms: Some(1),
                        parts: vec![],
                    },
                    MessageInput {
                        id: MESSAGE.into(),
                        run_id: None,
                        role: "assistant".into(),
                        status: "completed".into(),
                        ordinal: 2,
                        created_at_ms: 1,
                        completed_at_ms: Some(3),
                        parts: canonical_parts,
                    },
                ],
            })
            .unwrap();
        message
    }

    #[test]
    fn canonical_commit_live_query_and_cold_reopen_preserve_typed_public_parts() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("public.db");
        let mut store = NolaStore::open(&database).unwrap();
        open(&mut store);
        let mut parts: Vec<_> = [
            None,
            Some(Value::Null),
            Some(json!("")),
            Some(json!([])),
            Some(json!({})),
            Some(json!({"number":u64::MAX})),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, output)| tool(index, output))
        .collect();
        parts.extend([
            json!({"kind":"reasoning", "part_id":"reasoning", "revision":2, "state":"completed", "title":"独立推理标题", "summary":"独立正文"}),
            json!({"kind":"source", "part_id":"source-part", "revision":2, "source_id":"source", "title":"Evidence", "uri":"",
                "evidence":{"kind":"memory", "usage":{"record_id":"record", "component_id":"component", "namespace":{"kind":"workspace"}, "usage_count":u64::MAX}}}),
            json!({"kind":"citation", "part_id":"citation", "revision":2, "citation_id":"real-citation", "source_id":"source", "title":"Citation", "uri":"",
                "mime_type":"application/pdf", "origin_tool_call_id":"call-0", "location":{"page":0, "section":"方法"}, "page":0, "section":"方法", "range_start":7, "range_end":12}),
            json!({"kind":"citation", "part_id":"empty-location", "revision":2, "citation_id":"empty-location", "source_id":"source", "title":"", "uri":" ", "origin_tool_call_id":"call-0", "location":{}}),
            json!({"kind":"attachment", "part_id":"attachment", "revision":2, "attachment_id":"artifact", "name":"PDF", "uri":"file:///output.pdf", "operation":"create"}),
            json!({"kind":"text", "part_id":"answer", "revision":2, "markdown":"Visible answer", "is_final":true, "review":"approved"}),
            json!({"kind":"text", "part_id":"withheld", "revision":2, "markdown":"", "is_final":true, "review":"pending"}),
        ]);
        let expected = commit(&mut store, parts).parts;
        let mut canonical = None;
        for application in [store, NolaStore::open(&database).unwrap()] {
            let full = load_conversation_projection_v1(&application, CONVERSATION).unwrap();
            let window =
                load_conversation_projection_window_v1(&application, CONVERSATION, None, 1, 32768)
                    .unwrap();
            let page = load_conversation_projection_page_v1(&application, CONVERSATION, 1, 1).unwrap();
            for projection in [full, window.projection, page] {
                let actual = &projection
                    .messages
                    .iter()
                    .find(|message| message.message_id.0 == MESSAGE)
                    .unwrap()
                    .parts;
                assert_eq!(actual.len(), expected.len());
                for (actual, expected) in actual.iter().zip(&expected) {
                    let mut expected = serde_json::to_value(expected).unwrap();
                    expected["revision"] = json!(projection.revision);
                    assert_eq!(serde_json::to_value(actual).unwrap(), expected);
                    assert!(!matches!(actual, MessagePartView::Historical { .. }));
                    actual
                        .validate_public_semantics(&nola_domain::MessageId(MESSAGE.into()))
                        .unwrap();
                }
                if let Some(previous) = &canonical {
                    assert_eq!(actual, previous);
                }
                canonical = Some(actual.clone());
            }
        }
    }

    #[test]
    fn oversized_remote_result_remains_deferred_original_not_empty_typed_output() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("deferred.db");
        let mut store = NolaStore::open(&database).unwrap();
        open(&mut store);
        let output = "actual result🙂".repeat(20_000);
        commit(&mut store, vec![tool(0, Some(json!({"result":output})))]);
        drop(store);
        let store = NolaStore::open(&database).unwrap();
        let window =
            load_conversation_projection_window_v1(&store, CONVERSATION, None, 1, 1024).unwrap();
        let MessagePartView::Historical { projection, .. } = &window.projection.messages[0].parts[0]
        else {
            panic!("Deferred original must not become a complete/null/empty tool result")
        };
        assert_eq!(projection.owner.message_id(), MESSAGE);
        assert_eq!(projection.owner.part_id(), "tool-0");
        assert_eq!(projection.status, HistoricalProjectionStatus::Deferred);
        assert!(matches!(
            projection.fields["output"],
            SemanticValue::Deferred { .. }
        ));
        let json = store
            .load_conversation(CONVERSATION)
            .unwrap()
            .unwrap()
            .messages
            .into_iter()
            .find(|message| message.id == MESSAGE)
            .unwrap()
            .parts
            .remove(0)
            .content_json
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&json).unwrap()["output"]["result"],
            output
        );
    }

    #[test]
    fn unknown_private_fields_in_stored_public_source_cannot_be_filtered_into_valid_cards() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("unknown.db");
        let mut store = NolaStore::open(&database).unwrap();
        open(&mut store);
        commit(&mut store, vec![tool(0, Some(Value::Null))]);
        drop(store);
        // Isolated corruption fixture exercises the actual persisted-source reader,
        // rather than an already-filtered public dictionary.
        let connection = rusqlite::Connection::open(&database).unwrap();
        let mut source = tool(0, Some(Value::Null));
        source["private_grant"] = json!({"token":"PRIVATE_GRANT_MARKER"});
        connection
            .execute(
                "UPDATE message_parts SET content_json=?1 WHERE id='tool-0'",
                rusqlite::params![source.to_string()],
            )
            .unwrap();
        drop(connection);
        let store = NolaStore::open(&database).unwrap();
        let projection = load_conversation_projection_window_v1(&store, CONVERSATION, None, 1, 32768)
            .unwrap()
            .projection;
        let MessagePartView::Historical {
            projection: original,
            ..
        } = &projection.messages[0].parts[0]
        else {
            panic!("Unknown source fields cannot silently become a strict public Tool")
        };
        assert_eq!(original.owner.part_id(), "tool-0");
        assert!(
            matches!(&original.status, HistoricalProjectionStatus::Unsupported { code } if code == "schema_unknownfield")
        );
        assert!(original.fields.is_empty());
        assert!(!serde_json::to_string(&projection)
            .unwrap()
            .contains("PRIVATE_GRANT_MARKER"));
    }
}

mod kernel_projection_journal_e2e {
    // SPDX-License-Identifier: Apache-2.0
    // Real Application receipts and SQLite journal consumed by the shared Kernel.

    use crate::support;
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceiptOutcome, ApplicationProjectionScope,
        ExecuteReceivedCommand,
    };
    use nola_kernel::{ConversationProjectionReducer, ProjectionApplyError, ProjectionApplyOutcome};
    use nola_store::{CreateConversationInput, NolaStore};

    #[test]
    fn damaged_journal_rejects_the_whole_page_instead_of_skipping_a_sequence() {
        use nola_domain::{
            ConversationId, ConversationProjectionEvent, ConversationProjectionEventIdentity,
            ConversationProjectionMutation, RunEventId,
        };
        use nola_store::{AppendProjectionEventInput, ProjectionScopeKey};

        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("sequence-gap.db");
        let mut store = NolaStore::open(&database).unwrap();
        for sequence in 1..=3 {
            let event = ConversationProjectionEvent {
                identity: ConversationProjectionEventIdentity {
                    event_id: RunEventId(format!("gap-{sequence}")),
                    conversation_id: ConversationId("gap".into()),
                    run_id: None,
                    delegated_invocation_id: None,
                    attempt_id: None,
                    sequence,
                },
                mutation: ConversationProjectionMutation::SetTitle {
                    title: format!("Title {sequence}"),
                    revision: sequence,
                },
            };
            store
                .append_projection_event(&AppendProjectionEventInput {
                    event_id: event.identity.event_id.0.clone(),
                    scope: ProjectionScopeKey {
                        scope_type: "conversation".into(),
                        scope_id: "gap".into(),
                    },
                    expected_after_sequence: sequence - 1,
                    payload_json: serde_json::to_string(&event).unwrap(),
                    occurred_at: "2026-09-05T00:00:00Z".into(),
                })
                .unwrap();
        }
        drop(store);
        let connection = rusqlite::Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .execute("DELETE FROM projection_events WHERE event_id = 'gap-2'", [],)
                .unwrap(),
            1
        );
        drop(connection);
        let runtime = support::open_runtime(&database).unwrap();
        let scope = || ApplicationProjectionScope {
            scope_type: "conversation".into(),
            scope_id: "gap".into(),
        };
        let first = runtime.projection_event_suffix(scope(), 0, 1).unwrap();
        assert_eq!(first.events.len(), 1);
        assert!(first.has_more);
        for after_sequence in [0, 1] {
            let error = runtime
                .projection_event_suffix(scope(), after_sequence, 10)
                .unwrap_err();
            assert!(error.contains("expected 2, received 3"), "{error}");
        }
        assert!(runtime
            .projection_event_suffix(scope(), 4, 10)
            .unwrap_err()
            .contains("ahead of the journal cursor"));
        assert!(runtime
            .projection_event_suffix(scope(), 0, 0)
            .unwrap_err()
            .contains("page limit must be positive"));
        let caught_up = runtime.projection_event_suffix(scope(), 3, 10).unwrap();
        assert!(caught_up.events.is_empty());
        assert!(!caught_up.has_more);

        drop(runtime);
        let connection = rusqlite::Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .execute("DELETE FROM projection_events WHERE event_id = 'gap-3'", [],)
                .unwrap(),
            1
        );
        drop(connection);
        let runtime = support::open_runtime(&database).unwrap();
        let error = runtime.projection_event_suffix(scope(), 1, 10).unwrap_err();
        assert!(error.contains("suffix is missing after 1"), "{error}");
    }

    #[test]
    fn journal_rejects_foreign_conversation_payload_before_delivery() {
        use nola_domain::{
            ConversationId, ConversationProjectionEvent, ConversationProjectionEventIdentity,
            ConversationProjectionMutation, RunEventId,
        };
        use nola_store::{AppendProjectionEventInput, ProjectionScopeKey};

        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("foreign-event.db");
        let mut store = NolaStore::open(&database).unwrap();
        let event = ConversationProjectionEvent {
            identity: ConversationProjectionEventIdentity {
                event_id: RunEventId("foreign-event".into()),
                conversation_id: ConversationId("other-conversation".into()),
                run_id: None,
                delegated_invocation_id: None,
                attempt_id: None,
                sequence: 1,
            },
            mutation: ConversationProjectionMutation::SetTitle {
                title: "Must not be delivered".into(),
                revision: 1,
            },
        };
        store
            .append_projection_event(&AppendProjectionEventInput {
                event_id: "foreign-event".into(),
                scope: ProjectionScopeKey {
                    scope_type: "conversation".into(),
                    scope_id: "requested".into(),
                },
                expected_after_sequence: 0,
                payload_json: serde_json::to_string(&event).unwrap(),
                occurred_at: "2026-09-05T00:00:00Z".into(),
            })
            .unwrap();
        drop(store);
        let runtime = support::open_runtime(&database).unwrap();
        let error = runtime
            .projection_event_suffix(
                ApplicationProjectionScope {
                    scope_type: "conversation".into(),
                    scope_id: "requested".into(),
                },
                0,
                10,
            )
            .unwrap_err();
        assert!(error.contains("belongs to another conversation"), "{error}");
        for (scope_type, scope_id) in [("conversations", ""), ("conversation", "")] {
            assert!(runtime
                .projection_event_suffix(
                    ApplicationProjectionScope {
                        scope_type: scope_type.into(),
                        scope_id: scope_id.into(),
                    },
                    0,
                    10
                )
                .unwrap_err()
                .contains("nonempty conversation scope"));
        }
    }

    #[test]
    fn metadata_invalidation_requires_fresh_snapshot_after_restart_and_redelivery() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("kernel-journal.db");
        let mut store = NolaStore::open(&database).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "journal".into(),
                title: "Initial".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        drop(store);
        let mut runtime = support::open_runtime(&database).unwrap();
        let initial = runtime.conversation_projection_snapshot("journal").unwrap();
        let mut reducer = ConversationProjectionReducer::restore(
            initial.projection.unwrap(),
            initial.current_sequence,
        );

        for (index, title) in ["First", "Second", "Final"].into_iter().enumerate() {
            let command = ApplicationCommand::RenameConversation {
                title_source: None,
                conversation_id: "journal".into(),
                title: title.into(),
                updated_at_ms: index as i64 + 2,
            };
            let receipt = runtime
                .execute_received_command(
                    ExecuteReceivedCommand {
                        client_id: "kernel-client".into(),
                        request_id: format!("rename-{index}"),
                        command_type: "rename_conversation".into(),
                        command_json: serde_json::to_string(&command).unwrap(),
                        expected_revision: None,
                        received_at: "2026-09-05T00:00:00Z".into(),
                    },
                    command,
                )
                .unwrap();
            assert!(matches!(
                receipt.outcome,
                ApplicationCommandReceiptOutcome::Committed { .. }
            ));
            drop(runtime);
            runtime = support::open_runtime(&database).unwrap();
            reducer = ConversationProjectionReducer::resume(reducer.into_checkpoint());
            let authoritative = runtime.conversation_projection_snapshot("journal").unwrap();
            assert!(authoritative.current_sequence > reducer.last_sequence());
            let mut pages = 0;
            while reducer.last_sequence() < authoritative.current_sequence {
                pages += 1;
                assert!(pages <= 32, "journal paging must make bounded progress");
                let suffix = runtime
                    .projection_event_suffix(
                        ApplicationProjectionScope {
                            scope_type: "conversation".into(),
                            scope_id: "journal".into(),
                        },
                        reducer.last_sequence(),
                        1,
                    )
                    .unwrap();
                assert_eq!(suffix.events.len(), 1);
                assert_eq!(suffix.current_sequence, authoritative.current_sequence);
                let events: Vec<_> = suffix.events.into_iter().map(|value| value.event).collect();
                let before = reducer.projection().clone();
                let before_sequence = reducer.last_sequence();
                let error = reducer.apply_batch(events.clone()).unwrap_err();
                assert!(
                    matches!(error, ProjectionApplyError::SnapshotRequired { revision }
                    if revision == index as u64 + 2)
                );
                assert_eq!(
                    reducer.projection(),
                    &before,
                    "invalidation cannot publish a partial title-only state"
                );
                assert_eq!(
                    reducer.last_sequence(),
                    before_sequence,
                    "invalidation is unacknowledged until snapshot refresh"
                );
                let refresh = runtime.conversation_projection_snapshot("journal").unwrap();
                reducer = ConversationProjectionReducer::restore(
                    refresh.projection.unwrap(),
                    refresh.current_sequence,
                );
                let committed = reducer.projection().clone();
                let cursor = reducer.last_sequence();
                assert_eq!(
                    reducer.apply_batch(events),
                    Ok(vec![ProjectionApplyOutcome::Duplicate])
                );
                assert_eq!(reducer.last_sequence(), cursor);
                assert_eq!(reducer.projection(), &committed);
            }
            assert_eq!(
                reducer.projection(),
                authoritative.projection.as_ref().unwrap()
            );
            assert_eq!(reducer.last_sequence(), authoritative.current_sequence);
        }
    }
}
