// SPDX-License-Identifier: Apache-2.0
use super::*;
#[path = "field_tests.rs"]
mod field_tests;
use crate::{BeginTurnInput, CommitTransitionInput};
#[path = "planning_read_tests.rs"]
mod planning_read_tests;
#[path = "publication_set_tests.rs"]
mod publication_set_tests;
#[path = "range_tests.rs"]
mod range_tests;
#[path = "reads_tests.rs"]
mod reads_tests;
#[path = "resumption_tests.rs"]
mod resumption_tests;
#[path = "snapshot_lifecycle_tests.rs"]
mod snapshot_lifecycle_tests;
#[path = "receive_tests.rs"]
mod receive_tests;
#[path = "terminal_receive_tests.rs"]
mod terminal_receive_tests;
#[path = "successor_admission_tests.rs"]
mod successor_admission_tests;

fn begin(store: &mut NolaStore, name: &str) -> AssistantJournalIdentity {
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            conversation_id: name.into(),
            conversation_title: "Journal test".into(),
            user_message_id: format!("{name}-user"),
            user_part_id: format!("{name}-user-part"),
            user_text: "Continue".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: format!("{name}-assistant"),
            assistant_part_id: format!("{name}-part"),
            assistant_turn_bindings: vec![
                nola_domain::AssistantTurnBinding::Admission {
                    turn_id: format!("{name}-actual-turn"),
                    assistant_message_id: format!("{name}-assistant"),
                },
                nola_domain::AssistantTurnBinding::Part {
                    turn_id: format!("{name}-actual-turn"),
                    assistant_message_id: format!("{name}-assistant"),
                    part_id: format!("{name}-part"),
                },
            ],
            run_id: format!("{name}-run"),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "test-config".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 1,
        })
        .unwrap();
    AssistantJournalIdentity {
        source_store_epoch: store.assistant_source_epoch().unwrap(),
        conversation_id: name.into(),
        run_id: format!("{name}-run"),
        assistant_message_id: format!("{name}-assistant"),
    }
}
fn transition(store: &mut NolaStore, id: &AssistantJournalIdentity, revision: i64) -> String {
    let transition_id = format!("{}-{revision}", id.run_id);
    assert!(
        store
            .commit_transition(&CommitTransitionInput {
                transition_id: transition_id.clone(),
                run_id: id.run_id.clone(),
                expected_checkpoint_revision: revision - 1,
                checkpoint_revision: revision,
                checkpoint_event_cursor: 0,
                checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
                submission_reduction_json: None, host_event: None,
                events: vec![],
                ensured_messages: vec![],
                assistant_turn_bindings: vec![],
                part_message_owners: Default::default(),
                part_mutations: vec![],
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
                committed_at_ms: revision,
            })
            .unwrap()
            .applied
    );
    transition_id
}
fn declaration(id: &AssistantJournalIdentity) -> AssistantOperationBatchV1 {
    let part_id = format!("{}-journal-part", id.run_id);
    AssistantOperationBatchV1 {
        protocol_version: 1,
        identity: id.clone(),
        operations: vec![AssistantJournalOperationV1::Declare(DeclareJournalPart {
            part: JournalPartHead {
                identity: id.clone(),
                part_id: part_id.clone(),
                generation: JournalNumber(1),
                revision: JournalNumber(1),
                metadata_revision: JournalNumber(1),
                order_key: JournalOrderKey {
                    section: JournalSection::Transcript,
                    position: JournalNumber(0),
                },
                metadata: JournalPartMetadata::Text(JournalTextMetadata {
                    is_final: false,
                    visibility: JournalVisibility::Public,
                    review: None,
                }),
                bodies: vec![empty_journal_content(
                    id,
                    &part_id,
                    JournalContentField::Markdown,
                    JournalNumber(1),
                )
                .unwrap()],
            },
        })],
    }
}
fn declare(store: &mut NolaStore, id: &AssistantJournalIdentity) -> CommitAssistantJournal {
    store.begin_write_unit_of_work().unwrap();
    let stamp = transition(store, id, 2);
    let input = CommitAssistantJournal {
        stamp: AssistantSourceCommitStamp::Transition {
            transition_id: stamp,
        },
        expected: store.assistant_journal_head(id).unwrap(),
        batch: declaration(id),
        supplied_chunks: vec![],
    };
    assert!(matches!(
        store.commit_assistant_journal(&input).unwrap(),
        AssistantJournalCommit::Applied(_)
    ));
    store.commit_write_unit_of_work().unwrap();
    input
}
fn append_input(
    store: &NolaStore,
    id: &AssistantJournalIdentity,
    revision: i64,
    text: &str,
) -> CommitAssistantJournal {
    let part = format!("{}-journal-part", id.run_id);
    let reader = content::Reader {
        c: &store.connection,
        id,
        supplied: &[],
    };
    let head = reader.load_part_head(&part).unwrap().unwrap();
    let expected = head.bodies[0].clone();
    let chunk = AssistantTextChunkV1::new(text.into()).unwrap();
    let refs = vec![JournalChunkRef {
        digest: chunk.digest(),
        utf8_bytes: JournalNumber(chunk.byte_length() as u64),
    }];
    CommitAssistantJournal {
        stamp: AssistantSourceCommitStamp::Transition {
            transition_id: format!("{}-{revision}", id.run_id),
        },
        expected: store.assistant_journal_head(id).unwrap(),
        batch: AssistantOperationBatchV1 {
            protocol_version: 1,
            identity: id.clone(),
            operations: vec![AssistantJournalOperationV1::Append(AppendJournalText {
                part_id: part,
                expected_generation: head.generation,
                expected_revision: head.revision,
                resulting: append_journal_content(&expected, &refs).unwrap(),
                expected,
                chunks: refs,
            })],
        },
        supplied_chunks: vec![chunk],
    }
}

