// SPDX-License-Identifier: Apache-2.0
//! Real Store UoW proof for complete, ordered, multi-owner publication sets.
//! These leaf fixtures do not assert production producer source-change coverage.
use super::*;
#[path = "../../tests/support/terminal_sources.rs"]
mod terminal_sources;
use nola_store_port::protocol_journal::{
    BeginCommandReceiptInput, BeginCommandReceiptResult, CompleteCommandReceiptOutcome,
};
use nola_store_port::run_effect::OutboxEffectInput;


fn complete_run_for_idle(store: &mut NolaStore, id: &AssistantJournalIdentity) {
    terminal_sources::settle_fixture(store, &id.conversation_id, &id.run_id,
        &format!("{}-actual-turn",id.conversation_id), &format!("{}-user",id.conversation_id));
    assert!(store.active_run(&id.conversation_id).unwrap().is_none());
}

fn idle_source(store:&mut NolaStore, conversation:&str) -> AssistantSourceCommitStamp {
    let previous=store.load_checkpoint(conversation).unwrap().unwrap();
    let revision=previous.revision+1;
    let event_cursor=previous.event_cursor+1;
    store.save_checkpoint_with_memories(&crate::SaveCheckpointInput {
        conversation_id:conversation.into(),expected_revision:previous.revision,revision,event_cursor,
        checkpoint_json:format!("{{\"idle_revision\":{revision}}}"),updated_at_ms:revision,
    },&[],&[],&[]).unwrap();
    AssistantSourceCommitStamp::IdleCheckpoint {conversation_id:conversation.into(),checkpoint_revision:revision,event_cursor}
}

#[test]
fn idle_publication_requires_current_uow_and_exact_target_then_replays_after_restart() {
    let directory=tempfile::tempdir().unwrap();let path=directory.path().join("idle-journal.sqlite");
    let mut store=NolaStore::open_archive_mode(&path, false).unwrap();
    let id=begin(&mut store,"idle-owner");complete_run_for_idle(&mut store,&id);
    let guessed=AssistantSourceCommitStamp::IdleCheckpoint {
        conversation_id:id.conversation_id.clone(),checkpoint_revision:2,event_cursor:0,
    };
    let mut input=CommitAssistantPublicationSet {stamp:guessed,groups:vec![three_frames(
        &id,store.assistant_journal_head(&id).unwrap(),"idle task result",None)]};
    store.begin_write_unit_of_work().unwrap();
    assert!(store.commit_assistant_publication_set(&input).is_err(),"persisted checkpoint alone grants nothing");
    store.rollback_write_unit_of_work().unwrap();

    // Ordinary checkpoint APIs must not grant this dedicated idle publication capability.
    store.begin_write_unit_of_work().unwrap();
    store.save_checkpoint(&crate::SaveCheckpointInput {
        conversation_id:id.conversation_id.clone(),expected_revision:2,revision:3,event_cursor:6,
        checkpoint_json:"{}".into(),updated_at_ms:3,
    }).unwrap();
    input.stamp=AssistantSourceCommitStamp::IdleCheckpoint {
        conversation_id:id.conversation_id.clone(),checkpoint_revision:3,event_cursor:6,
    };
    assert!(store.commit_assistant_publication_set(&input).is_err());
    store.rollback_write_unit_of_work().unwrap();

    store.begin_write_unit_of_work().unwrap();input.stamp=idle_source(&mut store,&id.conversation_id);
    let actual_stamp=input.stamp.clone();
    if let AssistantSourceCommitStamp::IdleCheckpoint {event_cursor,..}=&mut input.stamp { *event_cursor+=1; }
    assert!(store.commit_assistant_publication_set(&input).is_err(),"event cursor is part of exact idle source");
    input.stamp=actual_stamp;
    assert!(matches!(store.commit_assistant_publication_set(&input).unwrap(),AssistantPublicationCommit::Applied(_)));
    store.commit_write_unit_of_work().unwrap();
    // A newer idle checkpoint is legitimate; historical exact publication replay remains immutable.
    store.begin_write_unit_of_work().unwrap();idle_source(&mut store,&id.conversation_id);store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store=NolaStore::open_archive_mode(&path, false).unwrap();store.begin_write_unit_of_work().unwrap();
    assert!(matches!(store.commit_assistant_publication_set(&input).unwrap(),AssistantPublicationCommit::Replay(_)));
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(store.assistant_journal_head(&id).unwrap().sequence,4);
}

