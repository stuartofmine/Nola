// SPDX-License-Identifier: Apache-2.0
mod support;
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
