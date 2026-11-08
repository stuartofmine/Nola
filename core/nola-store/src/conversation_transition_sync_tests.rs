// SPDX-License-Identifier: Apache-2.0
//! Actual-change replica publication, without weakening canonical write admission.

use super::*;
use nola_domain::{
    apply_canonical_stream, CanonicalStreamHead, CanonicalStreamOperation, CanonicalStreamOwner,
    PartBodyField,
};

const METADATA: &str =
    r#"{"kind":"text","id":"p","message_id":"a","turn_id":"r","text":"","is_complete":false}"#;

fn stream_owner() -> CanonicalStreamOwner {
    CanonicalStreamOwner {
        conversation_id: "c".into(),
        run_id: "r".into(),
        message_id: "a".into(),
        part_id: "p".into(),
        field: PartBodyField::ContentText,
    }
}

fn stream(operation: CanonicalStreamOperation, at: i64) -> PartMutation {
    PartMutation::Stream {
        operation,
        updated_at_ms: at,
    }
}

fn metadata(state: &str, json: Option<&str>, at: i64) -> PartMutation {
    PartMutation::Metadata {
        part_id: "p".into(),
        state: state.into(),
        content_json: json.map(str::to_string),
        updated_at_ms: at,
    }
}

fn transition(id: &str, revision: i64, mutations: Vec<PartMutation>) -> CommitTransitionInput {
    CommitTransitionInput {
        transition_id: id.into(),
        run_id: "r".into(),
        expected_checkpoint_revision: revision,
        checkpoint_revision: revision + 1,
        checkpoint_event_cursor: 0,
        checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
        submission_reduction_json: None, host_event: None,
        events: vec![],
        ensured_messages: vec![],
        assistant_turn_bindings: vec![],
        part_message_owners: Default::default(),
        part_mutations: mutations,
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        effect_delivery_attempt_mutations: vec![],
        acknowledged_effect_ids: vec![],
        memory_usage_entries: vec![],
        run_status: None,
        assistant_status: None,
        assistant_status_message_id: None,
        committed_at_ms: revision + 1,
    }
}

fn setup(path: impl AsRef<std::path::Path>) -> (NolaStore, CanonicalStreamHead) {
    let mut store = NolaStore::open(path).unwrap();
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            conversation_id: "c".into(),
            conversation_title: "actual changes".into(),
            user_message_id: "u".into(),
            user_part_id: "up".into(),
            user_text: "hello".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: "a".into(),
            assistant_part_id: "p".into(),
            assistant_turn_bindings: vec![],
            run_id: "r".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
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
    let declare = CanonicalStreamOperation::Declare {
        owner: stream_owner(),
    };
    let empty = apply_canonical_stream(None, &declare).unwrap();
    let append = CanonicalStreamOperation::Append {
        owner: stream_owner(),
        expected_head: empty.clone(),
        delta: "x".into(),
    };
    let head = apply_canonical_stream(Some(&empty), &append).unwrap();
    let mut initial = transition(
        "initial",
        1,
        vec![
            stream(declare, 2),
            stream(append, 2),
            metadata("open", Some(METADATA), 2),
        ],
    );
    initial.assistant_status = Some("running".into());
    assert!(store.commit_transition(&initial).unwrap().applied);
    (store, head)
}

fn append(head: &CanonicalStreamHead, delta: &str, at: i64) -> PartMutation {
    stream(
        CanonicalStreamOperation::Append {
            owner: stream_owner(),
            expected_head: head.clone(),
            delta: delta.into(),
        },
        at,
    )
}

