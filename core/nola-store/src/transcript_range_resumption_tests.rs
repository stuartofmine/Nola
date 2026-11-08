// SPDX-License-Identifier: Apache-2.0
//! Unversioned ranges retain finalized Part admission during trusted reuse.
use crate::*;
use nola_domain::{
    apply_canonical_stream, CanonicalStreamOperation, CanonicalStreamOwner, PartBodyField,
};
use nola_store_port::TranscriptReadRepository;
#[path = "../tests/support/terminal_sources.rs"]
mod terminal_sources;

const TEXT: &str = "old中🙂answer";
fn fixture(path: &std::path::Path, backing: &str) -> NolaStore {
    fixture_with_terminal(path, backing, "completed")
}
fn fixture_with_terminal(path: &std::path::Path, backing: &str, terminal: &str) -> NolaStore {
    let mut store = NolaStore::open(path).unwrap();
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            conversation_id: "c".into(),
            conversation_title: "ranges".into(),
            user_message_id: "u".into(),
            user_part_id: "up".into(),
            user_text: "question".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: "m".into(),
            assistant_part_id: "p".into(),
            assistant_turn_bindings: vec![],
            run_id: "old".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 0,
            checkpoint_json: serde_json::json!({
                "session_id":"fixture-core:old","revision":1,"last_event_cursor":{"sequence":0},
                "state":{"session_id":"fixture-core:old","task_projection":{"tasks":{}}}
            }).to_string(),
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
    let mut parts = Vec::new();
    if backing == "stream" {
        let owner = CanonicalStreamOwner {
            conversation_id: "c".into(),
            run_id: "old".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            field: PartBodyField::ContentText,
        };
        let declare = CanonicalStreamOperation::Declare {
            owner: owner.clone(),
        };
        let empty = apply_canonical_stream(None, &declare).unwrap();
        let append = CanonicalStreamOperation::Append {
            owner: owner.clone(),
            expected_head: empty.clone(),
            delta: TEXT.into(),
        };
        let head = apply_canonical_stream(Some(&empty), &append).unwrap();
        for operation in [
            declare,
            append,
            CanonicalStreamOperation::Close {
                owner,
                expected_head: head,
            },
        ] {
            parts.push(PartMutation::Stream {
                operation,
                updated_at_ms: 2,
            });
        }
    } else {
        parts.push(PartMutation::AppendText {
            part_id: "p".into(),
            expected_content_digest: format!("{:x}", Sha256::digest(b"")),
            text: TEXT.into(),
            updated_at_ms: 2,
        });
    }
    parts.push(PartMutation::Close {
        part_id: "p".into(),
        state: "completed".into(),
        content_json: None,
        content_digest: Some(format!("{:x}", Sha256::digest(TEXT.as_bytes()))),
        updated_at_ms: 2,
    });
    let mut finished = CommitTransitionInput {
            transition_id: "finish".into(),
            run_id: "old".into(),
            expected_checkpoint_revision: 1,
            checkpoint_revision: 2,
            checkpoint_event_cursor: 0,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
            submission_reduction_json: None, host_event: None,
            events: vec![],
            ensured_messages: vec![],
            assistant_turn_bindings: vec![],
            part_message_owners: Default::default(),
            part_mutations: parts,
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            acknowledged_effect_ids: vec![],
            memory_usage_entries: vec![],
            run_status: None,
            assistant_status: Some(terminal.into()),
            assistant_status_message_id: None,
            committed_at_ms: 2,
        };
    store.begin_write_unit_of_work().unwrap();
    terminal_sources::add_sources(&store, &mut finished, "c", "old-turn", "u", terminal == "cancelled");
    store.commit_transition(&finished).unwrap();
    if backing == "sealed" {
        // Finalized body storage is prepared before Run publication, in the same
        // outer unit of work. It is not a post-seal transcript mutation.
        let reference = nola_domain::CanonicalBodyReference::for_field(
            "body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            TEXT.len() as u64,
            1,
        )
        .unwrap();
        let tx = store.write_transaction().unwrap();
        crate::canonical_text_store::put_chunks(
            &tx,
            &reference,
            std::iter::once(Ok((0, TEXT.into()))),
            1024,
        )
        .unwrap();
        tx.execute("UPDATE message_parts SET content_text='' WHERE id='p'", [])
            .unwrap();
        tx.execute(
            "INSERT INTO message_part_body_references VALUES('p','content_text',?1)",
            [serde_json::to_string(&reference).unwrap()],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    terminal_sources::publish_and_seal(&mut store, &finished, "c");
    store.commit_write_unit_of_work().unwrap();
    store
}
fn resume(store: &mut NolaStore) {
    let target = store.internal_assistant_target("c").unwrap().unwrap();
    let expected_checkpoint_revision = store.checkpoint_revision("c").unwrap();
    store
        .begin_internal_turn(&BeginInternalTurnInput {
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            conversation_id: "c".into(),
            assistant_message_id: target.assistant_message_id,
            expected_previous_run_id: target.run_id,
            expected_checkpoint_revision,
            assistant_part_id: "unused".into(),
            assistant_turn_bindings: vec![],
            run_id: "next".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 3,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 3,
        })
        .unwrap();
}
fn original(store: &NolaStore) -> String {
    let mut text = String::new();
    let mut offset = 0;
    loop {
        let range =
            TranscriptReadRepository::read_conversation_text_range(store, "c", "m", "p", offset, 4)
                .unwrap();
        assert_eq!(range.total_utf8_bytes, TEXT.len() as u64);
        assert!(range.text.len() <= 4);
        text.push_str(&range.text);
        offset = range.next_offset_bytes;
        if range.end_of_content {
            break;
        }
    }
    text
}

fn window_reference(store: &NolaStore) -> DeferredTextPart {
    use nola_store_port::conversation_history::CanonicalHistoryRepository;
    let window =
        CanonicalHistoryRepository::load_conversation_message_window(store, "c", None, 1, 4)
            .unwrap()
            .unwrap();
    assert_eq!(window.deferred_text_parts.len(), 1);
    let reference = window.deferred_text_parts.into_iter().next().unwrap();
    assert_eq!(reference.message_id, "m");
    assert_eq!(reference.part_id, "p");
    assert_eq!(reference.total_utf8_bytes, TEXT.len() as u64);
    assert!(reference.preview_utf8_bytes <= 4);
    reference
}

#[test]
fn finalized_inline_sealed_and_stream_survive_real_resume_and_cold_ranges() {
    for backing in ["inline", "sealed", "stream"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("range.db");
        let mut store = fixture(&path, backing);
        assert!(window_reference(&store).can_read_original);
        resume(&mut store);
        let state: String = store
            .connection
            .query_row("SELECT status FROM messages WHERE id='m'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "queued");
        let reference = window_reference(&store);
        assert!(reference.is_final && reference.can_read_original);
        assert_eq!(original(&store), TEXT);
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        assert!(window_reference(&store).can_read_original);
        assert_eq!(original(&store), TEXT);
        for (c, m, p) in [
            ("other", "m", "p"),
            ("c", "other", "p"),
            ("c", "m", "other"),
        ] {
            assert!(
                TranscriptReadRepository::read_conversation_text_range(&store, c, m, p, 0, 16)
                    .is_err()
            );
        }
        store
            .connection
            .execute("UPDATE message_parts SET state='open' WHERE id='p'", [])
            .unwrap();
        assert!(TranscriptReadRepository::read_conversation_text_range(
            &store, "c", "m", "p", 0, 16
        )
        .is_err());
        store
            .connection
            .execute(
                "UPDATE message_parts SET state='completed' WHERE id='p'",
                [],
            )
            .unwrap();
        store
            .connection
            .execute("UPDATE messages SET run_id='untrusted' WHERE id='m'", [])
            .unwrap();
        assert!(TranscriptReadRepository::read_conversation_text_range(
            &store, "c", "m", "p", 0, 16
        )
        .is_err());
    }
}

#[test]
fn queued_message_without_trusted_handoff_cannot_use_unversioned_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("range.db"), "inline");
    store
        .connection
        .execute("UPDATE messages SET status='queued' WHERE id='m'", [])
        .unwrap();
    let reference = window_reference(&store);
    assert!(reference.is_final && !reference.can_read_original);
    assert!(
        TranscriptReadRepository::read_conversation_text_range(&store, "c", "m", "p", 0, 16)
            .is_err()
    );
}

#[test]
fn window_reports_range_eligibility_for_message_closure_inside_sealed_runs() {
    for backing in ["inline", "stream"] {
        for terminal in ["completed", "failed", "cancelled"] {
            // These are message closure states, not Run business results.
            // The fixture retains an unfinished requirement unless cancelled.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("range.db");
            let store = fixture_with_terminal(&path, backing, terminal);
            let reference = window_reference(&store);
            assert!(reference.is_final);
            assert_eq!(reference.can_read_original, terminal == "completed");
            assert_eq!(TranscriptReadRepository::read_conversation_text_range(
                &store, "c", "m", "p", 0, 4,
            ).is_ok(), reference.can_read_original);
            drop(store);
            let store = NolaStore::open(&path).unwrap();
            assert_eq!(window_reference(&store), reference);
        }
    }
}