#[test]
fn idle_stamp_rollback_and_foreign_or_unbound_target_cannot_borrow_authority() {
    let directory=tempfile::tempdir().unwrap();let mut store=NolaStore::open(directory.path().join("fences.sqlite")).unwrap();
    let id=begin(&mut store,"idle-source");let foreign=begin(&mut store,"foreign-owner");
    let unbound=successor(&id,"unbound");
    complete_run_for_idle(&mut store,&id);
    store.begin_write_unit_of_work().unwrap();store.begin_command_mutations().unwrap();
    let rolled=idle_source(&mut store,&id.conversation_id);
    store.rollback_command_mutations().unwrap();
    let rolled_input=CommitAssistantPublicationSet {stamp:rolled,groups:vec![three_frames(
        &id,store.assistant_journal_head(&id).unwrap(),"must roll back",None)]};
    assert!(store.commit_assistant_publication_set(&rolled_input).is_err());
    store.rollback_write_unit_of_work().unwrap();

    store.begin_write_unit_of_work().unwrap();let stamp=idle_source(&mut store,&id.conversation_id);
    for invalid in [&foreign,&unbound] {
        let input=CommitAssistantPublicationSet {stamp:stamp.clone(),groups:vec![three_frames(
            invalid,AssistantJournalCursor::initial(invalid.clone()).unwrap(),"unauthorized target",None)]};
        assert!(store.commit_assistant_publication_set(&input).is_err());
        assert!(store.reserve_assistant_journal_order(&stamp,invalid,JournalSection::Tasks,"fake-task").is_err());
    }
    let input=CommitAssistantPublicationSet {stamp,groups:vec![three_frames(
        &id,store.assistant_journal_head(&id).unwrap(),"allowed target",None)]};
    assert!(matches!(store.commit_assistant_publication_set(&input).unwrap(),AssistantPublicationCommit::Applied(_)));
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(store.assistant_journal_head(&id).unwrap().sequence,1);
}


#[test]
fn active_source_can_publish_exact_historical_owner_without_borrowing_current_message() {
    let directory=tempfile::tempdir().unwrap();let mut store=NolaStore::open(directory.path().join("cross-run.sqlite")).unwrap();
    let old=begin(&mut store,"cross-run");complete_run_for_idle(&mut store,&old);
    let frontier = store.load_conversation_message_window(&old.conversation_id, None, 1, 1).unwrap().unwrap();
    let parent = frontier.conversation.messages.last().map(|message| message.id.clone());
    store.begin_write_unit_of_work().unwrap();
    store.begin_turn(&BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: Some(frontier.conversation.configuration_revision),
        expected_parent_message_id: parent,
        initial_events: Vec::new(),
        conversation_id:old.conversation_id.clone(),conversation_title:"Same conversation".into(),
        user_message_id:"next-user".into(),user_part_id:"next-user-part".into(),user_text:"Continue".into(),
        user_content_json:None,additional_user_parts:vec![],assistant_message_id:"next-assistant".into(),
        assistant_part_id:"next-part".into(),initial_assistant_part_mutations:vec![],
        assistant_turn_bindings:vec![nola_domain::AssistantTurnBinding::Admission {
            turn_id:"next-core-turn".into(),assistant_message_id:"next-assistant".into(),
        }],run_id:"next-run".into(),context_checkpoint_id:None,checkpoint_revision:3,
        checkpoint_event_cursor:0,checkpoint_json:"{}".into(),invocation_config_json:"{}".into(),
        invocation_config_digest:"fixture".into(),outbox_effects:vec![],model_attempt_mutations:vec![],
        delegated_invocation_mutations:vec![],delegated_permission_mutations:vec![],memory_usage_entries:vec![],created_at_ms:3,
    }).unwrap();
    let current=AssistantJournalIdentity {run_id:"next-run".into(),assistant_message_id:"next-assistant".into(),..old.clone()};
    let stamp=AssistantSourceCommitStamp::Admission {run_id:"next-run".into()};
    let order=store.reserve_assistant_journal_order(&stamp,&old,JournalSection::Tasks,"old-task").unwrap();
    assert_eq!(order.section,JournalSection::Tasks);
    let input=CommitAssistantPublicationSet {stamp,groups:vec![
        three_frames(&old,store.assistant_journal_head(&old).unwrap(),"historical task completed",None),
        three_frames(&current,store.assistant_journal_head(&current).unwrap(),"current answer",None),
    ]};
    assert!(matches!(store.commit_assistant_publication_set(&input).unwrap(),AssistantPublicationCommit::Applied(ref cursors) if cursors.len()==2));
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(store.assistant_journal_head(&old).unwrap().sequence,4);
    assert_eq!(store.assistant_journal_head(&current).unwrap().sequence,3);
    let owner=store.assistant_owner_for_core_turn(&old.conversation_id,"cross-run-actual-turn").unwrap().unwrap();
    assert_eq!(owner.run_id,old.run_id);
}