fn publications(store: &NolaStore) -> Vec<Publication> {
    store
        .connection
        .prepare(
            "SELECT payload_json FROM sync_outbox
             WHERE entity_type='conversation_transition' ORDER BY local_sequence",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect()
}

fn copy_to_replica(source: &mut NolaStore, receiver: &mut NolaStore) {
    let mut cursor = receiver
        .sync_cursor("actual-change-replica")
        .unwrap()
        .map_or(0, |value| value.cursor);
    while let Some(change) = source.pending_sync_changes(1).unwrap().into_iter().next() {
        // Each fact crosses a page boundary, then is replayed with a new cursor.
        for replay in [false, true] {
            cursor += 1;
            let result = receiver
                .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                    replica_id: "actual-change-replica".into(),
                    next_cursor: cursor,
                    applied_at_ms: 99,
                    changes: vec![RemoteSyncChangeInput {
                        cursor,
                        operation_id: change.operation_id.clone(),
                        entity_type: change.entity_type.clone(),
                        entity_id: change.entity_id.clone(),
                        revision: change.revision,
                        deleted: change.deleted,
                        payload_json: change.payload_json.clone(),
                        created_at_ms: change.created_at_ms,
                    }],
                })
                .unwrap();
            assert_eq!(result.applied, usize::from(!replay));
            assert_eq!(result.skipped, usize::from(replay));
        }
        assert!(source
            .resolve_sync_change(&change.operation_id, "acknowledged", Some(cursor), 99)
            .unwrap());
    }
}

fn canonical_snapshot(store: &NolaStore) -> String {
    store.connection.query_row(
        "SELECT json_object(
            'conversation',(SELECT json_object('revision',revision,'updated',updated_at_ms) FROM conversations WHERE id='c'),
            'message',(SELECT json_object('run',run_id,'status',status,'completed',completed_at_ms) FROM messages WHERE id='a'),
            'part',(SELECT json_object('state',state,'text',content_text,'json',content_json,'digest',content_digest,'updated',updated_at_ms) FROM message_parts WHERE id='p'),
            'head',(SELECT head_json FROM canonical_stream_heads WHERE part_id='p'),
            'chunks',(SELECT json_group_array(json_object('generation',generation,'index',chunk_index,'offset',offset_bytes,'text',text,'root',manifest_root))
                FROM (SELECT * FROM canonical_stream_chunks WHERE part_id='p' ORDER BY generation,chunk_index)))",
        [], |row| row.get(0),
    ).unwrap()
}

#[test]
fn repeated_stream_metadata_and_running_status_publish_only_the_exact_stream_cas() {
    let (mut store, head) = setup(":memory:");
    let mutation = append(&head, "中🙂", 3);
    let mut input = transition(
        "delta",
        2,
        vec![mutation.clone(), metadata("open", Some(METADATA), 3)],
    );
    input.assistant_status = Some("running".into());
    assert!(store.commit_transition(&input).unwrap().applied);
    let values = publications(&store);
    let publication = values.last().unwrap();
    assert_eq!(publication.parts.len(), 1);
    assert_eq!(publication.parts[0].mutation, mutation);
    assert_eq!(publication.assistant_status, None);
    assert_eq!(publication.parts[0].message_id, "a");
    assert_eq!(publication.parts[0].run_id.as_deref(), Some("r"));
    assert!(!store.commit_transition(&input).unwrap().applied);
    assert_eq!(publications(&store).len(), values.len());
}

