// SPDX-License-Identifier: Apache-2.0
//! Selection budgets preserve every Part and expose revision-pinned recovery.
mod support;
use nola_application::ApplicationCommand;
use nola_agent_engine::agent::parts::{AgentPart, PartOwnership, ReasoningPart};
use nola_domain::{
    CanonicalPartOwner, HistoricalPartProjection, HistoricalProjectionStatus, MessagePartView,
    SemanticValue,
};
use nola_protocol::HistoricalProjectionCommand;
use nola_store::NolaStore;

#[test]
fn exhausted_window_retains_part_identity_and_independent_history_pagination() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.db");
    drop(NolaStore::open(&path).unwrap());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','History',1,1,1)", [])
        .unwrap();
    for message in 1..=2 {
        let id = format!("m{message}");
        db.execute(
            "INSERT INTO messages VALUES(?1,'c','historical-turn','assistant','completed',?2,1,1)",
            rusqlite::params![id, message],
        )
        .unwrap();
        for part in 1..=12 {
            let part_id = format!("p{message}-{part}");
            let raw = serde_json::to_string(&AgentPart::Reasoning(ReasoningPart {
                id: part_id.clone(),
                ownership: PartOwnership {
                    message_id: id.clone(),
                    turn_id: "historical-turn".into(),
                },
                title: "Historical reasoning".into(),
                detail: format!("marker-{message}-{part} {}", "x".repeat(4000)),
                is_complete: true,
            }))
            .unwrap();
            db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms) VALUES(?1,?2,'reasoning',?3,'completed','',?4,1,1)", rusqlite::params![part_id,id,part,raw]).unwrap();
        }
    }
    // The historical messages share one actual admitted Run. Their run_id
    // fields alone cannot authorize journal-backed historical reads.
    db.execute(
        "INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms)
         VALUES('historical-turn','c','conversation','m1','running',1,1,1,NULL)",
        [],
    ).unwrap();
    let store = NolaStore::open(&path).unwrap();
    let window =
        nola_application::load_conversation_projection_window_v1(&store, "c", None, 2, 200000)
            .unwrap();
    assert_eq!(window.projection.messages.len(), 2);
    assert_eq!(
        window
            .projection
            .messages
            .iter()
            .map(|m| m.parts.len())
            .sum::<usize>(),
        24
    );
    let mut available_bytes = 0;
    let mut pending = Vec::new();
    for message in &window.projection.messages {
        for part in &message.parts {
            let MessagePartView::Historical {
                part_id,
                projection,
                ..
            } = part
            else {
                panic!("lost historical node")
            };
            assert_eq!(projection.owner.part_id(), part_id.0);
            assert_eq!(projection.owner.message_id(), message.message_id.0);
            if matches!(
                projection.status,
                HistoricalProjectionStatus::NotLoaded { .. }
            ) {
                assert!(
                    projection.source.is_none()
                        && projection.binding.is_none()
                        && projection.schema.is_none()
                );
                assert!(projection.fields.is_empty());
                pending.push(projection.owner.clone());
            } else {
                available_bytes += serde_json::to_vec(projection).unwrap().len();
            }
        }
    }
    assert!(!pending.is_empty());
    assert!(available_bytes <= 32768);
    assert!(
        serde_json::to_vec(&window).unwrap().len() < 65536,
        "placeholders count toward encoded metadata"
    );
    let newest =
        nola_application::load_conversation_projection_window_v1(&store, "c", None, 1, 200000)
            .unwrap();
    assert_eq!(newest.projection.messages[0].message_id.0, "m2");
    assert!(newest.has_more);
    let older = nola_application::load_conversation_projection_window_v1(
        &store,
        "c",
        newest.before_ordinal,
        1,
        200000,
    )
    .unwrap();
    assert_eq!(older.projection.messages[0].message_id.0, "m1");
    assert!(!older.has_more);
    assert_eq!(newest.projection.messages[0].parts.len(), 12);
    assert_eq!(older.projection.messages[0].parts.len(), 12);
    drop(store);
    let mut runtime = support::open_runtime(&path).unwrap();
    for owner in pending {
        let command: ApplicationCommand = HistoricalProjectionCommand::ReadHistoricalPart {
            owner: owner.clone(),
            expected_conversation_revision: 1,
            maximum_output_bytes: 8192,
        }
        .into();
        let complete: HistoricalPartProjection =
            serde_json::from_value(runtime.execute(command).unwrap()).unwrap();
        assert_eq!(complete.owner, owner);
        assert_eq!(complete.status, HistoricalProjectionStatus::Supported);
        assert!(matches!(
            complete.fields.get("detail"),
            Some(SemanticValue::Complete { .. })
        ));
    }
    assert!(runtime
        .execute(
            HistoricalProjectionCommand::ReadHistoricalPart {
                owner: CanonicalPartOwner::new("c".into(), "m2".into(), "p2-12".into()).unwrap(),
                expected_conversation_revision: 2,
                maximum_output_bytes: 8192,
            }
            .into()
        )
        .is_err());
    assert!(runtime
        .execute(
            HistoricalProjectionCommand::ReadHistoricalPart {
                owner: CanonicalPartOwner::new("c".into(), "m1".into(), "p2-12".into()).unwrap(),
                expected_conversation_revision: 1,
                maximum_output_bytes: 8192,
            }
            .into()
        )
        .is_err());
}

