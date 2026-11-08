// SPDX-License-Identifier: Apache-2.0
//! Immutable source journal reads survive real, trusted execution handoffs.
use super::*;
use crate::BeginInternalTurnInput;
use nola_domain::PartMutation;
#[path = "../../tests/support/terminal_sources.rs"]
mod terminal_sources;

const ORIGINAL: &str = "old中🙂answer\n";

#[test]
fn internal_successor_cannot_replace_a_still_active_run() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let old = begin(&mut store, "active-predecessor");
    let revision = store.checkpoint_revision(&old.conversation_id).unwrap();
    let result = store.begin_internal_turn(&BeginInternalTurnInput {
        initial_events: vec![], initial_assistant_part_mutations: vec![],
        conversation_id: old.conversation_id.clone(),
        assistant_message_id: old.assistant_message_id.clone(),
        expected_previous_run_id: old.run_id.clone(),
        expected_checkpoint_revision: revision,
        assistant_part_id: "forbidden-successor-part".into(),
        assistant_turn_bindings: vec![], run_id: "forbidden-successor".into(),
        context_checkpoint_id: None, checkpoint_revision: revision.unwrap() + 1,
        checkpoint_event_cursor: 0, checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(), invocation_config_digest: "fixture".into(),
        outbox_effects: vec![], model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![], delegated_permission_mutations: vec![],
        memory_usage_entries: vec![], created_at_ms: 10,
    });
    assert!(matches!(result, Err(crate::StoreError::Conflict(_))));
    assert_eq!(store.active_run(&old.conversation_id).unwrap().unwrap().run_id, old.run_id);
    assert_eq!(store.checkpoint_revision(&old.conversation_id).unwrap(), revision);
    assert_eq!(store.conversation_id_for_run("forbidden-successor").unwrap(), None);
}

fn total_changes(store: &NolaStore) -> i64 {
    store
        .connection
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap()
}

struct ResumedJournal {
    old: AssistantJournalIdentity,
    current: AssistantJournalIdentity,
    snapshot: AssistantJournalSnapshot,
    second_snapshot: AssistantJournalSnapshot,
    window: AssistantJournalReadWindow,
    part: JournalPartHead,
}

