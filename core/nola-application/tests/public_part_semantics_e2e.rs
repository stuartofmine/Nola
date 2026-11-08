// SPDX-License-Identifier: Apache-2.0
//! Cold SQLite checkpoint -> real Application public projection. No model fixture
//! can conceal dropped metadata, private output paths or null-result absence.

#[path = "support/model_identity.rs"]
mod fixture_model_identity;
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
    assert!(
        matches!(&public.parts[0], MessagePartView::Reasoning { title, summary: Some(summary), .. } if title == &long_title && summary.is_empty())
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
        } = &public.parts[index + 1]
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
        public.parts[index + 1]
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
            .parts[2],
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
    let MessagePartView::Source {
        evidence: Some(actual),
        ..
    } = &public.parts[0]
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
        } = &public.parts[index + 1]
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
        matches!(&public.parts[4], MessagePartView::Attachment { operation: Some(operation), .. } if operation == "write")
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
        } = &public.parts[index + 5]
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