#[test]
fn on_demand_budget_cannot_be_exceeded_by_escaped_owner_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("escaped.db");
    drop(NolaStore::open(&path).unwrap());
    let db = rusqlite::Connection::open(&path).unwrap();
    let conversation = format!("c{}", "\"".repeat(127));
    let message = format!("m{}", "\"".repeat(127));
    let part = format!("p{}", "\"".repeat(127));
    let kind = "\"".repeat(128);
    db.execute(
        "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES(?1,'History',1,1,1)",
        [&conversation],
    )
    .unwrap();
    db.execute(
        "INSERT INTO messages VALUES(?1,?2,NULL,'user','completed',1,1,1)",
        rusqlite::params![message, conversation],
    )
    .unwrap();
    db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,1,'completed','','{}',1,1)", rusqlite::params![part,message,kind]).unwrap();
    let owner = CanonicalPartOwner::new(conversation, message, part).unwrap();
    let mut runtime = support::open_runtime(&path).unwrap();
    let result = runtime.execute(
        HistoricalProjectionCommand::ReadHistoricalPart {
            owner: owner.clone(),
            expected_conversation_revision: 1,
            maximum_output_bytes: 1024,
        }
        .into(),
    );
    assert!(
        result.is_err(),
        "a not-loaded placeholder must fit the requested frame budget"
    );
    let result = runtime
        .execute(
            HistoricalProjectionCommand::ReadHistoricalPart {
                owner,
                expected_conversation_revision: 1,
                maximum_output_bytes: 8192,
            }
            .into(),
        )
        .unwrap();
    assert!(serde_json::to_vec(&result).unwrap().len() <= 8192);
}

#[test]
fn imported_reviewed_text_obeys_disclosure_in_windows_details_and_ranges() {
    use nola_agent_engine::agent::parts::{AgentPart, PartOwnership, TextPart, TextReviewState};
    for review in [
        None,
        Some(TextReviewState::Approved),
        Some(TextReviewState::Pending),
        Some(TextReviewState::Rejected),
        Some(TextReviewState::Interrupted),
    ] {
        let public = review.is_none() || review == Some(TextReviewState::Approved);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reviewed-history.db");
        drop(NolaStore::open(&path).unwrap());
        let db = rusqlite::Connection::open(&path).unwrap();
        let text = "CANDIDATE_ONLY 中文🙂\n".repeat(2048);
        let raw = serde_json::to_string(&AgentPart::Text(TextPart {
            id: "p".into(),
            ownership: PartOwnership {
                message_id: "m".into(),
                turn_id: "history".into(),
            },
            text: text.clone(),
            is_complete: true,
            review,
        }))
        .unwrap();
        db.execute_batch(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Review history',1,1,1);
            INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,1,1);",
        )
        .unwrap();
        db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
            VALUES('p','m','text',1,'completed',?1,?2,1,1)", rusqlite::params![text, raw]).unwrap();
        drop(db);
        let mut store = NolaStore::open(&path).unwrap();
        // Raw fixture writes stage an unpublished generation. Reads must not
        // repair it on open; publish through the same transaction boundary as
        // official imports before exercising the reviewed disclosure contract.
        assert!(nola_application::load_conversation_projection_window_v1(
            &store, "c", None, 1, 512,
        ).is_err());
        store.begin_write_unit_of_work().unwrap();
        store.commit_write_unit_of_work().unwrap();
        assert!(
            store.load_checkpoint("c").unwrap().is_none(),
            "Historical display must not depend on a live Core"
        );
        let window =
            nola_application::load_conversation_projection_window_v1(&store, "c", None, 1, 512)
                .unwrap();
        assert_eq!(window.deferred_text_parts.len(), 1);
        let reference = &window.deferred_text_parts[0];
        assert!(reference.is_final);
        assert_eq!(reference.can_read_original, public);
        if !public {
            let encoded = serde_json::to_string(&window).unwrap();
            assert!(
                !encoded.contains("CANDIDATE_ONLY"),
                "Window revived {review:?} candidate"
            );
        }
        drop(store);
        let mut runtime = support::open_runtime(&path).unwrap();
        let projection: HistoricalPartProjection = serde_json::from_value(
            runtime
                .execute(
                    HistoricalProjectionCommand::ReadHistoricalPart {
                        owner: CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
                        expected_conversation_revision: 1,
                        maximum_output_bytes: 8192,
                    }
                    .into(),
                )
                .unwrap(),
        )
        .unwrap();
        if !public {
            assert!(projection.canonical_text.is_none());
            assert!(!projection.fields.contains_key("text"));
            assert!(!serde_json::to_string(&projection)
                .unwrap()
                .contains("CANDIDATE_ONLY"));
        } else {
            assert!(projection
                .canonical_text
                .as_ref()
                .is_some_and(|p| p.text.starts_with("CANDIDATE_ONLY")));
        }
        let range = runtime.execute(ApplicationCommand::ReadConversationTextRange {
            conversation_id: "c".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            offset_bytes: 0,
            limit_bytes: 32,
        });
        assert_eq!(range.is_ok(), public, "range review={review:?}");
        if let Ok(range) = range {
            assert!(range["text"]
                .as_str()
                .unwrap()
                .starts_with("CANDIDATE_ONLY"));
        }
        let search = runtime
            .execute(ApplicationCommand::SearchConversationText {
                conversation_id: "c".into(),
                query: "CANDIDATE_ONLY".into(),
                limit: 1,
            })
            .unwrap();
        assert_eq!(
            search.as_array().unwrap().len(),
            usize::from(public),
            "search review={review:?}"
        );
        drop(runtime);
        let cold = NolaStore::open(&path).unwrap();
        assert_eq!(
            cold.load_conversation_message_window("c", None, 1, 512)
                .unwrap()
                .unwrap()
                .deferred_text_parts[0]
                .can_read_original,
            public
        );
    }
}