fn resumed(store: &mut NolaStore) -> ResumedJournal {
    let old = begin(store, "resume");
    declare(store, &old);
    store.begin_write_unit_of_work().unwrap();
    transition(store, &old, 3);
    let append = append_input(store, &old, 3, ORIGINAL);
    store.commit_assistant_journal(&append).unwrap();
    store.commit_write_unit_of_work().unwrap();

    store.begin_write_unit_of_work().unwrap();
    let mut finished = CommitTransitionInput {
            transition_id: "finish-old".into(),
            run_id: old.run_id.clone(),
            expected_checkpoint_revision: 3,
            checkpoint_revision: 4,
            checkpoint_event_cursor: 0,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
            submission_reduction_json: None, host_event: None,
            events: vec![],
            ensured_messages: vec![],
            assistant_turn_bindings: vec![],
            part_message_owners: Default::default(),
            part_mutations: vec![PartMutation::Close {
                part_id: "resume-part".into(),
                state: "completed".into(),
                content_json: None,
                content_digest: None,
                updated_at_ms: 4,
            }],
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            acknowledged_effect_ids: vec![],
            memory_usage_entries: vec![],
            run_status: None,
            assistant_status: Some("completed".into()),
            assistant_status_message_id: None,
            committed_at_ms: 4,
        };
    terminal_sources::add_sources(store,&mut finished,"resume","resume-actual-turn","resume-user",false);
    store.commit_transition(&finished).unwrap();
    let part_id = format!("{}-journal-part", old.run_id);
    let head = store
        .assistant_journal_part(&old, &part_id)
        .unwrap()
        .unwrap();
    store
        .commit_assistant_journal(&CommitAssistantJournal {
            stamp: AssistantSourceCommitStamp::Transition {
                transition_id: "finish-old".into(),
            },
            expected: store.assistant_journal_head(&old).unwrap(),
            batch: AssistantOperationBatchV1 {
                protocol_version: 1,
                identity: old.clone(),
                operations: vec![AssistantJournalOperationV1::Update(UpdateJournalMetadata {
                    part_id,
                    expected_generation: head.generation,
                    expected_revision: head.revision,
                    metadata: JournalPartMetadata::Text(JournalTextMetadata {
                        is_final: true,
                        visibility: JournalVisibility::Public,
                        review: None,
                    }),
                })],
            },
            supplied_chunks: vec![],
        })
        .unwrap();
    terminal_sources::publish_and_seal(store,&finished,"resume");
    let snapshot = store.open_assistant_journal_snapshot(&old).unwrap();
    let second_snapshot = store.open_assistant_journal_snapshot(&old).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let window = store
        .open_assistant_journal_read(&AssistantJournalCursor::initial(old.clone()).unwrap())
        .unwrap();
    let part = store
        .read_assistant_snapshot_parts(&snapshot, None, 1, 65536)
        .unwrap()
        .parts
        .remove(0);

    // Only the production admission may record the historical/current pair.
    store.begin_write_unit_of_work().unwrap();
    let target = store
        .internal_assistant_target(&old.conversation_id)
        .unwrap()
        .unwrap();
    let expected_checkpoint_revision = store.checkpoint_revision(&old.conversation_id).unwrap();
    let accepted = store
        .begin_internal_turn(&BeginInternalTurnInput {
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            conversation_id: old.conversation_id.clone(),
            assistant_message_id: target.assistant_message_id,
            expected_previous_run_id: target.run_id,
            expected_checkpoint_revision,
            assistant_part_id: "unused-part".into(),
            assistant_turn_bindings: vec![],
            run_id: "resume-next-run".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 5,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 5,
        })
        .unwrap();
    assert_eq!(accepted.assistant_message_id, old.assistant_message_id);
    let current = AssistantJournalIdentity {
        run_id: accepted.run_id,
        ..old.clone()
    };
    store
        .commit_assistant_journal(&CommitAssistantJournal {
            stamp: AssistantSourceCommitStamp::Admission {
                run_id: current.run_id.clone(),
            },
            expected: store.assistant_journal_head(&current).unwrap(),
            batch: declaration(&current),
            supplied_chunks: vec![],
        })
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    store.begin_write_unit_of_work().unwrap();
    transition(store, &current, 6);
    let append = append_input(store, &current, 6, "new current answer");
    store.commit_assistant_journal(&append).unwrap();
    store.commit_write_unit_of_work().unwrap();

    ResumedJournal {
        old,
        current,
        snapshot,
        second_snapshot,
        window,
        part,
    }
}

fn assert_original_reads(store: &NolaStore, fixture: &ResumedJournal) {
    assert_eq!(
        store.assistant_journal_head(&fixture.old).unwrap(),
        fixture.snapshot.at
    );
    assert_eq!(
        store
            .assistant_journal_part(&fixture.old, &fixture.part.part_id)
            .unwrap(),
        Some(fixture.part.clone())
    );
    assert_eq!(
        store
            .read_assistant_snapshot_parts(&fixture.snapshot, None, 1, 65536)
            .unwrap()
            .parts,
        vec![fixture.part.clone()]
    );
    let content = &fixture.part.bodies[0];
    let manifest = store
        .read_assistant_snapshot_manifest(&fixture.snapshot, &fixture.part.part_id, content, 0, 1)
        .unwrap();
    assert!(manifest.complete);
    assert_eq!(manifest.chunks.len(), 1);
    assert_eq!(
        store
            .read_assistant_snapshot_chunk(&fixture.snapshot, &fixture.part.part_id, content, 0)
            .unwrap()
            .text(),
        ORIGINAL
    );
    let mut original = String::new();
    let mut offset = 0;
    loop {
        let range = store
            .read_assistant_snapshot_range(
                &fixture.snapshot,
                &fixture.part.part_id,
                content,
                offset,
                4,
            )
            .unwrap();
        assert!(range.text.len() <= 4);
        assert_eq!(range.total_utf8_bytes, ORIGINAL.len() as u64);
        original.push_str(&range.text);
        offset = range.next_offset_bytes;
        if range.end_of_content {
            break;
        }
    }
    assert_eq!(original, ORIGINAL);
    for window in [
        fixture.window.clone(),
        store
            .open_assistant_journal_read(&fixture.window.after)
            .unwrap(),
    ] {
        let AssistantJournalPage::Records {
            records,
            next,
            complete,
        } = store
            .read_assistant_journal_page(&window, 128, ASSISTANT_FRAME_MAX_BYTES)
            .unwrap()
        else {
            panic!("trusted journal must remain readable");
        };
        assert_eq!(records.len(), 3);
        assert!(complete);
        assert_eq!(next, fixture.snapshot.at);
    }
}