#[test]
fn epoch_is_local_durable_and_not_a_schema_constant() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let first = NolaStore::open(&path)
        .unwrap()
        .assistant_source_epoch()
        .unwrap();
    assert_eq!(
        NolaStore::open(&path)
            .unwrap()
            .assistant_source_epoch()
            .unwrap(),
        first
    );
    assert_ne!(
        NolaStore::open(dir.path().join("other.db"))
            .unwrap()
            .assistant_source_epoch()
            .unwrap(),
        first
    );
}

#[test]
fn real_transition_stamp_owner_replay_and_rollback_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    let input = CommitAssistantJournal {
        stamp: AssistantSourceCommitStamp::Transition {
            transition_id: "unknown".into(),
        },
        expected: store.assistant_journal_head(&id).unwrap(),
        batch: declaration(&id),
        supplied_chunks: vec![],
    };
    assert!(store.commit_assistant_journal(&input).is_err());
    store.begin_write_unit_of_work().unwrap();
    let stamp = transition(&mut store, &id, 2);
    store.rollback_write_unit_of_work().unwrap();
    store.begin_write_unit_of_work().unwrap();
    let mut rolled = input.clone();
    rolled.stamp = AssistantSourceCommitStamp::Transition {
        transition_id: stamp,
    };
    assert!(store.commit_assistant_journal(&rolled).is_err());
    store.rollback_write_unit_of_work().unwrap();
    let committed = declare(&mut store, &id);
    store.begin_write_unit_of_work().unwrap();
    assert!(matches!(
        store.commit_assistant_journal(&committed).unwrap(),
        AssistantJournalCommit::Replay(_)
    ));
    let mut changed = committed.clone();
    changed.batch.operations.clear();
    assert!(store.commit_assistant_journal(&changed).is_err());
    let mut wrong = committed.clone();
    wrong.batch.identity.assistant_message_id = "other".into();
    assert!(store.commit_assistant_journal(&wrong).is_err());
    store.rollback_write_unit_of_work().unwrap();
    // A persisted transition never stamped in this UoW cannot first-publish.
    transition(&mut store, &id, 3);
    let late = append_input(&store, &id, 3, "late");
    store.begin_write_unit_of_work().unwrap();
    assert!(store.commit_assistant_journal(&late).is_err());
    store.rollback_write_unit_of_work().unwrap();
}

#[test]
fn source_append_snapshot_and_fixed_watermark_survive_concurrent_growth() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut store = NolaStore::open(&path).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    store.begin_write_unit_of_work().unwrap();
    transition(&mut store, &id, 3);
    let input = append_input(&store, &id, 3, "你好 🌍");
    store.commit_assistant_journal(&input).unwrap();
    let snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let genesis = AssistantJournalCursor::initial(id.clone()).unwrap();
    let window = store.open_assistant_journal_read(&genesis).unwrap();
    let old = store
        .read_assistant_snapshot_parts(&snapshot, None, 128, ASSISTANT_FRAME_MAX_BYTES)
        .unwrap()
        .parts[0]
        .clone();
    store.begin_write_unit_of_work().unwrap();
    transition(&mut store, &id, 4);
    let next = append_input(&store, &id, 4, " second");
    store.commit_assistant_journal(&next).unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(
        store
            .read_assistant_snapshot_parts(&snapshot, None, 128, ASSISTANT_FRAME_MAX_BYTES)
            .unwrap()
            .parts,
        vec![old.clone()]
    );
    assert_eq!(
        store
            .read_assistant_snapshot_chunk(&snapshot, &old.part_id, &old.bodies[0], 0)
            .unwrap()
            .text(),
        "你好 🌍"
    );
    assert!(store
        .read_assistant_snapshot_chunk(&snapshot, &old.part_id, &old.bodies[0], 1)
        .is_err());
    let AssistantJournalPage::Records {
        records,
        next,
        complete,
    } = store
        .read_assistant_journal_page(&window, 128, ASSISTANT_FRAME_MAX_BYTES)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(records.len(), 2);
    assert!(complete);
    assert_eq!(next.sequence, 2);
    assert_eq!(store.assistant_journal_head(&id).unwrap().sequence, 3);
}