fn successor(id: &AssistantJournalIdentity, suffix: &str) -> AssistantJournalIdentity {
    AssistantJournalIdentity {
        assistant_message_id: format!("{}-{suffix}", id.conversation_id),
        ..id.clone()
    }
}

fn source_transition(
    store: &mut NolaStore,
    id: &AssistantJournalIdentity,
    revision: i64,
    successors: &[AssistantJournalIdentity],
    effect: bool,
) -> AssistantSourceCommitStamp {
    let transition_id = format!("{}-{revision}", id.run_id);
    let result = store
        .commit_transition(&CommitTransitionInput {
            transition_id: transition_id.clone(),
            run_id: id.run_id.clone(),
            expected_checkpoint_revision: revision - 1,
            checkpoint_revision: revision,
            checkpoint_event_cursor: 0,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: format!("{{\"publication_fixture_revision\":{revision}}}") },
            submission_reduction_json: None, host_event: None,
            events: vec![],
            ensured_messages: successors
                .iter()
                .map(|owner| nola_domain::TransitionMessageInput {
                    id: owner.assistant_message_id.clone(),
                    run_id: Some(id.run_id.clone()),
                    role: "assistant".into(),
                    status: "streaming".into(),
                    created_at_ms: revision,
                    completed_at_ms: None,
                    initial_parts: vec![],
                })
                .collect(),
            assistant_turn_bindings: vec![],
            part_message_owners: Default::default(),
            part_mutations: vec![],
            outbox_effects: if effect {
                vec![OutboxEffectInput {
                    model_attempt_id: None,
                    delegated_invocation_id: None,
                    effect_id: "publication-fixture-effect".into(),
                    request_id: "publication-fixture-request".into(),
                    effect_kind: "publication_fixture".into(),
                    write: crate::outbox_fixture::tool("publication-fixture-effect", "publication-fixture-request", serde_json::json!({"source":"actual-transition"})),
                }]
            } else {
                vec![]
            },
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            acknowledged_effect_ids: vec![],
            memory_usage_entries: vec![],
            run_status: None,
            assistant_status: None,
            assistant_status_message_id: None,
            committed_at_ms: revision,
        })
        .unwrap();
    assert!(result.applied);
    AssistantSourceCommitStamp::Transition { transition_id }
}

fn part_id(id: &AssistantJournalIdentity) -> String {
    format!("{}-journal-part", id.assistant_message_id)
}

fn initial_head(id: &AssistantJournalIdentity) -> JournalPartHead {
    let AssistantJournalOperationV1::Declare(value) = declaration(id).operations.remove(0) else {
        unreachable!()
    };
    let mut head = value.part;
    head.part_id = part_id(id);
    head.bodies = vec![empty_journal_content(
        id,
        &head.part_id,
        JournalContentField::Markdown,
        JournalNumber(1),
    )
    .unwrap()];
    head
}

fn three_frames(
    id: &AssistantJournalIdentity,
    expected: AssistantJournalCursor,
    text: &str,
    prepared: Option<JournalContentRef>,
) -> AssistantPublicationGroup {
    let mut head = initial_head(id);
    if let Some(content) = prepared {
        head.bodies = vec![content];
    }
    let prefix = head.bodies[0].clone();
    let chunk = AssistantTextChunkV1::new(text.into()).unwrap();
    let refs = vec![JournalChunkRef {
        digest: chunk.digest(),
        utf8_bytes: JournalNumber(chunk.byte_length() as u64),
    }];
    let resulting = append_journal_content(&prefix, &refs).unwrap();
    let batch = |operations| AssistantOperationBatchV1 {
        protocol_version: ASSISTANT_JOURNAL_VERSION,
        identity: id.clone(),
        operations,
    };
    AssistantPublicationGroup {
        expected,
        frames: vec![
            AssistantJournalFrame {
                batch: batch(vec![AssistantJournalOperationV1::Declare(
                    DeclareJournalPart { part: head.clone() },
                )]),
                supplied_chunks: vec![],
            },
            AssistantJournalFrame {
                batch: batch(vec![AssistantJournalOperationV1::Append(
                    AppendJournalText {
                        part_id: head.part_id.clone(),
                        expected_generation: head.generation,
                        expected_revision: head.revision,
                        expected: prefix,
                        chunks: refs,
                        resulting,
                    },
                )]),
                supplied_chunks: vec![chunk],
            },
            AssistantJournalFrame {
                batch: batch(vec![AssistantJournalOperationV1::Update(
                    UpdateJournalMetadata {
                        part_id: head.part_id,
                        expected_generation: JournalNumber(1),
                        expected_revision: JournalNumber(2),
                        metadata: JournalPartMetadata::Text(JournalTextMetadata {
                            is_final: true,
                            visibility: JournalVisibility::Public,
                            review: None,
                        }),
                    },
                )]),
                supplied_chunks: vec![],
            },
        ],
    }
}

