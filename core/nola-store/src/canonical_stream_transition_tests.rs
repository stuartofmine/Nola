// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_domain::{
    apply_canonical_stream, CanonicalStreamHead, CanonicalStreamOperation, CanonicalStreamOwner,
    CanonicalTextState, PartBodyField,
};
use nola_store_port::run_effect::RunEffectRepository;
#[path = "../tests/support/terminal_sources.rs"]
mod terminal_sources;

fn owner() -> CanonicalStreamOwner {
    CanonicalStreamOwner {
        conversation_id: "c".into(),
        run_id: "r".into(),
        message_id: "a".into(),
        part_id: "p".into(),
        field: PartBodyField::ContentText,
    }
}
fn begin(store: &mut NolaStore) {
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![],
            conversation_id: "c".into(),
            conversation_title: "stream".into(),
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
            created_at_ms: 1,
        })
        .unwrap();
}
fn transition(id: &str, revision: i64, mutations: Vec<PartMutation>) -> CommitTransitionInput {
    CommitTransitionInput {
        transition_id: id.into(),
        run_id: "r".into(),
        expected_checkpoint_revision: revision,
        checkpoint_revision: revision + 1,
        checkpoint_event_cursor: 0,
        checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline {
            checkpoint_json: "{}".into(),
        },
        submission_reduction_json: None,
        host_event: None,
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
fn stream(operation: CanonicalStreamOperation) -> PartMutation {
    PartMutation::Stream {
        operation,
        updated_at_ms: 2,
    }
}
fn initial(text: &str) -> (Vec<PartMutation>, CanonicalStreamHead) {
    let declare = CanonicalStreamOperation::Declare { owner: owner() };
    let empty = apply_canonical_stream(None, &declare).unwrap();
    let append = CanonicalStreamOperation::Append {
        owner: owner(),
        expected_head: empty.clone(),
        delta: text.into(),
    };
    let next = apply_canonical_stream(Some(&empty), &append).unwrap();
    (vec![stream(declare), stream(append)], next)
}
fn body(store: &NolaStore) -> String {
    store
        .load_conversation("c")
        .unwrap()
        .unwrap()
        .messages
        .into_iter()
        .find(|m| m.id == "a")
        .unwrap()
        .parts[0]
        .content_text
        .clone()
}
fn resume(store: &mut NolaStore) -> StoreResult<BeginTurnResult> {
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
        run_id: "next-run".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 4,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "fixture".into(),
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        memory_usage_entries: vec![],
        created_at_ms: 4,
    })
}
fn copy_to_replica(source: &mut NolaStore, target: &mut NolaStore) {
    let mut cursor = 0;
    loop {
        let changes = source.pending_sync_changes(16).unwrap();
        if changes.is_empty() {
            break;
        }
        for _ in 0..2 {
            let page = ApplyRemoteSyncPageInput {
                replica_id: "remote".into(),
                next_cursor: cursor + changes.len() as i64,
                applied_at_ms: 9,
                changes: changes
                    .iter()
                    .enumerate()
                    .map(|(i, c)| RemoteSyncChangeInput {
                        cursor: cursor + i as i64 + 1,
                        operation_id: c.operation_id.clone(),
                        entity_type: c.entity_type.clone(),
                        entity_id: c.entity_id.clone(),
                        revision: c.revision,
                        deleted: c.deleted,
                        payload_json: c.payload_json.clone(),
                        created_at_ms: c.created_at_ms,
                    })
                    .collect(),
            };
            cursor = target.apply_remote_sync_page(&page).unwrap().cursor;
        }
        for change in changes {
            source
                .resolve_sync_change(&change.operation_id, "acknowledged", Some(cursor), 9)
                .unwrap();
        }
    }
}