#[test]
fn forged_global_chunk_digest_and_cross_snapshot_reference_cannot_grant_access() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let a = begin(&mut store, "a");
    let b = begin(&mut store, "b");
    declare(&mut store, &a);
    declare(&mut store, &b);
    store.begin_write_unit_of_work().unwrap();
    transition(&mut store, &a, 3);
    let input = append_input(&store, &a, 3, "private bytes");
    store.commit_assistant_journal(&input).unwrap();
    let sa = store.open_assistant_journal_snapshot(&a).unwrap();
    store.commit_write_unit_of_work().unwrap();
    store.begin_write_unit_of_work().unwrap();
    transition(&mut store, &b, 3);
    let mut forged = append_input(&store, &b, 3, "private bytes");
    forged.supplied_chunks.clear();
    assert!(store.commit_assistant_journal(&forged).is_err());
    assert_eq!(store.assistant_journal_head(&b).unwrap().sequence, 1);
    let sb = store.open_assistant_journal_snapshot(&b).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let part = store
        .read_assistant_snapshot_parts(&sa, None, 10, 65536)
        .unwrap()
        .parts[0]
        .clone();
    assert!(store
        .read_assistant_snapshot_chunk(&sb, &part.part_id, &part.bodies[0], 0)
        .is_err());
}

#[test]
fn failed_record_insert_rolls_back_chunks_heads_and_nested_retry_is_safe() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    store.begin_write_unit_of_work().unwrap();
    store.begin_command_mutations().unwrap();
    transition(&mut store, &id, 3);
    store.rollback_command_mutations().unwrap();
    let pending = append_input(&store, &id, 3, "should not leak");
    assert!(store.commit_assistant_journal(&pending).is_err());
    transition(&mut store, &id, 3);
    store.connection.execute_batch("CREATE TEMP TRIGGER fail_journal BEFORE INSERT ON assistant_journal_records BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store.commit_assistant_journal(&pending).is_err());
    assert_eq!(store.assistant_journal_head(&id).unwrap().sequence, 1);
    let chunks: i64 = store
        .connection
        .query_row("SELECT count(*) FROM assistant_journal_chunks", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(chunks, 0);
    store
        .connection
        .execute_batch("DROP TRIGGER fail_journal")
        .unwrap();
    assert!(matches!(
        store.commit_assistant_journal(&pending).unwrap(),
        AssistantJournalCommit::Applied(_)
    ));
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn removed_part_id_cannot_restart_at_generation_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    store.begin_write_unit_of_work().unwrap();
    transition(&mut store, &id, 3);
    let input = CommitAssistantJournal {
        stamp: AssistantSourceCommitStamp::Transition {
            transition_id: format!("{}-3", id.run_id),
        },
        expected: store.assistant_journal_head(&id).unwrap(),
        batch: AssistantOperationBatchV1 {
            protocol_version: 1,
            identity: id.clone(),
            operations: vec![AssistantJournalOperationV1::Remove(RemoveJournalPart {
                part_id: format!("{}-journal-part", id.run_id),
                expected_generation: JournalNumber(1),
                expected_revision: JournalNumber(1),
            })],
        },
        supplied_chunks: vec![],
    };
    store.commit_assistant_journal(&input).unwrap();
    store.commit_write_unit_of_work().unwrap();
    store.begin_write_unit_of_work().unwrap();
    transition(&mut store, &id, 4);
    let redeclare = CommitAssistantJournal {
        stamp: AssistantSourceCommitStamp::Transition {
            transition_id: format!("{}-4", id.run_id),
        },
        expected: store.assistant_journal_head(&id).unwrap(),
        batch: declaration(&id),
        supplied_chunks: vec![],
    };
    assert!(store.commit_assistant_journal(&redeclare).is_err());
    store.rollback_write_unit_of_work().unwrap();
}

#[test]
fn admission_stamp_publishes_visible_initial_part_and_does_not_survive_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    store.begin_write_unit_of_work().unwrap();
    let id = begin(&mut store, "admission");
    let input = CommitAssistantJournal {
        stamp: AssistantSourceCommitStamp::Admission {
            run_id: id.run_id.clone(),
        },
        expected: store.assistant_journal_head(&id).unwrap(),
        batch: declaration(&id),
        supplied_chunks: vec![],
    };
    store.commit_assistant_journal(&input).unwrap();
    store.rollback_write_unit_of_work().unwrap();
    assert!(store.assistant_journal_head(&id).is_err());
    store.begin_write_unit_of_work().unwrap();
    let id = begin(&mut store, "admission");
    store.commit_assistant_journal(&input).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(store.assistant_journal_head(&id).unwrap().sequence, 1);
    store.begin_write_unit_of_work().unwrap();
    assert!(matches!(
        store.commit_assistant_journal(&input).unwrap(),
        AssistantJournalCommit::Replay(_)
    ));
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn successor_assistant_is_owned_by_same_run_not_initial_message_slot() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    let mut next = id.clone();
    next.assistant_message_id = "successor-assistant".into();
    store.begin_write_unit_of_work().unwrap();
    // This is the canonical ensure-message path used by successor Turns.
    let transition_id = "successor-commit";
    store
        .commit_transition(&CommitTransitionInput {
            transition_id: transition_id.into(),
            run_id: id.run_id.clone(),
            expected_checkpoint_revision: 1,
            checkpoint_revision: 2,
            checkpoint_event_cursor: 0,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
            submission_reduction_json: None, host_event: None,
            events: vec![],
            ensured_messages: vec![nola_domain::TransitionMessageInput {
                id: next.assistant_message_id.clone(),
                run_id: Some(id.run_id.clone()),
                role: "assistant".into(),
                status: "streaming".into(),
                created_at_ms: 2,
                completed_at_ms: None,
                initial_parts: vec![],
            }],
            assistant_turn_bindings: vec![],
            part_message_owners: Default::default(),
            part_mutations: vec![],
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
            committed_at_ms: 2,
        })
        .unwrap();
    let input = CommitAssistantJournal {
        stamp: AssistantSourceCommitStamp::Transition {
            transition_id: transition_id.into(),
        },
        expected: store.assistant_journal_head(&next).unwrap(),
        batch: declaration(&next),
        supplied_chunks: vec![],
    };
    store.commit_assistant_journal(&input).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(store.assistant_journal_head(&next).unwrap().sequence, 1);
    assert_eq!(store.assistant_journal_head(&id).unwrap().sequence, 0);
    let mut wrong = next.clone();
    wrong.run_id = "different-run".into();
    assert!(store.assistant_journal_head(&wrong).is_err());
}