#[test]
fn metadata_only_timestamp_and_real_json_changes_are_not_elided() {
    let (mut store, _) = setup(":memory:");
    let timestamp = metadata("open", Some(METADATA), 3);
    store
        .commit_transition(&transition("timestamp", 2, vec![timestamp.clone()]))
        .unwrap();
    assert_eq!(
        publications(&store).last().unwrap().parts[0].mutation,
        timestamp
    );
    let changed = metadata("open", Some(r#"{"kind":"text","review":"pending"}"#), 3);
    store
        .commit_transition(&transition("changed", 3, vec![changed.clone()]))
        .unwrap();
    assert_eq!(
        publications(&store).last().unwrap().parts[0].mutation,
        changed
    );
    let preserve_json = metadata("open", None, 5);
    store
        .commit_transition(&transition("preserve-json", 4, vec![preserve_json.clone()]))
        .unwrap();
    assert_eq!(
        publications(&store).last().unwrap().parts[0].mutation,
        preserve_json
    );
    let json: String = store
        .connection
        .query_row(
            "SELECT content_json FROM message_parts WHERE id='p'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(json, r#"{"kind":"text","review":"pending"}"#);
}

#[test]
fn ordered_metadata_changes_keep_each_real_value_and_stream_timestamp() {
    let (mut source, head) = setup(":memory:");
    let first = metadata("open", Some(r#"{"review":"pending"}"#), 3);
    let second = metadata("open", Some(r#"{"review":"hidden"}"#), 3);
    let last = metadata("open", Some(METADATA), 3);
    let stream = append(&head, "y", 3);
    source
        .commit_transition(&transition(
            "ordered",
            2,
            vec![
                stream.clone(),
                metadata("open", Some(METADATA), 3),
                first.clone(),
                first.clone(),
                second.clone(),
                last.clone(),
            ],
        ))
        .unwrap();
    let values = publications(&source);
    let mutations: Vec<_> = values
        .last()
        .unwrap()
        .parts
        .iter()
        .map(|part| part.mutation.clone())
        .collect();
    assert_eq!(mutations, vec![stream, first, second, last]);
    let mut receiver = NolaStore::open(":memory:").unwrap();
    copy_to_replica(&mut source, &mut receiver);
    assert_eq!(canonical_snapshot(&receiver), canonical_snapshot(&source));
}

#[test]
fn closed_text_review_and_terminal_completion_timestamp_are_preserved() {
    let (mut source, head) = setup(":memory:");
    let close = stream(
        CanonicalStreamOperation::Close {
            owner: stream_owner(),
            expected_head: head,
        },
        3,
    );
    let mut terminal = transition(
        "terminal",
        2,
        vec![
            close.clone(),
            PartMutation::Close {
                part_id: "p".into(),
                state: "completed".into(),
                content_json: Some(r#"{"review":"pending"}"#.into()),
                content_digest: None,
                updated_at_ms: 3,
            },
        ],
    );
    terminal.assistant_status = Some("completed".into());
    // Closing the assistant body is not sealing its Run. The subsequent
    // review mutation remains within this same active Run.
    terminal.run_status = None;
    source.commit_transition(&terminal).unwrap();
    assert_eq!(
        publications(&source)
            .last()
            .unwrap()
            .assistant_status
            .as_ref()
            .unwrap()
            .status,
        "completed"
    );
    let review = metadata("completed", Some(r#"{"review":"hidden"}"#), 4);
    let mut reviewed = transition("reviewed", 3, vec![review.clone()]);
    // Repeating a terminal status at a new time updates completed_at_ms.
    reviewed.assistant_status = Some("completed".into());
    source.commit_transition(&reviewed).unwrap();
    let values = publications(&source);
    let publication = values.last().unwrap();
    assert_eq!(publication.parts[0].mutation, review);
    assert_eq!(
        publication.assistant_status.as_ref().unwrap().status,
        "completed"
    );
    let mut receiver = NolaStore::open(":memory:").unwrap();
    copy_to_replica(&mut source, &mut receiver);
    assert_eq!(canonical_snapshot(&receiver), canonical_snapshot(&source));
}

#[test]
fn no_content_transition_keeps_the_published_base_across_cold_revision_gaps() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    let receiver_path = directory.path().join("receiver.db");
    let (mut source, head) = setup(&source_path);
    let mut receiver = NolaStore::open(&receiver_path).unwrap();
    copy_to_replica(&mut source, &mut receiver);
    let previous = publications(&source).last().unwrap().revision;
    let mut empty = transition("no-content", 2, vec![metadata("open", None, 2)]);
    empty.assistant_status = Some("running".into());
    assert!(source.commit_transition(&empty).unwrap().applied);
    assert!(source.pending_sync_changes(1).unwrap().is_empty());
    let local: i64 = source
        .connection
        .query_row(
            "SELECT revision FROM conversations WHERE id='c'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(local, previous + 1);
    let published: i64 = source.connection.query_row(
        "SELECT revision FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id='c'", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(published, previous);
    drop(source);
    drop(receiver);
    let mut source = NolaStore::open(&source_path).unwrap();
    let mut receiver = NolaStore::open(&receiver_path).unwrap();
    source
        .commit_transition(&transition("after-gap", 3, vec![append(&head, "z", 4)]))
        .unwrap();
    let values = publications(&source);
    let next = values.last().unwrap();
    assert_eq!(next.base_content_revision, previous);
    assert_eq!(next.revision, previous + 2);
    copy_to_replica(&mut source, &mut receiver);
    assert_eq!(canonical_snapshot(&receiver), canonical_snapshot(&source));
    assert!(receiver.pending_sync_changes(1).unwrap().is_empty());
}

#[test]
fn oversized_metadata_is_conservatively_retained_and_none_still_preserves_it() {
    let (mut store, _) = setup(":memory:");
    let large = serde_json::json!({"literal":"x".repeat(8192)}).to_string();
    let value = metadata("open", Some(&large), 3);
    store
        .commit_transition(&transition("large", 2, vec![value.clone()]))
        .unwrap();
    store
        .commit_transition(&transition("large-repeated", 3, vec![value.clone()]))
        .unwrap();
    assert_eq!(
        publications(&store).last().unwrap().parts[0].mutation,
        value
    );
    let before = publications(&store).len();
    store
        .commit_transition(&transition(
            "large-preserved",
            4,
            vec![metadata("open", None, 3)],
        ))
        .unwrap();
    assert_eq!(publications(&store).len(), before);
    let stored: String = store
        .connection
        .query_row(
            "SELECT content_json FROM message_parts WHERE id='p'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, large);
}

#[test]
fn no_op_filter_does_not_bypass_canonical_metadata_or_owner_admission() {
    let (mut store, _) = setup(":memory:");
    let before = canonical_snapshot(&store);
    let count = publications(&store).len();
    let illegal = transition(
        "illegal-state",
        2,
        vec![metadata("completed", Some(METADATA), 2)],
    );
    assert!(store.commit_transition(&illegal).is_err());
    let mut foreign = transition(
        "foreign-owner",
        2,
        vec![metadata("open", Some(METADATA), 2)],
    );
    foreign.part_message_owners.insert("p".into(), "u".into());
    assert!(store.commit_transition(&foreign).is_err());
    assert_eq!(canonical_snapshot(&store), before);
    assert_eq!(publications(&store).len(), count);
}

#[test]
fn final_replica_failure_rolls_back_canonical_changes_and_prepared_intent() {
    let (mut store, head) = setup(":memory:");
    let before = canonical_snapshot(&store);
    let count = publications(&store).len();
    let mut input = transition(
        "outbox-failure",
        2,
        vec![append(&head, "y", 3), metadata("open", Some(METADATA), 3)],
    );
    input.assistant_status = Some("running".into());
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER fail_replica BEFORE INSERT ON sync_outbox
         WHEN NEW.entity_type='conversation_transition'
         BEGIN SELECT RAISE(ABORT,'replica failure'); END",
        )
        .unwrap();
    assert!(store
        .commit_transition(&input)
        .unwrap_err()
        .to_string()
        .contains("replica failure"));
    assert_eq!(canonical_snapshot(&store), before);
    assert_eq!(publications(&store).len(), count);
    let checkpoint: i64 = store
        .connection
        .query_row(
            "SELECT revision FROM core_checkpoints WHERE conversation_id='c'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(checkpoint, 2);
    store
        .connection
        .execute_batch("DROP TRIGGER fail_replica")
        .unwrap();
    assert!(store.commit_transition(&input).unwrap().applied);
    assert_eq!(publications(&store).last().unwrap().parts.len(), 1);
}