#[test]
fn committed_stream_replica_without_runtime_run_replay_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("source.db");
    let replica_path = dir.path().join("replica.db");
    let mut source = NolaStore::open(&source_path).unwrap();
    begin(&mut source);
    let (mutations, head) = initial("中😀tail");
    let first = transition("first", 1, mutations);
    assert!(source.commit_transition(&first).unwrap().applied);
    assert!(!source.commit_transition(&first).unwrap().applied);
    let close = CanonicalStreamOperation::Close {
        owner: owner(),
        expected_head: head.clone(),
    };
    let mut last = transition(
        "close",
        2,
        vec![
            stream(close),
            PartMutation::Close {
                part_id: "p".into(),
                state: "completed".into(),
                content_json: Some("{}".into()),
                content_digest: Some(format!("{:x}", Sha256::digest("中😀tail".as_bytes()))),
                updated_at_ms: 3,
            },
        ],
    );
    last.assistant_status = Some("completed".into());
    terminal_sources::add_sources(&source, &mut last, "c", "r", "u", false);
    source.begin_write_unit_of_work().unwrap();
    source.commit_transition(&last).unwrap();
    terminal_sources::publish_and_seal(&mut source, &last, "c");
    source.commit_write_unit_of_work().unwrap();
    // Reuse the visible assistant without rewriting immutable Part ownership.
    assert_eq!(resume(&mut source).unwrap().assistant_message_id, "a");
    assert_eq!(body(&source), "中😀tail");
    assert_eq!(
        source.active_run("c").unwrap().unwrap().assistant_part_id,
        None
    );
    let initial: String = source
        .connection
        .query_row(
            "SELECT part_id FROM canonical_run_initial_parts WHERE run_id='r'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(initial, "p");
    let mut replica = NolaStore::open(&replica_path).unwrap();
    copy_to_replica(&mut source, &mut replica);
    let runs: i64 = replica
        .connection
        .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(runs, 0);
    assert_eq!(body(&replica), "中😀tail");
    // Historical read provenance must not grant the previous Run write access.
    assert!(replica.load_canonical_text_state(&owner()).is_err());
    replica
        .connection
        .execute("UPDATE messages SET run_id='unknown-run' WHERE id='a'", [])
        .unwrap();
    assert!(replica.load_conversation("c").is_err());
    replica
        .connection
        .execute("UPDATE messages SET run_id='next-run' WHERE id='a'", [])
        .unwrap();
    let tx = replica.connection.transaction().unwrap();
    assert!(crate::canonical_stream_store::apply(
        &tx,
        &CanonicalStreamOperation::Append {
            owner: owner(),
            expected_head: head,
            delta: "forbidden".into(),
        }
    )
    .is_err());
    drop(tx);
    drop(source);
    drop(replica);
    assert_eq!(body(&NolaStore::open(source_path).unwrap()), "中😀tail");
    assert_eq!(body(&NolaStore::open(replica_path).unwrap()), "中😀tail");
}