fn two_owner_set(
    store: &NolaStore,
    stamp: AssistantSourceCommitStamp,
    first: &AssistantJournalIdentity,
    second: &AssistantJournalIdentity,
    prepared: Option<JournalContentRef>,
) -> CommitAssistantPublicationSet {
    CommitAssistantPublicationSet {
        stamp,
        groups: vec![
            three_frames(
                first,
                store.assistant_journal_head(first).unwrap(),
                "first 中🙂\n\u{0} exact body",
                prepared,
            ),
            three_frames(
                second,
                store.assistant_journal_head(second).unwrap(),
                "successor 中文🙂 final body",
                None,
            ),
        ],
    }
}

fn applied(result: AssistantPublicationCommit) -> Vec<AssistantJournalCursor> {
    match result {
        AssistantPublicationCommit::Applied(cursors) => cursors,
        AssistantPublicationCommit::Replay(_) => panic!("first publication must be Applied"),
    }
}

fn counts(store: &NolaStore) -> Vec<i64> {
    [
        "assistant_journal_heads",
        "assistant_journal_records",
        "assistant_journal_parts",
        "assistant_journal_part_versions",
        "assistant_journal_chunks",
        "assistant_journal_manifests",
        "assistant_journal_manifest_chunks",
        "assistant_journal_publications",
        "assistant_journal_publication_frames",
    ]
    .into_iter()
    .map(|table| {
        store
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    })
    .collect()
}



fn assert_published_body(store: &mut NolaStore, id: &AssistantJournalIdentity, text: &str) {
    assert_eq!(store.assistant_journal_head(id).unwrap().sequence, 3);
    let head = store
        .assistant_journal_part(id, &part_id(id))
        .unwrap()
        .unwrap();
    assert_eq!(head.revision, JournalNumber(3));
    assert_eq!(head.metadata_revision, JournalNumber(2));
    assert!(matches!(head.metadata, JournalPartMetadata::Text(ref m) if m.is_final));
    store.begin_write_unit_of_work().unwrap();
    let snapshot = store.open_assistant_journal_snapshot(id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let body = &head.bodies[0];
    let actual = (0..body.chunk_count.0)
        .map(|index| {
            store
                .read_assistant_snapshot_chunk(&snapshot, &head.part_id, body, index)
                .unwrap()
                .text()
                .to_owned()
        })
        .collect::<String>();
    assert_eq!(actual, text);
    let window = store
        .open_assistant_journal_read(&AssistantJournalCursor::initial(id.clone()).unwrap())
        .unwrap();
    let AssistantJournalPage::Records {
        records, complete, ..
    } = store
        .read_assistant_journal_page(&window, 10, ASSISTANT_FRAME_MAX_BYTES)
        .unwrap()
    else {
        panic!("full committed publication must remain readable")
    };
    assert!(complete);
    assert_eq!(records.len(), 3);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record.header.sequence, (index + 1) as u64);
        assert_eq!(record.header.source_commit_revision, 2);
        record.batch.verify_header(&record.header).unwrap();
    }
    assert_ne!(
        records[0].header.operation_id,
        records[1].header.operation_id
    );
    assert_ne!(
        records[1].header.operation_id,
        records[2].header.operation_id
    );
}