#[test]
fn trusted_resumption_preserves_old_journal_reads_and_cold_release_without_old_write_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let mut store = NolaStore::open(&path).unwrap();
    let fixture = resumed(&mut store);
    assert_original_reads(&store, &fixture);
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert_original_reads(&store, &fixture);
    assert_eq!(
        store
            .assistant_journal_head(&fixture.current)
            .unwrap()
            .sequence,
        2
    );

    // A new read handle may pin the historical owner; it grants no authoring.
    store.begin_write_unit_of_work().unwrap();
    let historical = store.open_assistant_journal_snapshot(&fixture.old).unwrap();
    assert_eq!(historical.at, fixture.snapshot.at);
    store.commit_write_unit_of_work().unwrap();
    let table_counts = |store: &NolaStore| {
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
            "committed_transitions",
        ]
        .into_iter()
        .map(|table| {
            store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        })
        .collect::<Vec<_>>()
    };
    let before_counts = table_counts(&store);
    let before_heads = [
        store.assistant_journal_head(&fixture.old).unwrap(),
        store.assistant_journal_head(&fixture.current).unwrap(),
    ];
    let before_checkpoint = store
        .load_checkpoint(&fixture.current.conversation_id)
        .unwrap()
        .unwrap();
    let conversation_revision = |store: &NolaStore| {
        store
            .connection
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                [&fixture.current.conversation_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    let before_conversation_revision = conversation_revision(&store);
    store.begin_write_unit_of_work().unwrap();
    // Handoff read provenance alone cannot authorize historical writes: the
    // message now belongs to the successor Run, not this old journal identity.
    let current_source = transition(&mut store, &fixture.current, 7);
    let before_rejected_publication = total_changes(&store);
    let mut append = append_input(&store, &fixture.old, 7, "forbidden old Run write");
    append.stamp = AssistantSourceCommitStamp::Transition {
        transition_id: current_source,
    };
    let error = store.commit_assistant_journal(&append).unwrap_err();
    assert!(error.to_string().contains("journal source owner or epoch mismatch"), "{error}");
    let chunk = AssistantTextChunkV1::new("forbidden prepared content".into()).unwrap();
    let error = store
        .prepare_assistant_content(
            &fixture.old,
            &fixture.part.part_id,
            &fixture.part.bodies[0],
            &[chunk],
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("journal source owner or epoch mismatch"));
    assert_eq!(total_changes(&store), before_rejected_publication);
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(table_counts(&store), before_counts);
    assert_eq!(
        [
            store.assistant_journal_head(&fixture.old).unwrap(),
            store.assistant_journal_head(&fixture.current).unwrap(),
        ],
        before_heads
    );
    assert_eq!(
        store
            .load_checkpoint(&fixture.current.conversation_id)
            .unwrap()
            .unwrap(),
        before_checkpoint
    );
    assert_eq!(conversation_revision(&store), before_conversation_revision);
    assert_original_reads(&store, &fixture);

    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .release_assistant_journal_snapshot(&fixture.snapshot)
            .unwrap(),
        AssistantSnapshotRelease::Released
    );
    assert_eq!(
        store
            .release_assistant_journal_snapshot(&fixture.snapshot)
            .unwrap(),
        AssistantSnapshotRelease::AlreadyReleased
    );
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert!(store
        .read_assistant_snapshot_parts(&fixture.snapshot, None, 1, 65536)
        .is_err());
    assert!(store
        .read_assistant_snapshot_range(
            &fixture.snapshot,
            &fixture.part.part_id,
            &fixture.part.bodies[0],
            0,
            4,
        )
        .is_err());
    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .release_assistant_journal_snapshot(&fixture.snapshot)
            .unwrap(),
        AssistantSnapshotRelease::AlreadyReleased
    );
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .read_assistant_snapshot_chunk(
                &fixture.second_snapshot,
                &fixture.part.part_id,
                &fixture.part.bodies[0],
                0,
            )
            .unwrap()
            .text(),
        ORIGINAL
    );
}