#[test]
fn forged_read_watermarks_and_snapshot_identity_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    let mut window = store
        .open_assistant_journal_read(&AssistantJournalCursor::initial(id.clone()).unwrap())
        .unwrap();
    window.through.record_digest = window.after.record_digest;
    assert!(store
        .read_assistant_journal_page(&window, 10, 65536)
        .is_err());
    window.through = store.assistant_journal_head(&id).unwrap();
    window.through.identity.assistant_message_id = "foreign".into();
    assert!(store
        .read_assistant_journal_page(&window, 10, 65536)
        .is_err());
    store.begin_write_unit_of_work().unwrap();
    let mut snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    snapshot.at.sequence = 0;
    assert!(store
        .read_assistant_snapshot_parts(&snapshot, None, 10, 65536)
        .is_err());
}


#[test]
fn source_storage_growth_tracks_new_bytes_not_cumulative_body_length() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "large");
    declare(&mut store, &id);
    let mut cumulative = 0i64;
    for index in 0..128 {
        let text = format!("{index:04}{}", "x".repeat(65532));
        store.begin_write_unit_of_work().unwrap();
        transition(&mut store, &id, index + 3);
        let input = append_input(&store, &id, index + 3, &text);
        store.commit_assistant_journal(&input).unwrap();
        store.commit_write_unit_of_work().unwrap();
        cumulative += text.len() as i64;
        if index == 31 || index == 127 {
            let bytes: i64 = store
                .connection
                .query_row(
                    "SELECT SUM(length(bytes)) FROM assistant_journal_chunks",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(bytes, cumulative);
            let journal_bytes: i64 = store
                .connection
                .query_row(
                    "SELECT SUM(length(batch)+length(header_json)) FROM assistant_journal_records",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(
                journal_bytes < (index + 2) * 4096,
                "journal must not contain cumulative bodies"
            );
            let refs: i64 = store
                .connection
                .query_row(
                    "SELECT count(*) FROM assistant_journal_manifest_chunks",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(refs, index + 1);
        }
    }
}