#[test]
fn multi_owner_publication_cold_reopens_complete_ordered_frames_and_exact_replay() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("publication-set.db");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    let first = begin(&mut store, "set");
    let second = successor(&first, "successor");
    store.begin_write_unit_of_work().unwrap();
    let stamp = source_transition(&mut store, &first, 2, &[second.clone()], false);
    let input = two_owner_set(&store, stamp, &first, &second, None);
    let cursors = applied(store.commit_assistant_publication_set(&input).unwrap());
    assert_eq!(cursors.len(), 2);
    assert_eq!(cursors[0].identity, first);
    assert_eq!(cursors[1].identity, second);
    assert!(cursors.iter().all(|cursor| cursor.sequence == 3));
    store.commit_write_unit_of_work().unwrap();
    drop(store);

    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    assert_published_body(&mut store, &first, "first 中🙂\n\u{0} exact body");
    assert_published_body(&mut store, &second, "successor 中文🙂 final body");
    let before = counts(&store);
    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        store.commit_assistant_publication_set(&input).unwrap(),
        AssistantPublicationCommit::Replay(cursors)
    );
    assert_eq!(counts(&store), before);
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn complete_set_replay_rejects_missing_reordered_truncated_or_changed_members() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replay-set.db");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    let first = begin(&mut store, "set");
    let second = successor(&first, "successor");
    let other = successor(&first, "other-valid-assistant");
    store.begin_write_unit_of_work().unwrap();
    let stamp = source_transition(
        &mut store,
        &first,
        2,
        &[second.clone(), other.clone()],
        false,
    );
    let input = two_owner_set(&store, stamp, &first, &second, None);
    store.commit_assistant_publication_set(&input).unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();

    let mut variants = Vec::new();
    let mut missing_owner = input.clone();
    missing_owner.groups.pop();
    variants.push(("missing owner", missing_owner));
    let mut missing_frame = input.clone();
    missing_frame.groups[0].frames.pop();
    variants.push(("missing final frame", missing_frame));
    let mut reordered_frames = input.clone();
    reordered_frames.groups[0].frames.swap(0, 1);
    variants.push(("reordered frames", reordered_frames));
    let mut reordered_owners = input.clone();
    reordered_owners.groups.reverse();
    variants.push(("reordered owners", reordered_owners));
    let mut truncated = input.clone();
    truncated.groups[0].frames[2].batch.operations.clear();
    variants.push(("truncated operations", truncated));
    let mut changed_chunk = input.clone();
    changed_chunk.groups[0].frames[1].supplied_chunks[0] =
        AssistantTextChunkV1::new("same batch refs, changed supplied bytes".into()).unwrap();
    variants.push(("changed supplied bytes", changed_chunk));
    let mut extra_chunk = input.clone();
    extra_chunk.groups[0].frames[1]
        .supplied_chunks
        .push(AssistantTextChunkV1::new("unused extra bytes".into()).unwrap());
    variants.push(("changed supplied membership", extra_chunk));
    let mut changed_owner = input.clone();
    changed_owner.groups[1] = three_frames(
        &other,
        store.assistant_journal_head(&other).unwrap(),
        "successor 中文🙂 final body",
        None,
    );
    variants.push(("different legitimate owner", changed_owner));
    let mut changed_metadata = input.clone();
    let AssistantJournalOperationV1::Update(update) =
        &mut changed_metadata.groups[0].frames[2].batch.operations[0]
    else {
        unreachable!()
    };
    let JournalPartMetadata::Text(metadata) = &mut update.metadata else {
        unreachable!()
    };
    metadata.review = Some(nola_domain::TextReviewState::Approved);
    variants.push(("changed valid metadata", changed_metadata));

    let before = counts(&store);
    for (label, variant) in variants {
        store.begin_write_unit_of_work().unwrap();
        assert!(
            store.commit_assistant_publication_set(&variant).is_err(),
            "must reject {label}"
        );
        assert_eq!(counts(&store), before, "must not mutate on {label}");
        assert_eq!(store.assistant_journal_head(&first).unwrap().sequence, 3);
        assert_eq!(store.assistant_journal_head(&second).unwrap().sequence, 3);
        assert_eq!(store.assistant_journal_head(&other).unwrap().sequence, 0);
        store.rollback_write_unit_of_work().unwrap();
    }
    store.begin_write_unit_of_work().unwrap();
    assert!(matches!(
        store.commit_assistant_publication_set(&input).unwrap(),
        AssistantPublicationCommit::Replay(_)
    ));
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn replay_requires_every_persisted_frame_not_only_a_set_receipt() {
    let directory = tempfile::tempdir().unwrap();
    for (index, (corrupt, disable_foreign_keys)) in [
        ("DELETE FROM assistant_journal_records WHERE sequence=3 AND json_extract(owner,'$.assistant_message_id')='set-successor'", false),
        ("DELETE FROM assistant_journal_publication_frames WHERE group_ordinal=1 AND frame_ordinal=2", false),
        ("UPDATE assistant_journal_records SET batch=x'0001' WHERE sequence=3 AND json_extract(owner,'$.assistant_message_id')='set-successor'", false),
        ("UPDATE assistant_journal_chunks SET bytes=zeroblob(length(bytes))", false),
        ("DELETE FROM assistant_journal_chunks", true),
    ].into_iter().enumerate() {
        let path = directory.path().join(format!("corrupted-frame-{index}.db"));
        let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
        let first = begin(&mut store, "set");
        let second = successor(&first, "successor");
        store.begin_write_unit_of_work().unwrap();
        let stamp = source_transition(&mut store, &first, 2, &[second.clone()], false);
        let input = two_owner_set(&store, stamp, &first, &second, None);
        store.commit_assistant_publication_set(&input).unwrap();
        store.commit_write_unit_of_work().unwrap();
        let chunk_lengths = || {
            store.connection.prepare("SELECT digest,length(bytes) FROM assistant_journal_chunks ORDER BY digest")
                .unwrap().query_map([], |row|Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?)))
                .unwrap().collect::<Result<Vec<_>,_>>().unwrap()
        };
        let original_lengths = chunk_lengths();
        assert_eq!(original_lengths.len(), 2);
        if disable_foreign_keys {
            // Isolated corruption only. Reopened production Store restores FK
            // enforcement; the publication must reject, not silently re-supply.
            store.connection.pragma_update(None, "foreign_keys", false).unwrap();
        }
        store.connection.execute_batch(corrupt).unwrap();
        if index == 3 {
            assert_eq!(chunk_lengths(), original_lengths, "corruption must preserve every byte length");
        }
        drop(store);
        let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
        assert!(store.connection.pragma_query_value(None, "foreign_keys", |row|row.get::<_,bool>(0)).unwrap());
        let before = counts(&store);
        let before_heads = input.groups.iter().map(|group|store.assistant_journal_head(&group.expected.identity).unwrap()).collect::<Vec<_>>();
        store.begin_write_unit_of_work().unwrap();
        assert!(store.commit_assistant_publication_set(&input).is_err(),
            "a complete publication receipt cannot hide persisted frame/chunk corruption {index}");
        assert_eq!(counts(&store), before);
        assert_eq!(input.groups.iter().map(|group|store.assistant_journal_head(&group.expected.identity).unwrap()).collect::<Vec<_>>(), before_heads);
        store.rollback_write_unit_of_work().unwrap();
    }
}

