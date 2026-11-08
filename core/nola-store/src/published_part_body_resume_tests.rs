// SPDX-License-Identifier: Apache-2.0
//! Completed sealed fields survive real internal continuation, not arbitrary
//! changes to messages.run_id or a new unfinished field publication.
use super::*;
use crate::{BeginInternalTurnInput, NolaStore};
use nola_domain::{CanonicalBodyReference, PartBodyField};
#[path = "../tests/support/terminal_sources.rs"]
mod terminal_sources;

fn fixture(path: &std::path::Path) -> NolaStore {
    let mut store = NolaStore::open(path).unwrap();
    store.begin_turn(&crate::BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None, expected_parent_message_id: None,
        initial_events: vec![], initial_assistant_part_mutations: vec![], conversation_id: "c".into(), conversation_title: "sealed".into(),
        user_message_id: "u".into(), user_part_id: "up".into(), user_text: "Continue".into(),
        user_content_json: None, additional_user_parts: vec![], assistant_message_id: "m".into(),
        assistant_part_id: "p".into(), assistant_turn_bindings: vec![], run_id: "old".into(), context_checkpoint_id: None,
        checkpoint_revision: 1, checkpoint_event_cursor: 0, checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(), invocation_config_digest: "fixture-config".into(),
        outbox_effects: vec![], model_attempt_mutations: vec![], delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![], memory_usage_entries: vec![], created_at_ms: 1,
    }).unwrap();
    // Field publication is independent of business completion; the latter is
    // established below from original requirement/result sources.
    store.connection.execute_batch("UPDATE message_parts SET state='completed' WHERE id='p';
        UPDATE messages SET status='completed',completed_at_ms=2 WHERE id='m';").unwrap();
    for (id, field, text) in [
        ("text", PartBodyField::ContentText, "sealed中🙂"),
        (
            "json",
            PartBodyField::ContentJson,
            "{\"exact\":9007199254740993}",
        ),
    ] {
        let reference = CanonicalBodyReference::for_field(
            id.into(),
            "c".into(),
            "m".into(),
            "p".into(),
            field,
            text.len() as u64,
            1,
        )
        .unwrap();
        let encoded = serde_json::to_string(&reference).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO canonical_text_objects VALUES(?1,?2)",
                params![id, encoded],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO canonical_text_chunks VALUES(?1,0,?2)",
                params![id, text],
            )
            .unwrap();
        store.connection.execute("INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES('p',?1,?2)",params![field_name(field),encoded]).unwrap();
    }
    terminal_sources::settle_fixture(&mut store, "c", "old", "old", "u");
    store
}
fn resume(store: &mut NolaStore, run: &str, revision: i64) -> StoreResult<crate::BeginTurnResult> {
    let target = store.internal_assistant_target("c")?.unwrap();
    let expected_checkpoint_revision = store.checkpoint_revision("c")?;
    store.begin_internal_turn(&BeginInternalTurnInput {
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        conversation_id: "c".into(),
        assistant_message_id: target.assistant_message_id,
        expected_previous_run_id: target.run_id,
        expected_checkpoint_revision,
        assistant_part_id: "unused-part".into(),
        assistant_turn_bindings: vec![],
        run_id: run.into(),
        context_checkpoint_id: None,
        checkpoint_revision: revision + 1,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "fixture-config".into(),
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        memory_usage_entries: vec![],
        created_at_ms: revision + 2,
    })
}
fn assert_originals(store: &NolaStore) {
    for (field, expected) in [
        (PartBodyField::ContentText, "sealed中🙂"),
        (PartBodyField::ContentJson, "{\"exact\":9007199254740993}"),
    ] {
        let (_, range) = field_range(&store.connection, "c", "m", "p", field, 0, 128).unwrap();
        assert_eq!(range.text, expected);
        assert!(range.end_of_content);
    }
    let loaded = store.load_conversation("c").unwrap().unwrap();
    let part = &loaded.messages.iter().find(|m| m.id == "m").unwrap().parts[0];
    assert_eq!(part.content_text, "sealed中🙂");
    assert_eq!(
        part.content_json.as_deref(),
        Some("{\"exact\":9007199254740993}")
    );
}

#[test]
fn sealed_text_and_json_survive_internal_resume_and_cold_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resume.db");
    let mut store = fixture(&path);
    assert_originals(&store);
    resume(&mut store, "next", 2).unwrap();
    let identity: (String, String) = store
        .connection
        .query_row("SELECT run_id,status FROM messages WHERE id='m'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(identity, ("next".into(), "queued".into()));
    assert_originals(&store);
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert_originals(&store);
    assert!(resolve_field(
        &store.connection,
        "foreign",
        "m",
        "p",
        PartBodyField::ContentText
    )
    .is_err());
}

#[test]
fn trusted_resumption_never_admits_unfinished_part_or_inline_conflict() {
    for mutation in [
        "state='open'",
        "content_text='conflict'",
        "content_json='{}'",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conflict.db");
        let mut store = fixture(&path);
        resume(&mut store, "next", 2).unwrap();
        store
            .connection
            .execute(
                &format!("UPDATE message_parts SET {mutation} WHERE id='p'"),
                [],
            )
            .unwrap();
        assert!(store.load_conversation("c").is_err(), "{mutation}");
    }
}

#[test]
fn queued_or_arbitrarily_reparented_message_does_not_gain_publication_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("untrusted.db");
    let mut store = fixture(&path);
    store
        .connection
        .execute(
            "UPDATE messages SET status='queued',completed_at_ms=NULL WHERE id='m'",
            [],
        )
        .unwrap();
    assert!(resolve_field(&store.connection, "c", "m", "p", PartBodyField::ContentText).is_err());
    // The real resume API must validate old sealed publication before it records
    // membership; a first queued binding cannot authorize itself by resuming.
    assert!(resume(&mut store, "next", 2).is_err());
    store.connection.execute_batch("UPDATE messages SET status='completed',completed_at_ms=2 WHERE id='m';
        INSERT INTO runs(id,conversation_id,kind,user_message_id,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES('foreign','c','conversation','u','m','queued',1,3,3);
        UPDATE messages SET run_id='foreign',status='queued',completed_at_ms=NULL WHERE id='m';").unwrap();
    assert!(resolve_field(&store.connection, "c", "m", "p", PartBodyField::ContentJson).is_err());
}