#[test]
fn historical_journal_requires_both_trusted_handoff_members_and_exact_local_owner() {
    for tamper in ["current", "historical", "unknown-current", "remote-source"] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(dir.path().join("journal.db")).unwrap();
        let fixture = resumed(&mut store);
        match tamper {
            "current" | "historical" => {
                let run = if tamper == "current" {
                    &fixture.current.run_id
                } else {
                    &fixture.old.run_id
                };
                store
                    .connection
                    .execute(
                        "DELETE FROM canonical_message_run_owners WHERE run_id=?1",
                        [run],
                    )
                    .unwrap();
            }
            "unknown-current" => {
                // Historical membership alone cannot authorize an unknown owner.
                store
                    .connection
                    .execute(
                        "UPDATE messages SET run_id='unknown' WHERE id=?1",
                        [&fixture.old.assistant_message_id],
                    )
                    .unwrap();
            }
            "remote-source" => {
                store
                    .connection
                    .execute(
                        "INSERT INTO remote_run_sources(run_id,source_host_id) VALUES(?1,'remote')",
                        [&fixture.old.run_id],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            store.assistant_journal_head(&fixture.old).is_err(),
            "{tamper}"
        );
        assert!(
            store
                .assistant_journal_part(&fixture.old, &fixture.part.part_id)
                .is_err(),
            "{tamper}"
        );
        assert!(
            store
                .open_assistant_journal_read(&fixture.window.after)
                .is_err(),
            "{tamper}"
        );
        assert!(
            store
                .read_assistant_journal_page(&fixture.window, 128, ASSISTANT_FRAME_MAX_BYTES)
                .is_err(),
            "{tamper}"
        );
        assert!(
            store
                .read_assistant_snapshot_parts(&fixture.snapshot, None, 1, 65536)
                .is_err(),
            "{tamper}"
        );
        assert!(
            store
                .read_assistant_snapshot_range(
                    &fixture.snapshot,
                    &fixture.part.part_id,
                    &fixture.part.bodies[0],
                    0,
                    4,
                )
                .is_err(),
            "{tamper}"
        );
        let before = total_changes(&store);
        store.begin_write_unit_of_work().unwrap();
        assert!(
            store.open_assistant_journal_snapshot(&fixture.old).is_err(),
            "{tamper}"
        );
        assert!(
            store
                .release_assistant_journal_snapshot(&fixture.snapshot)
                .is_err(),
            "{tamper}"
        );
        store.rollback_write_unit_of_work().unwrap();
        assert_eq!(total_changes(&store), before);
    }

    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("journal.db")).unwrap();
    let fixture = resumed(&mut store);
    for field in ["epoch", "conversation", "message", "run"] {
        let mut forged = fixture.snapshot.clone();
        let identity = &mut forged.at.identity;
        match field {
            "epoch" => identity.source_store_epoch = "other-epoch".into(),
            "conversation" => identity.conversation_id = "other-conversation".into(),
            "message" => identity.assistant_message_id = "other-message".into(),
            "run" => identity.run_id = fixture.current.run_id.clone(),
            _ => unreachable!(),
        }
        assert!(store
            .read_assistant_snapshot_parts(&forged, None, 1, 65536)
            .is_err());
        store.begin_write_unit_of_work().unwrap();
        assert!(store.release_assistant_journal_snapshot(&forged).is_err());
        store.rollback_write_unit_of_work().unwrap();
    }
    assert_original_reads(&store, &fixture);
}