#[test]
fn admission_publication_rolls_back_and_requires_a_fresh_actual_same_uow_admission() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("admission.db");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    let before = counts(&store);
    store.begin_write_unit_of_work().unwrap();
    let first = begin(&mut store, "admission");
    let input = CommitAssistantPublicationSet {
        stamp: AssistantSourceCommitStamp::Admission {
            run_id: first.run_id.clone(),
        },
        groups: vec![three_frames(
            &first,
            store.assistant_journal_head(&first).unwrap(),
            "initial Unicode 中🙂 public body",
            None,
        )],
    };
    assert!(matches!(
        store.commit_assistant_publication_set(&input).unwrap(),
        AssistantPublicationCommit::Applied(_)
    ));
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(counts(&store), before);
    assert!(store.load_conversation("admission").unwrap().is_none());
    drop(store);
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    store.begin_write_unit_of_work().unwrap();
    assert!(store.commit_assistant_publication_set(&input).is_err());
    assert_eq!(begin(&mut store, "admission"), first);
    let cursors = applied(store.commit_assistant_publication_set(&input).unwrap());
    assert_eq!(cursors[0].sequence, 3);
    let source_revisions: (i64, i64) = store
        .connection
        .query_row(
            "SELECT min(source_revision),max(source_revision) FROM assistant_journal_records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(source_revisions, (1, 1));
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        store.commit_assistant_publication_set(&input).unwrap(),
        AssistantPublicationCommit::Replay(cursors)
    );
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn publication_requires_current_same_uow_source_not_unknown_old_or_rolled_stamp() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("stamps.db")).unwrap();
    let first = begin(&mut store, "set");
    let initial = store.assistant_journal_head(&first).unwrap();
    let input = |stamp| CommitAssistantPublicationSet {
        stamp,
        groups: vec![three_frames(&first, initial.clone(), "real body", None)],
    };
    assert!(store
        .commit_assistant_publication_set(&input(AssistantSourceCommitStamp::Admission {
            run_id: first.run_id.clone(),
        }))
        .is_err());
    store.begin_write_unit_of_work().unwrap();
    assert!(store
        .commit_assistant_publication_set(&input(AssistantSourceCommitStamp::Transition {
            transition_id: "unknown-source".into(),
        }))
        .is_err());
    let rolled = source_transition(&mut store, &first, 2, &[], false);
    store.rollback_write_unit_of_work().unwrap();
    store.begin_write_unit_of_work().unwrap();
    assert!(store
        .commit_assistant_publication_set(&input(rolled))
        .is_err());
    store.rollback_write_unit_of_work().unwrap();

    // Persisted canonical source outside the current outer UoW is not authority
    // for a first publication, even though its transition record exists.
    let old = AssistantSourceCommitStamp::Transition {
        transition_id: transition(&mut store, &first, 2),
    };
    store.begin_write_unit_of_work().unwrap();
    assert!(store.commit_assistant_publication_set(&input(old)).is_err());
    assert_eq!(store.assistant_journal_head(&first).unwrap(), initial);
    store.rollback_write_unit_of_work().unwrap();

    store.begin_write_unit_of_work().unwrap();
    let current = source_transition(&mut store, &first, 3, &[], false);
    assert!(matches!(
        store
            .commit_assistant_publication_set(&input(current))
            .unwrap(),
        AssistantPublicationCommit::Applied(_)
    ));
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn one_source_cannot_publish_for_another_legitimate_run() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("cross-run.db")).unwrap();
    let first = begin(&mut store, "first");
    let foreign = begin(&mut store, "foreign");
    store.begin_write_unit_of_work().unwrap();
    let stamp = source_transition(&mut store, &first, 2, &[], false);
    let input = two_owner_set(&store, stamp, &first, &foreign, None);
    let before = counts(&store);
    assert!(store.commit_assistant_publication_set(&input).is_err());
    assert_eq!(counts(&store), before);
    assert_eq!(store.assistant_journal_head(&first).unwrap().sequence, 0);
    assert_eq!(store.assistant_journal_head(&foreign).unwrap().sequence, 0);
    store.rollback_write_unit_of_work().unwrap();
}