#[test]
fn incorrect_full_digest_rolls_back_stream_close_and_allows_correct_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("digest-close.sqlite")).unwrap();
    begin(&mut store);
    let text = "中😀tail";
    let (mutations, head) = initial(text);
    store
        .commit_transition(&transition("first", 1, mutations))
        .unwrap();
    let close = |digest: String| {
        transition(
            "digest-close",
            2,
            vec![
                stream(CanonicalStreamOperation::Close {
                    owner: owner(),
                    expected_head: head.clone(),
                }),
                PartMutation::Close {
                    part_id: "p".into(),
                    state: "completed".into(),
                    content_json: Some("{\"closed\":true}".into()),
                    content_digest: Some(digest),
                    updated_at_ms: 3,
                },
            ],
        )
    };
    let before: (String, Option<String>, Option<String>) = store
        .connection
        .query_row(
            "SELECT state,content_json,content_digest FROM message_parts WHERE id='p'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(store
        .commit_transition(&close(format!("{:x}", Sha256::digest(b"wrong"))))
        .is_err());
    assert_eq!(store.load_checkpoint("c").unwrap().unwrap().revision, 2);
    assert_eq!(
        crate::canonical_stream_read::resolve(&store.connection, "c", "a", "p").unwrap(),
        Some(head.clone())
    );
    let after: (String, Option<String>, Option<String>) = store
        .connection
        .query_row(
            "SELECT state,content_json,content_digest FROM message_parts WHERE id='p'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(before, after);
    let closed_manifests: i64 = store
        .connection
        .query_row(
            "SELECT count(*) FROM canonical_stream_manifests WHERE operation='close'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(closed_manifests, 0);
    let correct = close(format!("{:x}", Sha256::digest(text.as_bytes())));
    assert!(store.commit_transition(&correct).unwrap().applied);
    assert!(!store.commit_transition(&correct).unwrap().applied);
    assert_eq!(store.load_checkpoint("c").unwrap().unwrap().revision, 3);
    assert!(
        crate::canonical_stream_read::resolve(&store.connection, "c", "a", "p")
            .unwrap()
            .unwrap()
            .closed
    );
    assert_eq!(body(&store), text);
}

#[test]
fn stream_failure_rolls_back_checkpoint_chunks_receipt_and_old_writers_cannot_clobber() {
    let mut store = NolaStore::open(":memory:").unwrap();
    begin(&mut store);
    assert!(crate::canonical_stream_store::apply(
        &store.connection,
        &CanonicalStreamOperation::Declare { owner: owner() },
    )
    .is_err());
    for wrong in ["conversation", "message", "run"] {
        let mut expected = owner();
        match wrong {
            "conversation" => expected.conversation_id = "other".into(),
            "message" => expected.message_id = "other".into(),
            _ => expected.run_id = "other".into(),
        }
        assert!(store.load_canonical_text_state(&expected).is_err());
    }
    let mut missing = owner();
    missing.part_id = "new-part".into();
    assert_eq!(
        store.load_canonical_text_state(&missing).unwrap(),
        CanonicalTextState::Missing
    );
    let (mut mutations, head) = initial("first");
    mutations.push(PartMutation::Close {
        part_id: "p".into(),
        state: "completed".into(),
        content_json: None,
        content_digest: None,
        updated_at_ms: 2,
    });
    assert!(store
        .commit_transition(&transition("failed", 1, mutations))
        .is_err());
    assert_eq!(store.load_checkpoint("c").unwrap().unwrap().revision, 1);
    let count: i64 = store
        .connection
        .query_row("SELECT count(*) FROM canonical_stream_heads", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let (mutations, _) = initial("first");
    store
        .commit_transition(&transition("failed", 1, mutations))
        .unwrap();
    for mutation in [
        PartMutation::AppendText {
            part_id: "p".into(),
            expected_content_digest: format!("{:x}", Sha256::digest(b"")),
            text: "bad".into(),
            updated_at_ms: 3,
        },
        PartMutation::Metadata {
            part_id: "p".into(),
            state: "completed".into(),
            content_json: None,
            updated_at_ms: 3,
        },
    ] {
        assert!(store
            .commit_transition(&transition("overwrite", 2, vec![mutation]))
            .is_err());
    }
    let mut foreign = owner();
    foreign.run_id = "wrong".into();
    assert!(store.load_canonical_text_state(&foreign).is_err());
    assert!(store
        .commit_transition(&transition(
            "wrong-owner",
            2,
            vec![stream(CanonicalStreamOperation::Append {
                owner: foreign,
                expected_head: head,
                delta: "bad".into()
            })]
        ))
        .is_err());
    assert_eq!(body(&store), "first");
    assert_eq!(store.load_checkpoint("c").unwrap().unwrap().revision, 2);
}


#[test]
fn ownership_handoff_failure_rolls_back_membership_and_initial_part_is_not_guessed() {
    let mut store = NolaStore::open(":memory:").unwrap();
    begin(&mut store);
    terminal_sources::settle_fixture(&mut store, "c", "r", "r", "u");
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER fail_next_run BEFORE INSERT ON runs WHEN NEW.id='next-run'
        BEGIN SELECT RAISE(ABORT,'handoff failure'); END",
        )
        .unwrap();
    assert!(resume(&mut store).is_err());
    let (run,count):(String,i64)=store.connection.query_row(
        "SELECT run_id,(SELECT count(*) FROM canonical_message_run_owners) FROM messages WHERE id='a'",[],|r|Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert_eq!(run, "r");
    assert_eq!(count, 0);
    store
        .connection
        .execute_batch("DROP TRIGGER fail_next_run")
        .unwrap();
    resume(&mut store).unwrap();
    assert!(crate::canonical_message_ownership::has_trusted_resumption(
        &store.connection,
        "c",
        "a"
    )
    .unwrap());
    assert!(!crate::canonical_message_ownership::matches_read_owner(
        &store.connection,
        "c",
        "a",
        "foreign"
    )
    .unwrap());
    assert!(!crate::canonical_message_ownership::matches_read_owner(
        &store.connection,
        "foreign",
        "a",
        "r"
    )
    .unwrap());
    assert_eq!(
        store.active_run("c").unwrap().unwrap().assistant_part_id,
        None
    );
}