#[test]
fn new_source_wrong_group_cursor_is_rejected_without_advancing_any_owner() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("cas.db")).unwrap();
    let first = begin(&mut store, "set");
    let second = successor(&first, "successor");
    store.begin_write_unit_of_work().unwrap();
    let stamp = source_transition(&mut store, &first, 2, &[second.clone()], false);
    let original = two_owner_set(&store, stamp, &first, &second, None);
    store.commit_assistant_publication_set(&original).unwrap();
    store.commit_write_unit_of_work().unwrap();

    store.begin_write_unit_of_work().unwrap();
    let stamp = source_transition(&mut store, &first, 3, &[], false);
    let groups = [&first, &second]
        .into_iter()
        .map(|id| {
            let head = store
                .assistant_journal_part(id, &part_id(id))
                .unwrap()
                .unwrap();
            AssistantPublicationGroup {
                expected: store.assistant_journal_head(id).unwrap(),
                frames: vec![AssistantJournalFrame {
                    batch: AssistantOperationBatchV1 {
                        protocol_version: ASSISTANT_JOURNAL_VERSION,
                        identity: id.clone(),
                        operations: vec![AssistantJournalOperationV1::Update(
                            UpdateJournalMetadata {
                                part_id: head.part_id,
                                expected_generation: head.generation,
                                expected_revision: head.revision,
                                metadata: JournalPartMetadata::Text(JournalTextMetadata {
                                    is_final: true,
                                    visibility: JournalVisibility::Public,
                                    review: Some(nola_domain::TextReviewState::Approved),
                                }),
                            },
                        )],
                    },
                    supplied_chunks: vec![],
                }],
            }
        })
        .collect();
    let good = CommitAssistantPublicationSet { stamp, groups };
    let mut stale = good.clone();
    stale.groups[1].expected = original.groups[1].expected.clone();
    let before = counts(&store);
    assert!(store.commit_assistant_publication_set(&stale).is_err());
    assert_eq!(counts(&store), before);
    assert_eq!(store.assistant_journal_head(&first).unwrap().sequence, 3);
    assert_eq!(store.assistant_journal_head(&second).unwrap().sequence, 3);
    let cursors = applied(store.commit_assistant_publication_set(&good).unwrap());
    assert!(cursors.iter().all(|cursor| cursor.sequence == 4));
    store.commit_write_unit_of_work().unwrap();
}

fn admission_receipt(store: &mut NolaStore) {
    // This uses actual receipt storage in the same UoW. It is explicitly a Store
    // transaction fixture, not a claim that a production Application command is wired.
    assert!(matches!(
        store
            .begin_command_receipt(&BeginCommandReceiptInput {
                client_id: "publication-fixture-client".into(),
                request_id: "publication-fixture-command".into(),
                command_type: "publication_transaction_fixture".into(),
                command_json: "{\"fixture\":true}".into(),
                expected_revision: None,
                received_at: "2026-10-02T00:00:00Z".into(),
            })
            .unwrap(),
        BeginCommandReceiptResult::Started(_)
    ));
}

fn prepare_prefix(store: &mut NolaStore, id: &AssistantJournalIdentity) -> JournalContentRef {
    store
        .prepare_assistant_content(
            id,
            &part_id(id),
            &empty_journal_content(
                id,
                &part_id(id),
                JournalContentField::Markdown,
                JournalNumber(1),
            )
            .unwrap(),
            &[AssistantTextChunkV1::new("prepared 中文 prefix; ".into()).unwrap()],
        )
        .unwrap()
}

#[test]
fn final_frame_failure_rolls_back_whole_set_and_outer_source_facts_cold_retry_succeeds() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollback.db");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    let first = begin(&mut store, "set");
    let second = successor(&first, "successor");
    let baseline = counts(&store);
    store
        .connection
        .execute_batch(
            "CREATE TEMP TRIGGER fail_publication_final_frame
         BEFORE INSERT ON assistant_journal_records
         WHEN NEW.sequence=3 AND json_extract(NEW.owner,'$.assistant_message_id')='set-successor'
         BEGIN SELECT RAISE(ABORT,'injected final publication frame'); END;",
        )
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    admission_receipt(&mut store);
    let stamp = source_transition(&mut store, &first, 2, &[second.clone()], true);
    let prefix = prepare_prefix(&mut store, &first);
    let before_set = counts(&store);
    let input = two_owner_set(&store, stamp, &first, &second, Some(prefix.clone()));
    let failure = store.commit_assistant_publication_set(&input).unwrap_err();
    assert!(failure
        .to_string()
        .contains("injected final publication frame"));
    assert_eq!(
        counts(&store),
        before_set,
        "all earlier public frames must roll back"
    );
    assert_eq!(store.assistant_journal_head(&first).unwrap().sequence, 0);
    assert_eq!(store.assistant_journal_head(&second).unwrap().sequence, 0);
    assert!(store
        .assistant_journal_part(&first, &part_id(&first))
        .unwrap()
        .is_none());
    assert!(store
        .assistant_journal_part(&second, &part_id(&second))
        .unwrap()
        .is_none());

    // The set savepoint does not pretend to own the source command. Its failed
    // caller must abort the outer UoW rather than commit checkpoint/effects alone.
    assert_eq!(store.load_checkpoint("set").unwrap().unwrap().revision, 2);
    assert!(store
        .outbox_effect("publication-fixture-effect")
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .command_receipt("publication-fixture-client", "publication-fixture-command")
            .unwrap()
            .unwrap()
            .status,
        "accepted"
    );
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(counts(&store), baseline);
    assert_eq!(store.load_checkpoint("set").unwrap().unwrap().revision, 1);
    assert!(store
        .outbox_effect("publication-fixture-effect")
        .unwrap()
        .is_none());
    assert!(store
        .command_receipt("publication-fixture-client", "publication-fixture-command")
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .connection
            .query_row::<i64, _, _>(
                "SELECT count(*) FROM messages WHERE id='set-successor'",
                [],
                |row| row.get(0)
            )
            .unwrap(),
        0
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_publication_final_frame")
        .unwrap();
    drop(store);

    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    assert_eq!(counts(&store), baseline);
    store.begin_write_unit_of_work().unwrap();
    admission_receipt(&mut store);
    source_transition(&mut store, &first, 2, &[second.clone()], true);
    assert_eq!(prepare_prefix(&mut store, &first), prefix);
    assert!(matches!(
        store.commit_assistant_publication_set(&input).unwrap(),
        AssistantPublicationCommit::Applied(_)
    ));
    // Only the successful set permits this fixture's actual receipt completion.
    store
        .complete_command_receipt(
            "publication-fixture-client",
            "publication-fixture-command",
            "2026-10-02T00:00:01Z",
            &CompleteCommandReceiptOutcome::Committed {
                committed_revision: Some(2),
                result_json: "{\"published\":true}".into(),
            },
        )
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    assert_published_body(
        &mut store,
        &first,
        "prepared 中文 prefix; first 中🙂\n\u{0} exact body",
    );
    assert_published_body(&mut store, &second, "successor 中文🙂 final body");
    assert_eq!(store.load_checkpoint("set").unwrap().unwrap().revision, 2);
    assert!(store
        .outbox_effect("publication-fixture-effect")
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .command_receipt("publication-fixture-client", "publication-fixture-command")
            .unwrap()
            .unwrap()
            .status,
        "committed"
    );
}

#[test]
fn final_frame_failure_can_retry_original_whole_set_inside_the_same_source_uow() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("savepoint-retry.db")).unwrap();
    let first = begin(&mut store, "set");
    let second = successor(&first, "successor");
    store.begin_write_unit_of_work().unwrap();
    let stamp = source_transition(&mut store, &first, 2, &[second.clone()], false);
    let input = two_owner_set(&store, stamp, &first, &second, None);
    store
        .connection
        .execute_batch(
            "CREATE TEMP TRIGGER fail_publication_final_frame
         BEFORE INSERT ON assistant_journal_records
         WHEN NEW.sequence=3 AND json_extract(NEW.owner,'$.assistant_message_id')='set-successor'
         BEGIN SELECT RAISE(ABORT,'injected final publication frame'); END;",
        )
        .unwrap();
    let before = counts(&store);
    assert!(store.commit_assistant_publication_set(&input).is_err());
    assert_eq!(counts(&store), before);
    store
        .connection
        .execute_batch("DROP TRIGGER fail_publication_final_frame")
        .unwrap();
    let cursors = applied(store.commit_assistant_publication_set(&input).unwrap());
    assert!(cursors.iter().all(|cursor| cursor.sequence == 3));
    assert_eq!(
        store.commit_assistant_publication_set(&input).unwrap(),
        AssistantPublicationCommit::Replay(cursors)
    );
    store.commit_write_unit_of_work().unwrap();
}
